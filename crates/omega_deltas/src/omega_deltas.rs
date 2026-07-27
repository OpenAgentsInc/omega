//! Mechanical checks for every deliberate Omega divergence from Zed.
//!
//! A fork accumulates silent divergence: someone changes a default, a rebase
//! quietly reverts it, and nobody notices until an owner sees upstream
//! behaviour again in a release candidate. A code comment does not survive
//! that, because a merge can drop the comment and the value together.
//!
//! Each delta in `OMEGA_DELTAS.md` has a test here that fails if the Omega
//! value reverts to the upstream one. The tests name the upstream value they
//! replace, so the diff stays legible to whoever is doing the rebase.

/// Repository-root-relative path of the shipped default settings.
pub const DEFAULT_SETTINGS_PATH: &str = "assets/settings/default.json";

/// The registry document every delta is recorded in.
pub const DELTA_REGISTRY_PATH: &str = "OMEGA_DELTAS.md";

/// Every delta ID enforced by a test in this crate.
///
/// Kept in sync in both directions by `the_registry_and_the_checks_agree`:
/// an ID here with no `### <ID>` heading in the registry fails, and a heading
/// in the registry with no ID here fails too. An earlier version of this
/// comment claimed a check that did not exist, which an adversarial review
/// caught.
///
/// Uniqueness is a separate check. Two lanes once allocated `0010` and `0011`
/// at the same time, so four entries shared two IDs and none of them could be
/// cited; `delta_ids_are_unique` fails on a repeat rather than letting the set
/// comparison above swallow it.
pub const ENFORCED_DELTAS: &[&str] = &[
    "OMEGA-DELTA-0001",
    "OMEGA-DELTA-0002",
    "OMEGA-DELTA-0003",
    "OMEGA-DELTA-0004",
    "OMEGA-DELTA-0005",
    "OMEGA-DELTA-0006",
    "OMEGA-DELTA-0007",
    "OMEGA-DELTA-0008",
    "OMEGA-DELTA-0009",
    "OMEGA-DELTA-0010",
    "OMEGA-DELTA-0011",
    "OMEGA-DELTA-0012",
    "OMEGA-DELTA-0013",
    "OMEGA-DELTA-0014",
    "OMEGA-DELTA-0015",
    "OMEGA-DELTA-0016",
    "OMEGA-DELTA-0017",
    "OMEGA-DELTA-0018",
    "OMEGA-DELTA-0019",
    "OMEGA-DELTA-0020",
    "OMEGA-DELTA-0021",
    "OMEGA-DELTA-0022",
    "OMEGA-DELTA-0023",
    "OMEGA-DELTA-0024",
    "OMEGA-DELTA-0025",
    "OMEGA-DELTA-0026",
    "OMEGA-DELTA-0027",
    "OMEGA-DELTA-0028",
    "OMEGA-DELTA-0029",
    "OMEGA-DELTA-0030",
    "OMEGA-DELTA-0031",
    "OMEGA-DELTA-0032",
    "OMEGA-DELTA-0033",
    "OMEGA-DELTA-0034",
    "OMEGA-DELTA-0035",
    "OMEGA-DELTA-0036",
    "OMEGA-DELTA-0037",
    "OMEGA-DELTA-0038",
    "OMEGA-DELTA-0039",
    "OMEGA-DELTA-0040",
    "OMEGA-DELTA-0041",
    "OMEGA-DELTA-0042",
    "OMEGA-DELTA-0043",
    "OMEGA-DELTA-0044",
    "OMEGA-DELTA-0045",
    "OMEGA-DELTA-0046",
    "OMEGA-DELTA-0047",
    "OMEGA-DELTA-0048",
    "OMEGA-DELTA-0049",
    "OMEGA-DELTA-0050",
    "OMEGA-DELTA-0051",
    "OMEGA-DELTA-0052",
    "OMEGA-DELTA-0053",
    "OMEGA-DELTA-0054",
    "OMEGA-DELTA-0055",
    "OMEGA-DELTA-0060",
    "OMEGA-DELTA-0061",
    "OMEGA-DELTA-0070",
    "OMEGA-DELTA-0080",
    "OMEGA-DELTA-0090",
    "OMEGA-DELTA-0091",
    "OMEGA-DELTA-0092",
    "OMEGA-DELTA-0093",
    "OMEGA-DELTA-0094",
    "OMEGA-DELTA-0095",
    "OMEGA-DELTA-0100",
    "OMEGA-DELTA-0102",
    "OMEGA-DELTA-0105",
];

/// OMEGA-DELTA-0094. The audience rules, which know nothing about a window.
pub const AUDIENCE_PATH: &str = "crates/omega_audience/src/omega_audience.rs";

/// OMEGA-DELTA-0094. The manifest that keeps Local reachable with nothing.
pub const AUDIENCE_MANIFEST_PATH: &str = "crates/omega_audience/Cargo.toml";

/// OMEGA-DELTA-0094. The composer control and the durable record.
pub const AUDIENCE_CONTROL_PATH: &str = "crates/agent_ui/src/omega_audience_control.rs";

/// OMEGA-DELTA-0094. Where a thread starts, and so where its audience is
/// recorded.
pub const CONVERSATION_VIEW_PATH: &str = "crates/agent_ui/src/conversation_view.rs";

/// OMEGA-DELTA-0105. The three sentences the audience menu says.
///
/// Written once, in `omega_audience`, beside the rule each describes. They are
/// the least verified thing in `OMEGA-DELTA-0094` — whether they read as a
/// deliberate control or as a broken one needs a window and a person who has
/// not read the source — so what is enforced is that changing them is one edit
/// rather than a hunt through a menu builder.
pub const AUDIENCE_MENU_SENTENCES: &[&str] = &[
    "New threads start in",
    "A thread keeps the audience it was started in.",
    "This thread is not in the selected audience.",
];

/// OMEGA-DELTA-0105. Section headers a manifest may declare dependencies under.
///
/// `local_needs_no_network_no_relay_and_no_account` reads `[dependencies]` and
/// stops at the next `[`, so `[dependencies.tokio]`, `[build-dependencies]`
/// and `[target.'cfg(unix)'.dependencies]` are all invisible to it. Rather than
/// teach it every spelling, the manifest is held to declaring dependencies in
/// exactly the two places that check can see.
pub const AUDIENCE_ALLOWED_MANIFEST_SECTIONS: &[&str] = &["[dependencies]", "[dev-dependencies]"];

/// OMEGA-DELTA-0094. Every crate `omega_audience` is allowed to depend on.
///
/// A closed list rather than a denylist of network crates, for the reason
/// `MEASURED_DIGEST_CONSTRUCTORS` gives: a denylist is a list of the ways
/// somebody already thought of, and "Local must never depend on a network, a
/// relay, or an account" has to survive a dependency nobody has heard of yet.
/// Serialisation is admitted because a record that does not survive a restart
/// is inferred after one.
pub const AUDIENCE_ALLOWED_DEPENDENCIES: &[&str] = &["serde"];
/// OMEGA-DELTA-0095. Where a detected coding agent becomes the router's
/// external ACP executor.
pub const DETECTED_EXECUTOR_ATTACH_PATH: &str = "crates/agent_ui/src/omega_agent_attach.rs";

/// OMEGA-DELTA-0095. The connect sequence a detected agent is attached
/// through.
///
/// The same three steps `agent_connection_store` uses when a person picks an
/// agent by hand — the store, a delegate over it, and the server's own
/// `connect`. `CustomAgentServer` is the bridge that makes this possible at
/// all: `AgentServerStore::get_external_agent` hands back a
/// `&mut dyn ExternalAgentServer`, which is a command source and not something
/// `connect` can take, and `CustomAgentServer` is the `AgentServer` that
/// resolves that borrow inside its own `connect`. Building a command here
/// instead would be a second, divergent copy of that resolution.
pub const DETECTED_EXECUTOR_CONNECT_STEPS: &[&str] = &[
    "CustomAgentServer::new(AgentId::new(agent.id))",
    "AgentServerDelegate::new(store, None, None)",
    "server.connect(delegate, project, cx)",
];

/// OMEGA-DELTA-0095. The agents the attach path will drive, named as
/// `agent_servers` names them.
pub const DETECTED_EXECUTOR_DRIVABLE_TOKENS: &[&str] =
    &["agent_servers::CODEX_ID", "agent_servers::CLAUDE_AGENT_ID"];

/// OMEGA-DELTA-0095. Settings keys that must declare a registry ACP server, so
/// a detected agent has something to be hosted by.
pub const DETECTED_EXECUTOR_SETTINGS_KEYS: &[&str] =
    &["agent_servers.codex-acp", "agent_servers.claude-acp"];

/// OMEGA-DELTA-0095, amended. What a registration failure must say it could not
/// reach.
///
/// Detection proves a binary is on `PATH`. What Omega actually spawns is a
/// *different* artifact — an ACP adapter resolved from the ACP registry
/// (`codex-acp` and `claude-acp` are both npx distributions there, not the
/// `codex` and `claude` binaries detection found). When the adapter cannot be
/// resolved, the reader's own installation is fine and Omega's supply chain is
/// not, so a message that leads with "`Codex` is installed at … but" and then
/// reports a failure is a false statement about the reader's machine. That is
/// the honest-attribution rule pointed the wrong way, and it is the rule this
/// whole surface exists to keep.
pub const DETECTED_EXECUTOR_REGISTRATION_FAILURE_PHRASES: &[&str] =
    &["ACP registry", "Omega retries"];

/// OMEGA-DELTA-0095, amended. Where a failed attach is re-driven.
///
/// The reason the attach is allowed to fail hard at all. See
/// `a_failed_attach_is_retried_when_the_adapter_registers`.
pub const CONNECTION_RETRY_PATH: &str = CONVERSATION_VIEW_PATH;

/// OMEGA-DELTA-0080. Where the agent panel declares its result-body ceiling and
/// applies it to every terminal it creates.
pub const TOOL_OUTPUT_CEILING_PATH: &str = "crates/agent_ui/src/entry_view_state.rs";

/// OMEGA-DELTA-0080. Where the thread draws the control that lifts the ceiling.
pub const TOOL_OUTPUT_CEILING_RENDER_PATH: &str =
    "crates/agent_ui/src/conversation_view/thread_view.rs";

/// OMEGA-DELTA-0080. Where a ceiling becomes a displayed line count.
pub const TOOL_OUTPUT_CEILING_TERMINAL_PATH: &str = "crates/terminal_view/src/terminal_view.rs";

/// OMEGA-DELTA-0080. The Omega ceiling on a tool call's result body, in lines.
pub const TOOL_OUTPUT_CEILING_LINES: usize = 16;

/// OMEGA-DELTA-0080. The only ceiling upstream Zed puts on the same body.
///
/// `TerminalView::MAX_EMBEDDED_LINES`. A result under it renders at its natural
/// height, so forty lines of JSON take forty lines of the window.
pub const UPSTREAM_TOOL_OUTPUT_CEILING_LINES: usize = 1_000;
/// OMEGA-DELTA-0061. Where a per-spawn executor request is resolved.
pub const SUBAGENT_EXECUTOR_PATH: &str = "crates/agent/src/tools/subagent_executor.rs";

/// OMEGA-DELTA-0061. The tool that accepts the request and reports what ran.
pub const SUBAGENT_SPAWN_TOOL_PATH: &str = "crates/agent/src/tools/spawn_agent_tool.rs";

/// OMEGA-DELTA-0061. Where an external ACP subagent is opened and driven.
pub const SUBAGENT_EXTERNAL_HANDLE_PATH: &str = "crates/agent/src/agent.rs";

/// OMEGA-DELTA-0061. Every outcome of resolving a requested executor.
///
/// A closed pair. The invariant is that there is no third answer meaning
/// "could not honour this, ran something else", so a new variant must fail by
/// existing rather than pass by not being on a denylist.
pub const SUBAGENT_EXECUTOR_OUTCOMES: &[&str] = &["Resolved", "Refused"];

/// OMEGA-DELTA-0060. The tool that reads a subagent's transcript.
pub const SUBAGENT_TRANSCRIPT_TOOL_PATH: &str =
    "crates/agent/src/tools/read_subagent_transcript_tool.rs";

/// OMEGA-DELTA-0060. Where the scoping decision is applied to a real thread.
pub const SUBAGENT_TRANSCRIPT_ENVIRONMENT_PATH: &str = "crates/agent/src/agent.rs";

/// OMEGA-DELTA-0060. Where the tool is registered and the environment trait
/// declares the read.
pub const SUBAGENT_TRANSCRIPT_REGISTRATION_PATH: &str = "crates/agent/src/thread.rs";

/// OMEGA-DELTA-0060. The tool name the model sees.
pub const SUBAGENT_TRANSCRIPT_TOOL_NAME: &str = "read_subagent_transcript";

/// OMEGA-DELTA-0060. Every refusal the scoping decision can produce.
///
/// A closed list, not a denylist. The invariant is "the access decision is
/// total", so a fifth outcome must fail by existing rather than pass by not
/// being on a list of the four somebody already thought of.
pub const SUBAGENT_TRANSCRIPT_ACCESS_VARIANTS: &[&str] = &[
    "Granted",
    "RefusedIsCaller",
    "RefusedNotASubagent",
    "RefusedOtherParent",
];

/// OMEGA-DELTA-0036. The uninstall script embedded in the shipped `cli`.
pub const UNINSTALL_SCRIPT_PATH: &str = "script/uninstall.sh";

/// OMEGA-DELTA-0036. Where the removal plan is derived from `paths::`.
pub const UNINSTALL_PLAN_PATH: &str = "crates/cli/src/uninstall.rs";

/// OMEGA-DELTA-0043, OMEGA-DELTA-0044. The CLI binary: it derives the
/// uninstall plan and renders the open-behavior prompt.
pub const CLI_MAIN_PATH: &str = "crates/cli/src/main.rs";

/// OMEGA-DELTA-0037. Outbound attribution on OpenRouter requests.
pub const OPEN_ROUTER_PATH: &str = "crates/open_router/src/open_router.rs";

/// OMEGA-DELTA-0039. The installed-proof secret tripwire collector.
pub const INSTALLED_TRIPWIRE_PATH: &str = "script/collect-omega-installed-tripwires";

/// OMEGA-DELTA-0039. The installed-proof observation collector.
pub const INSTALLED_OBSERVATION_PATH: &str = "script/collect-omega-installed-observations";

/// OMEGA-DELTA-0025. The file that declares the measured digest.
pub const MEASURED_DIGEST_PATH: &str = "crates/omega_harness/src/measured.rs";

/// OMEGA-DELTA-0025. Every admitted way into `MeasuredDigest`.
///
/// A closed list, not a denylist. The invariant is "there is no path from a
/// string into this type", and a denylist of `From<String>`, `FromStr`,
/// `Deserialize` and `new` would be a list of the four ways somebody already
/// thought of. Naming the admitted constructors instead makes a fifth one fail
/// by existing.
pub const MEASURED_DIGEST_CONSTRUCTORS: &[&str] = &["measure", "measure_tree"];

/// OMEGA-DELTA-0025. The filesystem half of harness maintenance.
pub const HARNESS_MAINTENANCE_PATH: &str = "crates/project/src/harness_maintenance.rs";

/// OMEGA-DELTA-0025. The launch path the provenance gate sits in.
pub const AGENT_SERVER_STORE_PATH: &str = "crates/project/src/agent_server_store.rs";

/// OMEGA-DELTA-0033. The decision layer for what the owner's front door shows.
pub const HARNESS_FRONT_DOOR_PATH: &str = "crates/omega_harness/src/front_door.rs";

/// OMEGA-DELTA-0033. The page that renders it.
pub const EXTERNAL_AGENTS_PAGE_PATH: &str = "crates/settings_ui/src/pages/external_agents_page.rs";

/// OMEGA-DELTA-0026. Shipped defaults that would otherwise point a running
/// Omega at one of Zed's production hosts, as
/// `(dotted key, upstream JSON, Omega JSON)`.
///
/// The values are written as JSON text rather than typed constants because they
/// are not all the same type — a URL, a boolean, an enum string and an object —
/// and a table that can hold all four is what makes this one check rather than
/// four.
pub const SERVICE_ISOLATION_DEFAULTS: &[(&str, &str, &str)] = &[
    (
        "server_url",
        "\"https://zed.dev\"",
        "\"https://services.openagents.invalid\"",
    ),
    ("auto_update", "true", "false"),
    ("edit_predictions.provider", "\"zed\"", "\"none\""),
    ("auto_install_extensions", "{\"html\": true}", "{}"),
];

/// The service-isolation test the registry cites for `OMEGA-DELTA-0026` and
/// `OMEGA-DELTA-0027`.
pub const SERVICE_ISOLATION_TEST_PATH: &str = "crates/app_identity/src/service_isolation.rs";

/// Assertions in `SERVICE_ISOLATION_TEST_PATH` that two delta entries cite.
///
/// Citing an existing check instead of duplicating it is the right call, and it
/// creates a new way to fail: the cited assertion can be deleted. `auto_update`
/// and `auto_install_extensions` read as off-topic inside a test named for Zed
/// service isolation, so they are the first lines a tidy-up drops — and nothing
/// would notice, because deleting an assertion turns a test green.
///
/// Matched with whitespace removed, so rustfmt rewrapping a long `assert_eq!`
/// does not read as somebody deleting it.
pub const PINNED_SERVICE_ISOLATION_ASSERTIONS: &[(&str, &str)] = &[
    (
        "OMEGA-DELTA-0026",
        "assert_eq!(settings[\"server_url\"], \"https://services.openagents.invalid\");",
    ),
    (
        "OMEGA-DELTA-0026",
        "assert_eq!(settings[\"auto_update\"], false);",
    ),
    (
        "OMEGA-DELTA-0026",
        "assert_eq!(settings[\"edit_predictions\"][\"provider\"], \"none\");",
    ),
    (
        "OMEGA-DELTA-0026",
        "assert_eq!(settings[\"auto_install_extensions\"], serde_json::json!({}));",
    ),
    (
        "OMEGA-DELTA-0027",
        "assert_eq!(settings[\"agent_servers\"][\"codex-acp\"][\"type\"], \"registry\");",
    ),
];

/// OMEGA-DELTA-0016. Every shipped settings file that names a theme.
///
/// `default.json` is the base settings layer. `initial_user_settings.json` is
/// the template copied verbatim into a new user's own settings file on first
/// start, where it becomes a user-layer value that overrides the base layer for
/// good — so a revert there survives being corrected in `default.json` later,
/// and is the more durable of the two.
pub const SHIPPED_THEME_SETTINGS_FILES: &[&str] = &[
    DEFAULT_SETTINGS_PATH,
    "assets/settings/initial_user_settings.json",
];

/// OMEGA-DELTA-0028. The file that declares the built-in icon theme's name.
///
/// The shipped `icon_theme` setting has to name the same theme, or the lookup
/// in `configured_icon_theme` misses and falls back with a logged error.
pub const DEFAULT_ICON_THEME_SOURCE: &str = "crates/theme/src/icon_theme.rs";

/// OMEGA-DELTA-0021. The file that holds the executor-disclosure record.
pub const EXECUTOR_DISCLOSURE_RECORD_PATH: &str = "crates/omega_front_door/src/omega_front_door.rs";

/// OMEGA-DELTA-0021. The file that binds the record to a live thread.
pub const EXECUTOR_DISCLOSURE_BINDING_PATH: &str =
    "crates/agent_ui/src/omega_executor_disclosure.rs";

/// OMEGA-DELTA-0021. The thread surface that has to render the line.
pub const THREAD_VIEW_PATH: &str = "crates/agent_ui/src/conversation_view/thread_view.rs";

/// OMEGA-DELTA-0045. The host method the engine calls to disclose a handoff.
pub const HOST_BRIDGE_PATH: &str = "crates/agent_ui/src/omega_host_bridge.rs";

/// OMEGA-DELTA-0045. The shared thread crate that carries the entry kinds.
pub const THREAD_ENTRY_PATH: &str = "crates/acp_thread/src/acp_thread.rs";

/// OMEGA-DELTA-0045. The refusal that shipped in every candidate through
/// `0.2.0-rc17`.
///
/// Named as a literal so the check fails on the exact bytes an independent
/// reviewer found in the shipped binary, rather than on a paraphrase. The
/// prose in `omega_host_bridge.rs` still quotes it in order to say why it is
/// gone, so the scan reads `code_of` and not the raw file.
pub const SYSTEM_NOTE_REFUSAL: &str =
    "Agent threads do not expose an owner-visible system-note authority.";

/// OMEGA-DELTA-0046. The native Metal proof for the Exo workspace.
pub const VISUAL_TEST_RUNNER_PATH: &str = "crates/zed/src/visual_test_runner.rs";

/// OMEGA-DELTA-0030. The typed start command sent to `omega-effectd`.
pub const FULL_AUTO_DISPATCH_PATH: &str = "crates/full_auto_ui/src/dispatch.rs";

/// OMEGA-DELTA-0030. The thread's projection of its linked engine run.
pub const THREAD_RUN_LINK_PATH: &str = "crates/full_auto_ui/src/thread_run_link.rs";

/// OMEGA-DELTA-0030. Every field a start request is allowed to carry.
///
/// A closed list for the same reason as `EXECUTOR_DISCLOSURE_FIELDS`: the
/// property that matters is "these parts and nothing else". A start request
/// that could name an `evidence` block, a `decision_ref`, or an
/// `authority_receipt_ref` would let a requester forge the very records the
/// host mints at the completion-admission gate, and omega#47 watched a live
/// engine ignore exactly that forgery. Here it cannot be written at all.
pub const FULL_AUTO_DISPATCH_FIELDS: &[(&str, &str)] = &[
    ("origin", "LaunchOrigin"),
    ("workspace_ref", "PublicRef"),
    ("project_ref", "PublicRef"),
    ("worktree_ref", "PublicRef"),
    ("lane", "String"),
    ("title", "String"),
    ("objective", "String"),
    ("done_condition", "String"),
    ("turn_cap", "u32"),
];

/// OMEGA-DELTA-0030. Crates allowed to name the Full Auto start command.
///
/// A dispatch is reachable only from the Full Auto surface and the panel that
/// hosts it. A tool crate, a language-model crate, or a context-server crate
/// appearing here would mean a model-authored path can reach run authority,
/// which owner gate 8 forbids.
///
/// `omega_deltas` is listed because this file names the symbol in order to
/// check for it. It declares no dependency on `full_auto_ui`, so naming the
/// type here cannot build one.
pub const FULL_AUTO_DISPATCH_CALLERS: &[&str] = &["full_auto_ui", "omega_deltas"];

/// OMEGA-DELTA-0021. Every field the disclosure record is allowed to hold.
///
/// A closed list, not a forbidden-substring scan. The owner's condition on
/// omega#74 is that disclosure is *a record a label renders*, so the check that
/// matters is "these parts and nothing else" — a `line`, `text`, `summary` or
/// `display` field would be a rendered label under a name no denylist thought
/// of. Adding a genuine new part is a one-line edit here, and that edit is the
/// record that somebody decided it was a part rather than a caption.
pub const EXECUTOR_DISCLOSURE_FIELDS: &[(&str, &str)] = &[
    ("class", "ExecutorClass"),
    ("agent_id", "String"),
    ("provider", "Option<String>"),
    ("model", "Option<String>"),
    ("run_ref", "Option<String>"),
    // OMEGA-DELTA-0029. Why the router sent the thread here. A typed reason
    // from a closed set, not a sentence: the same law as every other part.
    ("route", "Option<RouteReason>"),
];

/// OMEGA-DELTA-0029. The routing law.
pub const ROUTE_DECISION_PATH: &str = "crates/omega_front_door/src/router.rs";

/// OMEGA-DELTA-0029. The dispatch half of the router.
pub const ROUTER_DISPATCH_PATH: &str = "crates/agent_ui/src/omega_router.rs";

/// OMEGA-DELTA-0034. The panel that owns the front door.
pub const AGENT_PANEL_PATH: &str = "crates/agent_ui/src/agent_panel.rs";

/// OMEGA-DELTA-0034. Front-door entry points that must work with no project.
///
/// Each of these stood between a fresh install and a composer. The check is on
/// the *function body*, not the file, because a `has_open_project` line
/// elsewhere in a six-thousand-line panel is fine and one of these is not.
pub const PROJECT_OPTIONAL_FRONT_DOOR_FNS: &[&str] = &[
    "activate_new_thread",
    "activate_draft",
    "new_thread",
    "ensure_native_agent_connection",
    "toggle_new_thread_menu",
];

/// OMEGA-DELTA-0034. Paths that must keep requiring a project.
///
/// Removing the guard from these would not be project-optional threads; it
/// would be threads that fail later and less legibly. Asserted in the opposite
/// direction from `PROJECT_OPTIONAL_FRONT_DOOR_FNS` so a blanket deletion of
/// every guard fails as loudly as restoring one.
pub const PROJECT_REQUIRED_FNS: &[&str] = &[
    "restore_new_draft",
    "new_external_agent_thread",
    "refresh_skills",
    "load_thread_from_clipboard",
    "initialize_from_source_workspace_if_needed",
    // Not `should_create_terminal_for_new_entry`: it asks `supports_terminal`,
    // which is where the requirement lives. `project.supports_terminal` is
    // `true` for any local project, worktree or not, so the check has to be on
    // the panel's wrapper.
    "supports_terminal",
];

/// OMEGA-DELTA-0035. Where the native agent entry is built.
pub const AGENT_SERVER_FACTORY_PATH: &str = "crates/agent_ui/src/agent_ui.rs";

/// OMEGA-DELTA-0035. The pin-setting methods that must name a human gesture.
pub const PIN_SETTING_CALLS: &[&str] = &["pin_session(", "unpin_session(", "pin_next_session("];

/// OMEGA-DELTA-0013. The chord that opens a new agent thread, per platform.
///
/// omega#76's exit is that this chord reaches the New Agent Thread surface from
/// the editor, the welcome surface and the panel. All three live inside
/// `Workspace`, so the property is "bound window-globally and not shadowed by
/// anything narrower", which is what `the_new_thread_chord_is_window_global`
/// checks.
pub const NEW_THREAD_CHORDS: &[(&str, &str)] = &[
    ("assets/keymaps/default-macos.json", "cmd-shift-a"),
    ("assets/keymaps/default-linux.json", "ctrl-shift-a"),
    ("assets/keymaps/default-windows.json", "ctrl-shift-a"),
];

/// OMEGA-DELTA-0013. The narrower surfaces admitted to take the chord back.
///
/// An allowlist and not a count. omega#76 asked for the shadowed lower-priority
/// bindings to be *resolved deliberately*, and the deliberate resolution is
/// that a modal the user opened on purpose, and a terminal whose select-all is
/// a decades-old convention, may keep the chord while they have focus — but
/// nothing else may take it, and an `Editor` or `AgentPanel` binding appearing
/// here would put the chord back to being focus-dependent everywhere.
pub const NEW_THREAD_CHORD_NARROW_CONTEXTS: &[&str] = &[
    "ToolchainSelector",
    "RecentProjects || (RecentProjects > Picker > Editor)",
    "Terminal",
];

/// OMEGA-DELTA-0041. The crate that owns the served ACP socket.
pub const ACP_SERVER_PATH: &str = "crates/omega_acp_server/src/omega_acp_server.rs";

/// OMEGA-DELTA-0041. That crate's manifest, which must not reach GPUI.
pub const ACP_SERVER_MANIFEST_PATH: &str = "crates/omega_acp_server/Cargo.toml";

/// OMEGA-DELTA-0041. The supervisor layer that owns the listener's lifecycle.
pub const EFFECTD_PATH: &str = "crates/omega_effectd/src/omega_effectd.rs";

/// OMEGA-DELTA-0041. Where the first-party agent identity is declared.
pub const NATIVE_AGENT_IDENTITY_PATH: &str = "crates/agent/src/agent.rs";

/// OMEGA-DELTA-0040. The startup path that opens Omega's first window.
pub const STARTUP_PATH: &str = "crates/zed/src/main.rs";

/// OMEGA-DELTA-0040. First-run onboarding, which that startup path waits on.
pub const ONBOARDING_PATH: &str = "crates/onboarding/src/onboarding.rs";

/// OMEGA-DELTA-0040. The coordinator that releases the startup path.
pub const IDENTITY_STARTUP_PATH: &str = "crates/onboarding/src/identity_startup.rs";

/// OMEGA-DELTA-0029. Vocabulary that would make a route irreproducible.
///
/// The packet's exit is a *deterministic* router, and determinism is not a
/// property a unit test can establish on its own: a test can only show that the
/// inputs it happened to try gave the same answer twice. This scan covers the
/// ways the answer could stop depending only on its inputs at all, so a later
/// edit that reaches for a clock or a hash map fails here rather than making
/// two identical threads route differently in a way nobody reproduces.
pub const NON_DETERMINISTIC_ROUTING_TOKENS: &[(&str, &str)] = &[
    ("a clock", "SystemTime"),
    ("a clock", "Instant"),
    ("a clock", "::now("),
    ("a clock", "chrono"),
    ("a clock", "timestamp"),
    ("randomness", "rand::"),
    ("hash iteration order", "HashMap"),
    ("hash iteration order", "HashSet"),
    ("the environment", "std::env"),
];

/// OMEGA-DELTA-0029. Vocabulary that would mean the router started executing.
///
/// omega#74 admitted Omega Agent as a router that owns routing, disclosure and
/// receipts and owns **no execution**. "Owns no execution" is a property of the
/// source, not of its author's intent, so it is read off the source. The last
/// four entries are owner gate 8: the router must not be able to start, stop,
/// or resume run authority, because only an explicit human action may.
pub const ROUTER_EXECUTION_TOKENS: &[&str] = &[
    "LanguageModel",
    "stream_completion",
    "system_prompt",
    "run_turn",
    "start_run",
    "stop_run",
    "resume_run",
    "LaunchOrigin",
];

/// OMEGA-DELTA-0029. Methods of the router's `AgentConnection` impl that are
/// allowed not to delegate, and why.
///
/// `into_any` is the trait's own downcast escape hatch: it hands back the
/// router itself by definition, so requiring it to delegate would require it to
/// lie about what it is.
pub const ROUTER_NON_DELEGATING_METHODS: &[&str] = &["into_any"];

/// OMEGA-DELTA-0029. How a router method is allowed to reach an executor.
pub const ROUTER_DELEGATION_MARKERS: &[&str] = &[
    "self.native",
    "self.executor(",
    "self.executor_for(",
    "self.agent_id",
    "executor.new_session",
];

/// Files deleted from the fork, checked by absence.
///
/// A deletion is the easiest kind of delta for a rebase to undo, because the
/// file simply reappears and nothing else references it yet.
pub const REMOVED_FILES: &[&str] = &[
    // OMEGA-DELTA-0005
    "crates/ai_onboarding/src/plan_definitions.rs",
    "crates/ai_onboarding/src/young_account_banner.rs",
    "crates/agent_ui/src/ui/end_trial_upsell.rs",
    // OMEGA-DELTA-0006
    "crates/extensions_ui/src/extension_suggest.rs",
    "crates/recent_projects/src/dev_container_suggest.rs",
    "crates/zed/src/zed/move_to_applications.rs",
    // OMEGA-DELTA-0009
    "crates/workspace/src/security_modal.rs",
    // OMEGA-DELTA-0012
    "crates/collab_ui/src/collab_panel.rs",
    "crates/collab/Cargo.toml",
];

/// Strings that must not appear anywhere under `crates/`.
///
/// These are checked across the whole source tree rather than in one file,
/// because the last two both survived a source-level review and were caught
/// only by scanning the packaged binary.
/// Action namespaces whose crates Omega deleted.
///
/// A keybinding naming one of these is not a cosmetic leftover: the built-in
/// keymap is loaded and unwrapped at startup, so an unresolvable action is a
/// hard panic before any window opens. `cargo check --workspace` passes
/// happily, because keymaps are runtime assets rather than compiled code.
/// This was shipped once, in 0.2.0-rc6.
pub const FORBIDDEN_KEYMAP_NAMESPACES: &[(&str, &str)] = &[
    ("OMEGA-DELTA-0012", "collab_panel::"),
    ("OMEGA-DELTA-0012", "channel_modal::"),
];

/// A keybinding Omega adds, checked by presence, scope, and resolvability.
///
/// The mirror image of `FORBIDDEN_KEYMAP_NAMESPACES`: that table catches
/// bindings whose action was deleted, this one catches an added binding being
/// dropped, rescoped, or pointed at an action that no longer exists.
pub struct RequiredKeymapBinding {
    /// Delta this binding belongs to.
    pub delta: &'static str,
    /// Keymap asset the binding must appear in.
    pub keymap: &'static str,
    /// Keystroke, exactly as written in the keymap.
    pub keystroke: &'static str,
    /// Fully qualified action name the keystroke must dispatch.
    pub action: &'static str,
    /// Source file whose `actions!` declaration must still define `action`.
    ///
    /// A keymap naming an undeclared action is a hard panic at startup, not a
    /// compile error, so the binding is resolved back to its declaration here.
    pub declared_in: &'static str,
}

/// Contexts that match from anywhere inside a window.
///
/// `Workspace` is the root context of the window tree, so a binding declared
/// there fires from an editor, a terminal, or any panel — the same scope
/// `workspace::Save` and `agent::NewThread` use. A section with no context at
/// all is equally global and is also accepted. Anything narrower (`Editor`,
/// `Terminal`, `Pane`) would make the binding depend on focus, which is what
/// `REQUIRED_KEYMAP_BINDINGS` exists to forbid.
pub const WINDOW_GLOBAL_KEYMAP_CONTEXTS: &[&str] = &["Workspace"];

/// OMEGA-DELTA-0015. Opening the Sarah workroom must not depend on focus, so
/// each of these is asserted to live in a window-global section.
///
/// Each of these chords was `workspace::SaveAs` upstream. Taking them is a
/// real trade, and `SAVE_AS_MENU_ITEM` holds the mitigation in place.
pub const REQUIRED_KEYMAP_BINDINGS: &[RequiredKeymapBinding] = &[
    RequiredKeymapBinding {
        delta: "OMEGA-DELTA-0015",
        keymap: "assets/keymaps/default-macos.json",
        keystroke: "cmd-shift-s",
        action: "workroom::OpenPanel",
        declared_in: "crates/zed_actions/src/lib.rs",
    },
    // OMEGA-DELTA-0034. `cmd-?` is macOS's reserved Help chord, so the agent
    // panel's toggle was bound to a keystroke Omega cannot win. Checked here
    // rather than only moved, because a rebase that restores upstream's
    // `cmd-?` line would otherwise be invisible.
    RequiredKeymapBinding {
        delta: "OMEGA-DELTA-0034",
        keymap: "assets/keymaps/default-macos.json",
        keystroke: "ctrl-cmd-a",
        action: "agent::ToggleFocus",
        declared_in: "crates/zed_actions/src/lib.rs",
    },
    RequiredKeymapBinding {
        delta: "OMEGA-DELTA-0015",
        keymap: "assets/keymaps/default-linux.json",
        keystroke: "ctrl-shift-s",
        action: "workroom::OpenPanel",
        declared_in: "crates/zed_actions/src/lib.rs",
    },
    RequiredKeymapBinding {
        delta: "OMEGA-DELTA-0015",
        keymap: "assets/keymaps/default-windows.json",
        keystroke: "ctrl-shift-s",
        action: "workroom::OpenPanel",
        declared_in: "crates/zed_actions/src/lib.rs",
    },
];

/// OMEGA-DELTA-0015. Where Save As went when the workroom took its chord.
///
/// `cmd-shift-s` / `ctrl-shift-s` was `workspace::SaveAs` in all three default
/// keymaps, so macOS and Windows now have no Save As keystroke at all and
/// Linux keeps only the `shift-save` media key. The File menu is the whole
/// mitigation. If it goes, Save As is reachable only by knowing its command
/// name, and this delta is recording a fallback that no longer exists.
pub const SAVE_AS_MENU_ITEM: (&str, &str) = (
    "crates/zed/src/zed/app_menus.rs",
    "MenuItem::action(\"Save As…\", workspace::SaveAs)",
);

pub const FORBIDDEN_SOURCE_STRINGS: &[(&str, &str)] = &[
    ("OMEGA-DELTA-0008", "Zed\u{27}s hosted models"),
    ("OMEGA-DELTA-0008", "14 day free trial"),
    ("OMEGA-DELTA-0009", "Review .zed/settings.json"),
];

/// Restricted Mode UI and key bindings removed by OMEGA-DELTA-0009.
pub const FORBIDDEN_RESTRICTED_MODE_UI: &[(&str, &str)] = &[
    ("crates/agent_ui/src/profile_selector.rs", "Restricted Mode"),
    ("crates/language_tools/src/lsp_button.rs", "Restricted Mode"),
    (
        "crates/workspace/src/workspace.rs",
        "ToggleWorktreeSecurity",
    ),
    (
        "assets/keymaps/default-linux.json",
        "workspace::ToggleWorktreeSecurity",
    ),
    (
        "assets/keymaps/default-macos.json",
        "workspace::ToggleWorktreeSecurity",
    ),
    (
        "assets/keymaps/default-windows.json",
        "workspace::ToggleWorktreeSecurity",
    ),
];

/// Read a repository file relative to the workspace root.
///
/// `CARGO_MANIFEST_DIR` is `crates/omega_deltas`, so the root is two levels up.
/// Resolving it this way keeps the checks runnable from any working directory.
#[must_use]
pub fn repository_path(relative: &str) -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(relative)
}

/// Parse the shipped default settings, which are JSONC: `//` comments and
/// trailing commas, neither of which `serde_json` accepts.
///
/// The normalizer is string-aware on purpose. A naive strip would corrupt any
/// setting whose value legitimately contains `//` (a URL) or a comma before a
/// brace, and would do it silently — producing a parse that succeeds with the
/// wrong values, which is worse than failing.
///
/// # Errors
///
/// Returns an error when the file cannot be read or does not parse.
pub fn default_settings() -> Result<serde_json::Value, String> {
    let path = repository_path(DEFAULT_SETTINGS_PATH);
    let raw = std::fs::read_to_string(&path)
        .map_err(|error| format!("cannot read {}: {error}", path.display()))?;
    serde_json::from_str(&strip_jsonc(&raw))
        .map_err(|error| format!("cannot parse {}: {error}", path.display()))
}

/// Remove `//` comments and trailing commas without touching string contents.
///
/// Two passes, because a trailing comma may be separated from its closing
/// brace by a comment. Removing comments first makes the comma visible.
#[must_use]
pub fn strip_jsonc(source: &str) -> String {
    strip_trailing_commas(&strip_line_comments(source))
}

/// Scan `source`, applying `visit` to each character outside a string literal.
///
/// Shared so both passes agree on what "inside a string" means; two separate
/// scanners would be two chances to disagree.
fn scan_outside_strings(
    source: &str,
    mut visit: impl FnMut(&[char], usize, &mut String) -> usize,
) -> String {
    let characters: Vec<char> = source.chars().collect();
    let mut output = String::with_capacity(source.len());
    let mut index = 0;
    let mut in_string = false;

    while index < characters.len() {
        let character = characters[index];
        if in_string {
            output.push(character);
            if character == '\\' && index + 1 < characters.len() {
                output.push(characters[index + 1]);
                index += 2;
                continue;
            }
            if character == '"' {
                in_string = false;
            }
            index += 1;
            continue;
        }
        if character == '"' {
            in_string = true;
            output.push(character);
            index += 1;
            continue;
        }
        index = visit(&characters, index, &mut output);
    }
    output
}

fn strip_line_comments(source: &str) -> String {
    scan_outside_strings(source, |characters, index, output| {
        if characters[index] == '/' && characters.get(index + 1) == Some(&'/') {
            let mut cursor = index;
            while cursor < characters.len() && characters[cursor] != '\n' {
                cursor += 1;
            }
            return cursor;
        }
        output.push(characters[index]);
        index + 1
    })
}

fn strip_trailing_commas(source: &str) -> String {
    scan_outside_strings(source, |characters, index, output| {
        if characters[index] == ',' {
            let mut lookahead = index + 1;
            while lookahead < characters.len() && characters[lookahead].is_whitespace() {
                lookahead += 1;
            }
            if matches!(characters.get(lookahead), Some('}') | Some(']')) {
                return index + 1;
            }
        }
        output.push(characters[index]);
        index + 1
    })
}

/// Look up a dotted path in the default settings.
#[must_use]
pub fn default_setting<'a>(
    settings: &'a serde_json::Value,
    dotted: &str,
) -> Option<&'a serde_json::Value> {
    let mut cursor = settings;
    for segment in dotted.split('.') {
        cursor = cursor.get(segment)?;
    }
    Some(cursor)
}

/// Every action declared by an `actions!(namespace, [..])` macro in `source`,
/// as fully qualified `namespace::Name` strings.
///
/// Textual rather than compiled on purpose: `omega_deltas` deliberately
/// depends on nothing but `serde_json`, so that a check cannot be made to pass
/// by a change to the crate it is checking. Doc comments are stripped before
/// parsing, because a comma inside one would otherwise split an entry.
#[must_use]
pub fn declared_actions(source: &str) -> std::collections::BTreeSet<String> {
    let stripped: String = source
        .lines()
        // Attribute lines are dropped as well as comments. An `#[action(...)]`
        // attribute may carry a bracketed list — `deprecated_aliases = [..]` —
        // and the `]` that closes it would otherwise end the `actions!` body
        // early, hiding every action declared after it. `agent::ToggleFocus` was
        // exactly that: declared, and invisible to this parser.
        .filter(|line| {
            let line = line.trim_start();
            !line.starts_with("//") && !line.starts_with("#[")
        })
        .collect::<Vec<_>>()
        .join("\n");

    let mut declared = std::collections::BTreeSet::new();
    let mut rest = stripped.as_str();
    while let Some(start) = rest.find("actions!(") {
        rest = &rest[start + "actions!(".len()..];
        let Some(close) = rest.find(']') else {
            break;
        };
        let (body, remainder) = rest.split_at(close);
        rest = remainder;
        let Some((namespace, names)) = body.split_once('[') else {
            continue;
        };
        let namespace = namespace.trim().trim_end_matches(',').trim();
        if namespace.is_empty() {
            continue;
        }
        for name in names.split(',').map(str::trim) {
            // Guard against a stray fragment being read as an action name.
            let is_identifier = name.starts_with(char::is_uppercase)
                && name.chars().all(|c| c.is_alphanumeric() || c == '_');
            if is_identifier {
                declared.insert(format!("{namespace}::{name}"));
            }
        }
    }
    declared
}

/// Every theme name declared by the shipped theme families under
/// `assets/themes/`.
///
/// # Errors
///
/// Returns an error when a shipped theme file cannot be read or parsed. A
/// silent skip would make the default-theme check vacuous.
pub fn shipped_theme_names() -> Result<std::collections::BTreeSet<String>, String> {
    let root = repository_path("assets/themes");
    let families = std::fs::read_dir(&root)
        .map_err(|error| format!("cannot read {}: {error}", root.display()))?;
    let mut names = std::collections::BTreeSet::new();
    for family in families.flatten() {
        let Ok(entries) = std::fs::read_dir(family.path()) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().is_none_or(|extension| extension != "json") {
                continue;
            }
            let raw = std::fs::read_to_string(&path)
                .map_err(|error| format!("cannot read {}: {error}", path.display()))?;
            let value: serde_json::Value = serde_json::from_str(&strip_jsonc(&raw))
                .map_err(|error| format!("cannot parse {}: {error}", path.display()))?;
            let themes = value
                .get("themes")
                .and_then(serde_json::Value::as_array)
                .ok_or_else(|| format!("{} has no themes array", path.display()))?;
            for theme in themes {
                let name = theme
                    .get("name")
                    .and_then(serde_json::Value::as_str)
                    .ok_or_else(|| format!("{} has an unnamed theme", path.display()))?;
                names.insert(name.to_owned());
            }
        }
    }
    Ok(names)
}

/// Shared brand-gate policy.
///
/// OMEGA-DELTA-0017 and OMEGA-DELTA-0018. The same file is read by the tests
/// here, which check the source tree, and by `script/verify-omega-brand`,
/// which checks the packaged application. One file so the two sides cannot
/// come to disagree about what counts as a competitor's name.
pub const BRAND_GATE_POLICY_PATH: &str = "script/omega-brand-gate.json";

/// The packaged-side brand gate.
pub const BRAND_VERIFIER_PATH: &str = "script/verify-omega-brand";

/// The RC packaging entry point the brand gate has to be wired into.
pub const RC_BUNDLE_SCRIPT_PATH: &str = "script/bundle-omega-rc";

/// Parse the shared brand-gate policy.
///
/// # Errors
/// If the policy file is unreadable or is not valid JSON.
pub fn brand_policy() -> Result<serde_json::Value, String> {
    let path = repository_path(BRAND_GATE_POLICY_PATH);
    let raw = std::fs::read_to_string(&path)
        .map_err(|error| format!("cannot read {}: {error}", path.display()))?;
    serde_json::from_str(&raw).map_err(|error| format!("cannot parse {}: {error}", path.display()))
}

fn policy_strings(policy: &serde_json::Value, first: &str, second: &str) -> Vec<String> {
    policy
        .get(first)
        .and_then(|value| value.get(second))
        .and_then(serde_json::Value::as_array)
        .map(|values| {
            values
                .iter()
                .filter_map(serde_json::Value::as_str)
                .map(str::to_owned)
                .collect()
        })
        .unwrap_or_default()
}

/// Competitor brand names appearing in `text`.
///
/// `brand.words` match case-sensitively at ASCII alphanumeric boundaries, so
/// `Zed` and `Zed's` match while `Zedd` does not. Lowercase `zed` is
/// deliberately not a word: it is a substring of `authorized`, `organized` and
/// `normalized`, and a gate that cries wolf gets deleted rather than fixed.
/// `brand.substrings` match case-insensitively.
#[must_use]
pub fn brand_hits(text: &str, policy: &serde_json::Value) -> Vec<String> {
    let mut hits = Vec::new();
    for word in policy_strings(policy, "brand", "words") {
        if text.match_indices(&word).any(|(at, _)| {
            let before = text[..at].chars().next_back();
            let after = text[at + word.len()..].chars().next();
            !before.is_some_and(|character| character.is_ascii_alphanumeric())
                && !after.is_some_and(|character| character.is_ascii_alphanumeric())
        }) {
            hits.push(word);
        }
    }
    let lowered = text.to_lowercase();
    for substring in policy_strings(policy, "brand", "substrings") {
        if lowered.contains(&substring.to_lowercase()) {
            hits.push(substring);
        }
    }
    hits
}

/// Every `<string>` in an `Info.plist` fragment, paired with the `<key>` that
/// most recently preceded it.
///
/// Values, not a list of known keys: a brand-new key carrying a competitor's
/// name has to fail the same as `NSMicrophoneUsageDescription` does.
#[must_use]
pub fn plist_fragment_values(source: &str) -> Vec<(String, String)> {
    let mut values = Vec::new();
    let mut key = "<unkeyed>".to_owned();
    let mut rest = source;
    loop {
        let next_key = next_element(rest, "key");
        let next_string = next_element(rest, "string");
        match (next_key, next_string) {
            (Some((_, content, after)), None) => {
                key = content.trim().to_owned();
                rest = &rest[after..];
            }
            (None, Some((_, content, after))) => {
                values.push((key.clone(), content.to_owned()));
                rest = &rest[after..];
            }
            (
                Some((key_at, key_content, key_after)),
                Some((string_at, string_content, string_after)),
            ) => {
                if key_at < string_at {
                    key = key_content.trim().to_owned();
                    rest = &rest[key_after..];
                } else {
                    values.push((key.clone(), string_content.to_owned()));
                    rest = &rest[string_after..];
                }
            }
            (None, None) => break,
        }
    }
    values
}

/// `(start of the opening tag, content, offset just past the closing tag)`.
fn next_element<'a>(source: &'a str, tag: &str) -> Option<(usize, &'a str, usize)> {
    let open = format!("<{tag}>");
    let close = format!("</{tag}>");
    let start = source.find(&open)?;
    let content_start = start + open.len();
    let end = source[content_start..].find(&close)? + content_start;
    Some((start, &source[content_start..end], end + close.len()))
}

/// `source` with every whitespace character removed.
///
/// Used to match a pinned assertion against a source file without asserting
/// rustfmt's current line wrapping, which is not the thing being protected.
#[must_use]
pub fn without_whitespace(source: &str) -> String {
    source
        .chars()
        .filter(|character| !character.is_whitespace())
        .collect()
}

/// `source` with every `<tag>…</tag>` block removed.
#[must_use]
pub fn without_elements(source: &str, tag: &str) -> String {
    let mut remaining = source;
    let mut out = String::new();
    while let Some((start, _, after)) = next_element(remaining, tag) {
        out.push_str(&remaining[..start]);
        remaining = &remaining[after..];
    }
    out.push_str(remaining);
    out
}

/// Lowercase hex SHA-256 of `bytes`, the form the pins are written in.
#[must_use]
pub fn sha256_hex(bytes: &[u8]) -> String {
    use sha2::Digest as _;
    let mut hasher = sha2::Sha256::new();
    hasher.update(bytes);
    hasher
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

/// `OmegaAgentTwo` -> `omega_agent_two`, the mapping `IconName::path` uses.
#[must_use]
pub fn icon_stem(variant: &str) -> String {
    let mut stem = String::new();
    for (index, character) in variant.char_indices() {
        if character.is_ascii_uppercase() && index != 0 {
            stem.push('_');
        }
        stem.push(character.to_ascii_lowercase());
    }
    stem
}

/// Every variant of the `IconName` enum, in declaration order.
#[must_use]
pub fn icon_name_variants(source: &str) -> Vec<String> {
    let Some((_, body, _)) = next_enum_body(source, "IconName") else {
        return Vec::new();
    };
    body.lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(|line| line.trim_end_matches(',').to_owned())
        .filter(|name| {
            name.chars()
                .all(|character| character.is_ascii_alphanumeric())
        })
        .collect()
}

/// The declared fields of a `pub struct NAME { … }`, as `(name, type)`.
///
/// Doc comments, attributes and blank lines are skipped, so the result is the
/// data the struct actually holds. Returns an empty vector if the struct is not
/// found, which the callers assert against separately — a check that reads no
/// fields must fail loudly rather than pass vacuously.
#[must_use]
pub fn struct_fields(source: &str, name: &str) -> Vec<(String, String)> {
    let header = format!("pub struct {name} {{\n");
    let Some(start) = source.find(&header) else {
        return Vec::new();
    };
    let body_start = start + header.len();
    let Some(end) = source[body_start..].find("\n}") else {
        return Vec::new();
    };
    source[body_start..body_start + end]
        .lines()
        .map(str::trim)
        .filter(|line| {
            !line.is_empty()
                && !line.starts_with("///")
                && !line.starts_with("//")
                && !line.starts_with("#[")
        })
        .filter_map(|line| {
            let line = line.trim_end_matches(',');
            let (field, type_name) = line.split_once(':')?;
            Some((
                field.trim_start_matches("pub ").trim().to_owned(),
                type_name.trim().to_owned(),
            ))
        })
        .collect()
}

fn next_enum_body<'a>(source: &'a str, name: &str) -> Option<(usize, &'a str, usize)> {
    let header = format!("pub enum {name} {{\n");
    let start = source.find(&header)?;
    let body_start = start + header.len();
    let end = source[body_start..].find("\n}")? + body_start;
    Some((start, &source[body_start..end], end))
}

/// A source file with its line comments removed.
///
/// OMEGA-DELTA-0029's scans read code, not prose: the doc comments in the
/// router deliberately name the tokens they forbid, in the course of saying why
/// they are not there. A scan that could not tell the two apart would force the
/// explanation out of the file, which is the opposite of what these checks are
/// for.
#[must_use]
pub fn code_of(source: &str) -> String {
    source
        .lines()
        .filter(|line| !line.trim_start().starts_with("//"))
        .collect::<Vec<_>>()
        .join("\n")
}

/// Read the value of a `pub const NAME: &str = "value";` declaration.
#[must_use]
pub fn string_constant(source: &str, name: &str) -> Option<String> {
    source
        .lines()
        .find(|line| line.contains(&format!("const {name}:")))
        .and_then(|line| line.split_once('='))
        .and_then(|(_, value)| value.split('"').nth(1))
        .map(str::to_owned)
}

/// Visit every file under `root` whose extension is in `extensions`.
///
/// Symlinks and `target` directories are skipped, and unreadable or non-UTF-8
/// files are ignored, matching the tree walk the Zed-copy check already uses.
pub fn for_each_source_file(
    root: &std::path::Path,
    extensions: &[&str],
    mut visit: impl FnMut(&std::path::Path, &str),
) {
    let mut stack = vec![root.to_path_buf()];
    while let Some(directory) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&directory) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_symlink() {
                continue;
            }
            if path.is_dir() {
                if path.file_name().is_some_and(|name| name == "target") {
                    continue;
                }
                stack.push(path);
                continue;
            }
            let matches = path
                .extension()
                .and_then(std::ffi::OsStr::to_str)
                .is_some_and(|extension| extensions.contains(&extension));
            if !matches {
                continue;
            }
            let Ok(source) = std::fs::read_to_string(&path) else {
                continue;
            };
            visit(&path, &source);
        }
    }
}

// ------------------------------------------------------------ OMEGA-DELTA-0022

/// Every `#[folder = "…"]` in the repository, as (declaring file, folder).
///
/// `rust-embed` resolves the attribute against `CARGO_MANIFEST_DIR`, so the
/// base is the crate root rather than the directory the attribute is written
/// in. Deriving the embed roots instead of listing them is the whole point:
/// `assets/images/` was never in a list, and that is where the Zed artwork sat
/// while `OMEGA-DELTA-0018` reported `assets/icons/` clean.
#[must_use]
pub fn embed_folders() -> Vec<(String, std::path::PathBuf)> {
    let root = repository_path("crates");
    let mut found = Vec::new();
    for_each_source_file(&root, &["rs"], |path, source| {
        let mut folders = Vec::new();
        let mut rest = source;
        while let Some(at) = rest.find("#[folder") {
            rest = &rest[at..];
            let Some((_, after_equals)) = rest.split_once('=') else {
                break;
            };
            let Some(value) = after_equals.split('"').nth(1) else {
                break;
            };
            folders.push(value.to_owned());
            rest = &rest[1..];
        }
        if folders.is_empty() {
            return;
        }
        let crate_root = path
            .ancestors()
            .find(|ancestor| ancestor.join("Cargo.toml").is_file())
            .unwrap_or_else(|| path.parent().unwrap_or(path));
        for folder in folders {
            found.push((
                path.display().to_string(),
                normalize_path(&crate_root.join(folder)),
            ));
        }
    });
    found.sort();
    found
}

/// Lexically resolve `.` and `..` without touching the filesystem.
fn normalize_path(path: &std::path::Path) -> std::path::PathBuf {
    let mut out = std::path::PathBuf::new();
    for component in path.components() {
        match component {
            std::path::Component::ParentDir => {
                out.pop();
            }
            std::path::Component::CurDir => {}
            other => out.push(other),
        }
    }
    out
}

/// Every file `rust-embed` can ship, as a repository-relative path.
///
/// The assets tree plus every directory an embed declaration points at. A
/// subdirectory list would be a claim about the subdirectories somebody
/// remembered; this is the set the embed macros actually read.
#[must_use]
pub fn embedded_asset_inventory() -> Vec<String> {
    let repository = normalize_path(&repository_path("."));
    let mut roots = vec![normalize_path(&repository_path("assets"))];
    roots.extend(embed_folders().into_iter().map(|(_, folder)| folder));
    let mut files = std::collections::BTreeSet::new();
    for root in roots {
        let mut stack = vec![root];
        while let Some(directory) = stack.pop() {
            let Ok(entries) = std::fs::read_dir(&directory) else {
                continue;
            };
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_symlink() {
                    continue;
                }
                if path.is_dir() {
                    stack.push(path);
                    continue;
                }
                if path.file_name().is_some_and(|name| name == ".DS_Store") {
                    continue;
                }
                if let Ok(relative) = normalize_path(&path).strip_prefix(&repository) {
                    files.insert(relative.display().to_string());
                }
            }
        }
    }
    files.into_iter().collect()
}

/// Every gpui action in the repository, as `(namespace, name, file)`.
///
/// There are exactly two ways to declare one — `actions!(namespace, [..])` and
/// `#[action(namespace = ..)]` on a derive — and both are read here, so this
/// is the complete set of `namespace: action name` labels the command palette
/// can display. Nothing had ever read an action declaration before
/// `0.2.0-rc11` offered `zed: about`, `zed: quit` and `zed: get merch`.
#[must_use]
pub fn action_declarations() -> Vec<(String, String, String)> {
    let root = repository_path("crates");
    let mut declarations = Vec::new();
    for_each_source_file(&root, &["rs"], |path, source| {
        let file = path.display().to_string();
        let mut rest = source;
        while let Some(at) = rest.find("actions!(") {
            let after = &rest[at + "actions!(".len()..];
            rest = after;
            let Some((namespace, body)) = actions_macro_body(after) else {
                continue;
            };
            for line in body.lines() {
                let name = line.trim().trim_end_matches(',');
                if is_action_variant(name) {
                    declarations.push((namespace.clone(), name.to_owned(), file.clone()));
                }
            }
        }
        for (namespace, name) in derived_actions(source) {
            declarations.push((namespace, name, file.clone()));
        }
    });
    declarations.sort();
    declarations
}

/// `namespace` and the bracketed body of an `actions!(namespace, [ .. ])` call.
fn actions_macro_body(after_open: &str) -> Option<(String, &str)> {
    let (head, rest) = after_open.split_once(",")?;
    let namespace = head.trim().to_owned();
    if namespace.is_empty()
        || !namespace
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || character == '_')
    {
        return None;
    }
    let open = rest.find('[')?;
    let mut depth = 1_usize;
    let bytes = rest.as_bytes();
    let mut index = open + 1;
    while index < bytes.len() && depth > 0 {
        match bytes[index] {
            b'[' => depth += 1,
            b']' => depth -= 1,
            _ => {}
        }
        index += 1;
    }
    Some((namespace, &rest[open + 1..index.saturating_sub(1)]))
}

fn is_action_variant(name: &str) -> bool {
    !name.is_empty()
        && name.starts_with(|character: char| character.is_ascii_uppercase())
        && name
            .chars()
            .all(|character| character.is_ascii_alphanumeric())
}

/// Actions declared with `#[action(namespace = ..)]` on a struct or enum.
fn derived_actions(source: &str) -> Vec<(String, String)> {
    let mut found = Vec::new();
    let mut rest = source;
    while let Some(at) = rest.find("#[action(") {
        let after = &rest[at + "#[action(".len()..];
        rest = &rest[at + 1..];
        let Some(close) = after.find(")]") else {
            continue;
        };
        let attribute = &after[..close];
        let Some(namespace) = attribute
            .split("namespace")
            .nth(1)
            .and_then(|tail| tail.trim_start().strip_prefix('='))
            .map(|tail| {
                tail.trim_start()
                    .chars()
                    .take_while(|character| character.is_ascii_alphanumeric() || *character == '_')
                    .collect::<String>()
            })
            .filter(|namespace| !namespace.is_empty())
        else {
            continue;
        };
        let tail = &after[close + 2..];
        let mut name = None;
        for keyword in ["pub struct ", "pub enum "] {
            if let Some(at) = tail.find(keyword) {
                // Only accept a declaration that follows immediately, allowing
                // for other attributes in between but not a whole other item.
                if tail[..at].contains("\n\n") {
                    continue;
                }
                let candidate: String = tail[at + keyword.len()..]
                    .chars()
                    .take_while(|character| character.is_ascii_alphanumeric() || *character == '_')
                    .collect();
                if !candidate.is_empty() && name.is_none() {
                    name = Some(candidate);
                }
            }
        }
        if let Some(name) = name {
            found.push((namespace, name));
        }
    }
    found
}

/// Old action names kept resolvable for existing user keymaps.
///
/// A `deprecated_aliases` entry names the action Omega used to have. It is
/// never shown in the palette, so it is exempt from the label rules — and only
/// the strings actually inside such a list are.
#[must_use]
pub fn deprecated_action_aliases() -> std::collections::BTreeSet<String> {
    let root = repository_path("crates");
    let mut aliases = std::collections::BTreeSet::new();
    for_each_source_file(&root, &["rs"], |_path, source| {
        let mut rest = source;
        while let Some(at) = rest.find("deprecated_aliases") {
            let after = &rest[at..];
            rest = &rest[at + 1..];
            let Some(open) = after.find('[') else {
                continue;
            };
            let Some(close) = after[open..].find(']') else {
                continue;
            };
            let list = &after[open..open + close];
            for (index, part) in list.split('"').enumerate() {
                if index % 2 == 1 {
                    aliases.insert(part.to_owned());
                }
            }
        }
    });
    aliases
}

/// Enums whose `impl` builds an embedded asset path, and their variants.
///
/// Discovered, not listed. `OMEGA-DELTA-0018` named `crates/icons/src/icons.rs`
/// in the policy file; `VectorName` in `crates/ui/src/components/image.rs` does
/// the identical job for `assets/images/` and was outside the gate, which is
/// how `VectorName::ZedLogo` and `VectorName::ZedXCopilot` survived
/// `0.2.0-rc11`.
#[must_use]
pub fn asset_name_enums() -> std::collections::BTreeMap<String, Vec<String>> {
    let directories: std::collections::BTreeSet<String> = embedded_asset_inventory()
        .iter()
        .filter_map(|relative| {
            let mut parts = relative.split('/');
            if parts.next()? != "assets" {
                return None;
            }
            let directory = parts.next()?;
            parts.next()?;
            Some(directory.to_owned())
        })
        .collect();

    let root = repository_path("crates");
    let mut discovered = std::collections::BTreeMap::new();
    for_each_source_file(&root, &["rs"], |_path, source| {
        let names_an_asset = directories
            .iter()
            .any(|directory| source.contains(&format!("format!(\"{directory}/{{")));
        if !names_an_asset {
            return;
        }
        let mut rest = source;
        while let Some(at) = rest.find("pub enum ") {
            let after = &rest[at + "pub enum ".len()..];
            rest = &rest[at + 1..];
            let name: String = after
                .chars()
                .take_while(|character| character.is_ascii_alphanumeric() || *character == '_')
                .collect();
            if name.is_empty() {
                continue;
            }
            let Some((_, body, _)) = next_enum_body(source, &name) else {
                continue;
            };
            let variants: Vec<String> = body
                .lines()
                .map(str::trim)
                .map(|line| line.trim_end_matches(','))
                .filter(|line| is_action_variant(line))
                .map(str::to_owned)
                .collect();
            if !variants.is_empty() {
                discovered.insert(name, variants);
            }
        }
    });
    discovered
}

/// Public product claims the compatibility allow-list records as `blocked`.
///
/// The allow-list is the reviewed record of every retained Zed identifier, and
/// `blocked` is its strongest disposition — but until `OMEGA-DELTA-0022`
/// nothing read those entries back against the tree, so `Welcome to Zed` was
/// listed while `Use GitHub Copilot in Zed` shipped in three places.
pub const COMPATIBILITY_ALLOWLIST_PATH: &str =
    "crates/app_identity/fixtures/compatibility_allowlist.json";

/// Files whose job is to hold the forbidden strings.
///
/// A corpus file names a blocked claim on purpose, to assert it is absent
/// everywhere else. Exempting them by path is not a hole: each one is a test
/// or a policy record, and each is checked by name here so the list cannot
/// quietly grow to cover a real surface.
pub const BLOCKED_COPY_CORPUS: &[&str] = &[
    COMPATIBILITY_ALLOWLIST_PATH,
    "crates/app_identity/src/public_branding.rs",
    "crates/app_identity/src/shell_branding.rs",
    "crates/omega_deltas/src/omega_deltas.rs",
];

// ------------------------------------------------------------ OMEGA-DELTA-0031

/// One brand-bearing prose literal that can reach a user.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct ProseLiteral {
    /// Which mechanism puts this text in front of a user.
    pub kind: &'static str,
    /// Repository-relative file it was read from.
    pub file: String,
    /// Line the literal starts on.
    pub line: usize,
    /// The literal, with runs of whitespace collapsed.
    pub text: String,
}

/// What the prose scanners read, as opposed to what they found.
///
/// The anti-vacuity guard. A clean tree yields almost no brand-bearing prose
/// and so does a scanner that stopped parsing, and those two must not look the
/// same from the outside.
#[derive(Debug, Default, Clone, Copy)]
pub struct ProseReadCounts {
    /// Rust sources walked.
    pub rust_files: usize,
    /// Rust string literals lexed.
    pub literals: usize,
    /// Doc lines in files that really derive `JsonSchema`.
    pub schema_docs: usize,
    /// Doc lines inside an action declaration.
    pub action_docs: usize,
    /// Doc lines in files clap prints as `--help`.
    pub clap_docs: usize,
    /// Files in the embedded-asset inventory.
    pub embedded: usize,
}

/// Collapse runs of whitespace so a literal has one spelling.
///
/// A multi-line literal is indented to wherever it happens to sit in the
/// source, and that indentation is not part of the sentence.
#[must_use]
pub fn normalize_prose(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Whether `text` reads as a sentence rather than an identifier or a path.
///
/// Three tokens or more, at least two of them plain alphabetic words. This is
/// the one judgement in the whole derivation: it keeps `crates/zed/src` and
/// `X-Zed-Predict-Edits-Mode` out of a registry that would otherwise be mostly
/// noise. It is deliberately loose — `Zed Plex Sans` is three words and is in
/// the inventory, classified, rather than quietly filtered away.
#[must_use]
pub fn is_prose(text: &str) -> bool {
    let tokens: Vec<&str> = text.split_whitespace().collect();
    if tokens.len() < 3 {
        return false;
    }
    let plain = tokens
        .iter()
        .filter(|token| {
            let trimmed = token.trim_matches(|character: char| {
                !character.is_ascii_alphabetic() && character != '\''
            });
            !trimmed.is_empty()
                && trimmed
                    .chars()
                    .all(|character| character.is_ascii_alphabetic() || character == '\'')
        })
        .count();
    plain >= 2
}

/// Whether `text` is a command line whose program name is a competitor's
/// binary.
///
/// OMEGA-DELTA-0044. `is_prose` needs three tokens, so `zed --existing`,
/// `zed --classic` and `zed <path>` were invisible to it — and all three
/// shipped in the signed `cli` of `0.2.0-rc16`, in an interactive panel whose
/// surrounding copy says "Omega window" and "Omega settings" around them
/// (omega#93). Substitute our own name and each stays true, so by the rule
/// this gate is written around they are product claims, not references.
///
/// This admits the narrow shape only: a brand word standing in `argv[0]`,
/// followed by flags, placeholders or paths. Admitting every two-token literal
/// that starts with a brand word instead adds 8 more across the tree — `Zed
/// Pro`, `Zed (Default)`, `Zed Repository` — which are genuine references to
/// somebody else's product; admitting a bare brand token adds 55. This shape
/// adds exactly the three, which is why the gate can afford to carry it
/// instead of writing the three literals into a denylist.
#[must_use]
pub fn is_command_form(text: &str, policy: &serde_json::Value) -> bool {
    let words: Vec<&str> = policy["brand"]["words"]
        .as_array()
        .map(|values| {
            values
                .iter()
                .filter_map(serde_json::Value::as_str)
                .collect()
        })
        .unwrap_or_default();
    let tokens: Vec<&str> = text.split_whitespace().collect();
    if tokens.len() < 2 || !words.contains(&tokens[0]) {
        return false;
    }
    tokens[1..].iter().all(|token| is_command_argument(token))
}

/// A flag (`-n`, `--existing`), a placeholder (`<path>`, `[FILE]`), or a path
/// (`.`, `..`, `~/x`, `./x`, `/x`).
fn is_command_argument(token: &str) -> bool {
    let flag = token.strip_prefix("--").or_else(|| token.strip_prefix('-'));
    if let Some(rest) = flag {
        return rest
            .chars()
            .next()
            .is_some_and(|character| character.is_ascii_alphanumeric())
            && rest
                .chars()
                .all(|character| character.is_ascii_alphanumeric() || "-_.".contains(character));
    }
    let placeholder = matches!(token.chars().next(), Some('<' | '[' | '{'))
        && matches!(token.chars().last(), Some('>' | ']' | '}'))
        && token.len() >= 2
        && !token[1..token.len() - 1].contains(['>', ']', '}']);
    if placeholder {
        return true;
    }
    token == "." || token == ".." || token.starts_with(['~', '/']) || token.starts_with("./")
}

/// Prose, or a command form. What the prose inventory admits.
#[must_use]
pub fn is_user_facing_text(text: &str, policy: &serde_json::Value) -> bool {
    is_prose(text) || is_command_form(&normalize_prose(text), policy)
}

/// Every Rust string literal in `source`, as `(start line, contents)`.
///
/// Raw and multi-line literals included, comments skipped. A regex over single
/// lines misses exactly the literals carrying the longest copy: the OAuth
/// callback page, the run-as-root warning and four provider error messages are
/// all multi-line, and every one of them named a competitor as the product.
#[must_use]
pub fn rust_string_literals(source: &str) -> Vec<(usize, String)> {
    let mut out = Vec::new();
    let mut index = 0;
    let mut line = 1usize;
    while index < source.len() {
        let rest = &source[index..];
        let character = rest.as_bytes()[0];
        if character == b'\n' {
            line += 1;
            index += 1;
        } else if rest.starts_with("//") {
            index += rest.find('\n').unwrap_or(rest.len());
        } else if rest.starts_with("/*") {
            let (consumed, newlines) = skip_block_comment(rest);
            line += newlines;
            index += consumed;
        } else if character == b'r'
            && matches!(rest.as_bytes().get(1), Some(b'"' | b'#'))
            && let Some((body, consumed)) = raw_string_literal(rest)
        {
            let start = line;
            line += body.matches('\n').count();
            out.push((start, body));
            index += consumed;
        } else if character == b'"' {
            let (body, consumed) = quoted_string_literal(rest);
            let start = line;
            line += body.matches('\n').count();
            out.push((start, body));
            index += consumed;
        } else if character == b'\'' {
            // A char literal or a lifetime; neither can hold prose. Only
            // `'"'` has to be stepped over as a unit, so that its quote does
            // not read as the start of a string.
            index += if rest.as_bytes().get(1) == Some(&b'"')
                && rest.as_bytes().get(2) == Some(&b'\'')
            {
                3
            } else {
                1
            };
        } else {
            index += rest.chars().next().map_or(1, char::len_utf8);
        }
    }
    out
}

/// Consume a possibly nested block comment, returning `(bytes, newlines)`.
fn skip_block_comment(source: &str) -> (usize, usize) {
    let mut depth = 1;
    let mut index = 2;
    let mut newlines = 0;
    while index < source.len() && depth > 0 {
        let rest = &source[index..];
        if rest.starts_with("/*") {
            depth += 1;
            index += 2;
        } else if rest.starts_with("*/") {
            depth -= 1;
            index += 2;
        } else {
            let character = rest.chars().next().unwrap_or('*');
            if character == '\n' {
                newlines += 1;
            }
            index += character.len_utf8();
        }
    }
    (index, newlines)
}

/// Parse `r"..."` / `r#"..."#` at the start of `source`.
fn raw_string_literal(source: &str) -> Option<(String, usize)> {
    let bytes = source.as_bytes();
    let mut cursor = 1;
    while bytes.get(cursor) == Some(&b'#') {
        cursor += 1;
    }
    if bytes.get(cursor) != Some(&b'"') {
        return None;
    }
    cursor += 1;
    let close = format!("\"{}", "#".repeat(cursor - 2));
    let end = source[cursor..].find(&close)? + cursor;
    Some((source[cursor..end].to_owned(), end + close.len()))
}

/// Parse `"..."` at the start of `source`, keeping escapes as written.
///
/// Escapes are kept rather than resolved because the source is what a reviewer
/// reads and what the classification registry records. The packaged half of
/// the gate resolves them, because the binary holds the value.
fn quoted_string_literal(source: &str) -> (String, usize) {
    let mut body = String::new();
    let mut cursor = 1;
    while cursor < source.len() {
        let rest = &source[cursor..];
        if let Some(escaped) = rest.strip_prefix('\\') {
            let width = 1 + escaped.chars().next().map_or(0, char::len_utf8);
            body.push_str(&rest[..width]);
            cursor += width;
        } else if rest.starts_with('"') {
            cursor += 1;
            break;
        } else {
            let character = rest.chars().next().unwrap_or('"');
            body.push(character);
            cursor += character.len_utf8();
        }
    }
    (body, cursor)
}

/// Line numbers inside a `#[cfg(test)]` item, which a release build drops.
///
/// Over-excluding here would be a hole, which is why the packaged half of the
/// gate reads the binary that was actually built and honours none of this.
#[must_use]
pub fn cfg_test_lines(source: &str) -> std::collections::BTreeSet<usize> {
    let lines: Vec<&str> = source.lines().collect();
    let mut out = std::collections::BTreeSet::new();
    for (start, line) in lines.iter().enumerate() {
        let trimmed = line.trim_start();
        if !trimmed.starts_with("#[cfg(") || !trimmed.contains("test") {
            continue;
        }
        let mut depth: i64 = 0;
        let mut opened = false;
        for (cursor, item) in lines.iter().enumerate().skip(start) {
            depth += i64::try_from(item.matches('{').count()).unwrap_or(0);
            depth -= i64::try_from(item.matches('}').count()).unwrap_or(0);
            if item.contains('{') {
                opened = true;
            }
            out.insert(cursor + 1);
            if opened && depth <= 0 {
                break;
            }
        }
    }
    out
}

/// Doc lines the keymap editor renders as an action's description.
///
/// Inside an `actions!(..)` body, or immediately above an `#[action(..)]`
/// derive. `OMEGA-DELTA-0022` recorded action doc comments as unchecked;
/// `client::SignIn` describes itself as signing in to a *Zed* account, which
/// is true, and only a check that reads them can say so on purpose.
#[must_use]
pub fn action_doc_lines(source: &str) -> std::collections::BTreeSet<usize> {
    let mut out = std::collections::BTreeSet::new();
    let lines: Vec<&str> = source.lines().collect();
    let mut rest = source;
    let mut base = 0;
    while let Some(at) = rest.find("actions!(") {
        let open = base + at + "actions!(".len() - 1;
        let mut depth = 0;
        let mut index = open;
        let bytes = source.as_bytes();
        while index < bytes.len() {
            match bytes[index] {
                b'(' => depth += 1,
                b')' => {
                    depth -= 1;
                    if depth == 0 {
                        break;
                    }
                }
                _ => {}
            }
            index += 1;
        }
        let first = source[..open].matches('\n').count() + 1;
        let last = source[..index.min(source.len())].matches('\n').count() + 1;
        out.extend(first..=last);
        base += at + "actions!(".len();
        rest = &source[base..];
    }
    for (index, line) in lines.iter().enumerate() {
        if !line.contains("#[action(") {
            continue;
        }
        let mut cursor = index;
        while cursor > 0 {
            cursor -= 1;
            let trimmed = lines[cursor].trim_start();
            if doc_comment_body(lines[cursor]).is_some() {
                out.insert(cursor + 1);
            } else if !trimmed.starts_with("#[") {
                break;
            }
        }
    }
    out
}

/// The text of a doc comment, however it was spelled.
///
/// `///` and `//!` are sugar. `#[doc = "..."]` and
/// `#[cfg_attr(<predicate>, doc = "...")]` are the same thing written the long
/// way, and both clap and schemars read them identically. This function did not,
/// which is why every candidate up to `0.2.0-rc14` printed
/// `~/Library/Application Support/Zed` as the data directory from `cli --help`
/// while the gate reported green: the line is a `cfg_attr`, and nothing here
/// had ever looked inside one (omega#89).
fn doc_comment_body(line: &str) -> Option<&str> {
    let trimmed = line.trim_start();
    if let Some(body) = trimmed
        .strip_prefix("///")
        .or_else(|| trimmed.strip_prefix("//!"))
    {
        return Some(body);
    }
    attribute_doc_body(trimmed)
}

/// The string in a `doc = "..."` attribute, or the `doc = "..."` line of an
/// attribute written across several lines.
///
/// Anchored at the start of the trimmed line so a Rust `let doc = "..."`
/// binding is not mistaken for documentation.
fn attribute_doc_body(trimmed: &str) -> Option<&str> {
    let rest = if let Some(rest) = trimmed.strip_prefix("#[") {
        let at = rest.find("doc")?;
        &rest[at..]
    } else if trimmed.starts_with("doc") {
        trimmed
    } else {
        return None;
    };
    let rest = rest.strip_prefix("doc")?;
    let rest = rest.trim_start();
    let rest = rest.strip_prefix('=')?.trim_start();
    let rest = rest.strip_prefix('"')?;
    let mut end = 0;
    let bytes = rest.as_bytes();
    while end < bytes.len() {
        match bytes[end] {
            b'\\' => end += 2,
            b'"' => return Some(&rest[..end]),
            _ => end += 1,
        }
    }
    None
}

/// Every brand-bearing prose literal that can reach a user, plus read counts.
///
/// Five streams, each derived from a mechanism that exists in the tree rather
/// than from a list of files somebody remembered. See the `prose` section of
/// `script/omega-brand-gate.json`, which both halves of the gate read.
#[must_use]
pub fn prose_inventory(policy: &serde_json::Value) -> (Vec<ProseLiteral>, ProseReadCounts) {
    let mut items = Vec::new();
    let mut read = ProseReadCounts::default();
    let repository = normalize_path(&repository_path("."));
    let rust_root = policy["prose"]["rust_root"].as_str().unwrap_or("crates");

    for_each_source_file(&repository_path(rust_root), &["rs"], |path, source| {
        read.rust_files += 1;
        let literals = rust_string_literals(source);
        read.literals += literals.len();
        let lines: Vec<&str> = source.lines().collect();
        let code: String = lines
            .iter()
            .map(|line| {
                if doc_comment_body(line).is_some() {
                    ""
                } else {
                    *line
                }
            })
            .collect::<Vec<_>>()
            .join("\n");
        let schema = derives_any(&code, &["JsonSchema"]);
        let clap = derives_any(&code, &["Parser", "Args", "Subcommand"])
            || code.contains("#[command(")
            || code.contains("clap::Parser");
        let actions = action_doc_lines(source);
        for (number, _line) in lines.iter().enumerate() {
            if doc_comment_body(lines[number]).is_none() {
                continue;
            }
            if actions.contains(&(number + 1)) {
                read.action_docs += 1;
            } else if schema {
                read.schema_docs += 1;
            } else if clap {
                read.clap_docs += 1;
            }
        }
        if brand_hits(source, policy).is_empty() {
            return;
        }
        let relative = normalize_path(path).strip_prefix(&repository).map_or_else(
            |_| path.display().to_string(),
            |tail| tail.display().to_string(),
        );
        let is_test_file = is_test_path(&relative);
        let skipped = if is_test_file {
            std::collections::BTreeSet::new()
        } else {
            cfg_test_lines(source)
        };
        for (number, body) in literals {
            if is_test_file || skipped.contains(&number) {
                continue;
            }
            if !brand_hits(&body, policy).is_empty() && is_user_facing_text(&body, policy) {
                items.push(ProseLiteral {
                    kind: "rust_string",
                    file: relative.clone(),
                    line: number,
                    text: normalize_prose(&body),
                });
            }
        }
        for (index, line) in lines.iter().enumerate() {
            let number = index + 1;
            if is_test_file || skipped.contains(&number) {
                continue;
            }
            let Some(body) = doc_comment_body(line) else {
                continue;
            };
            if brand_hits(body, policy).is_empty() || !is_user_facing_text(body, policy) {
                continue;
            }
            let kind = if actions.contains(&number) {
                "action_doc"
            } else if schema {
                "schema_doc"
            } else if clap {
                "clap_doc"
            } else {
                continue;
            };
            items.push(ProseLiteral {
                kind,
                file: relative.clone(),
                line: number,
                text: normalize_prose(body),
            });
        }
    });

    for relative in embedded_asset_inventory() {
        read.embedded += 1;
        let Ok(source) = std::fs::read_to_string(repository_path(&relative)) else {
            continue;
        };
        if brand_hits(&source, policy).is_empty() {
            continue;
        }
        for (index, line) in source.lines().enumerate() {
            if !brand_hits(line, policy).is_empty() && is_user_facing_text(line, policy) {
                items.push(ProseLiteral {
                    kind: "asset",
                    file: relative.clone(),
                    line: index + 1,
                    text: normalize_prose(line),
                });
            }
        }
    }
    items.sort();
    (items, read)
}

/// Whether any `#[derive(..)]` in `code` names one of `traits`.
///
/// Doc lines are stripped by the caller before this runs, so a derive written
/// inside a rustdoc EXAMPLE does not pull a whole framework crate's internal
/// documentation into the inventory. `gpui/src/action.rs` documents
/// `#[derive(.., schemars::JsonSchema, Action)]` in a code sample and has no
/// settings type in it at all.
fn derives_any(code: &str, traits: &[&str]) -> bool {
    let mut rest = code;
    while let Some(at) = rest.find("#[derive(") {
        rest = &rest[at + "#[derive(".len()..];
        let list = rest.split(')').next().unwrap_or("");
        if traits.iter().any(|name| list.contains(name)) {
            return true;
        }
    }
    false
}

/// Whether `relative` names a file a release build never compiles.
fn is_test_path(relative: &str) -> bool {
    let normalized = relative.replace('\\', "/");
    normalized.ends_with("_test.rs")
        || normalized.ends_with("_tests.rs")
        || ["/tests/", "/test/", "/benches/", "/examples/", "/fixtures/"]
            .iter()
            .any(|segment| normalized.contains(segment))
}

// ------ OMEGA-DELTA-0042

/// OMEGA-DELTA-0042. The Exo harness lane's law. A leaf, checkable in a second.
pub const EXO_LANE_LAW_PATH: &str = "crates/omega_exo_lane/src/omega_exo_lane.rs";

/// OMEGA-DELTA-0042. Which Exo the lane admits.
pub const EXO_LANE_PIN_PATH: &str = "crates/omega_exo_lane/src/pin.rs";

/// OMEGA-DELTA-0042. Every command line the lane can produce.
pub const EXO_LANE_COMMAND_PATH: &str = "crates/omega_exo_lane/src/command.rs";

/// OMEGA-DELTA-0042. The half that spawns a process and builds a thread.
pub const EXO_CONNECTION_PATH: &str = "crates/agent_ui/src/omega_exo_connection.rs";

/// OMEGA-DELTA-0051. The first-run page zero base reduces to its identity step.
pub const ONBOARDING_BASICS_PAGE_PATH: &str = "crates/onboarding/src/basics_page.rs";

/// OMEGA-DELTA-0051. The setup questions zero base answers from defaults
/// instead of asking.
///
/// Named as the render functions rather than as the visible headings, because a
/// heading is copy and these are the calls that put the sections on screen.
pub const ONBOARDING_SECTIONS_ZERO_BASE_SKIPS: &[&str] = &[
    "render_theme_section",
    "render_base_keymap_section",
    "render_ai_section",
    "render_import_settings_section",
    "render_vim_mode_switch",
    "render_worktree_auto_trust_switch",
    "render_telemetry_section",
];

/// OMEGA-DELTA-0047, 0048. The mode itself: what reads the command line, what
/// the admitted set is, and what a refusal says.
pub const ZERO_BASE_MODE_PATH: &str = "crates/omega_zero_base/src/omega_zero_base.rs";

/// OMEGA-DELTA-0048, 0050, 0052. The surface: the palette restriction and the
/// action gate. It carried the visible way out until `OMEGA-DELTA-0052`.
pub const ZERO_BASE_UI_PATH: &str = "crates/zed/src/omega_zero_base_ui.rs";

/// OMEGA-DELTA-0048, 0053. Where panels are added, where zero base skips them,
/// and where the mode seals the window after the identity gate.
pub const WORKSPACE_INITIALIZATION_PATH: &str = "crates/zed/src/zed.rs";

/// OMEGA-DELTA-0053. The workspace render, which draws no editor pane, no
/// title bar and no status bar once zero base is sealed.
pub const WORKSPACE_RENDER_PATH: &str = "crates/workspace/src/workspace.rs";

/// OMEGA-DELTA-0054. The plausibility test for a working directory.
pub const WORKDIR_PATH: &str = "crates/omega_workdir/src/omega_workdir.rs";

/// OMEGA-DELTA-0054, 0052. The argument parser and the startup open path.
pub const STARTUP_OPEN_PATH: &str = "crates/zed/src/main.rs";

/// OMEGA-DELTA-0048. The palette filter the restriction extends.
pub const COMMAND_PALETTE_FILTER_PATH: &str =
    "crates/command_palette_hooks/src/command_palette_hooks.rs";

/// OMEGA-DELTA-0048. The dispatch point the action gate is consulted from.
pub const ACTION_DISPATCH_PATH: &str = "crates/gpui/src/window.rs";

/// OMEGA-DELTA-0050. The Full Auto launch surface and its start control.
pub const FULL_AUTO_PANEL_PATH: &str = "crates/full_auto_ui/src/panel.rs";

/// OMEGA-DELTA-0050. Gate 8's two closed lists.
pub const GATE_EIGHT_PATH: &str = "crates/omega_front_door/src/omega_front_door.rs";

/// OMEGA-DELTA-0092. How many places a state root is looked for.
///
/// An explicit `OMEGA_EXO_ROOT`, the working directory, the checkout, and a
/// root named by an existing lane file. Written here as well as in the search
/// so that adding or removing one is a change to this crate, and therefore to
/// the registry entry, rather than a quiet edit to a list.
pub const ROOT_CANDIDATE_ORDER_LENGTH: usize = 4;

/// OMEGA-DELTA-0092. The variable that names a root outright.
pub const ROOT_ENV_VAR_SPELLING: &str = "OMEGA_EXO_ROOT";

/// OMEGA-DELTA-0092. The variable that names a lane file to mine a root from.
pub const LANE_FILE_ENV_VAR_SPELLING: &str = "OMEGA_EXO_LANE_FILE";

/// OMEGA-DELTA-0092. Where an Exo install is found and a lane derived from it.
pub const EXO_DETECT_PATH: &str = "crates/omega_agent_detect/src/exo.rs";

/// OMEGA-DELTA-0092. Where derivation is allowed to stand in for a lane file.
///
/// The same file as [`EXO_CONNECTION_PATH`], named separately because the two
/// deltas assert different things about it and a shared constant would make a
/// rename look like it belonged to whichever check happened to fail first.
pub const EXO_LANE_RESOLUTION_PATH: &str = "crates/agent_ui/src/omega_exo_connection.rs";

/// OMEGA-DELTA-0070. Where every compiled-in skill is registered and loaded.
pub const BUILTIN_SKILLS_PATH: &str = "crates/agent_skills/agent_skills.rs";

/// OMEGA-DELTA-0070. The shipped public NIP-29 chat skill.
pub const PUBLIC_NOSTR_CHAT_SKILL_PATH: &str =
    "crates/agent_skills/builtin/public-nostr-chat/SKILL.md";

/// OMEGA-DELTA-0070. The name the skill is registered and invoked under. The
/// entry name in the registration table and the `name` in the file's own
/// frontmatter must both be this, because the loader keys the embedded body by
/// a synthetic path built from the table name while the catalog entry takes its
/// name from the frontmatter. A disagreement produces a skill whose body cannot
/// be fetched.
pub const PUBLIC_NOSTR_CHAT_SKILL_NAME: &str = "public-nostr-chat";

/// OMEGA-DELTA-0070. The description limit the skill loader enforces, mirrored
/// from `agent_skills::MAX_SKILL_DESCRIPTION_LEN`. Mirrored rather than
/// imported: `agent_skills` pulls in gpui, and the delta checks are meant to
/// run on their own without building the UI framework. The check below asserts
/// the mirrored value against the source that owns it, so drift fails here.
pub const MAX_SKILL_DESCRIPTION_LEN: usize = 1024;

// ------ OMEGA-DELTA-0090

/// OMEGA-DELTA-0090. The episode law: which Exo requests, which comparison,
/// which loop.
pub const EPISODE_LAW_PATH: &str = "crates/omega_exo_episode/src/omega_exo_episode.rs";

/// OMEGA-DELTA-0090. The partition of Exo's protocol into admitted and refused
/// families.
pub const EPISODE_FAMILY_PATH: &str = "crates/omega_exo_episode/src/family.rs";

/// OMEGA-DELTA-0090. The four requests an episode sends, and their wire shapes.
pub const EPISODE_REQUEST_PATH: &str = "crates/omega_exo_episode/src/request.rs";

/// OMEGA-DELTA-0090. The reset, its admission table, and the falsification
/// loop.
pub const EPISODE_RESET_PATH: &str = "crates/omega_exo_episode/src/reset.rs";

/// OMEGA-DELTA-0090. The state comparison two forks are judged identical by.
pub const EPISODE_STATE_PATH: &str = "crates/omega_exo_episode/src/state.rs";

/// OMEGA-DELTA-0090. Single-writer claims on an Exo root.
pub const EPISODE_ROOT_PATH: &str = "crates/omega_exo_episode/src/root.rs";

/// OMEGA-DELTA-0090. Every source file of the episode crate.
///
/// Listed rather than walked, so a new module added to the crate has to be
/// added here too. A walk would silently include a file nobody reviewed, and
/// the scan below is exactly the kind that is worth failing loudly when the
/// surface grows.
pub const EPISODE_CRATE_SOURCES: &[&str] = &[
    EPISODE_LAW_PATH,
    EPISODE_FAMILY_PATH,
    "crates/omega_exo_episode/src/ids.rs",
    EPISODE_REQUEST_PATH,
    EPISODE_RESET_PATH,
    EPISODE_ROOT_PATH,
    EPISODE_STATE_PATH,
];

/// OMEGA-DELTA-0090. Vocabulary that would mean the episode crate can reach the
/// working tree.
///
/// The manual falsification loop this replaces destroyed uncommitted work in
/// two files with `git checkout --`. The crate's answer is not care: it is that
/// there is no path, no process, and no filesystem anywhere in it, so no
/// version of that mistake is expressible. Each token below is a way that would
/// stop being true.
pub const EPISODE_FORBIDDEN_REACH: &[&str] = &[
    "std::fs",
    "std::process",
    "std::path",
    "PathBuf",
    "Command",
    "include_str!",
    "env!",
    "std::env",
];

/// OMEGA-DELTA-0090. The request families an episode may send.
pub const EPISODE_ADMITTED_FAMILIES: &[&str] = &["Query", "Fork", "Reset"];

/// OMEGA-DELTA-0090. The request families an episode may never send.
///
/// `exo serve` has no authentication and answers the whole 52-variant protocol,
/// so the boundary that matters is not the endpoint, it is the family.
pub const EPISODE_REFUSED_FAMILIES: &[&str] = &["Write", "Secret"];

/// OMEGA-DELTA-0090. Exo request types the episode client must never name.
///
/// A sample rather than a full list — the family partition is what actually
/// decides. These are the ones whose appearance would be worst: appending to
/// somebody else's history, deleting their records, or reading their secrets.
pub const EPISODE_REFUSED_REQUEST_TYPES: &[&str] = &[
    "conversation_add_events",
    "turn_add_events",
    "turn_finish",
    "conversation_begin_turn",
    "delete_conversation",
    "delete_agent",
    "get_secret",
    "agent_get_secret",
    "conversation_get_secret",
    "conversation_put_secret",
];

/// OMEGA-DELTA-0090. The three event fields `conversation_fork` rewrites, and
/// therefore the only three an episode comparison may ignore.
///
/// Read off the replay loop in `BasicConversationHandle::fork`. A fourth entry
/// would make more episodes compare equal by ignoring more, which is a green
/// check that read less — the exact failure shape this delta exists to end.
pub const EPISODE_IDENTITY_FIELDS: &[&str] = &["id", "conversation_id", "created_at"];

/// OMEGA-DELTA-0090. The prefixes Exo's `fork` copies out of a conversation.
///
/// `snapshots` is deliberately absent: it is absent upstream, which is why the
/// filesystem half of the reset does not compose at the pin.
pub const EPISODE_FORK_COPIES_PREFIXES: &[&str] =
    &["bindings", "secrets", "artifacts", "sandboxes"];

/// OMEGA-DELTA-0048. The namespaces zero base hides that the three default
/// keymaps must still bind.
///
/// Each one is a surface the mode takes off the screen. If a later change
/// deletes one of these actions to make zero base simpler, the keymap that
/// still names it panics Omega at startup — which is the failure this list
/// exists to make loud, and the reason it is checked against the shipped keymap
/// files rather than against a memory of them.
pub const ZERO_BASE_HIDDEN_KEYMAP_NAMESPACES: &[&str] = &[
    "debugger::",
    "git::",
    "git_panel::",
    "outline_panel::",
    "pane::",
    "project_panel::",
    "search::",
    "terminal::",
    "workspace::",
];

/// OMEGA-DELTA-0042. The two Exos, which share only a name.
///
/// omega#86 was closed for targeting the wrong one — exo labs'
/// `exo-explore/exo` cluster-inference appliance — and omega#87 supersedes it
/// with `exoharness/exo`, the agent harness. A repository name is a cheap thing
/// to get wrong twice, so it is a checked fact rather than a remembered one.
pub const EXO_HARNESS_UPSTREAM: &str = "exoharness/exo";

/// OMEGA-DELTA-0042. The maintained fork can carry the ACP transport while
/// its contribution is under review upstream.
pub const EXO_HARNESS_MAINTAINED_FORK: &str = "OpenAgentsInc/exo";

/// OMEGA-DELTA-0042. The other Exo, which must appear nowhere as a target.
pub const EXO_CLUSTER_UPSTREAM: &str = "exo-explore";

/// OMEGA-DELTA-0042. Placeholders in `ADMITTED_LANE_ARGV` that stand for text
/// reachable from a person or a model.
///
/// Exo accepts its global options *after* the subcommand, so an unterminated
/// prompt is not a string Exo receives — it is Exo's command line. Driven
/// against the pinned Exo, a prompt of `--help` exits 0, prints usage, and runs
/// no turn.
pub const EXO_LANE_USER_TEXT_SLOTS: &[&str] = &["<agent>", "<conversation>", "<prompt>"];

/// OMEGA-DELTA-0042. Vocabulary that would mean Omega is standing between Exo's
/// unauthenticated endpoint and something else.
///
/// `exo serve` has no authentication and full secret access; its own
/// documentation says loopback is the entire boundary. Tier A needs no address
/// at all — the CLI reaches the state root on disk — so any of this appearing in
/// the lane is a surface nobody asked for.
pub const EXO_OFF_MACHINE_TOKENS: &[(&str, &str)] = &[
    ("a listener", "TcpListener"),
    ("a listener", "bind("),
    ("a listener", "0.0.0.0"),
    ("a proxy", "proxy"),
    ("a bearer token Exo never checks", "bearer"),
];

/// OMEGA-DELTA-0042. Flags that would point Exo somewhere other than the state
/// root on disk.
///
/// Checked against the *argv table* rather than the whole lane, because the
/// lane has to be able to name `EXO_EXOHARNESS_URL` in order to refuse an
/// off-loopback one it inherited from the environment. Omega passing the flag
/// and Omega refusing the variable are opposite acts that share a spelling.
pub const EXO_REDIRECTING_FLAGS: &[&str] = &["exoharness-url", "--url", "serve", "bearer-env"];

/// OMEGA-DELTA-0042, and owner gate 8 behind it.
///
/// *No model-initiated path may start Full Auto authority.* Three such paths
/// were removed from OpenAgents Desktop on 2026-07-25, one of them a rename of
/// another. An Exo agent has an unrestricted networked shell and can rebuild
/// itself, which makes it exactly the caller that gate exists for — so adding an
/// executor lane must not open a fourth door. The lane names none of this
/// vocabulary, and the check is that it cannot start naming it quietly.
pub const EXO_FULL_AUTO_TOKENS: &[&str] = &[
    "LaunchOrigin",
    "full_auto",
    "FullAuto",
    "PinGesture",
    "EngineLane",
    "run_ref: Some",
];

// ------ OMEGA-DELTA-0032

/// OMEGA-DELTA-0032. The law that decides what a send during a turn does.
pub const SEND_DURING_TURN_PATH: &str = "crates/omega_front_door/src/send_during_turn.rs";

/// OMEGA-DELTA-0032. The durable half of the queue.
pub const SEND_QUEUE_JOURNAL_PATH: &str = "crates/agent_ui/src/omega_send_queue.rs";

/// OMEGA-DELTA-0032. Where the composer decides what to do with a queued
/// message.
pub const CONVERSATION_SEND_PATH: &str = "crates/agent_ui/src/conversation_view/thread_view.rs";

/// OMEGA-DELTA-0032. Vocabulary that would make the queue law irreproducible,
/// for the same reason `NON_DETERMINISTIC_ROUTING_TOKENS` exists: a queue whose
/// decision depends on a clock or on hash order cannot be replayed from a
/// journal, and a journal that cannot be replayed is not a durable admission.
pub const NON_DETERMINISTIC_QUEUE_TOKENS: &[(&str, &str)] = &[
    ("a clock", "SystemTime"),
    ("a clock", "Instant"),
    ("a clock", "::now("),
    ("a clock", "chrono"),
    ("randomness", "rand::"),
    ("hash iteration order", "HashMap"),
    ("hash iteration order", "HashSet"),
    ("the environment", "std::env"),
];

/// OMEGA-DELTA-0032. Every executor class the send law must answer for.
pub const SEND_LAW_EXECUTOR_TOKENS: &[&str] = &["NativeLoop", "ExternalAcp", "EngineLane"];

// ------ OMEGA-DELTA-0091

/// OMEGA-DELTA-0091. Every source file of the read-only Exo log client.
///
/// Scanned as a set rather than one file, because "this crate cannot name a
/// write" is a property of the crate. A write kind moved into a helper module
/// would satisfy a check that only read the query type.
pub const EXO_LOG_SOURCE_PATHS: &[&str] = &[
    "crates/omega_exo_log/src/omega_exo_log.rs",
    EXO_LOG_ADMISSION_PATH,
    "crates/omega_exo_log/src/client.rs",
    "crates/omega_exo_log/src/history.rs",
    "crates/omega_exo_log/src/query.rs",
    "crates/omega_exo_log/src/record.rs",
];

/// OMEGA-DELTA-0091. The closed set of request kinds the client may name.
///
/// Eight of Exo's fifty-two. `OMEGA-DELTA-0102` holds this against the crate's
/// own decision by calling it, so this is the registry's independent statement
/// of what the client is for rather than a copy of the crate's answer.
pub const EXO_LOG_ADMITTED_KINDS: &[&str] = &[
    "get_agent",
    "agent_list_artifacts",
    "agent_read_artifact",
    "get_conversation",
    "conversation_get_events",
    "conversation_get_event",
    "conversation_list_artifacts",
    "conversation_read_artifact",
];

// ------ OMEGA-DELTA-0102

/// OMEGA-DELTA-0102. The one place Exo's request protocol is transcribed.
///
/// Two lanes landed on 2026-07-26 each holding its own copy of the same 52
/// variants — `omega_exo_episode::family` for the episode reset's admitted and
/// refused families, `omega_exo_log` for the eight reads its client may name.
/// The copies agreed exactly, which is the good case and is still one copy too
/// many: the next upstream variant would have to be noticed twice, by two
/// people who each already believed their list was complete.
pub const EXO_PROTOCOL_PATH: &str = "crates/omega_exo_lane/src/protocol.rs";

/// OMEGA-DELTA-0102. The read-only client's decision over that enumeration.
pub const EXO_LOG_ADMISSION_PATH: &str = "crates/omega_exo_log/src/admission.rs";

/// OMEGA-DELTA-0102. The two decisions taken over the one enumeration, each as
/// the file it lives in and the function that carries it.
///
/// Two, not one. They are deliberately not merged, because they admit different
/// subsets: the episode law admits `conversation_fork` because forking *is* the
/// episode reset, and the log client refuses it because that client is
/// read-only. A merge would hand one side a capability nobody granted it, and
/// `the_two_decisions_over_exos_protocol_are_not_merged` is what stops a later
/// tidy-up from performing one.
pub const EXO_PROTOCOL_DECISIONS: &[(&str, &str)] = &[
    (EPISODE_FAMILY_PATH, "family_of"),
    (EXO_LOG_ADMISSION_PATH, "is_admitted_read"),
];

/// OMEGA-DELTA-0102. Every production source that decides something about Exo's
/// protocol and must therefore hold no second copy of it.
///
/// The scan below asserts that none of these files contains a *string literal*
/// equal to one of Exo's request kinds. That is the mechanical statement of
/// "transcribed once": a file that spells `conversation_fork` for itself has
/// started a second list, whatever it calls it.
pub const EXO_PROTOCOL_CONSUMER_SOURCES: &[&str] = &[
    EPISODE_LAW_PATH,
    EPISODE_FAMILY_PATH,
    EPISODE_REQUEST_PATH,
    EPISODE_RESET_PATH,
    EPISODE_ROOT_PATH,
    EPISODE_STATE_PATH,
    "crates/omega_exo_episode/src/ids.rs",
    "crates/omega_exo_log/src/omega_exo_log.rs",
    EXO_LOG_ADMISSION_PATH,
    "crates/omega_exo_log/src/client.rs",
    "crates/omega_exo_log/src/history.rs",
    "crates/omega_exo_log/src/query.rs",
    "crates/omega_exo_log/src/record.rs",
];

/// OMEGA-DELTA-0102. Wildcard arms, which turn a total decision into a default.
///
/// Every decision over `ExoRequestKind` is written as a `match` with no
/// wildcard, so upstream's 53rd variant is a build failure on the person adding
/// it. A `_ =>` arm would make the same variant compile and be silently
/// classified — safe, in the sense that the default is refusal, and invisible,
/// which is how one protocol stayed transcribed twice in two crates for a day
/// without either lane being able to fix it mid-flight.
pub const EXO_DECISION_WILDCARD_ARMS: &[&str] = &["_ =>", "_ if"];

/// OMEGA-DELTA-0091. Headers and vocabulary that would assert an authentication
/// Exo does not have.
///
/// `exo serve` accepts a bearer token and never checks it — its own HTTP client
/// has `with_bearer_token`, and the server reads no `Authorization` header at
/// all. Sending one would leave a capture in which the endpoint looks protected.
pub const EXO_LOG_FALSE_AUTH_TOKENS: &[&str] = &["Authorization", "bearer", "Bearer"];

/// OMEGA-DELTA-0091. The type Exo-reported cost and token counts must keep.
pub const EXO_LOG_USAGE_TYPE: &str = "HarnessReportedUsage";

/// OMEGA-DELTA-0091. What every rendering of those numbers must say.
pub const EXO_LOG_USAGE_PROVENANCE: &str = "harness-reported";

#[cfg(test)]
mod tests {
    use super::*;

    /// OMEGA-DELTA-0001. Upstream Zed defaults this to `false` and shows the
    /// "Unrecognized Project" Restricted Mode modal. Omega never restricts.
    #[test]
    fn trust_all_worktrees_defaults_to_true() {
        let settings = default_settings().expect("default settings parse");
        let value = default_setting(&settings, "session.trust_all_worktrees")
            .expect("session.trust_all_worktrees is present in default settings");
        assert_eq!(
            value.as_bool(),
            Some(true),
            "OMEGA-DELTA-0001: Omega must default session.trust_all_worktrees to \
             true. Upstream Zed defaults it to false, which restricts \
             unrecognized projects and shows the trust prompt the owner \
             removed. If a rebase reverted this, restore it rather than \
             editing this test."
        );
    }

    /// OMEGA-DELTA-0002. Upstream Zed defaults to "confirm", which asks before
    /// every agent tool action. Omega runs agents unattended, where a prompt
    /// is a hang rather than a safeguard.
    #[test]
    fn agent_tool_permissions_default_to_allow() {
        let settings = default_settings().expect("default settings parse");
        let value = default_setting(&settings, "agent.tool_permissions.default")
            .expect("agent.tool_permissions.default is present in default settings");
        assert_eq!(
            value.as_str(),
            Some("allow"),
            "OMEGA-DELTA-0002: Omega must default agent.tool_permissions.default \
             to \"allow\". Upstream Zed defaults to \"confirm\", which blocks \
             unattended agent work entirely. Draw lines with always_confirm / \
             always_deny patterns instead of by reverting this default."
        );
    }

    /// OMEGA-DELTA-0003. Already correct upstream-divergent value, locked so a
    /// rebase cannot quietly reintroduce the quit prompt.
    #[test]
    fn quitting_is_never_confirmed() {
        let settings = default_settings().expect("default settings parse");
        assert_eq!(
            default_setting(&settings, "confirm_quit").and_then(serde_json::Value::as_bool),
            Some(false),
            "OMEGA-DELTA-0003: Omega must not ask for confirmation on quit."
        );
    }

    /// OMEGA-DELTA-0004. Telemetry stays off. This is a privacy posture, so it
    /// is locked rather than left to whatever upstream ships next.
    #[test]
    fn telemetry_stays_off() {
        let settings = default_settings().expect("default settings parse");
        for key in ["telemetry.diagnostics", "telemetry.metrics"] {
            assert_eq!(
                default_setting(&settings, key).and_then(serde_json::Value::as_bool),
                Some(false),
                "OMEGA-DELTA-0004: {key} must default to false in Omega."
            );
        }
    }

    /// The normalizer must not touch string contents. If it did, a setting
    /// whose value contains `//` or a comma would be silently corrupted and
    /// every check above would be reading the wrong document.
    #[test]
    fn the_normalizer_leaves_string_contents_alone() {
        let parsed: serde_json::Value = serde_json::from_str(&strip_jsonc(
            r#"{
                // a leading comment
                "url": "https://example.com/a//b",
                "text": "a, b",
                "glob": "**/*.pem",
                "trailing": [1, 2,],
            }"#,
        ))
        .expect("normalized JSONC parses");
        assert_eq!(parsed["url"], "https://example.com/a//b");
        assert_eq!(parsed["text"], "a, b");
        // default.json really does contain this shape; a naive block-comment
        // patch would treat the `/*` as an opening delimiter.
        assert_eq!(parsed["glob"], "**/*.pem");
        assert_eq!(parsed["trailing"], serde_json::json!([1, 2]));
    }

    /// A trailing comma separated from its brace by a comment is the case that
    /// a single-pass normalizer gets wrong.
    #[test]
    fn the_normalizer_handles_a_comma_then_comment_then_brace() {
        let parsed: serde_json::Value =
            serde_json::from_str(&strip_jsonc("{\n  \"a\": 1,\n  // trailing note\n}"))
                .expect("normalized JSONC parses");
        assert_eq!(parsed["a"], 1);
    }

    /// Block comments are not supported, and must fail loudly rather than
    /// truncate the document into something that still parses.
    #[test]
    fn a_block_comment_fails_closed() {
        let normalized = strip_jsonc("{ /* note */ \"a\": 1 }");
        assert!(
            serde_json::from_str::<serde_json::Value>(&normalized).is_err(),
            "an unsupported block comment must fail the check, never parse to \
             a different document"
        );
    }

    /// The delta check has to be able to fail, or it proves nothing.
    #[test]
    fn the_delta_check_detects_a_reverted_default() {
        let reverted: serde_json::Value = serde_json::from_str(&strip_jsonc(
            "{ \"session\": { \"trust_all_worktrees\": false } }",
        ))
        .expect("parses");
        assert_eq!(
            default_setting(&reverted, "session.trust_all_worktrees")
                .and_then(serde_json::Value::as_bool),
            Some(false),
            "the lookup used by the delta check must observe an upstream revert"
        );
    }

    /// OMEGA-DELTA-0005 and 0006. Deleted surfaces stay deleted.
    #[test]
    fn removed_surfaces_stay_removed() {
        for relative in REMOVED_FILES {
            let path = repository_path(relative);
            assert!(
                !path.exists(),
                "{relative} was deleted from Omega and has come back. If a \
                 rebase restored it, delete it again rather than editing this \
                 test; see OMEGA_DELTAS.md."
            );
        }
    }

    /// OMEGA-DELTA-0007. Terminating a debug session must not ask first.
    #[test]
    fn debug_terminate_never_prompts() {
        let path = repository_path("crates/debugger_ui/src/debugger_panel.rs");
        let source = std::fs::read_to_string(&path).expect("debugger panel is readable");
        assert!(
            !source.contains("Are you sure you want to terminate it?"),
            "OMEGA-DELTA-0007: the debug-session terminate confirmation has \
             returned to {}",
            path.display()
        );
    }

    /// OMEGA-DELTA-0008 and 0009. Strings that must not ship.
    ///
    /// Checked across the tree rather than per file: both of these survived a
    /// source-level review and were caught only by scanning the binary.
    #[test]
    fn no_zed_product_copy_survives_anywhere() {
        let crates_root = repository_path("crates");
        let mut offenders: Vec<String> = Vec::new();
        let mut stack = vec![crates_root];
        while let Some(directory) = stack.pop() {
            let Ok(entries) = std::fs::read_dir(&directory) else {
                continue;
            };
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_symlink() {
                    continue;
                }
                if path.is_dir() {
                    if path.file_name().is_some_and(|name| name == "target") {
                        continue;
                    }
                    stack.push(path);
                    continue;
                }
                if path.extension().is_none_or(|extension| extension != "rs") {
                    continue;
                }
                // This crate names the strings in order to forbid them.
                if path.ends_with("omega_deltas.rs") {
                    continue;
                }
                let Ok(source) = std::fs::read_to_string(&path) else {
                    continue;
                };
                for (delta, needle) in FORBIDDEN_SOURCE_STRINGS {
                    if source.contains(needle) {
                        offenders.push(format!("{delta}: {needle:?} in {}", path.display()));
                    }
                }
            }
        }
        assert!(
            offenders.is_empty(),
            "forbidden Zed product copy has returned:\n{}",
            offenders.join("\n")
        );
    }

    /// OMEGA-DELTA-0009. Restricted Mode cannot return through a surviving
    /// profile selector, language-server status, workspace action, or key binding.
    #[test]
    fn restricted_mode_ui_and_shortcuts_are_absent() {
        let mut offenders = Vec::new();
        for (relative_path, needle) in FORBIDDEN_RESTRICTED_MODE_UI {
            let path = repository_path(relative_path);
            let source = std::fs::read_to_string(&path)
                .unwrap_or_else(|error| panic!("cannot read {}: {error}", path.display()));
            if source.contains(needle) {
                offenders.push(format!("{needle:?} in {}", path.display()));
            }
        }
        assert!(
            offenders.is_empty(),
            "OMEGA-DELTA-0009 Restricted Mode UI or shortcuts returned:\n{}",
            offenders.join("\n")
        );
    }

    /// OMEGA-DELTA-0010. The title-bar identity entry stays local instead of
    /// invoking the inherited hosted-account sign-in flow.
    #[test]
    fn title_bar_identity_entry_opens_onboarding() {
        let path = repository_path("crates/title_bar/src/title_bar.rs");
        let source = std::fs::read_to_string(&path)
            .unwrap_or_else(|error| panic!("cannot read {}: {error}", path.display()));
        assert!(
            source.contains("Button::new(\"omega_identity\", \"Omega Identity\")")
                && source.contains("dispatch_action(OpenOnboarding.boxed_clone()"),
            "OMEGA-DELTA-0010: the title-bar identity entry must dispatch local Omega onboarding"
        );
        assert!(
            !source.contains("Button::new(\"sign_in\", \"Sign In\")")
                && !source.contains(".sign_in_with_optional_connect(true"),
            "OMEGA-DELTA-0010: the title bar must not restore hosted account sign-in"
        );
    }

    /// OMEGA-DELTA-0011. Agent onboarding keeps direct provider setup and
    /// excludes the inherited hosted account and plan path.
    #[test]
    fn ai_onboarding_is_provider_only() {
        let crate_root = repository_path("crates/ai_onboarding");
        let mut source = String::new();
        for relative_path in [
            "src/ai_onboarding.rs",
            "src/agent_api_keys_onboarding.rs",
            "src/agent_panel_onboarding_content.rs",
            "src/edit_prediction_onboarding_content.rs",
            "Cargo.toml",
        ] {
            let path = crate_root.join(relative_path);
            source.push_str(
                &std::fs::read_to_string(&path)
                    .unwrap_or_else(|error| panic!("cannot read {}: {error}", path.display())),
            );
        }
        let agent_panel_path = repository_path("crates/agent_ui/src/agent_panel.rs");
        source.push_str(
            &std::fs::read_to_string(&agent_panel_path).unwrap_or_else(|error| {
                panic!("cannot read {}: {error}", agent_panel_path.display())
            }),
        );

        for forbidden in [
            "ZedAiOnboarding",
            "Plan::Zed",
            "sign_in_with_optional_connect",
            "Hosted agent plans",
            "client.workspace = true",
            "cloud_api_types.workspace = true",
        ] {
            assert!(
                !source.contains(forbidden),
                "OMEGA-DELTA-0011: hosted onboarding path returned: {forbidden:?}"
            );
        }
        for required in [
            "Connect an AI provider",
            "Configure Providers",
            "zed_actions::agent::OpenSettings",
            "Configure edit predictions",
            "Configure Copilot",
        ] {
            assert!(
                source.contains(required),
                "OMEGA-DELTA-0011: required provider setup disappeared: {required:?}"
            );
        }
    }

    /// A deleted crate must not leave keybindings behind.
    ///
    /// The built-in keymap is loaded and unwrapped during startup, so a
    /// binding naming an action whose crate is gone panics before any window
    /// opens. The workspace still compiles, which is exactly why this needs
    /// its own check rather than trusting the build.
    #[test]
    fn keymaps_name_no_deleted_action() {
        let mut offenders: Vec<String> = Vec::new();
        for keymap in [
            "assets/keymaps/default-macos.json",
            "assets/keymaps/default-linux.json",
            "assets/keymaps/default-windows.json",
        ] {
            let path = repository_path(keymap);
            let Ok(source) = std::fs::read_to_string(&path) else {
                offenders.push(format!("{keymap} is unreadable"));
                continue;
            };
            for (delta, namespace) in FORBIDDEN_KEYMAP_NAMESPACES {
                if source.contains(namespace) {
                    offenders.push(format!("{delta}: {namespace:?} still bound in {keymap}"));
                }
            }
        }
        assert!(
            offenders.is_empty(),
            "a deleted crate left keybindings behind, which panics Omega at \
             startup:\n{}",
            offenders.join("\n")
        );
    }

    /// OMEGA-DELTA-0013. The agent ships reachable.
    ///
    /// `enabled: false` also strips the agent namespaces from the command
    /// palette, and the Settings UI exposes only `agent.button`, so a
    /// regression here makes the feature unreachable rather than merely off.
    #[test]
    fn the_agent_ships_enabled() {
        let settings = default_settings().expect("default settings parse");
        for key in ["agent.enabled", "agent.button"] {
            assert_eq!(
                default_setting(&settings, key).and_then(serde_json::Value::as_bool),
                Some(true),
                "OMEGA-DELTA-0013: {key} must default to true. With it false the \
                 agent is not reachable from the command palette or the panel, \
                 and no Settings control turns it back on."
            );
        }
    }

    /// OMEGA-DELTA-0013. The shipped default model is pinned by name.
    ///
    /// The service-isolation test asserts only that the default provider is
    /// `google`, because what it protects is that the default never points at
    /// a Zed service. That leaves the model string free: a rebase could swap
    /// `gemini-3.6-flash` for any other Google model and every existing check
    /// would still pass. The owner chose this model specifically, so it is
    /// pinned here by name.
    ///
    /// Changing the default model is a real decision. Update this constant and
    /// the `OMEGA-DELTA-0013` entry together, so the registry never disagrees
    /// with what ships.
    #[test]
    fn the_default_model_is_pinned() {
        const EXPECTED_PROVIDER: &str = "google";
        const EXPECTED_MODEL: &str = "gemini-3.6-flash";

        let settings = default_settings().expect("default settings parse");
        let default_model = default_setting(&settings, "agent.default_model")
            .expect("agent.default_model must be present in the shipped defaults");

        for (key, expected) in [("provider", EXPECTED_PROVIDER), ("model", EXPECTED_MODEL)] {
            assert_eq!(
                default_model.get(key).and_then(serde_json::Value::as_str),
                Some(expected),
                "OMEGA-DELTA-0013: agent.default_model.{key} must be \
                 {expected:?}. The owner selected {EXPECTED_PROVIDER}/\
                 {EXPECTED_MODEL} deliberately; the service-isolation test \
                 pins only the provider, so without this the model can change \
                 silently."
            );
        }
    }

    /// OMEGA-DELTA-0014. A protected recovery must not present a control whose
    /// label claims the protection has not happened.
    ///
    /// The behavioural assertion lives in the `onboarding` crate, where the
    /// presentation function is. This one is source-level so that it survives
    /// a rebase of that crate, and it also pins the behavioural test in place,
    /// because deleting the test is the cheapest way to revert the fix.
    #[test]
    fn protected_recovery_offers_a_different_action() {
        let path = repository_path("crates/onboarding/src/identity_section.rs");
        let source = std::fs::read_to_string(&path)
            .unwrap_or_else(|error| panic!("cannot read {}: {error}", path.display()));

        for required in [
            "ReplaceRecovery,",
            "\"Replace recovery file\"",
            "actions: vec![if needs_recovery {",
            "assert_ne!(protected.actions, needed.actions)",
        ] {
            assert!(
                source.contains(required),
                "OMEGA-DELTA-0014: the recovery-state split lost {required:?}. \
                 A protected identity must offer replacement, not protection; \
                 see omega#68."
            );
        }
        assert!(
            !source.contains("actions: vec![IdentityAction::Protect],"),
            "OMEGA-DELTA-0014: the Ready branch emits a constant Protect \
             action again, so a protected identity shows \"Protect recovery\" \
             beneath \"Recovery protected\". That is the omega#68 defect."
        );
    }

    /// OMEGA-DELTA-0015. The workroom binding exists, is unscoped, and names an
    /// action that still exists.
    ///
    /// Presence alone would not be enough. A keymap naming an undeclared action
    /// panics Omega before any window opens and compiles fine, which is how
    /// 0.2.0-rc6 shipped 27 dead bindings, so the action is resolved back to
    /// its declaration. The context is checked because a binding that only
    /// fires from one pane is the falsifier omega#69 named.
    #[test]
    fn required_keymap_bindings_resolve() {
        for binding in REQUIRED_KEYMAP_BINDINGS {
            let RequiredKeymapBinding {
                delta,
                keymap,
                keystroke,
                action,
                declared_in,
            } = binding;

            let path = repository_path(keymap);
            let raw = std::fs::read_to_string(&path)
                .unwrap_or_else(|error| panic!("cannot read {}: {error}", path.display()));
            let sections: serde_json::Value = serde_json::from_str(&strip_jsonc(&raw))
                .unwrap_or_else(|error| panic!("cannot parse {}: {error}", path.display()));
            let sections = sections
                .as_array()
                .unwrap_or_else(|| panic!("{keymap} is not an array of sections"));

            let bound: Vec<(Option<&str>, &serde_json::Value)> = sections
                .iter()
                .filter_map(|section| {
                    let binding = section.get("bindings")?.get(*keystroke)?;
                    Some((
                        section.get("context").and_then(serde_json::Value::as_str),
                        binding,
                    ))
                })
                .collect();
            assert_eq!(
                bound.len(),
                1,
                "{delta}: {keymap} must bind {keystroke:?} exactly once. A \
                 second, narrower binding shadows the global one depending on \
                 focus. Found: {bound:?}"
            );
            let (context, dispatched) = bound[0];
            assert_eq!(
                dispatched.as_str(),
                Some(*action),
                "{delta}: {keymap} must bind {keystroke:?} to {action:?}"
            );
            assert!(
                context.is_none_or(|context| WINDOW_GLOBAL_KEYMAP_CONTEXTS.contains(&context)),
                "{delta}: {keymap} binds {keystroke:?} in context {context:?}, \
                 which is narrower than the window. The binding must fire from \
                 an editor, a terminal, or any panel; a focus-dependent one is \
                 the omega#69 falsifier. Window-global contexts: \
                 {WINDOW_GLOBAL_KEYMAP_CONTEXTS:?}"
            );

            let declaration_path = repository_path(declared_in);
            let declaration = std::fs::read_to_string(&declaration_path).unwrap_or_else(|error| {
                panic!("cannot read {}: {error}", declaration_path.display())
            });
            assert!(
                declared_actions(&declaration).contains(*action),
                "{delta}: {keymap} binds {keystroke:?} to {action:?}, which is \
                 no longer declared in {declared_in}. The built-in keymap is \
                 unwrapped at startup, so this panics Omega before any window \
                 opens — the 0.2.0-rc6 failure."
            );
        }

        let (relative_path, menu_item) = SAVE_AS_MENU_ITEM;
        let path = repository_path(relative_path);
        let source = std::fs::read_to_string(&path)
            .unwrap_or_else(|error| panic!("cannot read {}: {error}", path.display()));
        assert!(
            source.contains(menu_item),
            "OMEGA-DELTA-0015: the workroom binding took the chord that was \
             workspace::SaveAs in all three default keymaps, leaving the File \
             menu as the only discoverable Save As on macOS and Windows. That \
             menu item has gone from {relative_path}, so the trade this delta \
             recorded is no longer the trade being made."
        );
    }

    /// The action parser has to actually find actions, or the resolvability
    /// check above passes on an empty set and proves nothing.
    #[test]
    fn the_action_parser_reaches_real_declarations() {
        let declared = declared_actions(
            "pub mod workroom {\n    use gpui::actions;\n    actions!(\n        workroom,\n \
             [\n            /// Opens it, and then, having opened it, focuses.\n            \
             OpenPanel,\n            FocusComposer\n        ]\n    );\n}",
        );
        assert!(declared.contains("workroom::OpenPanel"));
        assert!(declared.contains("workroom::FocusComposer"));
        assert!(
            !declared.contains("workroom::Opens"),
            "a doc comment must not be parsed as an action name"
        );
    }

    /// OMEGA-DELTA-0016. Aiur is one dark theme, and no light variant can be
    /// reintroduced without failing here.
    #[test]
    fn aiur_is_a_single_dark_theme() {
        let path = repository_path("assets/themes/aiur/aiur.json");
        let raw = std::fs::read_to_string(&path)
            .unwrap_or_else(|error| panic!("cannot read {}: {error}", path.display()));
        let family: serde_json::Value = serde_json::from_str(&strip_jsonc(&raw))
            .unwrap_or_else(|error| panic!("cannot parse {}: {error}", path.display()));

        assert_eq!(
            family.get("name").and_then(serde_json::Value::as_str),
            Some("Aiur"),
            "OMEGA-DELTA-0016: the shipped family must be named Aiur"
        );
        let themes = family
            .get("themes")
            .and_then(serde_json::Value::as_array)
            .expect("aiur.json declares a themes array");
        assert_eq!(
            themes.len(),
            1,
            "OMEGA-DELTA-0016: Aiur is dark-only, so it declares exactly one \
             theme. A second variant has returned; see omega#70."
        );
        assert_eq!(
            themes[0].get("name").and_then(serde_json::Value::as_str),
            Some("Aiur"),
            "OMEGA-DELTA-0016: the theme is named exactly Aiur, with no suffix"
        );
        assert_eq!(
            themes[0]
                .get("appearance")
                .and_then(serde_json::Value::as_str),
            Some("dark"),
            "OMEGA-DELTA-0016: Aiur is a dark theme"
        );

        let mut offenders: Vec<String> = Vec::new();
        for root in ["assets", "crates"] {
            for_each_source_file(
                &repository_path(root),
                &["rs", "json", "toml", "md"],
                |path, source| {
                    // This crate names the strings in order to forbid them.
                    if path.ends_with("omega_deltas.rs") {
                        return;
                    }
                    for needle in ["Aiur Light", "Aiur Dark"] {
                        if source.contains(needle) {
                            offenders.push(format!("{needle:?} in {}", path.display()));
                        }
                    }
                },
            );
        }
        assert!(
            offenders.is_empty(),
            "OMEGA-DELTA-0016: a suffixed Aiur name has returned:\n{}",
            offenders.join("\n")
        );
    }

    /// OMEGA-DELTA-0016. Both appearance defaults must name a theme Omega
    /// actually ships.
    ///
    /// This is omega#70's falsifier stated as a check: deleting a variant
    /// without repointing the default that named it fails here rather than at
    /// the owner's first light-mode launch.
    #[test]
    fn default_themes_exist_in_shipped_assets() {
        let shipped = shipped_theme_names().expect("shipped themes parse");
        assert!(
            shipped.contains("Aiur"),
            "OMEGA-DELTA-0016: Aiur must ship; found {shipped:?}"
        );

        let mut dark_defaults: Vec<(&str, String)> = Vec::new();
        for (relative_path, constant) in [
            (
                "crates/settings_content/src/theme.rs",
                "DEFAULT_LIGHT_THEME",
            ),
            ("crates/settings_content/src/theme.rs", "DEFAULT_DARK_THEME"),
            ("crates/theme/src/theme.rs", "DEFAULT_DARK_THEME"),
        ] {
            let path = repository_path(relative_path);
            let source = std::fs::read_to_string(&path)
                .unwrap_or_else(|error| panic!("cannot read {}: {error}", path.display()));
            let value = string_constant(&source, constant).unwrap_or_else(|| {
                panic!("{constant} is not declared as a string literal in {relative_path}")
            });
            assert!(
                shipped.contains(&value),
                "OMEGA-DELTA-0016: {constant} in {relative_path} names {value:?}, \
                 which no shipped theme declares. Omega would fall back to a \
                 missing theme on that appearance. Shipped: {shipped:?}"
            );
            if constant == "DEFAULT_DARK_THEME" {
                dark_defaults.push((relative_path, value));
            }
        }

        assert_eq!(
            dark_defaults[0].1, dark_defaults[1].1,
            "OMEGA-DELTA-0016: the two DEFAULT_DARK_THEME constants disagree \
             ({dark_defaults:?}); they are read by different crates and must \
             name the same theme."
        );
        assert_eq!(
            dark_defaults[0].1, "Aiur",
            "OMEGA-DELTA-0016: the dark default must be Aiur"
        );
    }

    /// OMEGA-DELTA-0016. The theme values that actually decide are the shipped
    /// ones, not the constants.
    ///
    /// `default_themes_exist_in_shipped_assets` reads `DEFAULT_LIGHT_THEME` and
    /// `DEFAULT_DARK_THEME`, which `theme_settings` consults only when no
    /// settings layer supplies a theme selection at all.
    /// `assets/settings/default.json` is the base layer and always supplies
    /// one, so it is what ships. A rebase restoring `"One Light"` / `"One Dark"`
    /// there would have shipped One Dark with every check green: One Dark is
    /// still a shipped theme, and both constants would still have said Aiur.
    ///
    /// Both shipped settings files are read, because the theme divergence lives
    /// in two of them and only one had ever been looked at.
    #[test]
    fn the_shipped_theme_defaults_are_the_omega_themes() {
        let shipped = shipped_theme_names().expect("shipped themes parse");

        for settings_file in SHIPPED_THEME_SETTINGS_FILES {
            let settings_path = repository_path(settings_file);
            let raw = std::fs::read_to_string(&settings_path)
                .unwrap_or_else(|error| panic!("cannot read {}: {error}", settings_path.display()));
            let settings: serde_json::Value = serde_json::from_str(&strip_jsonc(&raw))
                .unwrap_or_else(|error| {
                    panic!("cannot parse {}: {error}", settings_path.display())
                });

            for (key, relative_path, constant) in [
                (
                    "theme.light",
                    "crates/settings_content/src/theme.rs",
                    "DEFAULT_LIGHT_THEME",
                ),
                (
                    "theme.dark",
                    "crates/settings_content/src/theme.rs",
                    "DEFAULT_DARK_THEME",
                ),
            ] {
                let configured = default_setting(&settings, key)
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or_else(|| {
                        panic!("OMEGA-DELTA-0016: {settings_file} no longer names {key}")
                    });
                assert!(
                    shipped.contains(configured),
                    "OMEGA-DELTA-0016: {settings_file} sets {key} to \
                     {configured:?}, which no theme under assets/themes/ \
                     declares. Omega would resolve to a missing theme on that \
                     appearance. Shipped: {shipped:?}"
                );

                let path = repository_path(relative_path);
                let source = std::fs::read_to_string(&path)
                    .unwrap_or_else(|error| panic!("cannot read {}: {error}", path.display()));
                let declared = string_constant(&source, constant).unwrap_or_else(|| {
                    panic!("{constant} is not declared as a string literal in {relative_path}")
                });
                assert_eq!(
                    configured, declared,
                    "OMEGA-DELTA-0016: {settings_file} sets {key} to \
                     {configured:?} but {constant} in {relative_path} is \
                     {declared:?}. These are two mechanisms for one decision — \
                     the setting decides, the constant is the fallback for an \
                     absent selection — and a rebase that reverts one and not \
                     the other ships the reverted one silently."
                );
            }
        }
    }

    /// OMEGA-DELTA-0026. The shipped defaults still point away from Zed's
    /// production hosts.
    ///
    /// Four values from one commit, `9e585569cb`. They are asserted here as
    /// well as in `SERVICE_ISOLATION_TEST_PATH` because `cargo test -p
    /// omega_deltas` is what the registry tells a reader to run: a delta whose
    /// only value assertion lives in another crate is green under the command
    /// this file documents.
    #[test]
    fn the_service_isolation_defaults_are_still_the_omega_values() {
        let settings = default_settings().expect("default settings parse");
        for (key, upstream, omega) in SERVICE_ISOLATION_DEFAULTS {
            let expected: serde_json::Value =
                serde_json::from_str(omega).expect("the recorded Omega value is JSON");
            let actual = default_setting(&settings, key).unwrap_or_else(|| {
                panic!("OMEGA-DELTA-0026: {key} is absent from the shipped defaults")
            });
            assert_eq!(
                actual, &expected,
                "OMEGA-DELTA-0026: {key} must default to {omega}. Upstream Zed \
                 ships {upstream}, which points a running Omega at one of Zed's \
                 production hosts — for auto_update, at the one that can replace \
                 the binary."
            );
        }
    }

    /// OMEGA-DELTA-0027. The Codex ACP executor is configured out of the box.
    ///
    /// Full Auto dispatches to `codex-acp`, so an empty `agent_servers` is not
    /// a neutral default here: it is Full Auto failing on a missing agent.
    #[test]
    fn codex_acp_is_configured_by_default() {
        let settings = default_settings().expect("default settings parse");
        let entry = default_setting(&settings, "agent_servers.codex-acp").unwrap_or_else(|| {
            panic!(
                "OMEGA-DELTA-0027: agent_servers no longer declares codex-acp. \
                 Upstream Zed ships agent_servers as {{}}; Full Auto is routed \
                 through this agent, so an empty map is a Full Auto run that \
                 cannot start."
            )
        });
        assert_eq!(
            entry.get("type").and_then(serde_json::Value::as_str),
            Some("registry"),
            "OMEGA-DELTA-0027: codex-acp must resolve from the ACP registry. \
             Any other type is a different agent than the one the endpoint \
             allow-list approved."
        );
    }

    /// OMEGA-DELTA-0026 and OMEGA-DELTA-0027. The cited check still asserts
    /// what the registry says it asserts.
    ///
    /// Citing an existing check rather than duplicating it is right, and it
    /// adds a failure mode: the cited assertion can be deleted, and deleting an
    /// assertion turns a test green. This also requires the delta ID to appear
    /// beside it, so a reader who finds the assertion finds the reason.
    #[test]
    fn the_service_isolation_test_still_asserts_the_registered_defaults() {
        let path = repository_path(SERVICE_ISOLATION_TEST_PATH);
        let source = std::fs::read_to_string(&path)
            .unwrap_or_else(|error| panic!("cannot read {}: {error}", path.display()));
        let compact = without_whitespace(&source);

        for (delta, assertion) in PINNED_SERVICE_ISOLATION_ASSERTIONS {
            assert!(
                compact.contains(&without_whitespace(assertion)),
                "{delta}: {SERVICE_ISOLATION_TEST_PATH} no longer asserts \
                 {assertion:?}. That is the check the registry entry cites, and \
                 it reads as off-topic inside a test named for Zed service \
                 isolation, so it is the first line a tidy-up drops."
            );
            assert!(
                source.contains(delta),
                "{delta}: {SERVICE_ISOLATION_TEST_PATH} carries an assertion \
                 this delta cites but never names the delta, so a reader who \
                 finds the assertion cannot find the reason for it."
            );
        }
    }

    /// OMEGA-DELTA-0028. The default icon theme is Omega's own, in both places
    /// that have to agree about its name.
    ///
    /// `configured_icon_theme` looks the settings value up in the registry, and
    /// the only built-in icon theme is registered under
    /// `DEFAULT_ICON_THEME_NAME`. Reverting one and not the other does not
    /// break anything visible — the lookup misses, the error is logged, the
    /// fallback renders — so the product keeps working while the settings file
    /// the owner opens names a competitor. Agreement is therefore the check,
    /// and the brand rule catches a revert of both.
    #[test]
    fn the_default_icon_theme_is_omegas() {
        let policy = brand_policy().expect("brand gate policy parses");
        let settings = default_settings().expect("default settings parse");
        let configured = default_setting(&settings, "icon_theme")
            .and_then(serde_json::Value::as_str)
            .expect("icon_theme is present in the shipped defaults");

        let path = repository_path(DEFAULT_ICON_THEME_SOURCE);
        let source = std::fs::read_to_string(&path)
            .unwrap_or_else(|error| panic!("cannot read {}: {error}", path.display()));
        let declared = string_constant(&source, "DEFAULT_ICON_THEME_NAME").unwrap_or_else(|| {
            panic!("DEFAULT_ICON_THEME_NAME is not a string literal in {DEFAULT_ICON_THEME_SOURCE}")
        });

        assert_eq!(
            configured, declared,
            "OMEGA-DELTA-0028: the shipped icon_theme is {configured:?} but \
             DEFAULT_ICON_THEME_NAME in {DEFAULT_ICON_THEME_SOURCE} is \
             {declared:?}. The registry has exactly one built-in icon theme, \
             registered under the constant, so a settings value that disagrees \
             misses the lookup and falls back with a logged error rather than \
             failing."
        );

        for (what, value) in [
            ("the shipped icon_theme setting", configured),
            ("DEFAULT_ICON_THEME_NAME", declared.as_str()),
        ] {
            let hits = brand_hits(value, &policy);
            assert!(
                hits.is_empty(),
                "OMEGA-DELTA-0028: {what} is {value:?}, which names {hits:?}. \
                 The icon theme name is rendered by the icon-theme selector and \
                 written into the settings file the owner opens, so it is \
                 product copy. base_keymap: \"Zed\" stays and is why this reads \
                 one key rather than scanning the file: that value names Zed's \
                 keybinding scheme, offered beside VS Code and JetBrains."
            );
        }
    }

    /// OMEGA-DELTA-0017. No merged `Info.plist` value names a competitor.
    ///
    /// Every file in the fragment directory is merged into the packaged
    /// `Info.plist` by cargo-bundle, so the directory is walked rather than a
    /// list of known files, and every `<string>` is read rather than a list of
    /// known keys. `0.2.0-rc10` shipped thirteen of these signed and
    /// notarized, and macOS renders `NS*UsageDescription` inside its own
    /// permission dialog: the operating system told the owner that an
    /// application in Zed wanted the microphone.
    #[test]
    fn no_info_plist_value_names_a_competitor() {
        let policy = brand_policy().expect("brand gate policy parses");
        let directory = repository_path(
            policy["info_plist"]["fragment_dir"]
                .as_str()
                .expect("info_plist.fragment_dir is a string"),
        );
        let mut fragments: Vec<std::path::PathBuf> = std::fs::read_dir(&directory)
            .unwrap_or_else(|error| panic!("cannot read {}: {error}", directory.display()))
            .flatten()
            .map(|entry| entry.path())
            .filter(|path| path.is_file())
            .collect();
        fragments.sort();
        assert!(
            !fragments.is_empty(),
            "OMEGA-DELTA-0017: {} is empty, so this check would be vacuous",
            directory.display()
        );

        let mut offenders: Vec<String> = Vec::new();
        for path in &fragments {
            let source = std::fs::read_to_string(path)
                .unwrap_or_else(|error| panic!("cannot read {}: {error}", path.display()));
            for (key, value) in plist_fragment_values(&source) {
                for hit in brand_hits(&value, &policy) {
                    offenders.push(format!("{}:{key} names {hit:?}: {value:?}", path.display()));
                }
            }
            // A brand name in a key, comment or attribute would not appear in
            // a dialog, but it is still a rebase restoring upstream identity.
            for hit in brand_hits(&without_elements(&source, "string"), &policy) {
                offenders.push(format!(
                    "{} names {hit:?} outside a <string> value",
                    path.display()
                ));
            }
        }
        assert!(
            offenders.is_empty(),
            "OMEGA-DELTA-0017: a competitor's name is back in the packaged \
             Info.plist:\n{}",
            offenders.join("\n")
        );
    }

    /// The plist parser has to reach real values, or the check above passes on
    /// an empty list and reports a clean tree that was never read.
    #[test]
    fn the_plist_fragment_parser_reaches_real_values() {
        let policy = brand_policy().expect("brand gate policy parses");
        let path = repository_path("crates/zed/resources/info/Permissions.plist");
        let source = std::fs::read_to_string(&path).expect("Permissions.plist is readable");
        let values = plist_fragment_values(&source);
        let microphone = values
            .iter()
            .find(|(key, _)| key == "NSMicrophoneUsageDescription")
            .map(|(_, value)| value.clone())
            .expect("NSMicrophoneUsageDescription is present and keyed");
        assert!(
            microphone.contains("Omega"),
            "the microphone permission dialog must name Omega, got {microphone:?}"
        );
        assert!(
            !brand_hits(
                "An application in Zed wants to use your microphone.",
                &policy
            )
            .is_empty(),
            "the brand matcher must still recognise the string 0.2.0-rc10 shipped"
        );
        assert!(
            brand_hits(
                "The request was not authorized and is now normalized.",
                &policy
            )
            .is_empty(),
            "the brand matcher must not fire on 'authorized' or 'normalized'"
        );
    }

    /// OMEGA-DELTA-0018. No shipped icon carries a competitor's name.
    ///
    /// `assets/icons` and the `IconName` enum are a bijection, enforced by the
    /// icons crate's own `test_all_icons_exist` and `test_no_dangling_icons`,
    /// so scanning both is a complete inventory of every icon that can ship.
    /// Both halves are checked because renaming only the file leaves the next
    /// rebase an identifier to restore the artwork under.
    #[test]
    fn no_shipped_icon_carries_a_competitor_name() {
        let policy = brand_policy().expect("brand gate policy parses");
        let icons = &policy["icons"];
        let allowed: std::collections::BTreeSet<String> = icons["third_party_allowances"]
            .as_object()
            .expect("icons.third_party_allowances is an object")
            .keys()
            .cloned()
            .collect();
        let stem_tokens: Vec<String> = policy_strings(&policy, "icons", "forbidden_stem_tokens");
        let variant_tokens: Vec<String> =
            policy_strings(&policy, "icons", "forbidden_variant_tokens");
        assert!(
            !stem_tokens.is_empty() && !variant_tokens.is_empty(),
            "OMEGA-DELTA-0018: an empty token list makes this check vacuous"
        );

        let asset_dir = repository_path(icons["asset_dir"].as_str().expect("icons.asset_dir"));
        let mut stems: Vec<String> = std::fs::read_dir(&asset_dir)
            .unwrap_or_else(|error| panic!("cannot read {}: {error}", asset_dir.display()))
            .flatten()
            .map(|entry| entry.path())
            .filter(|path| path.extension().is_some_and(|extension| extension == "svg"))
            .filter_map(|path| path.file_stem()?.to_str().map(str::to_owned))
            .collect();
        stems.sort();
        assert!(
            !stems.is_empty(),
            "OMEGA-DELTA-0018: {} holds no icons, so this check would be vacuous",
            asset_dir.display()
        );

        let mut offenders: Vec<String> = Vec::new();
        for stem in &stems {
            if allowed.contains(stem) {
                continue;
            }
            let lowered = stem.to_lowercase();
            if stem_tokens.iter().any(|token| lowered.contains(token)) {
                offenders.push(format!("asset {stem}.svg"));
            }
        }

        let enum_source =
            repository_path(icons["enum_source"].as_str().expect("icons.enum_source"));
        let source = std::fs::read_to_string(&enum_source)
            .unwrap_or_else(|error| panic!("cannot read {}: {error}", enum_source.display()));
        let variants = icon_name_variants(&source);
        assert!(
            variants.len() > 100,
            "OMEGA-DELTA-0018: parsed {} IconName variants, which means the \
             enum parser broke and this check is vacuous",
            variants.len()
        );
        for variant in &variants {
            if allowed.contains(&icon_stem(variant)) {
                continue;
            }
            if variant_tokens.iter().any(|token| variant.contains(token)) {
                offenders.push(format!("IconName::{variant}"));
            }
        }

        assert!(
            offenders.is_empty(),
            "OMEGA-DELTA-0018: a competitor's name is back in the shipped icon \
             set: {offenders:?}. Replace the artwork and the name, or record in \
             {} why it identifies somebody else's product.",
            BRAND_GATE_POLICY_PATH
        );
    }

    /// OMEGA-DELTA-0018. The Omega marks are the artwork that was looked at.
    ///
    /// A name check cannot see a logo. Three Zed marks shipped on the status
    /// bar of `0.2.0-rc10` while every brand check on omega#16 passed, because
    /// all of them were string comparisons. Pinning the bytes is the only part
    /// of this delta that can tell one drawing from another, so swapping the
    /// artwork back under an Omega file name fails here.
    #[test]
    fn the_omega_marks_are_the_reviewed_artwork() {
        let policy = brand_policy().expect("brand gate policy parses");
        let marks = policy["icons"]["reviewed_marks"]
            .as_object()
            .expect("icons.reviewed_marks is an object");
        assert!(
            !marks.is_empty(),
            "OMEGA-DELTA-0018: no artwork is pinned, so nothing checks the marks"
        );

        let mut offenders: Vec<String> = Vec::new();
        for (relative, expected) in marks {
            let expected = expected.as_str().expect("a pinned digest is a string");
            let path = repository_path(relative);
            let Ok(bytes) = std::fs::read(&path) else {
                offenders.push(format!("{relative} is missing"));
                continue;
            };
            let actual = sha256_hex(&bytes);
            if actual != expected {
                offenders.push(format!("{relative}: {actual} != {expected}"));
            }
        }
        assert!(
            offenders.is_empty(),
            "OMEGA-DELTA-0018: shipped Omega artwork is not the reviewed \
             artwork:\n{}\nIf the change is deliberate, look at the rendered \
             icon and update the pin in {} in the same commit.",
            offenders.join("\n"),
            BRAND_GATE_POLICY_PATH
        );
    }

    /// OMEGA-DELTA-0022. No file `rust-embed` ships names a competitor.
    ///
    /// The inventory is the assets tree plus every directory an embed
    /// declaration points at, not a list of asset directories. `0.2.0-rc11`
    /// shipped `assets/images/zed_logo.svg` and `zed_x_copilot.svg` embedded in
    /// its signed binary while `OMEGA-DELTA-0018` reported `assets/icons/`
    /// clean — truthfully, because `assets/images/` was in no list.
    #[test]
    fn no_embedded_asset_carries_a_competitor_name() {
        let policy = brand_policy().expect("brand gate policy parses");
        let embedded = &policy["embedded_assets"];
        let allowed: std::collections::BTreeSet<String> = embedded["third_party_allowances"]
            .as_object()
            .expect("embedded_assets.third_party_allowances is an object")
            .keys()
            .cloned()
            .collect();
        let tokens: Vec<String> =
            policy_strings(&policy, "embedded_assets", "forbidden_path_tokens")
                .iter()
                .map(|token| token.to_lowercase())
                .collect();
        assert!(
            !tokens.is_empty(),
            "OMEGA-DELTA-0022: an empty token list makes this check vacuous"
        );

        let folders = embed_folders();
        assert!(
            !folders.is_empty(),
            "OMEGA-DELTA-0022: no rust-embed #[folder] declarations were found, \
             so the inventory is no longer derived from what decides what ships"
        );

        let inventory = embedded_asset_inventory();
        let minimum = usize::try_from(
            embedded["minimum_inventory"]
                .as_u64()
                .expect("embedded_assets.minimum_inventory is a number"),
        )
        .expect("minimum_inventory fits in usize");
        assert!(
            inventory.len() >= minimum,
            "OMEGA-DELTA-0022: only {} embeddable files were found, below the \
             {minimum} floor. The walk broke and this check is reporting green \
             about nothing.",
            inventory.len()
        );

        let offenders: Vec<&String> = inventory
            .iter()
            .filter(|relative| !allowed.contains(*relative))
            .filter(|relative| {
                let lowered = relative.to_lowercase();
                tokens.iter().any(|token| lowered.contains(token))
            })
            .collect();
        assert!(
            offenders.is_empty(),
            "OMEGA-DELTA-0022: a competitor's name is back in a directory \
             rust-embed ships: {offenders:?}. Replace the artwork and the file \
             name, or record in {BRAND_GATE_POLICY_PATH} why it identifies \
             somebody else's product."
        );
    }

    /// OMEGA-DELTA-0022. No enum that names a shipped asset names a competitor.
    ///
    /// The enums are discovered from the source. Listing them is what left
    /// `VectorName` outside `OMEGA-DELTA-0018` while `IconName` was inside it,
    /// which is the entire reason `VectorName::ZedLogo` reached rc11.
    #[test]
    fn no_asset_name_enum_carries_a_competitor_name() {
        let policy = brand_policy().expect("brand gate policy parses");
        let section = &policy["asset_name_enums"];
        let allowed: std::collections::BTreeSet<String> = section["third_party_allowances"]
            .as_object()
            .expect("asset_name_enums.third_party_allowances is an object")
            .keys()
            .cloned()
            .collect();
        let tokens: Vec<String> =
            policy_strings(&policy, "asset_name_enums", "forbidden_variant_tokens");
        assert!(
            !tokens.is_empty(),
            "OMEGA-DELTA-0022: an empty token list makes this check vacuous"
        );

        let discovered = asset_name_enums();
        for required in policy_strings(&policy, "asset_name_enums", "required_discoveries") {
            assert!(
                discovered.contains_key(&required),
                "OMEGA-DELTA-0022: the asset-name-enum discovery no longer finds \
                 {required}. The parser broke and this check is reporting green \
                 about nothing. Found: {:?}",
                discovered.keys().collect::<Vec<_>>()
            );
        }

        let mut offenders: Vec<String> = Vec::new();
        for (name, variants) in &discovered {
            assert!(
                variants.len() >= 2,
                "OMEGA-DELTA-0022: {name} parsed as {} variants",
                variants.len()
            );
            for variant in variants {
                let label = format!("{name}::{variant}");
                if allowed.contains(&label) {
                    continue;
                }
                if tokens.iter().any(|token| variant.contains(token)) {
                    offenders.push(label);
                }
            }
        }
        assert!(
            offenders.is_empty(),
            "OMEGA-DELTA-0022: an identifier that resolves to a shipped asset \
             names a competitor: {offenders:?}. Renaming the file without \
             renaming the identifier leaves the next rebase a name to restore \
             the artwork under."
        );
    }

    /// OMEGA-DELTA-0022. No command-palette label names a competitor.
    ///
    /// The palette renders `namespace: action name` for every registered
    /// action. `0.2.0-rc11` offered `zed: about`, `zed: quit` and
    /// `zed: get merch` while the compatibility allow-list recorded that
    /// namespace as "not user-facing product copy". The targets were already
    /// correct; the label was the defect, and nothing had ever read an action
    /// declaration.
    #[test]
    fn no_command_palette_label_names_a_competitor() {
        let policy = brand_policy().expect("brand gate policy parses");
        let section = &policy["actions"];
        let declarations = action_declarations();
        let minimum = usize::try_from(
            section["minimum_inventory"]
                .as_u64()
                .expect("actions.minimum_inventory is a number"),
        )
        .expect("minimum_inventory fits in usize");
        assert!(
            declarations.len() >= minimum,
            "OMEGA-DELTA-0022: only {} action declarations were parsed, below \
             the {minimum} floor; this check is vacuous",
            declarations.len()
        );

        let labels: std::collections::BTreeSet<String> = declarations
            .iter()
            .map(|(namespace, name, _)| format!("{namespace}::{name}"))
            .collect();
        for required in policy_strings(&policy, "actions", "required_actions") {
            assert!(
                labels.contains(&required),
                "OMEGA-DELTA-0022: {required} is not in the parsed action set, \
                 so the parser is not reading the declarations it claims to"
            );
        }

        let allowed: std::collections::BTreeSet<String> = section["third_party_allowances"]
            .as_object()
            .expect("actions.third_party_allowances is an object")
            .keys()
            .cloned()
            .collect();
        let namespace_tokens: Vec<String> =
            policy_strings(&policy, "actions", "forbidden_namespace_tokens")
                .iter()
                .map(|token| token.to_lowercase())
                .collect();
        let name_tokens: Vec<String> = policy_strings(&policy, "actions", "forbidden_name_tokens");
        assert!(
            !namespace_tokens.is_empty() && !name_tokens.is_empty(),
            "OMEGA-DELTA-0022: an empty token list makes this check vacuous"
        );

        let mut offenders: Vec<String> = Vec::new();
        for (namespace, name, file) in &declarations {
            if allowed.contains(&format!("{namespace}::{name}")) {
                continue;
            }
            let lowered = namespace.to_lowercase();
            if namespace_tokens.iter().any(|token| lowered.contains(token)) {
                offenders.push(format!("{namespace}: … (declared in {file})"));
            }
            if name_tokens.iter().any(|token| name.contains(token)) {
                offenders.push(format!("{namespace}::{name} (declared in {file})"));
            }
        }
        offenders.sort();
        offenders.dedup();
        assert!(
            offenders.is_empty(),
            "OMEGA-DELTA-0022: the command palette would offer a competitor's \
             name to the owner: {offenders:?}. An action namespace is a \
             user-facing label, not an internal seam. Rename it and keep the \
             old name as a deprecated alias so existing keymaps resolve."
        );
    }

    /// OMEGA-DELTA-0022. Renaming an action keeps existing keymaps working.
    ///
    /// The built-in keymap is loaded and unwrapped at startup, so an
    /// unresolvable action is a hard panic before any window opens — the same
    /// shape that shipped in `0.2.0-rc6`. Every `zed::` name the namespace
    /// rename retired therefore has to survive as a deprecated alias.
    #[test]
    fn the_retired_action_namespace_still_resolves() {
        let aliases = deprecated_action_aliases();
        for retired in [
            "zed::About",
            "zed::Quit",
            "zed::GetMerch",
            "zed::OpenSettings",
            "zed::NoAction",
            "zed::Unbind",
        ] {
            assert!(
                aliases.contains(retired),
                "OMEGA-DELTA-0022: {retired} is not a deprecated alias any more. \
                 An existing user keymap naming it would stop resolving, and an \
                 unresolvable action in the built-in keymap panics at startup."
            );
        }

        let keymaps = repository_path("assets/keymaps");
        let mut offenders = Vec::new();
        for_each_source_file(&keymaps, &["json"], |path, source| {
            if source.contains("\"zed::") {
                offenders.push(path.display().to_string());
            }
        });
        assert!(
            offenders.is_empty(),
            "OMEGA-DELTA-0022: a shipped keymap still dispatches a retired \
             `zed::` action: {offenders:?}. Aliases exist for the owner's own \
             keymap, not for ours."
        );
    }

    /// OMEGA-DELTA-0022. A blocked public claim appears nowhere in the tree.
    ///
    /// The compatibility allow-list has recorded `blocked` claims since the
    /// identity work began, but nothing read them back against the source, so
    /// `Welcome to Zed` and `About Zed` were listed while
    /// `Use GitHub Copilot in Zed` shipped in `0.2.0-rc11` as a window title,
    /// as a modal `Headline`, and beside the Zed x Copilot lockup — one
    /// surface, three presentations, in no entry at all.
    #[test]
    fn blocked_public_copy_appears_nowhere_in_the_tree() {
        let path = repository_path(COMPATIBILITY_ALLOWLIST_PATH);
        let raw = std::fs::read_to_string(&path)
            .unwrap_or_else(|error| panic!("cannot read {}: {error}", path.display()));
        let allowlist: serde_json::Value =
            serde_json::from_str(&raw).expect("compatibility allow-list parses");
        let blocked: Vec<String> = allowlist["entries"]
            .as_array()
            .expect("the allow-list has entries")
            .iter()
            .filter(|entry| entry["disposition"] == "blocked")
            .filter_map(|entry| entry["match"].as_str())
            .map(str::to_owned)
            .collect();
        assert!(
            blocked.len() >= 4,
            "OMEGA-DELTA-0022: only {} blocked claims are recorded, so this \
             check has almost nothing to enforce",
            blocked.len()
        );
        assert!(
            blocked
                .iter()
                .any(|claim| claim == "Use GitHub Copilot in Zed"),
            "OMEGA-DELTA-0022: the claim that shipped in rc11 must stay recorded \
             as blocked, or the next lane has no record that it was a defect"
        );

        let corpus: std::collections::BTreeSet<&str> =
            BLOCKED_COPY_CORPUS.iter().copied().collect();
        for relative in &corpus {
            assert!(
                repository_path(relative).is_file(),
                "OMEGA-DELTA-0022: exempt corpus file {relative} does not exist, \
                 so the exemption is hiding nothing and should be deleted"
            );
        }

        let mut offenders: Vec<String> = Vec::new();
        for directory in ["crates", "assets"] {
            let root = repository_path(directory);
            for_each_source_file(&root, &["rs", "json", "md", "toml"], |path, source| {
                let normalized = normalize_path(path);
                let relative = normalized
                    .strip_prefix(normalize_path(&repository_path(".")))
                    .map_or_else(
                        |_| normalized.display().to_string(),
                        |tail| tail.display().to_string(),
                    );
                if corpus.contains(relative.as_str()) {
                    return;
                }
                for claim in &blocked {
                    if source.contains(claim.as_str()) {
                        offenders.push(format!("{relative}: {claim:?}"));
                    }
                }
            });
        }
        offenders.sort();
        assert!(
            offenders.is_empty(),
            "OMEGA-DELTA-0022: a public product claim the allow-list records as \
             blocked is back in the tree:\n{}",
            offenders.join("\n")
        );
    }

    /// OMEGA-DELTA-0022. The component preview is not in the release palette.
    ///
    /// It renders every component's developer-authored `preview` fn, which is
    /// not reviewed as product copy or product artwork. It shipped in the
    /// release palette of `0.2.0-rc11` with no dev gate — unlike
    /// `dev::ToggleInspector` and `dev::ResetOnboarding`, both of which are
    /// gated — and opening it drew the Zed `Z` through the `Vector` preview.
    #[test]
    fn the_component_preview_is_gated_to_dev_builds() {
        let path = repository_path("crates/component_preview/src/component_preview.rs");
        let source = std::fs::read_to_string(&path)
            .unwrap_or_else(|error| panic!("cannot read {}: {error}", path.display()));
        for required in [
            "#[cfg(not(debug_assertions))]",
            "hide_action_types",
            "OpenComponentPreview",
        ] {
            assert!(
                source.contains(required),
                "OMEGA-DELTA-0022: {} no longer contains {required:?}, so a \
                 release build would offer `workspace: open component preview` \
                 and render unreviewed developer previews to the owner.",
                path.display()
            );
        }
    }

    /// OMEGA-DELTA-0017 and OMEGA-DELTA-0018. The packaging path runs the gate.
    ///
    /// The source checks above cannot see a packaging regression, and the
    /// packaged gate cannot run if nothing calls it. `0.2.0-rc6` hard-panicked
    /// at startup on assets that `cargo check --workspace` had been happy with
    /// all along, which is the same shape of gap. This asserts the wiring
    /// itself, so deleting the call from the bundle script fails here.
    #[test]
    fn the_packaging_path_runs_the_brand_gate() {
        let verifier = repository_path(BRAND_VERIFIER_PATH);
        assert!(
            verifier.is_file(),
            "OMEGA-DELTA-0017: {BRAND_VERIFIER_PATH} is missing"
        );
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            let mode = std::fs::metadata(&verifier)
                .expect("brand verifier metadata")
                .permissions()
                .mode();
            assert!(
                mode & 0o111 != 0,
                "OMEGA-DELTA-0017: {BRAND_VERIFIER_PATH} is not executable, so \
                 the packaging path cannot run it"
            );
        }

        let bundle = repository_path(RC_BUNDLE_SCRIPT_PATH);
        let script = std::fs::read_to_string(&bundle)
            .unwrap_or_else(|error| panic!("cannot read {}: {error}", bundle.display()));
        for required in [
            "verify_source_brand\n",
            "verify_packaged_brand \"${app_path}\"\n",
            BRAND_VERIFIER_PATH,
            BRAND_GATE_POLICY_PATH,
        ] {
            assert!(
                script.contains(required),
                "OMEGA-DELTA-0017: {RC_BUNDLE_SCRIPT_PATH} no longer contains \
                 {required:?}. A candidate could be packaged without the brand \
                 gate ever reading its Info.plist or its icons."
            );
        }
    }

    /// OMEGA-DELTA-0023. The packaging path staples the application.
    ///
    /// `script/bundle-omega-rc` stapled the DMG only. A DMG ticket covers the
    /// disk image the owner throws away; it does not travel with the
    /// application that ends up in `/Applications`, so
    /// `stapler validate /Applications/Omega.app` reported no ticket and
    /// Gatekeeper acceptance could rest on an online lookup with Apple —
    /// which cannot prove the offline first start omega#16 requires.
    ///
    /// The order matters as much as the call: the app has to be stapled
    /// *before* the disk image is built, or the DMG is assembled from an
    /// unstapled bundle and the ticket never reaches the installed product.
    #[test]
    fn the_packaging_path_staples_the_application() {
        let path = repository_path(RC_BUNDLE_SCRIPT_PATH);
        let script = std::fs::read_to_string(&path)
            .unwrap_or_else(|error| panic!("cannot read {}: {error}", path.display()));

        for required in [
            "stapler\" staple \"${app_path}\"",
            "stapler\" validate \"${app_path}\"",
            "stapler\" validate \"${DMG_PATH}\"",
            "stapler\" validate \"${stapled_app_path}\"",
            "OMEGA_RC_NOTARIZATION_APP_STAPLED",
            "notarization.app_stapled is required",
        ] {
            assert!(
                script.contains(required),
                "OMEGA-DELTA-0023: {RC_BUNDLE_SCRIPT_PATH} no longer contains \
                 {required:?}. A candidate could be published with a stapled \
                 disk image and an unstapled application, which is exactly the \
                 state that blocks the offline-start proof on omega#16."
            );
        }

        let staple_app = script
            .find("stapler\" staple \"${app_path}\"")
            .expect("the app staple call was asserted above");
        let create_dmg = script
            .find("hdiutil create")
            .expect("the bundle script creates a disk image");
        assert!(
            staple_app < create_dmg,
            "OMEGA-DELTA-0023: the disk image is built before the application is \
             stapled, so it would be assembled from an unstapled bundle and the \
             ticket would never reach /Applications."
        );

        let record = repository_path("crates/app_identity/fixtures/release_record_v1.json");
        let fixture = std::fs::read_to_string(&record)
            .unwrap_or_else(|error| panic!("cannot read {}: {error}", record.display()));
        let parsed: serde_json::Value =
            serde_json::from_str(&fixture).expect("release record fixture parses");
        assert!(
            parsed["notarization"].get("app_stapled").is_some(),
            "OMEGA-DELTA-0023: the release record must carry app_stapled \
             separately from stapled, or the two get conflated again"
        );
    }

    /// OMEGA-DELTA-0024. The first-party identity is Omega Agent everywhere a
    /// user can select, configure, or inspect the native executor.
    #[test]
    fn the_first_party_agent_identity_is_omega_agent() {
        let agent_path = repository_path("crates/agent/src/agent.rs");
        let agent_source = std::fs::read_to_string(&agent_path)
            .unwrap_or_else(|error| panic!("cannot read {}: {error}", agent_path.display()));
        assert!(
            agent_source.contains(r#"AgentId::new("Omega Agent")"#),
            "OMEGA-DELTA-0024: the native AgentId must be Omega Agent"
        );

        let agent_ui_path = repository_path("crates/agent_ui/src/agent_ui.rs");
        let agent_ui_source = std::fs::read_to_string(&agent_ui_path)
            .unwrap_or_else(|error| panic!("cannot read {}: {error}", agent_ui_path.display()));
        for required in [
            r#"Self::NativeAgent => "Omega Agent".into()"#,
            "Self::NativeAgent => Some(IconName::OmegaAgent)",
        ] {
            assert!(
                agent_ui_source.contains(required),
                "OMEGA-DELTA-0024: the native Agent projection lost {required:?}"
            );
        }

        for (relative_path, required) in [
            (
                "crates/agent_ui/src/agent_panel.rs",
                r#"ContextMenuEntry::new("Omega Agent")"#,
            ),
            (
                "crates/ui/src/components/ai/agent_setup_button.rs",
                r#".name("Omega Agent")"#,
            ),
            (
                "crates/eval_cli/src/headless.rs",
                r#""Omega Agent CLI/{} ({}; {})""#,
            ),
        ] {
            let path = repository_path(relative_path);
            let source = std::fs::read_to_string(&path)
                .unwrap_or_else(|error| panic!("cannot read {}: {error}", path.display()));
            assert!(
                source.contains(required),
                "OMEGA-DELTA-0024: {relative_path} lost {required:?}"
            );
        }

        // Read from the policy rather than spelling the symbol here. A
        // tree-wide rename would rewrite this test's own literal along with
        // the code it guards, and the check would rename itself into
        // agreement. That is not hypothetical: it is what happened the first
        // time this arm was falsified.
        let policy = brand_policy().expect("brand gate policy parses");
        let symbol = policy["first_party_agent"]["symbol"]
            .as_str()
            .expect("first_party_agent.symbol is a string");
        assert!(
            agent_source.contains(&format!("pub static {symbol}")),
            "OMEGA-DELTA-0024: the identity symbol must be {symbol}. Renaming \
             only the string it holds leaves the next upstream rebase an \
             obvious symbol to restore Zed's identity under"
        );
    }

    /// OMEGA-DELTA-0024. No phrasing of the first-party agent names Zed.
    ///
    /// Separate from the pins above because it is a different kind of failure.
    /// Those say the identity is present; this says the old one is gone, which
    /// is the half that `0.2.0-rc10` got wrong: an evidence table truthfully
    /// reported a clean binary while 13 Zed strings shipped in the signed
    /// `Info.plist`, because the scan knew exactly three literals.
    ///
    /// So this matches a phrase family from `script/omega-brand-gate.json`,
    /// not one string, and the references we deliberately keep are named there
    /// with a reason rather than skipped silently. Zed still exists; it is
    /// just not us.
    #[test]
    fn no_phrasing_presents_zed_as_the_first_party_agent() {
        let policy = brand_policy().expect("brand gate policy parses");
        let phrases = policy_strings(&policy, "first_party_agent", "phrases");
        let symbols = policy_strings(&policy, "first_party_agent", "symbols");
        let roots = policy_strings(&policy, "first_party_agent", "scan_roots");
        let extensions = policy_strings(&policy, "first_party_agent", "extensions");
        assert!(
            !phrases.is_empty() && !symbols.is_empty() && !roots.is_empty(),
            "OMEGA-DELTA-0024: {BRAND_GATE_POLICY_PATH} declares no phrases, \
             symbols, or roots, so this gate checks nothing"
        );

        let allowances = policy["first_party_agent"]["allowances"]
            .as_object()
            .expect("first_party_agent.allowances is an object");
        for relative in allowances.keys() {
            assert!(
                repository_path(relative).exists(),
                "OMEGA-DELTA-0024: allowance for {relative} outlived the file. \
                 A stale allowance is a hole nobody is watching"
            );
        }

        let stems = policy_strings(&policy, "first_party_agent", "forbidden_path_stems");
        assert!(
            !stems.is_empty(),
            "OMEGA-DELTA-0024: no forbidden path stems, so a file could carry \
             the old identity in its name"
        );

        let extensions: Vec<&str> = extensions.iter().map(String::as_str).collect();
        let repository_root = repository_path(".");
        let mut offenders: Vec<String> = Vec::new();
        for root in &roots {
            for_each_source_file(&repository_path(root), &extensions, |path, source| {
                let relative = path
                    .strip_prefix(&repository_root)
                    .unwrap_or(path)
                    .display()
                    .to_string();

                // The name before the contents. A renamed file reads as clean
                // either way, so a content scan cannot tell `zed-agent.md`
                // from `omega-agent.md`. Falsifying this delta found exactly
                // that: restoring the old file name passed everything.
                let stem = path
                    .file_stem()
                    .and_then(std::ffi::OsStr::to_str)
                    .unwrap_or_default()
                    .to_lowercase();
                for forbidden in &stems {
                    if stem.contains(&forbidden.to_lowercase()) {
                        offenders.push(format!("{relative}: file name {forbidden:?}"));
                    }
                }

                if allowances.contains_key(&relative) {
                    return;
                }
                let lowercased = source.to_lowercase();
                for phrase in &phrases {
                    if lowercased.contains(&phrase.to_lowercase()) {
                        offenders.push(format!("{relative}: {phrase:?}"));
                    }
                }
                for symbol in &symbols {
                    if source.contains(symbol) {
                        offenders.push(format!("{relative}: {symbol:?}"));
                    }
                }
            });
        }

        assert!(
            offenders.is_empty(),
            "OMEGA-DELTA-0024: Zed is presented as the first-party agent \
             again. Rename it, or — only if the text names Zed's own product \
             or Zed's history the way ai_zed.svg does — add an allowance with \
             a reason to {BRAND_GATE_POLICY_PATH}:\n{}",
            offenders.join("\n")
        );
    }

    /// OMEGA-DELTA-0024. The inherited telemetry id is not a product identity.
    ///
    /// `NativeAgentConnection::telemetry_id` still reports `"zed"`, because it
    /// keys inherited analytics and rewriting it would silently break the
    /// series rather than rename anything a user sees. The rule that matters
    /// is that it stays out of the identity path: the product name comes from
    /// `OMEGA_AGENT_ID`, and the two are not allowed to become the same value.
    #[test]
    fn the_inherited_telemetry_id_is_not_the_product_identity() {
        let policy = brand_policy().expect("brand gate policy parses");
        let identity = policy["first_party_agent"]["identity"]
            .as_str()
            .expect("first_party_agent.identity is a string");

        let path = repository_path("crates/agent/src/agent.rs");
        let source = std::fs::read_to_string(&path)
            .unwrap_or_else(|error| panic!("cannot read {}: {error}", path.display()));
        let telemetry_body = source
            .split_once("fn telemetry_id(&self) -> SharedString {")
            .map(|(_, rest)| rest.split_once('}').map_or(rest, |(body, _)| body))
            .expect("the native connection reports a telemetry id");

        assert!(
            telemetry_body.contains(r#""zed""#),
            "OMEGA-DELTA-0024: the native telemetry id is no longer the \
             inherited \"zed\" key. Changing it breaks the analytics series \
             without renaming anything a user sees"
        );
        assert!(
            !telemetry_body.contains(identity),
            "OMEGA-DELTA-0024: the inherited telemetry id now reports \
             {identity:?}, which presents an inherited analytics key as an \
             OpenAgents service identity"
        );
    }

    /// An ID that names two entries names none of them.
    ///
    /// `the_registry_and_the_checks_agree` compares sets, so a duplicate is
    /// invisible to it. Two lanes allocating numbers at the same time produced
    /// exactly that: two `OMEGA-DELTA-0010` entries and two `0011` entries,
    /// which shipped uncaught.
    /// OMEGA-DELTA-0019. Upstream Zed answers an empty window with
    /// `Editor::new_file`. Omega answers it with the agent.
    ///
    /// Checked in both directions, because either half alone is weak: the
    /// absence of `Editor::new_file` would also pass if somebody deleted the
    /// startup path entirely, and the presence of `open_front_door` would also
    /// pass if it were added beside the buffer rather than instead of it.
    #[test]
    fn a_fresh_window_opens_on_the_agent() {
        let path = repository_path("crates/zed/src/main.rs");
        let source = std::fs::read_to_string(&path).expect("zed main is readable");

        assert!(
            !source.contains("Editor::new_file("),
            "{} calls Editor::new_file again. Upstream Zed opens an empty \
             untitled buffer on a window with nothing to restore; Omega opens \
             the agent front door (OMEGA-DELTA-0019).",
            path.display()
        );

        let openings = source.matches("AgentPanel::open_front_door(").count();
        assert_eq!(
            openings,
            2,
            "{} reaches the front door {openings} time(s); \
             restore_or_create_workspace has two no-restorable-session paths \
             and both must land there (OMEGA-DELTA-0019).",
            path.display()
        );

        // The launchpad behaviour opens no content at all upstream, and Omega
        // does not override a setting the user set.
        assert!(
            source.contains("RestoreOnStartupBehavior::Launchpad => {}"),
            "{} no longer leaves the launchpad startup behaviour alone \
             (OMEGA-DELTA-0019).",
            path.display()
        );
    }

    /// OMEGA-DELTA-0020. Full Auto is a surface of the chat panel now.
    ///
    /// The owner asked for the dock panel to go. The failure this guards is a
    /// rebase or a well-meaning cleanup putting it back — or, worse, removing
    /// the dock panel without moving its two actions, which would silently
    /// break a user keybinding rather than fail anything.
    #[test]
    fn full_auto_is_folded_into_the_chat_panel() {
        let zed_path = repository_path("crates/zed/src/zed.rs");
        let zed = std::fs::read_to_string(&zed_path).expect("zed.rs is readable");
        assert!(
            !zed.contains("FullAutoPanel"),
            "{} registers a Full Auto dock panel again. The owner asked for \
             Full Auto to be folded into the Omega chat UI \
             (OMEGA-DELTA-0020).",
            zed_path.display()
        );

        let panel_path = repository_path("crates/agent_ui/src/agent_panel.rs");
        let panel = std::fs::read_to_string(&panel_path).expect("agent panel is readable");
        assert!(
            panel.contains("FullAutoPanel::new("),
            "{} no longer constructs the Full Auto surface. Folding it in is \
             what makes the dock panel's removal a move rather than a deletion \
             (OMEGA-DELTA-0020).",
            panel_path.display()
        );

        // Both actions outlived their panel on purpose: a user keymap may name
        // them, and the fold moves where Full Auto lives without taking a
        // binding away. An unanswered action is a silent no-op, not an error,
        // which is exactly why this needs a test.
        for action in ["OpenLauncher", "ToggleFullAutoFocus"] {
            assert!(
                panel.contains(&format!("_: &{action},")),
                "{} does not answer the {action} action. It was answered by \
                 the retired full_auto_ui::init, so dropping it here makes a \
                 keybinding silently do nothing (OMEGA-DELTA-0020).",
                panel_path.display()
            );
        }

        let lib_path = repository_path("crates/full_auto_ui/src/full_auto_ui.rs");
        let lib = std::fs::read_to_string(&lib_path).expect("full_auto_ui is readable");
        assert!(
            !lib.contains("pub use panel::{init"),
            "{} exports a panel init again; there is no dock panel to \
             register (OMEGA-DELTA-0020).",
            lib_path.display()
        );
    }

    /// OMEGA-DELTA-0020, and owner gate 8 behind it.
    ///
    /// Only an explicit human action may start Full Auto authority. Folding
    /// the surface into chat moves where that action lives; it must not add a
    /// second way to reach it. The strongest mechanical form of that is a call
    /// count: `start_run` is reached from exactly one place in the GPUI tree,
    /// and that place is a click handler.
    ///
    /// This deliberately scans every crate rather than the one file, because
    /// the failure worth catching is a *new* caller somewhere else — a tool, a
    /// slash command, a restored draft — not a change to the known one.
    ///
    /// Two distinct calls are counted, because they are two distinct hazards.
    /// `guard.start_run(` is the supervisor call that creates run authority; a
    /// second one of those means a second way to create a run. `this.start_run(`
    /// is the UI entry into it; a second one of those means a second gesture
    /// can reach the first.
    #[test]
    fn only_a_click_listener_starts_a_full_auto_run() {
        let crates = repository_path("crates");
        let mut authority: Vec<(String, String)> = Vec::new();
        let mut entries: Vec<(String, String)> = Vec::new();
        for_each_source_file(&crates, &["rs"], |path, source| {
            // `repository_path` yields `…/crates/omega_deltas/../../crates/x`,
            // so a plain `contains` would see `omega_deltas` in every path.
            // The crate-relative tail is what identifies the file.
            let display = path
                .display()
                .to_string()
                .rsplit("crates/")
                .next()
                .unwrap_or_default()
                .to_owned();
            // `omega_effectd` declares and transports the call; it does not
            // decide to make one. This file is the check itself, and matching
            // its own needles would make the count meaningless.
            if display.starts_with("omega_effectd/") || display.starts_with("omega_deltas/") {
                return;
            }
            for line in source.lines() {
                let line = line.trim();
                if line.contains("guard.start_run(") {
                    authority.push((display.clone(), line.to_owned()));
                } else if line.contains(".start_run(") {
                    entries.push((display.clone(), line.to_owned()));
                }
            }
        });

        assert_eq!(
            authority.len(),
            1,
            "Full Auto run authority is created from {} place(s): \
             {authority:#?}. `omega-effectd` is the sole run authority and the \
             launch surface is the sole way to ask it for a run \
             (OMEGA-DELTA-0020).",
            authority.len()
        );
        assert!(
            authority[0].0.ends_with("full_auto_ui/src/panel.rs"),
            "Full Auto's run-start moved to {}. It belongs on the launch \
             surface (OMEGA-DELTA-0020).",
            authority[0].0
        );

        assert_eq!(
            entries.len(),
            1,
            "Full Auto's run-start is reachable from {} place(s): {entries:#?}. \
             Owner gate 8 admits exactly one, and it is a human clicking Start \
             (OMEGA-DELTA-0020).",
            entries.len()
        );
        assert_eq!(
            entries[0].1, ".on_click(cx.listener(|this, _, _, cx| this.start_run(cx))),",
            "Full Auto's run-start is no longer behind a click. Reaching the \
             launch surface is one human act and starting the run is a second; \
             collapsing them, or moving the start behind an action, a timer, or \
             a message handler, is what owner gate 8 forbids \
             (OMEGA-DELTA-0020)."
        );
    }

    /// OMEGA-DELTA-0031. Every brand-bearing prose literal is classified.
    ///
    /// `OMEGA-DELTA-0022` named this class and could not close it. The string
    /// rule it shipped enforces the compatibility allow-list's `blocked`
    /// claims, which is a written-down denylist: a *new* sentence naming a
    /// competitor as the product passes until somebody remembers to add it.
    /// That is exactly how "Use GitHub Copilot in Zed" reached a signed,
    /// notarized `0.2.0-rc11`, and why the signed, notarized `0.2.0-rc13`
    /// still told the user "Click 'Connect' below to start using Ollama in
    /// Zed" from two provider onboarding pages, titled the OAuth callback page
    /// in their browser after somebody else's product, and handed the model a
    /// system prompt beginning "You are the Zed coding agent running inside
    /// the Zed editor".
    ///
    /// This inverts the default. The inventory is derived from five shipping
    /// mechanisms; everything in it must be *classified* in the policy; an
    /// unclassified literal fails. A new sentence is unclassified the moment
    /// it is written.
    #[test]
    fn no_unclassified_prose_names_a_competitor() {
        let policy = brand_policy().expect("brand policy parses");
        let prose = &policy["prose"];
        let (items, read) = prose_inventory(&policy);

        for (key, actual, label) in [
            ("minimum_rust_files", read.rust_files, "Rust sources"),
            (
                "minimum_rust_string_literals",
                read.literals,
                "Rust string literals",
            ),
            (
                "minimum_schema_doc_lines",
                read.schema_docs,
                "settings-schema doc lines",
            ),
            (
                "minimum_action_doc_lines",
                read.action_docs,
                "action doc lines",
            ),
            ("minimum_clap_doc_lines", read.clap_docs, "--help doc lines"),
            (
                "minimum_embedded_files",
                read.embedded,
                "embedded asset files",
            ),
        ] {
            let floor = prose[key].as_u64().unwrap_or(u64::MAX) as usize;
            assert!(
                actual >= floor,
                "OMEGA-DELTA-0034: only {actual} {label} were read, below the \
                 {floor} floor in {BRAND_GATE_POLICY_PATH}. That parser broke, \
                 and a check that reads nothing reports green about nothing."
            );
        }

        let classified = prose["classified"]
            .as_object()
            .expect("prose.classified is an object");
        assert!(
            classified.len() >= 20,
            "OMEGA-DELTA-0034: only {} prose literals are classified, so this \
             registry is not a record of anybody having read the tree",
            classified.len()
        );
        for (text, entry) in classified {
            assert!(
                entry["class"].is_string() && entry["reason"].is_string(),
                "OMEGA-DELTA-0034: prose.classified[{text:?}] needs a class and \
                 a reason. An entry is a record that somebody read the sentence \
                 and decided it names somebody else rather than us."
            );
            let class = entry["class"].as_str().unwrap_or_default();
            assert!(
                prose["classes"].get(class).is_some(),
                "OMEGA-DELTA-0034: prose.classified[{text:?}] uses class \
                 {class:?}, which prose.classes does not define"
            );
        }

        let corpus: std::collections::BTreeSet<&str> = prose["corpus_files"]
            .as_object()
            .expect("prose.corpus_files is an object")
            .keys()
            .map(String::as_str)
            .collect();
        for relative in &corpus {
            assert!(
                repository_path(relative).is_file(),
                "OMEGA-DELTA-0034: exempt corpus file {relative} does not exist, \
                 so the exemption is hiding nothing and should be deleted"
            );
        }

        let mut unclassified: Vec<String> = items
            .iter()
            .filter(|item| {
                !corpus.contains(item.file.as_str()) && !classified.contains_key(&item.text)
            })
            .map(|item| {
                format!(
                    "{}:{} [{}] {:?}",
                    item.file,
                    item.line,
                    item.kind,
                    item.text.chars().take(160).collect::<String>()
                )
            })
            .collect();
        unclassified.sort();
        unclassified.dedup();
        assert!(
            unclassified.is_empty(),
            "OMEGA-DELTA-0034: brand-bearing prose that nobody has classified. \
             Substitute our own name: if the sentence stays true with 'Omega' \
             in it, the brand was standing where our product's name belongs, so \
             rewrite it. If it becomes false, it names somebody else's product \
             and belongs in prose.classified in {BRAND_GATE_POLICY_PATH} with a \
             class and a reason.\n{}",
            unclassified.join("\n")
        );

        let present: std::collections::BTreeSet<&str> =
            items.iter().map(|item| item.text.as_str()).collect();
        let mut stale: Vec<&String> = classified
            .iter()
            .filter(|(text, entry)| {
                entry["packaged_only"] != serde_json::Value::Bool(true)
                    && !present.contains(text.as_str())
            })
            .map(|(text, _)| text)
            .collect();
        stale.sort();
        assert!(
            stale.is_empty(),
            "OMEGA-DELTA-0034: these classified literals are no longer anywhere \
             in the tree. Either the sentence went and the entry should go with \
             it, or a scanner stopped reading the stream that used to find it — \
             which is what this assertion exists to catch.\n{}",
            stale
                .iter()
                .map(|text| text.chars().take(100).collect::<String>())
                .collect::<Vec<_>>()
                .join("\n")
        );
    }

    /// OMEGA-DELTA-0031. The prose lexer reads what it claims to read.
    ///
    /// The first version of this scanner counted lines wrongly across a `\`
    /// line continuation, so six multi-line literals — four provider error
    /// toasts among them — were attributed to the wrong line and silently
    /// skipped by the rewrite that was supposed to fix them.
    #[test]
    fn the_prose_lexer_reads_multi_line_and_raw_literals() {
        let source = concat!(
            "fn demo() {\n",
            "    // \"not a literal\"\n",
            "    let a = \"first\";\n",
            "    let b = \"second \\\n",
            "        continued\";\n",
            "    let c = r#\"raw \"quoted\" body\"#;\n",
            "    let d = \"after\";\n",
            "}\n",
        );
        let literals = rust_string_literals(source);
        let found: Vec<(usize, &str)> = literals
            .iter()
            .map(|(line, body)| (*line, body.as_str()))
            .collect();
        assert_eq!(found[0], (3, "first"), "single-line literal and its line");
        assert_eq!(
            found[1].0, 4,
            "a continued literal starts on its first line"
        );
        assert!(
            found[1].1.contains("continued"),
            "the continuation is part of the literal: {:?}",
            found[1].1
        );
        assert_eq!(
            found[2],
            (6, "raw \"quoted\" body"),
            "a raw literal keeps its inner quotes"
        );
        assert_eq!(
            found[3],
            (7, "after"),
            "the line count survives the raw literal"
        );
        assert!(
            !found.iter().any(|(_, body)| body.contains("not a literal")),
            "a comment is not a literal"
        );

        assert!(is_prose(
            "Click 'Connect' below to start using Ollama in Omega"
        ));
        assert!(!is_prose("crates/zed/src/main.rs"));
        assert!(!is_prose("X-Zed-Predict-Edits-Mode"));
        assert!(
            is_prose("Zed Plex Sans"),
            "a three-word font family name stays in the inventory to be classified, \
             not filtered out of it"
        );
    }

    /// OMEGA-DELTA-0044. A brand-bearing command form is user-facing text.
    ///
    /// `is_prose` needs three tokens, and `0.2.0-rc16` shipped three two-token
    /// command literals in the signed `cli` because of it: `zed --existing`,
    /// `zed --classic` and `zed <path>`, rendered by an interactive panel that
    /// says "Omega window" and "Omega settings" around them (omega#93). They
    /// were in neither `prose.classified` nor the compatibility allow-list, and
    /// `verify-omega-brand --app` exited 0.
    ///
    /// Both directions matter. Widening this to "any two tokens beginning with
    /// a brand word" pulls in eight more literals that really are references to
    /// somebody else's product, and widening it to a bare brand token pulls in
    /// 55; a rule that cries wolf gets deleted, which is how the blind spot
    /// stayed open once it was known about.
    #[test]
    fn a_brand_bearing_command_form_is_user_facing_text() {
        let policy = brand_policy().expect("brand policy parses");

        for shipped in ["zed --existing", "zed --classic", "zed <path>"] {
            assert!(
                !is_prose(shipped),
                "OMEGA-DELTA-0044: {shipped:?} is being kept out of the \
                 inventory by the three-token rule, which is the blind spot \
                 this exists to close"
            );
            assert!(
                is_command_form(shipped, &policy),
                "OMEGA-DELTA-0044: {shipped:?} shipped in a signed `cli` and \
                 the inventory still cannot see it"
            );
            assert!(is_user_facing_text(shipped, &policy));
        }
        for form in ["zed .", "zed ~/project", "zed -n", "Zed --help"] {
            assert!(
                is_command_form(form, &policy),
                "OMEGA-DELTA-0044: {form:?} is a command form"
            );
        }

        // The other direction: the shapes that would make this noisy enough to
        // be deleted, and are therefore excluded on purpose.
        for benign in [
            "Zed Pro",
            "Zed (Default)",
            "Zed Repository",
            "zed",
            "crates/zed/src",
            "X-Zed-Predict-Edits-Mode",
            "zed_llm_client",
        ] {
            assert!(
                !is_command_form(benign, &policy),
                "OMEGA-DELTA-0044: {benign:?} is not a command form. This rule \
                 covers a brand word standing in argv[0]; a two-word label is \
                 either prose already or a reference to be classified."
            );
        }

        // The shell gate carries the same rule, on the same three streams, or
        // the source and packaged sides disagree about what is inventoried.
        let verifier =
            std::fs::read_to_string(repository_path(BRAND_VERIFIER_PATH)).expect("brand verifier");
        assert!(
            verifier.contains("def is_command_form("),
            "OMEGA-DELTA-0044: {BRAND_VERIFIER_PATH} does not carry the \
             command-form rule, so `verify-omega-brand` and these tests \
             inventory different things."
        );
        let wired = verifier.matches("is_user_facing_text(").count();
        assert!(
            wired >= 4,
            "OMEGA-DELTA-0044: {BRAND_VERIFIER_PATH} calls is_user_facing_text \
             {wired} time(s) — one definition and one call per stream (Rust \
             literals, doc lines, embedded assets) is the minimum. A predicate \
             that is defined and never called is the state \
             first_party_agent.phrases was in for four release candidates."
        );
        assert!(
            !verifier.contains("and is_prose(body)") && !verifier.contains("not is_prose(doc)"),
            "OMEGA-DELTA-0044: a stream in {BRAND_VERIFIER_PATH} still filters \
             on is_prose alone, so command forms are invisible to it."
        );
    }

    /// OMEGA-DELTA-0044. The prompt spells its command forms with the binary's
    /// own name.
    ///
    /// Fixing the three literals is not the same as making them unable to come
    /// back. Any command form written out by hand can drift from the binary it
    /// describes; one built from `paths::BINARY_NAME` cannot.
    #[test]
    fn the_cli_prompt_names_our_own_binary() {
        let path = repository_path(CLI_MAIN_PATH);
        let source = std::fs::read_to_string(&path)
            .unwrap_or_else(|error| panic!("cannot read {}: {error}", path.display()));
        let prompt = source
            .split("fn prompt_open_behavior()")
            .nth(1)
            .expect("OMEGA-DELTA-0044: prompt_open_behavior is gone");
        let prompt = &prompt[..prompt.find("\n}\n").unwrap_or(prompt.len())];
        // Ordinary comments are dropped, doc comments are not: a `//` line does
        // not reach a user, and the comment recording what shipped here has to
        // be able to quote it. `///` and `//!` stay, because clap and schemars
        // print those.
        let rendered: String = prompt
            .lines()
            .filter(|line| {
                let trimmed = line.trim_start();
                !trimmed.starts_with("//")
                    || trimmed.starts_with("///")
                    || trimmed.starts_with("//!")
            })
            .collect::<Vec<_>>()
            .join("\n");

        let policy = brand_policy().expect("brand policy parses");
        let hits = brand_hits(&rendered, &policy);
        assert!(
            hits.is_empty(),
            "OMEGA-DELTA-0044: the open-behavior prompt in {} names {hits:?}. \
             This panel already says \"Omega window\" and \"Omega settings\"; \
             the commands beside them have to be ours too (omega#93).",
            path.display()
        );
        let derived = rendered.matches("paths::BINARY_NAME").count();
        assert!(
            derived >= 3,
            "OMEGA-DELTA-0044: the prompt builds only {derived} of its command \
             forms from paths::BINARY_NAME. All three — the two choices and the \
             heading — come from the binary's own name, so a rename cannot \
             leave one behind."
        );
    }

    #[test]
    fn delta_ids_are_unique() {
        let mut seen = std::collections::BTreeSet::new();
        let repeated: Vec<&&str> = ENFORCED_DELTAS
            .iter()
            .filter(|id| !seen.insert(**id))
            .collect();
        assert!(
            repeated.is_empty(),
            "ENFORCED_DELTAS lists an ID more than once: {repeated:?}. \
             Allocate the next free number instead of reusing one."
        );

        let path = repository_path(DELTA_REGISTRY_PATH);
        let registry = std::fs::read_to_string(&path).expect("delta registry is readable");
        let mut seen_headings = std::collections::BTreeSet::new();
        let repeated_headings: Vec<String> = registry
            .lines()
            .filter_map(|line| line.strip_prefix("### "))
            .filter_map(|heading| heading.split_whitespace().next())
            .filter(|token| token.starts_with("OMEGA-DELTA-"))
            .filter(|token| !seen_headings.insert((*token).to_owned()))
            .map(str::to_owned)
            .collect();
        assert!(
            repeated_headings.is_empty(),
            "{} has more than one entry for: {repeated_headings:?}",
            path.display()
        );
    }

    /// The registry and the checks must agree, in both directions.
    ///
    /// A check with no registry entry is an unexplained rule. A registry entry
    /// with no check is a promise nothing keeps — and the registry's own rules
    /// require every delta to have one.
    #[test]
    fn the_registry_and_the_checks_agree() {
        let path = repository_path(DELTA_REGISTRY_PATH);
        let registry = std::fs::read_to_string(&path).expect("delta registry is readable");

        // Match the heading that opens an entry, not any mention of the ID.
        // A substring match would accept "OMEGA-DELTA-0001 was withdrawn".
        let documented: std::collections::BTreeSet<String> = registry
            .lines()
            .filter_map(|line| line.strip_prefix("### "))
            .filter_map(|heading| heading.split_whitespace().next())
            .filter(|token| token.starts_with("OMEGA-DELTA-"))
            .map(str::to_owned)
            .collect();
        let enforced: std::collections::BTreeSet<String> =
            ENFORCED_DELTAS.iter().map(|id| (*id).to_owned()).collect();

        let undocumented: Vec<&String> = enforced.difference(&documented).collect();
        assert!(
            undocumented.is_empty(),
            "enforced but missing a `### <ID>` entry in {}: {undocumented:?}",
            path.display()
        );
        let unenforced: Vec<&String> = documented.difference(&enforced).collect();
        assert!(
            unenforced.is_empty(),
            "documented in {} but not listed in ENFORCED_DELTAS: {unenforced:?}",
            path.display()
        );
    }

    /// OMEGA-DELTA-0021. Disclosure is a typed record, never a label string.
    ///
    /// This is the owner's binding condition from omega#74, mechanised. The
    /// owner accepted that the first-party agent does not sign with its own
    /// principal *on the condition* that disclosure is stored as a record a
    /// label renders. A stored label would make that reversible decision
    /// irreversible: switching to a signing principal would then mean parsing
    /// prose back into parts for every thread, instead of adding a signer.
    ///
    /// So the field set is asserted exactly, and `label` is asserted to be a
    /// method. Upstream Zed has no equivalent — there is nothing to revert to
    /// here, only something to quietly add.
    #[test]
    fn executor_disclosure_is_a_typed_record_not_a_label_string() {
        let path = repository_path(EXECUTOR_DISCLOSURE_RECORD_PATH);
        let source = std::fs::read_to_string(&path)
            .unwrap_or_else(|error| panic!("cannot read {}: {error}", path.display()));

        let fields = struct_fields(&source, "ExecutorDisclosure");
        assert!(
            !fields.is_empty(),
            "OMEGA-DELTA-0021: no ExecutorDisclosure struct found in {}. \
             The check would be vacuous, so it fails instead.",
            path.display()
        );

        let found: Vec<(&str, &str)> = fields
            .iter()
            .map(|(name, type_name)| (name.as_str(), type_name.as_str()))
            .collect();
        assert_eq!(
            found, EXECUTOR_DISCLOSURE_FIELDS,
            "OMEGA-DELTA-0021: ExecutorDisclosure holds different parts than \
             the ones recorded. If a part was genuinely added, add it to \
             EXECUTOR_DISCLOSURE_FIELDS and say so in the delta. If a rendered \
             line was added, that is the failure this test exists for: the \
             owner's identity decision on omega#74 stays reversible only while \
             disclosure is a record a label renders."
        );

        assert!(
            source.contains("pub fn label(&self) -> String"),
            "OMEGA-DELTA-0021: {} must render the line from the record with a \
             `label` method.",
            path.display()
        );
    }

    /// OMEGA-DELTA-0029. The routing law reads nothing but its inputs.
    ///
    /// Determinism is the packet's exit and the hardest half to establish: a
    /// unit test shows only that the inputs it tried gave the same answer
    /// twice. This reads the source for the ways the answer could stop
    /// depending on its inputs at all.
    #[test]
    fn the_routing_law_has_no_clock_no_randomness_and_no_hash_order() {
        for path in [ROUTE_DECISION_PATH, ROUTER_DISPATCH_PATH] {
            let full = repository_path(path);
            let source = std::fs::read_to_string(&full)
                .unwrap_or_else(|error| panic!("cannot read {}: {error}", full.display()));
            assert!(
                source.contains("pub fn route(") || source.contains("route(&self.inputs_for("),
                "OMEGA-DELTA-0029: {path} does not look like part of the \
                 routing path any more; the scan below would be vacuous."
            );
            // Code only: the doc comments name several of these tokens in the
            // course of explaining why they are not there.
            let code = code_of(&source);
            for (what, token) in NON_DETERMINISTIC_ROUTING_TOKENS {
                assert!(
                    !code.contains(token),
                    "OMEGA-DELTA-0029: {path} reaches for {what} (`{token}`). \
                     The router's exit is that the same inputs give the same \
                     route; anything here that is not an input breaks that in \
                     a way nobody can reproduce afterwards."
                );
            }
        }
    }

    /// OMEGA-DELTA-0029. The router owns no execution.
    ///
    /// omega#74 admitted Omega Agent as a router that owns routing, disclosure
    /// and receipts and owns no execution. The last four tokens are owner gate
    /// 8: the router must not be able to start run authority at all, because
    /// only an explicit human action may — and today three model-callable
    /// starts were removed from Desktop, one of them a rename of another.
    #[test]
    fn the_router_owns_no_execution_and_starts_no_run() {
        let path = repository_path(ROUTER_DISPATCH_PATH);
        let source = std::fs::read_to_string(&path)
            .unwrap_or_else(|error| panic!("cannot read {}: {error}", path.display()));
        assert!(
            source.contains("impl AgentConnection for OmegaAgentConnection"),
            "OMEGA-DELTA-0029: {} no longer implements the seam it routes at.",
            path.display()
        );
        // Doc comments name several of these tokens on purpose, so the scan
        // reads code only.
        let code = code_of(&source);
        for token in ROUTER_EXECUTION_TOKENS {
            assert!(
                !code.contains(token),
                "OMEGA-DELTA-0029: the router reaches for `{token}`. It routes, \
                 discloses, and records; it does not execute, and it does not \
                 start run authority."
            );
        }
    }

    /// OMEGA-DELTA-0029. Every routed method hands the work to an executor.
    ///
    /// A method that stopped delegating would be the router quietly becoming an
    /// executor — the packet's falsifier — and it would still compile, still
    /// pass every behavioural test that did not happen to call it, and still
    /// read as a router from its module docs.
    #[test]
    fn the_router_delegates_every_agent_connection_method() {
        let path = repository_path(ROUTER_DISPATCH_PATH);
        let source = std::fs::read_to_string(&path)
            .unwrap_or_else(|error| panic!("cannot read {}: {error}", path.display()));

        let header = "impl AgentConnection for OmegaAgentConnection {\n";
        let start = source
            .find(header)
            .unwrap_or_else(|| panic!("{} has no router impl block", path.display()))
            + header.len();
        let end = source[start..]
            .find("\n}")
            .unwrap_or_else(|| panic!("{} has an unterminated impl block", path.display()))
            + start;
        let block = &source[start..end];

        let mut checked = 0;
        for item in block.split("\n    fn ").skip(1) {
            let name = item
                .split(['(', '<'])
                .next()
                .unwrap_or_default()
                .trim()
                .to_owned();
            if ROUTER_NON_DELEGATING_METHODS.contains(&name.as_str()) {
                continue;
            }
            checked += 1;
            assert!(
                ROUTER_DELEGATION_MARKERS
                    .iter()
                    .any(|marker| item.contains(marker)),
                "OMEGA-DELTA-0029: `{name}` in {} does not hand its work to an \
                 executor. The router owns no execution, so every method of the \
                 seam it implements has to reach one of {ROUTER_DELEGATION_MARKERS:?}.",
                path.display()
            );
        }
        assert!(
            checked >= 10,
            "OMEGA-DELTA-0029: only {checked} router methods were checked. The \
             impl block parse is finding almost nothing, so this check is close \
             to vacuous."
        );
    }

    /// OMEGA-DELTA-0029. The decision record is a record, like the disclosure.
    ///
    /// Same law, same reason: a stored sentence cannot be re-rendered, re-read,
    /// or handed to a later signer, and the whole point of writing a route down
    /// is that somebody can ask it a question later.
    #[test]
    fn the_route_decision_is_a_record_that_round_trips() {
        let path = repository_path(ROUTE_DECISION_PATH);
        let source = std::fs::read_to_string(&path)
            .unwrap_or_else(|error| panic!("cannot read {}: {error}", path.display()));

        let fields = struct_fields(&source, "RouteDecision");
        let found: Vec<(&str, &str)> = fields
            .iter()
            .map(|(name, type_name)| (name.as_str(), type_name.as_str()))
            .collect();
        assert_eq!(
            found,
            [
                ("chosen", "ExecutorClass"),
                ("reason", "RouteReason"),
                ("pin", "Option<ExecutorPin>"),
                ("lane_ref", "Option<String>"),
            ],
            "OMEGA-DELTA-0029: RouteDecision holds different parts than the \
             ones recorded. A rendered explanation stored as a field is the \
             failure this exists for."
        );
        for required in [
            "pub fn canonical_record(&self) -> String",
            "pub fn parse_canonical_record(record: &str) -> Option<Self>",
            "pub fn is_coherent(&self) -> bool",
        ] {
            assert!(
                source.contains(required),
                "OMEGA-DELTA-0029: {} must define `{required}`; a record that \
                 cannot be read back is a log line.",
                path.display()
            );
        }
    }

    /// OMEGA-DELTA-0021. The thread surface renders the line, from the record.
    ///
    /// omega#77's falsifier is a thread surface showing work without naming its
    /// executor. A record nothing draws satisfies every other check here and
    /// discloses nothing, so the call site is pinned as well as the type.
    #[test]
    fn the_thread_surface_renders_the_executor_line_from_the_record() {
        let path = repository_path(THREAD_VIEW_PATH);
        let source = std::fs::read_to_string(&path)
            .unwrap_or_else(|error| panic!("cannot read {}: {error}", path.display()));

        assert!(
            source.contains("fn render_executor_disclosure("),
            "OMEGA-DELTA-0021: {} must define the executor line.",
            path.display()
        );
        assert!(
            source.contains(".child(self.render_executor_disclosure(cx))"),
            "OMEGA-DELTA-0021: {} must draw the executor line in the thread. \
             Defining it without rendering it discloses nothing.",
            path.display()
        );
        assert!(
            source.contains("Label::new(disclosure.label())"),
            "OMEGA-DELTA-0021: the executor line must be rendered from the \
             disclosure record in {}, not from a stored or hand-built string.",
            path.display()
        );

        // omega#99. The disclosure became conditional on the mode, so one call
        // site is no longer enough to establish that every thread names its
        // executor. Zero base draws the record in the composer bar instead of
        // above the entries; without the second half of this check, deleting
        // that bar would leave the assertions above green while the mode whose
        // entire purpose is to show one executor working stopped naming it.
        assert!(
            source.contains("fn render_zero_base_executor_bar("),
            "OMEGA-DELTA-0021: {} must define zero base's composer bar. The \
             ordinary disclosure line is skipped in that mode, so this is the \
             only surface left that names the executor.",
            path.display()
        );
        assert!(
            source.contains(".child(self.render_zero_base_executor_bar(cx))"),
            "OMEGA-DELTA-0021: {} must draw zero base's composer bar. Defining \
             it without rendering it leaves a zero-base turn unattributed, \
             which is omega#77's falsifier.",
            path.display()
        );
        let bar = source
            .split_once("fn render_zero_base_executor_bar(")
            .map(|(_, rest)| rest)
            .unwrap_or_default();
        let bar = bar.split_once("\n    fn ").map_or(bar, |(body, _)| body);
        assert!(
            bar.contains("Label::new(disclosure.label())"),
            "OMEGA-DELTA-0021: zero base's composer bar in {} must render the \
             line from the disclosure record, like every other surface that \
             discloses. A bar that names a model from the selector beside it \
             and nothing else discloses the model, not the executor.",
            path.display()
        );
        // OMEGA-DELTA-0055 replaced the assertion that used to be here. It
        // required the bar to carry the executor pin, on the reasoning that the
        // pin was how a thread reached the Exo lane. That reasoning was correct
        // while the pin was the door, and the door is automatic routing now.
        // The owner asked for the control to go: "that UI selector makes no
        // sense, i have no fucking clue what youre talking about so the user
        // won't, remove that UI piece and handle it smartly in the background".
        //
        // The half of the rule that still holds is above: the bar renders the
        // line from the disclosure record. A person is entitled to know which
        // runtime and model spent their budget, and that is what the owner
        // objected to losing — he objected to the *selector*.
        assert!(
            !bar.contains("render_executor_pin"),
            "OMEGA-DELTA-0021: zero base's composer bar in {} carries the \
             executor pin again. It showed `native_loop`, `external_acp` and \
             `engine_lane` to a person as a choice, which are wire tokens \
             nobody outside this codebase can read.",
            path.display()
        );
    }

    /// OMEGA-DELTA-0021. The disclosure rides an extension trait, not a fork.
    ///
    /// omega#77 requires the disclosure to be carried "via an Omega extension
    /// trait behind a checked downcast on the shared thread types — never a
    /// fork of `AcpThread`". Two halves: the `impl` exists here, and the
    /// upstream crate stays clean. Putting the record into `crates/acp_thread`
    /// would work fine and would make every future rebase of that crate an
    /// Omega merge conflict.
    #[test]
    fn the_disclosure_is_an_extension_trait_and_not_a_fork_of_the_shared_thread() {
        let path = repository_path(EXECUTOR_DISCLOSURE_BINDING_PATH);
        let source = std::fs::read_to_string(&path)
            .unwrap_or_else(|error| panic!("cannot read {}: {error}", path.display()));
        assert!(
            source.contains("impl ThreadExecutorDisclosure for AcpThread"),
            "OMEGA-DELTA-0021: {} must implement the Omega extension trait for \
             the shared thread type.",
            path.display()
        );
        assert!(
            source.contains("downcast::<NativeAgentConnection>()"),
            "OMEGA-DELTA-0021: {} must classify the executor by a checked \
             downcast. Matching on the agent's display name would make the \
             disclosure a string comparison on a label #75 is renaming.",
            path.display()
        );

        let mut offenders = Vec::new();
        for_each_source_file(
            &repository_path("crates/acp_thread"),
            &["rs"],
            |file, contents| {
                if contents.contains("ExecutorDisclosure") {
                    offenders.push(file.display().to_string());
                }
            },
        );
        assert!(
            offenders.is_empty(),
            "OMEGA-DELTA-0021: the disclosure record leaked into the shared \
             thread crate, which is the fork omega#77 forbids: {offenders:?}"
        );
    }

    // ------------------------------------------------------ OMEGA-DELTA-0025

    /// OMEGA-DELTA-0025. A digest is a measurement, never a parsed claim.
    ///
    /// The whole provenance packet rests on this. If a `MeasuredDigest` can be
    /// built from a string, then a receipt saying "this update was applied at
    /// this digest" can be written by code that read a registry document
    /// instead of the bytes, and the receipt stops meaning anything a reader
    /// could act on — while continuing to look identical on the wire.
    ///
    /// This is the kind of invariant a convenience constructor undoes in good
    /// faith: `impl From<String>` to make a test fixture shorter, or a
    /// `Deserialize` derive to reuse the type in the pin ledger. So the
    /// admitted constructors are named, and anything that returns the type
    /// another way fails.
    #[test]
    fn a_measured_digest_cannot_be_built_from_a_string() {
        let path = repository_path(MEASURED_DIGEST_PATH);
        let source = std::fs::read_to_string(&path)
            .unwrap_or_else(|error| panic!("cannot read {}: {error}", path.display()));

        let header = "impl MeasuredDigest {\n";
        let start = source.find(header).unwrap_or_else(|| {
            panic!(
                "OMEGA-DELTA-0025: no `impl MeasuredDigest` block in {}. The \
                 check would be vacuous, so it fails instead.",
                path.display()
            )
        });
        let body_start = start + header.len();
        let end = source[body_start..]
            .find("\n}")
            .expect("the impl block closes");
        let body = &source[body_start..body_start + end];

        // Every associated function that hands back a `MeasuredDigest`.
        let mut constructors: Vec<String> = Vec::new();
        let mut lines = body.lines().peekable();
        while let Some(line) = lines.next() {
            let trimmed = line.trim();
            let Some(rest) = trimmed
                .strip_prefix("pub fn ")
                .or_else(|| trimmed.strip_prefix("fn "))
            else {
                continue;
            };
            let Some((name, signature)) = rest.split_once('(') else {
                continue;
            };
            // A one-line signature, or the return type on the following line.
            let returns_self = signature.contains("-> Self")
                || signature.contains("-> MeasuredDigest")
                || lines.peek().is_some_and(|next| {
                    next.contains("-> Self") || next.contains("-> MeasuredDigest")
                });
            if returns_self {
                constructors.push(name.trim().to_owned());
            }
        }
        constructors.sort();
        constructors.dedup();

        let mut admitted: Vec<String> = MEASURED_DIGEST_CONSTRUCTORS
            .iter()
            .map(|name| (*name).to_owned())
            .collect();
        admitted.sort();
        assert_eq!(
            constructors,
            admitted,
            "OMEGA-DELTA-0025: {} declares a different set of ways to obtain a \
             MeasuredDigest than the admitted one. Every entry has to take \
             bytes or already-measured digests; a constructor that takes a \
             digest string turns every receipt into a claim.",
            path.display()
        );

        // The primitive constructor takes bytes. Without this the closed list
        // above would still pass if `measure` were changed to take a `&str`
        // digest.
        assert!(
            source.contains("pub fn measure(bytes: &[u8]) -> Self"),
            "OMEGA-DELTA-0025: MeasuredDigest::measure no longer takes bytes in {}",
            path.display()
        );
        assert!(
            source.contains("pub fn measure_tree(files: &mut [(String, MeasuredDigest)]) -> Self"),
            "OMEGA-DELTA-0025: MeasuredDigest::measure_tree no longer folds \
             already-measured digests in {}",
            path.display()
        );

        // A trait implementation is not a `fn` in the inherent block, so the
        // closed list above cannot see one. These are the three that would
        // reintroduce a string path.
        //
        // Read from the code only: this module's own documentation explains
        // why `Deserialize` is absent, and a whole-file scan reads that
        // explanation as the violation it describes. The first run of this
        // check did exactly that.
        let source: String = source
            .lines()
            .filter(|line| !line.trim_start().starts_with("//"))
            .collect::<Vec<_>>()
            .join("\n");
        for forbidden in [
            "impl From<String> for MeasuredDigest",
            "impl From<&str> for MeasuredDigest",
            "impl std::str::FromStr for MeasuredDigest",
        ] {
            assert!(
                !source.contains(forbidden),
                "OMEGA-DELTA-0025: {forbidden} reintroduces a path from a claim \
                 into a measurement, in {}",
                path.display()
            );
        }
        assert!(
            !source.contains("Deserialize"),
            "OMEGA-DELTA-0025: MeasuredDigest became deserializable in {}. A \
             value that arrived as text is not a measurement this host made.",
            path.display()
        );
    }

    /// OMEGA-DELTA-0025. An external harness does not run unmeasured.
    ///
    /// Upstream resolves whichever version the ACP registry advertises,
    /// downloads it, and returns the command. A registry agent then runs with
    /// the tool permissions of the thread that started it, which is exactly the
    /// falsifier omega#81 names: a binary swapped under an auto-update path
    /// with no verifiable provenance record.
    ///
    /// So the gate has to sit between the installed tree and the returned
    /// command, and its refusal has to propagate. A gate whose result is logged
    /// and dropped is the rc11 defect — `appendSystemNote` bound to `() => {}`
    /// — in a different file.
    #[test]
    fn the_external_harness_launch_path_is_gated_on_a_measurement() {
        let path = repository_path(AGENT_SERVER_STORE_PATH);
        let source = std::fs::read_to_string(&path)
            .unwrap_or_else(|error| panic!("cannot read {}: {error}", path.display()));

        let gate = source
            .find("authorize_installed_harness(")
            .unwrap_or_else(|| {
                panic!(
                    "OMEGA-DELTA-0025: {} no longer measures an installed harness \
                 before returning its command",
                    path.display()
                )
            });
        let prefetch_gate = source.find("authorize_version_fetch(").unwrap_or_else(|| {
            panic!(
                "OMEGA-DELTA-0025: {} no longer consults the pin before fetching \
                 a harness version",
                path.display()
            )
        });

        // The archive agent is the only registry path that installs a tree
        // Omega controls, and its command construction must come after the
        // gate.
        let command = source[gate..]
            .find("let command = AgentServerCommand {")
            .map(|offset| gate + offset)
            .unwrap_or_else(|| {
                panic!(
                    "OMEGA-DELTA-0025: no AgentServerCommand is constructed after \
                     the provenance gate in {}. Either the gate moved below the \
                     command it is supposed to gate, or the command moved out of \
                     reach of this check.",
                    path.display()
                )
            });
        assert!(
            gate < command,
            "OMEGA-DELTA-0025: the provenance gate no longer precedes the \
             command it gates in {}",
            path.display()
        );
        assert!(
            prefetch_gate < gate,
            "OMEGA-DELTA-0025: the pin is consulted after the download rather \
             than before it in {}",
            path.display()
        );

        // The refusal has to be propagated, not logged. `.log_err()` on either
        // call would leave the harness running.
        for call in ["authorize_version_fetch(", "authorize_installed_harness("] {
            let start = source.find(call).expect("call site located above");
            let tail = &source[start..];
            let end = tail.find(";").expect("the call statement ends");
            assert!(
                tail[..end].contains(".await?") || tail[..end].ends_with(".await?"),
                "OMEGA-DELTA-0025: the result of {call} is not propagated in {}. \
                 A refusal that does not stop the launch is not a refusal.",
                path.display()
            );
        }
    }

    /// OMEGA-DELTA-0025. One decision, one receipt.
    ///
    /// `receipt_for_decision` takes the gate's own output, so a receipt cannot
    /// describe an outcome the gate did not reach. Calling the lower-level
    /// emitter from the enforcement path would reopen that gap: the receipt
    /// would be assembled from whatever the caller believed, beside a decision
    /// made somewhere else.
    #[test]
    fn the_enforcement_path_writes_receipts_only_from_decisions() {
        let path = repository_path(HARNESS_MAINTENANCE_PATH);
        let source = std::fs::read_to_string(&path)
            .unwrap_or_else(|error| panic!("cannot read {}: {error}", path.display()));

        assert!(
            source.contains("receipt_for_decision("),
            "OMEGA-DELTA-0025: {} no longer writes maintenance receipts",
            path.display()
        );
        let code_only: String = source
            .lines()
            .filter(|line| !line.trim_start().starts_with("//"))
            .collect::<Vec<_>>()
            .join("\n");
        assert!(
            !code_only.contains("build_harness_maintenance_receipt("),
            "OMEGA-DELTA-0025: {} builds a receipt without a decision. The \
             receipt and the enforcement would then be two answers to the same \
             question, and nothing on the wire would say which one ran.",
            path.display()
        );
        assert!(
            code_only.contains("decide_maintenance("),
            "OMEGA-DELTA-0025: {} no longer consults the maintenance gate",
            path.display()
        );
    }

    // ------------------------------------------------------ OMEGA-DELTA-0030

    /// OMEGA-DELTA-0030. A start request cannot carry what the host mints.
    ///
    /// omega#47 watched a live engine refuse a start request that deliberately
    /// carried a forged `evidence` block, a forged `decisionRef` and a forged
    /// `authorityReceiptRef`. That proof is about the host. This one is about
    /// the desktop: the request has no field to write the forgery into, so the
    /// claim holds before anything reaches the wire.
    #[test]
    fn a_full_auto_dispatch_carries_no_evidence() {
        let path = repository_path(FULL_AUTO_DISPATCH_PATH);
        let source = std::fs::read_to_string(&path)
            .unwrap_or_else(|error| panic!("cannot read {}: {error}", path.display()));

        let fields = struct_fields(&source, "FullAutoDispatch");
        assert!(
            !fields.is_empty(),
            "OMEGA-DELTA-0030: no FullAutoDispatch struct found in {}. The \
             check would be vacuous, so it fails instead.",
            path.display()
        );
        let found: Vec<(&str, &str)> = fields
            .iter()
            .map(|(name, type_name)| (name.as_str(), type_name.as_str()))
            .collect();
        assert_eq!(
            found, FULL_AUTO_DISPATCH_FIELDS,
            "OMEGA-DELTA-0030: FullAutoDispatch carries different parts than \
             the ones recorded. Evidence is minted by the host at the \
             completion-admission gate; a requester that can name it can forge \
             it. If a genuine request field was added, record it here and say \
             in the delta what the requester knows that the host does not."
        );

        // The origin is the whole of owner gate 8 at this layer: the only
        // constructor takes one, and every variant is a human gesture.
        assert!(
            source.contains("origin: LaunchOrigin,"),
            "OMEGA-DELTA-0030: a dispatch must record the human gesture that \
             produced it, in {}",
            path.display()
        );
        assert!(
            source.contains("pub fn from_validated("),
            "OMEGA-DELTA-0030: {} must expose exactly one constructor, so a \
             dispatch without an origin is not expressible.",
            path.display()
        );
    }

    /// OMEGA-DELTA-0030. Only the Full Auto surface can dispatch a run.
    ///
    /// Owner gate 8 says only an explicit human action starts Full Auto
    /// authority. The type enforces that a dispatch needs a `LaunchOrigin`;
    /// this enforces that no tool, model, or context-server crate is in a
    /// position to build one at all.
    #[test]
    fn no_model_callable_crate_can_dispatch_full_auto() {
        let mut offenders = Vec::new();
        let crates_root = repository_path("crates");
        for entry in std::fs::read_dir(&crates_root).expect("crates directory is readable") {
            let path = entry.expect("directory entry").path();
            if !path.is_dir() {
                continue;
            }
            let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
                continue;
            };
            if FULL_AUTO_DISPATCH_CALLERS.contains(&name) {
                continue;
            }
            let name = name.to_owned();
            for_each_source_file(&path, &["rs"], |file, contents| {
                if contents.contains("FullAutoDispatch") {
                    offenders.push(format!("{name}: {}", file.display()));
                }
            });
        }
        assert!(
            offenders.is_empty(),
            "OMEGA-DELTA-0030: the Full Auto start command is reachable from \
             crates that are not the Full Auto surface: {offenders:?}. Owner \
             gate 8 allows only an explicit human action to start Full Auto \
             authority, and a crate that can build the command is a crate that \
             can start a run."
        );
    }

    /// OMEGA-DELTA-0030. A thread shows the receipt chain of the run it names.
    ///
    /// omega#77 gave a thread its run reference. A reference the reader has to
    /// go elsewhere to resolve is not accountability, so the chain renders in
    /// the thread — and renders whatever it says, including a refusal.
    #[test]
    fn a_thread_renders_the_receipt_chain_of_its_linked_run() {
        let path = repository_path(THREAD_VIEW_PATH);
        let source = std::fs::read_to_string(&path)
            .unwrap_or_else(|error| panic!("cannot read {}: {error}", path.display()));

        assert!(
            source.contains("fn render_omega_run_link("),
            "OMEGA-DELTA-0030: {} must define the linked-run surface.",
            path.display()
        );
        assert!(
            source.contains(".children(self.render_omega_run_link(cx))"),
            "OMEGA-DELTA-0030: {} must draw the linked run in the thread. \
             Defining it without rendering it shows the reader nothing.",
            path.display()
        );
        assert!(
            source.contains("project_thread_run_link("),
            "OMEGA-DELTA-0030: the linked run in {} must be projected from the \
             engine's records on every draw, not read from a stored view.",
            path.display()
        );
    }

    /// OMEGA-DELTA-0030. The thread projects a run; it never owns one.
    ///
    /// omega#80's falsifier is "a run's source of truth ends up in a panel
    /// entity". Two halves. The thread stores the engine's *records* and their
    /// read time, never a projected link — a cached record can be re-derived
    /// and can expire, a cached conclusion silently outlives its source. And
    /// the projection module holds no state and no control: pause, resume and
    /// stop live on the Full Auto surface, bound to the run generation the host
    /// minted them for.
    #[test]
    fn the_thread_run_link_is_a_projection_and_not_a_second_authority() {
        let thread_view_path = repository_path(THREAD_VIEW_PATH);
        let thread_view = std::fs::read_to_string(&thread_view_path)
            .unwrap_or_else(|error| panic!("cannot read {}: {error}", thread_view_path.display()));
        assert!(
            thread_view.contains("omega_run_records: Option<ThreadRunRecords>,"),
            "OMEGA-DELTA-0030: {} must hold the engine's own records, so the \
             link can be re-derived and can go stale.",
            thread_view_path.display()
        );
        for stored_conclusion in [
            "omega_run_link: Option<ThreadRunLink>",
            "omega_run_state:",
            "omega_run_chain:",
        ] {
            assert!(
                !thread_view.contains(stored_conclusion),
                "OMEGA-DELTA-0030: {} stores `{stored_conclusion}`. A cached \
                 conclusion about a run is the panel entity becoming the \
                 source of truth that omega#80 forbids.",
                thread_view_path.display()
            );
        }

        let link_path = repository_path(THREAD_RUN_LINK_PATH);
        let link = std::fs::read_to_string(&link_path)
            .unwrap_or_else(|error| panic!("cannot read {}: {error}", link_path.display()));
        // Tests are excluded (they deliberately name refusals), and so are
        // comments: an earlier version of this check passed after the
        // constant was renamed, because the module documentation still
        // mentioned the old name. A check a comment can satisfy is not a
        // check.
        let body: String = link
            .split("#[cfg(test)]")
            .next()
            .unwrap_or(&link)
            .lines()
            .filter(|line| !line.trim_start().starts_with("//"))
            .collect::<Vec<_>>()
            .join("\n");
        let body = body.as_str();
        // `&'static str` is fine and common; a `static` *item* is process state.
        for forbidden in [
            "\nstatic ",
            "\npub static ",
            "thread_local!",
            "LazyLock",
            "Mutex",
            "RefCell",
            "supervisor",
            "pause_run",
            "stop_run",
            "resume_run",
            "start_run",
        ] {
            assert!(
                !body.contains(forbidden),
                "OMEGA-DELTA-0030: {} contains `{forbidden}`. The thread's run \
                 link is a pure projection: state would let it disagree with \
                 the engine, and a control would make it a second place that \
                 believes it can command a run.",
                link_path.display()
            );
        }
        assert!(
            body.contains("THREAD_RUN_LINK_MAX_AGE_MS"),
            "OMEGA-DELTA-0030: {} must expire the engine's answer. A thread \
             that keeps rendering the last chain it saw has outlived the \
             authority it is projecting.",
            link_path.display()
        );
        assert!(
            body.contains("project_issue31_evidence_pair"),
            "OMEGA-DELTA-0030: {} must read the chain through the omega#47 \
             producer. A second chain implementation is a second opinion about \
             one run, and the desktop and the phone would drift.",
            link_path.display()
        );
    }

    /// Read one `fn` body out of a Rust source file, by brace depth.
    ///
    /// A `contains` over the whole file cannot answer "does *this* function
    /// still refuse a projectless window", because the panel legitimately
    /// checks `has_open_project` in a dozen other places. Depth counting is
    /// crude but honest: it stops at the closing brace of the function it
    /// started on, and the test asserts it found a plausible body rather than
    /// an empty string, so a parse that silently matched nothing fails instead
    /// of passing.
    fn function_body<'a>(source: &'a str, name: &str) -> Option<&'a str> {
        let needle = format!(" fn {name}(");
        let start = source.find(&needle)? + needle.len();
        let open = source[start..].find('{')? + start;
        let mut depth = 0usize;
        for (offset, character) in source[open..].char_indices() {
            match character {
                '{' => depth += 1,
                '}' => {
                    depth -= 1;
                    if depth == 0 {
                        return Some(&source[open..open + offset]);
                    }
                }
                _ => {}
            }
        }
        None
    }

    /// Read one `const` or `struct` body out of a Rust source file.
    ///
    /// Cruder than `function_body` and enough for the two declarations that
    /// need it: it starts after the opening text and stops at the first line
    /// that closes at column zero or with a `];`.
    fn declaration_body<'a>(source: &'a str, opening: &str) -> Option<&'a str> {
        let start = source.find(opening)? + opening.len();
        let rest = &source[start..];
        let end = rest.find("\n];").or_else(|| rest.find("\n}"))?;
        Some(&rest[..end])
    }

    /// A source file with its line comments removed.
    ///
    /// `contains` cannot tell a call from a call somebody commented out, and a
    /// commented-out call is exactly how a wiring check stays green across the
    /// change that unwires it. Line comments only: a block comment spanning a
    /// call is not a spelling anybody reaches for, and removing them properly
    /// means tracking string literals.
    fn uncommented(source: &str) -> String {
        source
            .lines()
            .filter(|line| !line.trim_start().starts_with("//"))
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// OMEGA-DELTA-0042. A source file with its test module removed.
    ///
    /// Every scan below asks "does the shipped lane name this?", and the tests
    /// beside it must be free to name exactly the things it refuses — a test
    /// that asserts the wrong Exo is rejected has to write the wrong Exo down.
    /// Scanning the whole file would make those tests unwritable, which is the
    /// wrong pressure: it would push the refusals out of the file rather than
    /// into it.
    fn production_source(source: &str) -> &str {
        source
            .split_once("\n#[cfg(test)]")
            .map_or(source, |(before, _)| before)
    }

    /// OMEGA-DELTA-0042. Whether a token is named in code rather than in prose.
    fn named_in_code(source: &str, token: &str) -> bool {
        production_source(source)
            .lines()
            .filter(|line| line.contains(token))
            .any(|line| {
                let line = line.trim_start();
                !line.starts_with("//") && !line.starts_with("*")
            })
    }

    /// OMEGA-DELTA-0034. The front door works with no project open.
    ///
    /// Checked in both directions. Upstream's guard restored on a front-door
    /// path is the omega#76 defect coming back — a fresh install lands on the
    /// agent and has no composer to type into. A guard *removed* from a
    /// workspace-touching path is the opposite mistake: a terminal with no
    /// working directory, or a clipboard thread with nowhere to put its files,
    /// failing later and less legibly than a refusal would have.
    #[test]
    fn the_front_door_does_not_require_an_open_project() {
        let path = repository_path(AGENT_PANEL_PATH);
        let source = std::fs::read_to_string(&path)
            .unwrap_or_else(|error| panic!("cannot read {}: {error}", path.display()));

        for name in PROJECT_OPTIONAL_FRONT_DOOR_FNS {
            let body = function_body(&source, name).unwrap_or_else(|| {
                panic!(
                    "OMEGA-DELTA-0034: {} no longer has a `{name}` to check. \
                     If it was renamed, rename it here too; a check that \
                     cannot find its subject passes for the wrong reason.",
                    path.display()
                )
            });
            assert!(
                !body.contains("has_open_project"),
                "OMEGA-DELTA-0034: `{name}` in {} refuses a window with no \
                 project again. A window with nothing to restore is by \
                 definition a window with no project, so this is omega#76's \
                 exit failing: the front door opens and there is no composer \
                 to type into.",
                path.display()
            );
        }

        for name in PROJECT_REQUIRED_FNS {
            let body = function_body(&source, name).unwrap_or_else(|| {
                panic!(
                    "OMEGA-DELTA-0034: {} no longer has a `{name}` to check.",
                    path.display()
                )
            });
            assert!(
                body.contains("has_open_project"),
                "OMEGA-DELTA-0034: `{name}` in {} stopped requiring an open \
                 project. Project-optional *threads* is the delta; a terminal \
                 with no working directory, a resumed draft with no worktree, \
                 or a clipboard import with nowhere to land is not.",
                path.display()
            );
        }
    }

    /// OMEGA-DELTA-0013. The new-thread chord fires from anywhere in the window.
    ///
    /// Upstream binds `agent::NewThread` to `cmd-n` inside panel-scoped
    /// contexts only, so it cannot start a thread unless the panel already has
    /// focus. Omega's chord is window-global, which is the whole of omega#76's
    /// "from every context" — editor, welcome and panel are all inside
    /// `Workspace`.
    ///
    /// The check is two-sided, because each side alone is weak. A global
    /// binding that exists proves nothing if something narrower shadows it, and
    /// counting bindings would either forbid the modal pickers that legitimately
    /// hold the chord or permit any new binding at all.
    #[test]
    fn the_new_thread_chord_is_window_global() {
        for (keymap, chord) in NEW_THREAD_CHORDS {
            let path = repository_path(keymap);
            let raw = std::fs::read_to_string(&path)
                .unwrap_or_else(|error| panic!("cannot read {}: {error}", path.display()));
            let sections: serde_json::Value = serde_json::from_str(&strip_jsonc(&raw))
                .unwrap_or_else(|error| panic!("cannot parse {}: {error}", path.display()));
            let sections = sections
                .as_array()
                .unwrap_or_else(|| panic!("{keymap} is not an array of sections"));

            let bound: Vec<(Option<&str>, Option<&str>)> = sections
                .iter()
                .filter_map(|section| {
                    let binding = section.get("bindings")?.get(*chord)?;
                    Some((
                        section.get("context").and_then(serde_json::Value::as_str),
                        binding.as_str(),
                    ))
                })
                .collect();

            let global: Vec<&(Option<&str>, Option<&str>)> = bound
                .iter()
                .filter(|(context, _)| {
                    context.is_none_or(|context| WINDOW_GLOBAL_KEYMAP_CONTEXTS.contains(&context))
                })
                .collect();
            assert_eq!(
                global.len(),
                1,
                "OMEGA-DELTA-0013: {keymap} must bind {chord:?} window-globally \
                 exactly once. Found: {global:?}"
            );
            assert_eq!(
                global[0].1,
                Some("agent::NewThread"),
                "OMEGA-DELTA-0013: {keymap} binds {chord:?} window-globally to \
                 something other than the new agent thread, so omega#76's \
                 chord no longer reaches the front door."
            );

            for (context, action) in &bound {
                let Some(context) = context else { continue };
                if WINDOW_GLOBAL_KEYMAP_CONTEXTS.contains(context) {
                    continue;
                }
                assert!(
                    NEW_THREAD_CHORD_NARROW_CONTEXTS.contains(context),
                    "OMEGA-DELTA-0013: {keymap} binds {chord:?} to {action:?} in \
                     context {context:?}, which shadows the window-global new-thread \
                     chord while that surface has focus. omega#76 asked for the \
                     shadowed bindings to be resolved deliberately; a new one \
                     appearing is not a deliberate resolution. Admitted: \
                     {NEW_THREAD_CHORD_NARROW_CONTEXTS:?}"
                );
            }
        }
    }

    /// OMEGA-DELTA-0040. A first-ever launch lands on identity onboarding, and
    /// finishing it opens the front door.
    ///
    /// The owner decided the ordering: Omega is identity-first, so an agent
    /// thread before an identity would invert the thing omega#9's packet exists
    /// to establish. That decision is only sound while the handoff is real —
    /// "onboarding first" and "onboarding instead" are the same picture on a
    /// first launch and completely different products on the second.
    ///
    /// So the chain is checked link by link, because each link fails silently
    /// on its own:
    ///
    /// - the startup path *waits* — without the await, the front door would
    ///   open behind onboarding and Omega would be asking for an identity over
    ///   the top of a composer;
    /// - finishing *releases* the wait — without the release, completing setup
    ///   would leave the user on the launchpad with nothing else ever opening,
    ///   and no test that only looks at the front door would notice;
    /// - releasing *completes the channel* the startup path is parked on —
    ///   without that, `release_identity_waiters` is a call that returns.
    #[test]
    fn first_run_onboarding_hands_the_startup_off_to_the_front_door() {
        let startup_path = repository_path(STARTUP_PATH);
        let startup = std::fs::read_to_string(&startup_path)
            .unwrap_or_else(|error| panic!("cannot read {}: {error}", startup_path.display()));
        let restore = function_body(&startup, "restore_or_create_workspace").unwrap_or_else(|| {
            panic!(
                "OMEGA-DELTA-0040: {} no longer has a `restore_or_create_workspace`. \
                 A check that cannot find its subject passes for the wrong reason.",
                startup_path.display()
            )
        });

        let waits_at = restore.find("await_identity_ready(").unwrap_or_else(|| {
            panic!(
                "OMEGA-DELTA-0040: `restore_or_create_workspace` in {} no longer \
                 waits for identity. The front door would open behind first-run \
                 onboarding, which is Omega asking for an identity on top of a \
                 composer — the inversion the owner's ordering decision rejects.",
                startup_path.display()
            )
        });
        let opens_at = restore
            .find("AgentPanel::open_front_door(")
            .unwrap_or_else(|| {
                panic!(
                    "OMEGA-DELTA-0040: `restore_or_create_workspace` in {} no \
                     longer opens the front door at all (OMEGA-DELTA-0019).",
                    startup_path.display()
                )
            });
        assert!(
            waits_at < opens_at,
            "OMEGA-DELTA-0040: {} opens the front door before it waits for \
             identity. Onboarding is first *and* the agent is what follows it; \
             reversing them makes the first-run window a race.",
            startup_path.display()
        );

        let onboarding_path = repository_path(ONBOARDING_PATH);
        let onboarding = std::fs::read_to_string(&onboarding_path)
            .unwrap_or_else(|error| panic!("cannot read {}: {error}", onboarding_path.display()));
        let finish = function_body(&onboarding, "on_finish").unwrap_or_else(|| {
            panic!(
                "OMEGA-DELTA-0040: {} no longer has an `on_finish` to check.",
                onboarding_path.display()
            )
        });
        // The **first-run** arm specifically. `on_finish` releases the waiters
        // on both journeys, and the editor-setup journey is not the one the
        // startup path is parked on — asserting against the whole function
        // would let the first-run release be deleted while the check stayed
        // green on the other arm's copy of the same call.
        let first_run_arm = finish
            .split_once("OnboardingMode::FirstRun(window_handle) => {")
            .and_then(|(_, rest)| rest.split_once("OnboardingMode::EditorSetup"))
            .map(|(arm, _)| arm)
            .unwrap_or_else(|| {
                panic!(
                    "OMEGA-DELTA-0040: `on_finish` in {} no longer has a \
                     first-run arm to check.",
                    onboarding_path.display()
                )
            });
        assert!(
            first_run_arm.contains("release_identity_waiters(cx)"),
            "OMEGA-DELTA-0040: finishing first-run onboarding in {} no longer \
             releases the startup path. Setup would complete and nothing would \
             open: the user is left on the launchpad, the agent dock closed, \
             and the only way forward is relaunching the app.",
            onboarding_path.display()
        );
        assert!(
            first_run_arm.contains("window.remove_window()"),
            "OMEGA-DELTA-0040: the first-run branch of `on_finish` in {} no \
             longer closes its own window, so the front door would open beside \
             a finished onboarding screen rather than instead of it.",
            onboarding_path.display()
        );

        let coordinator_path = repository_path(IDENTITY_STARTUP_PATH);
        let coordinator = std::fs::read_to_string(&coordinator_path)
            .unwrap_or_else(|error| panic!("cannot read {}: {error}", coordinator_path.display()));
        let release =
            function_body(&coordinator, "release_identity_waiters").unwrap_or_else(|| {
                panic!(
                    "OMEGA-DELTA-0040: {} no longer has a `release_identity_waiters`.",
                    coordinator_path.display()
                )
            });
        assert!(
            release.contains("finish(Ok(()), cx)"),
            "OMEGA-DELTA-0040: `release_identity_waiters` in {} no longer \
             completes the startup channel, so it releases nobody.",
            coordinator_path.display()
        );
    }

    /// OMEGA-DELTA-0035. The router is what the native agent entry resolves to.
    ///
    /// omega#78 shipped the router unwired, which is the failure this guards:
    /// every piece present, tested, and reachable by nobody. Three facts, and
    /// each one alone is weak — the server could be built and never connected,
    /// the poll could exist and feed nothing, the pin could exist and be
    /// unreachable.
    #[test]
    fn the_router_is_wired_into_the_native_agent_entry() {
        let factory_path = repository_path(AGENT_SERVER_FACTORY_PATH);
        let factory = std::fs::read_to_string(&factory_path)
            .unwrap_or_else(|error| panic!("cannot read {}: {error}", factory_path.display()));
        assert!(
            factory.contains("omega_router::OmegaRouterServer::new("),
            "OMEGA-DELTA-0035: {} builds the native agent server directly \
             again, so nothing constructs an OmegaAgentConnection and every \
             thread discloses `route: None` with an empty journal — the state \
             omega#78 shipped in.",
            factory_path.display()
        );

        let panel_path = repository_path(AGENT_PANEL_PATH);
        let panel = std::fs::read_to_string(&panel_path)
            .unwrap_or_else(|error| panic!("cannot read {}: {error}", panel_path.display()));
        assert!(
            panel.contains("observe_capacity(Ok(capacity))") && panel.contains("get_capacity()"),
            "OMEGA-DELTA-0035: {} no longer feeds the engine's framed \
             get_capacity answer into the router. Without it every engine-lane \
             pin is decided against a default of \"not running\" whatever \
             omega-effectd is actually doing.",
            panel_path.display()
        );

        let disclosure_path = repository_path(THREAD_VIEW_PATH);
        let disclosure = std::fs::read_to_string(&disclosure_path)
            .unwrap_or_else(|error| panic!("cannot read {}: {error}", disclosure_path.display()));
        // OMEGA-DELTA-0055 replaced the assertion that used to be here. It
        // required the thread surface to render the executor pin control,
        // because a pin was the only way a thread reached anything but the
        // native loop. That is no longer true and the control is gone, so the
        // rule became one protecting a control the owner asked to remove.
        //
        // What replaced it is the thing the pin was for: an unpinned thread
        // reaching an attached external agent. If that arm goes, the pin's
        // removal really would have left the Exo lane unreachable, which is the
        // failure the old assertion was written against.
        assert!(
            !disclosure.contains("\"omega-executor-pin\""),
            "OMEGA-DELTA-0035: {} renders the executor pin control again.",
            disclosure_path.display()
        );
        let router_path = repository_path(ROUTE_DECISION_PATH);
        let law = std::fs::read_to_string(&router_path)
            .unwrap_or_else(|error| panic!("cannot read {}: {error}", router_path.display()));
        assert!(
            law.contains("if inputs.external_acp.is_some() {"),
            "OMEGA-DELTA-0035: {} no longer routes an unpinned thread to an \
             attached external agent, and the pin control that used to be the \
             only door is gone. Together that makes the Exo lane unreachable.",
            router_path.display()
        );
    }

    /// OMEGA-DELTA-0035, and owner gate 8 behind it.
    ///
    /// A pin is the only door to an engine lane and an engine lane *is* Full
    /// Auto authority, so the gate reaches the pin as directly as it reaches
    /// the Start button. The guard is the argument type: every pin-setting call
    /// must pass a literal `PinGesture::` variant, so a caller cannot launder
    /// a gesture it was handed by something model-facing.
    ///
    /// Scanned across every crate rather than the one file, because the failure
    /// worth catching is a *new* caller somewhere else.
    #[test]
    fn only_a_named_human_gesture_can_pin_an_executor() {
        let crates = repository_path("crates");
        let mut calls: Vec<(String, String)> = Vec::new();
        for_each_source_file(&crates, &["rs"], |path, source| {
            let display = path
                .display()
                .to_string()
                .rsplit("crates/")
                .next()
                .unwrap_or_default()
                .to_owned();
            // This file is the check itself; matching its own needles would
            // make the count meaningless.
            if display.starts_with("omega_deltas/") {
                return;
            }
            for (index, line) in source.lines().enumerate() {
                let trimmed = line.trim();
                // The declarations are not calls. `fn pin_session(` and a
                // doc-comment mentioning it are both skipped here so the scan
                // counts callers only.
                if trimmed.starts_with("//")
                    || trimmed.starts_with("pub fn ")
                    || trimmed.starts_with("fn ")
                {
                    continue;
                }
                if !PIN_SETTING_CALLS.iter().any(|call| trimmed.contains(call)) {
                    continue;
                }
                // Reassemble the whole call, because rustfmt puts the arguments
                // of a nested call on their own lines. Walking forward to the
                // line whose parens balance is what makes this a check on the
                // *argument list* rather than on one line of it — the earlier
                // draft accepted any line ending in `(` or `,`, which passed
                // for every multi-line call regardless of its arguments.
                let mut statement = String::new();
                let mut depth = 0i32;
                for following in source.lines().skip(index) {
                    statement.push_str(following.trim());
                    statement.push(' ');
                    for character in following.chars() {
                        match character {
                            '(' => depth += 1,
                            ')' => depth -= 1,
                            _ => {}
                        }
                    }
                    if depth <= 0 {
                        break;
                    }
                }
                calls.push((display.clone(), statement));
            }
        });

        assert!(
            !calls.is_empty(),
            "OMEGA-DELTA-0035: no pin-setting call was found anywhere. Either \
             the pin control is gone or this scan stopped matching, and both \
             make the gate vacuous."
        );

        for (file, statement) in &calls {
            assert!(
                statement.contains("PinGesture::"),
                "OMEGA-DELTA-0035: {file} sets a pin without naming a literal \
                 PinGesture: {statement:?}. Owner gate 8 admits only an \
                 explicit human action into Full Auto authority, a pin is the \
                 only door to an engine lane, and a gesture the caller was \
                 *handed* is exactly the laundering the literal forbids."
            );
        }
    }

    /// OMEGA-DELTA-0035. Nothing asks "is this the native agent?" with a bare
    /// downcast.
    ///
    /// The first-party agent is a router over the native server now. A bare
    /// `downcast::<NativeAgentServer>()` on it returns `None`, which reads as
    /// "this is an external agent" — a silently wrong `false`, not a compile
    /// error. The two call sites that ask this question go through
    /// `is_native_agent_server`, and this fails if a third appears.
    #[test]
    fn nothing_asks_for_the_native_agent_with_a_bare_downcast() {
        let crates = repository_path("crates");
        let mut bare: Vec<(String, String)> = Vec::new();
        for_each_source_file(&crates, &["rs"], |path, source| {
            let display = path
                .display()
                .to_string()
                .rsplit("crates/")
                .next()
                .unwrap_or_default()
                .to_owned();
            if display.starts_with("omega_deltas/")
                // The unwrapping helpers are where the bare downcast belongs.
                || display.starts_with("agent_ui/src/omega_router.rs")
                // omega#77's disclosure classifies a *thread's* connection,
                // which is the executor's and never the router's.
                || display.starts_with("agent_ui/src/omega_executor_disclosure.rs")
            {
                return;
            }
            for line in source.lines() {
                let line = line.trim();
                if line.contains("downcast::<agent::NativeAgentServer>()")
                    || line.contains("downcast::<NativeAgentServer>()")
                {
                    bare.push((display.clone(), line.to_owned()));
                }
            }
        });
        assert!(
            bare.is_empty(),
            "OMEGA-DELTA-0035: {} place(s) still ask for the native agent \
             server with a bare downcast: {bare:#?}. The native agent is \
             wrapped by the router, so this answers `false` for the \
             first-party agent instead of failing to compile. Use \
             `omega_router::is_native_agent_server`.",
            bare.len()
        );
    }

    /// The parser has to actually strip comments, or every check reads `None`
    /// and silently passes.
    #[test]
    fn the_settings_parser_reaches_real_values() {
        let settings = default_settings().expect("default settings parse");
        assert!(
            default_setting(&settings, "session.restore_unsaved_buffers").is_some(),
            "settings parser did not reach a known key; the checks would be vacuous"
        );
        assert!(
            default_setting(&settings, "session.no_such_key").is_none(),
            "settings lookup must return None for an absent key"
        );
    }

    // ------ OMEGA-DELTA-0032

    /// OMEGA-DELTA-0032. Upstream Zed decides a send during a running turn in
    /// the view: `MessageQueue::front_wants_steer` sets a boundary flag on the
    /// native thread, and every other executor falls through to a cancel. That
    /// is three different behaviours behind one button, and the user cannot
    /// tell which one they got.
    ///
    /// Omega decides it with a total law over all three executor classes. This
    /// checks the law is where the delta says it is and answers for each class
    /// by name — a law that never mentions a class cannot have declared
    /// anything for it.
    #[test]
    fn the_send_during_turn_law_answers_for_every_executor_class() {
        let path = repository_path(SEND_DURING_TURN_PATH);
        let source = std::fs::read_to_string(&path).expect("the send law is readable");
        for token in SEND_LAW_EXECUTOR_TOKENS {
            assert!(
                source.contains(&format!("ExecutorClass::{token}")),
                "OMEGA-DELTA-0035: {} does not answer for ExecutorClass::{token}.                  A class the law does not name has no declared behaviour, which                  is the state this delta replaces.",
                path.display()
            );
        }
        assert!(
            source.contains("pub const fn disposition("),
            "OMEGA-DELTA-0035: the law must be a const fn of its inputs. A              disposition that can read anything else is not reproducible from a              journal."
        );
    }

    /// OMEGA-DELTA-0032. The law and the journal are pure the way the router is
    /// pure. A queued message restored from disk re-derives its disposition, so
    /// a clock or a hash order in either file would make the same record decide
    /// differently on the next launch.
    #[test]
    fn the_queue_law_and_its_journal_read_nothing_but_their_inputs() {
        for relative in [SEND_DURING_TURN_PATH, SEND_QUEUE_JOURNAL_PATH] {
            let path = repository_path(relative);
            let source = std::fs::read_to_string(&path).expect("readable");
            // The test module is allowed a temporary directory and its own
            // scaffolding; the law is not.
            let production = source
                .split("#[cfg(test)]")
                .next()
                .expect("a file has a first section");
            for (why, token) in NON_DETERMINISTIC_QUEUE_TOKENS {
                assert!(
                    !production.contains(token),
                    "OMEGA-DELTA-0035: {} reads {why} (`{token}`). The queue is                      replayed from a journal, and a decision that depends on                      {why} does not replay.",
                    path.display()
                );
            }
        }
    }

    /// OMEGA-DELTA-0032. The composer asks the law rather than reading the
    /// steer flag.
    ///
    /// This is the exact upstream line the delta replaces. Reading
    /// `front_wants_steer` to set the boundary flag is what made "steer" mean
    /// "end at a boundary" on the native loop and "cancel the turn" everywhere
    /// else, and it is why an engine lane and an external ACP peer both got a
    /// behaviour nobody declared for them.
    #[test]
    fn the_composer_decides_a_mid_turn_send_through_the_law() {
        let path = repository_path(CONVERSATION_SEND_PATH);
        let source = std::fs::read_to_string(&path).expect("the composer is readable");
        assert!(
            source.contains("omega_front_door::disposition("),
            "OMEGA-DELTA-0035: {} no longer calls the send law. Deciding a              mid-turn send in the view is the upstream behaviour this delta              replaces.",
            path.display()
        );
        assert!(
            source.contains("SendDisposition::SteerAtMessageBoundary"),
            "OMEGA-DELTA-0035: the boundary flag must be set from the law's              own answer, not from a steer flag the other two classes never see."
        );
        assert!(
            source.contains(".reaches_running_turn()"),
            "OMEGA-DELTA-0035: {} must gate its cancel on whether this              executor's declared answer reaches the running turn. An              unconditional cancel turns a refused steer into an interrupted              turn.",
            path.display()
        );
    }

    /// OMEGA-DELTA-0032. Queue state is durable, and its acknowledgement is the
    /// write.
    ///
    /// Upstream holds the queue on the view: `Entity<MessageEditor>` handles in
    /// a `VecDeque` that dies with the panel. A message the composer had
    /// already called "queued" did not exist after a restart. The falsifier for
    /// omega#79 names this directly.
    #[test]
    fn the_send_queue_is_a_durable_record_and_not_renderer_memory() {
        let path = repository_path(SEND_QUEUE_JOURNAL_PATH);
        let source = std::fs::read_to_string(&path).expect("the journal is readable");
        assert!(
            source.contains("openagents.omega.agent_send_queue.v1"),
            "OMEGA-DELTA-0035: the durable queue must carry a schema, so a              foreign document is refused rather than adopted."
        );
        assert!(
            source.contains("std::fs::rename(&temporary, &self.path)"),
            "OMEGA-DELTA-0035: {} must rewrite atomically. A crash mid-write              must leave the previous queue, not a truncated one.",
            path.display()
        );
        assert!(
            !source.contains("Entity<MessageEditor>"),
            "OMEGA-DELTA-0035: a live GPUI handle cannot be a durable fact.              That is exactly what made the upstream queue renderer-only."
        );
    }

    // ------------------------------------------------------ OMEGA-DELTA-0033

    /// OMEGA-DELTA-0033. The front door renders the decision; it does not make
    /// one.
    ///
    /// omega#81 landed a decision layer nothing rendered, and the settings page
    /// is the first thing to render it. The risk that creates is a page that
    /// starts deciding: an `if` on a pin, a locally composed reason, a control
    /// enabled because the widget thought it should be. Any of those and the
    /// front door and the launch gate can disagree, which shows up to an owner
    /// as a button that looks live and then fails.
    ///
    /// So the page may match on the decision layer's types and may not call the
    /// decision functions itself.
    #[test]
    fn the_front_door_page_renders_decisions_it_did_not_make() {
        let path = repository_path(EXTERNAL_AGENTS_PAGE_PATH);
        let source = std::fs::read_to_string(&path)
            .unwrap_or_else(|error| panic!("cannot read {}: {error}", path.display()));

        assert!(
            source.contains("PinControl::Take")
                && source.contains("PinControl::Remove")
                && source.contains("PinControl::Unavailable"),
            "OMEGA-DELTA-0033: {} must render all three pin-control states. A \
             page that renders two of them silently drops the case where the \
             control is withheld, which is the case that needs a sentence most.",
            path.display()
        );
        for decider in [
            "decide_maintenance(",
            "admits_version(",
            "admits_package_manager_launch(",
            "update_affordance(",
            "harness_front_door_state(",
        ] {
            assert!(
                !source.contains(decider),
                "OMEGA-DELTA-0033: {} calls {decider} itself. The page must \
                 render the state the store computed, or the row and the gate \
                 become two answers to the same question.",
                path.display()
            );
        }
        assert!(
            !source.contains("MaintenanceAffordance::Disabled"),
            "OMEGA-DELTA-0033: {} constructs a refusal of its own. Every \
             sentence on this page comes from omega_harness.",
            path.display()
        );
    }

    /// OMEGA-DELTA-0033. A withheld control still says why.
    ///
    /// The reason `MaintenanceAffordance::Disabled` carries a `String` rather
    /// than an `Option<String>` is that omega 0.2.0-rc11 shipped a refusal
    /// nobody could see. [`PinControl`] has to hold the same shape, and the
    /// page has to put both sentences somewhere a person reads.
    #[test]
    fn a_withheld_control_carries_a_sentence_all_the_way_to_the_widget() {
        let decisions = repository_path(HARNESS_FRONT_DOOR_PATH);
        let source = std::fs::read_to_string(&decisions)
            .unwrap_or_else(|error| panic!("cannot read {}: {error}", decisions.display()));
        assert!(
            source.contains("Unavailable { reason: String }")
                || source.contains("Unavailable {\n        /// The sentence")
                || source.contains("Unavailable { reason: String },"),
            "OMEGA-DELTA-0033: {} must make the withheld pin control carry its \
             reason by construction. An `Option` here is a control that can be \
             withheld silently.",
            decisions.display()
        );

        let page = repository_path(EXTERNAL_AGENTS_PAGE_PATH);
        let rendered = std::fs::read_to_string(&page)
            .unwrap_or_else(|error| panic!("cannot read {}: {error}", page.display()));
        assert!(
            rendered.contains("state.launch.reason()"),
            "OMEGA-DELTA-0033: {} no longer renders the launch refusal. A \
             refusal that reaches the owner only as agent-launch error text is \
             the gap omega#81 stayed open for.",
            page.display()
        );
        assert!(
            rendered.contains("PinControl::Unavailable { reason }")
                && rendered
                    .contains("Tooltip::with_meta(\"Cannot Pin\", None, reason.clone(), cx)"),
            "OMEGA-DELTA-0033: {} must show the withheld control's reason. A \
             disabled button with no sentence reads as a bug in Omega rather \
             than as a fact about the owner's machine.",
            page.display()
        );
    }

    /// OMEGA-DELTA-0033. The pin controls exist in production code.
    ///
    /// `HarnessPinLedger::set_pin` and `remove_pin` existed before this delta
    /// and were called only by tests: the ledger was a JSON file with no writer
    /// Omega shipped. A "pin" an owner can only take by hand-editing a file is
    /// not a control, and the standing rule is that owner-facing operations get
    /// working controls rather than a runbook.
    #[test]
    fn the_pin_ledger_has_a_writer_the_owner_can_reach() {
        let filesystem = repository_path(HARNESS_MAINTENANCE_PATH);
        let source = std::fs::read_to_string(&filesystem)
            .unwrap_or_else(|error| panic!("cannot read {}: {error}", filesystem.display()));
        for writer in [
            "pub async fn pin_installed_harness(",
            "pub async fn unpin_harness(",
        ] {
            assert!(
                source.contains(writer),
                "OMEGA-DELTA-0033: {} no longer offers {writer}. Without it the \
                 ledger is a file the owner must edit by hand.",
                filesystem.display()
            );
        }
        assert!(
            source.contains("encode_harness_pin_ledger(ledger)?"),
            "OMEGA-DELTA-0033: {} must write the ledger through its own \
             encoder, which routes back through the reader. A writer that can \
             emit a file its reader refuses turns the next restart into the \
             moment every pin fails closed.",
            filesystem.display()
        );

        let page = repository_path(EXTERNAL_AGENTS_PAGE_PATH);
        let rendered = std::fs::read_to_string(&page)
            .unwrap_or_else(|error| panic!("cannot read {}: {error}", page.display()));
        assert!(
            rendered.contains("this.pin_harness(&id, cx)")
                && rendered.contains("this.unpin_harness(&id, cx)"),
            "OMEGA-DELTA-0033: {} must wire both pin controls to something a \
             person can press.",
            page.display()
        );
    }

    /// OMEGA-DELTA-0033. The npx launch path consults the ledger.
    ///
    /// `LocalRegistryNpxAgent` resolves its package inside the node runtime's
    /// own cache, so there is no tree for the measured gate to hash — and
    /// before this delta that meant pinning such a harness did nothing at all.
    /// The owner's "not that one" has to refuse even where Omega cannot verify
    /// the bytes, or the pin is a decoration.
    #[test]
    fn the_package_manager_launch_path_is_gated_on_the_pin() {
        let path = repository_path(AGENT_SERVER_STORE_PATH);
        let source = std::fs::read_to_string(&path)
            .unwrap_or_else(|error| panic!("cannot read {}: {error}", path.display()));

        let gate = source
            .find("authorize_package_manager_launch(")
            .unwrap_or_else(|| {
                panic!(
                    "OMEGA-DELTA-0033: {} no longer consults the pin on the \
                     package-manager launch path",
                    path.display()
                )
            });
        // Same rule as OMEGA-DELTA-0025: the refusal must stop the launch, not
        // be logged beside it.
        let tail = &source[gate..];
        let end = tail.find(';').expect("the call statement ends");
        assert!(
            tail[..end].contains(".await?"),
            "OMEGA-DELTA-0033: the result of authorize_package_manager_launch \
             is not propagated in {}. A refusal that does not stop the launch \
             is not a refusal.",
            path.display()
        );

        let npx_command = source
            .find("let command = AgentServerCommand {")
            .map(|_| {
                source[gate..]
                    .find("let command = AgentServerCommand {")
                    .map(|offset| gate + offset)
                    .expect("the npx command is constructed after the gate")
            })
            .expect("a command is constructed somewhere");
        assert!(
            gate < npx_command,
            "OMEGA-DELTA-0033: the package-manager gate no longer precedes the \
             command it gates in {}",
            path.display()
        );
    }

    /// OMEGA-DELTA-0033. Resolving the channel is its own recorded action.
    ///
    /// omega#81's first deliverable named four actions and landed three.
    /// Resolving what the channel advertises happens when nothing is about to
    /// launch, so if it is not recorded there it is not recorded at all — and a
    /// frozen harness offered an update it will refuse is the front door
    /// promising what the gate takes back.
    #[test]
    fn resolving_a_channel_is_a_recorded_action_that_gates_the_offer() {
        let filesystem = repository_path(HARNESS_MAINTENANCE_PATH);
        let source = std::fs::read_to_string(&filesystem)
            .unwrap_or_else(|error| panic!("cannot read {}: {error}", filesystem.display()));
        assert!(
            source.contains("MaintenanceAction::ResolveChannel"),
            "OMEGA-DELTA-0033: {} no longer records channel resolution under \
             its own action.",
            filesystem.display()
        );
        assert!(
            source.contains("MaintenanceAction::ReprobeCapability"),
            "OMEGA-DELTA-0033: {} no longer records a re-probe under its own \
             action. Collapsing it into Verify would leave the log unable to \
             say whether a measurement was taken because something was about \
             to run or because a person asked.",
            filesystem.display()
        );

        let store = repository_path(AGENT_SERVER_STORE_PATH);
        let launch = std::fs::read_to_string(&store)
            .unwrap_or_else(|error| panic!("cannot read {}: {error}", store.display()));
        let resolve = launch.find("resolve_channel(").unwrap_or_else(|| {
            panic!(
                "OMEGA-DELTA-0033: {} no longer resolves the channel before \
                 announcing a new version",
                store.display()
            )
        });
        let announce = launch[resolve..]
            .find("tx.send(Some(version))")
            .map(|offset| resolve + offset)
            .unwrap_or_else(|| {
                panic!(
                    "OMEGA-DELTA-0033: no version is announced after the \
                     channel is resolved in {}. Either the announcement moved \
                     above the gate or it moved out of reach of this check.",
                    store.display()
                )
            });
        assert!(
            resolve < announce,
            "OMEGA-DELTA-0033: a version is announced before the pin is \
             consulted in {}",
            store.display()
        );
    }

    /// OMEGA-DELTA-0033. The front door measures the tree the launch path
    /// gates.
    ///
    /// The measured tree is not the installation directory — it is a version
    /// directory whose name is derived from the version, the archive URL and
    /// its checksum. A settings page that measured the parent would attest a
    /// different set of bytes than the gate reads, and would then show
    /// "verified" for an installation the launch path refuses.
    #[test]
    fn the_front_door_measures_the_tree_the_launch_path_gates() {
        let path = repository_path(AGENT_SERVER_STORE_PATH);
        let source = std::fs::read_to_string(&path)
            .unwrap_or_else(|error| panic!("cannot read {}: {error}", path.display()));
        // Tests call it freely; production must not. Two derivations of the
        // measured tree outside the test module are the launch path's and the
        // front door's, and they are what this check pins.
        let production = source
            .split_once("#[cfg(test)]")
            .map(|(before, _)| before)
            .unwrap_or(source.as_str());
        assert_eq!(
            production.matches("versioned_archive_cache_dir(").count(),
            3,
            "OMEGA-DELTA-0033: {} must derive the measured tree in exactly the \
             places this check knows about — the definition, the launch path, \
             and the front door's target. A new caller is a new way for the row \
             and the gate to disagree.",
            path.display()
        );
        assert!(
            source.contains("fn installed_version_dir(&self) -> Option<PathBuf>"),
            "OMEGA-DELTA-0033: {} no longer tells the front door which tree the \
             launch path measures.",
            path.display()
        );
    }

    /// OMEGA-DELTA-0036. `--uninstall` removes Omega and nothing else.
    ///
    /// The shipped, signed `0.2.0-rc14` advertised `--uninstall` as "Uninstall
    /// Omega from user system" and ran upstream's uninstaller verbatim: it
    /// deleted the other editor's application bundle, its whole
    /// application-support tree, its logs, caches, preferences and saved state,
    /// asked whether to keep *that* product's preferences, printed that *that*
    /// product had been uninstalled — and removed no Omega path at all
    /// (omega#88).
    ///
    /// The end-to-end proof lives in `crates/cli/src/uninstall.rs`, where the
    /// real script runs against a fabricated home holding both an Omega
    /// installation and another product's, and both halves are read back. This
    /// asserts the two structural properties that made the defect possible: the
    /// script names no other product, and it has no path table of its own.
    #[test]
    fn the_uninstall_path_removes_omega_and_names_no_competitor() {
        let policy = brand_policy().expect("brand policy parses");
        let script_path = repository_path(UNINSTALL_SCRIPT_PATH);
        let script = std::fs::read_to_string(&script_path)
            .unwrap_or_else(|error| panic!("cannot read {}: {error}", script_path.display()));

        let hits = brand_hits(&script, &policy);
        assert!(
            hits.is_empty(),
            "OMEGA-DELTA-0036: {UNINSTALL_SCRIPT_PATH} names {hits:?}. It is \
             embedded in the signed `cli` binary with include_bytes! and it \
             removes whatever it names, so a competitor's directory appearing \
             here is a destructive regression, not a copy regression."
        );

        for required in ["OMEGA_UNINSTALL_PATHS", "OMEGA_UNINSTALL_PRODUCT"] {
            assert!(
                script.contains(required),
                "OMEGA-DELTA-0036: {UNINSTALL_SCRIPT_PATH} no longer reads \
                 {required} from the caller, so it has a hand-written path \
                 table again. A table disconnected from the code that creates \
                 those directories is exactly how omega#88 shipped."
            );
        }

        // Every root the plan removes is read from the function that writes it.
        let plan_path = repository_path(UNINSTALL_PLAN_PATH);
        let plan = std::fs::read_to_string(&plan_path)
            .unwrap_or_else(|error| panic!("cannot read {}: {error}", plan_path.display()));
        let constructor = plan
            .split("pub fn from_installed_paths")
            .nth(1)
            .expect("OMEGA-DELTA-0036: from_installed_paths is gone");
        let derived = constructor.matches("paths::").count();
        assert!(
            derived >= 6,
            "OMEGA-DELTA-0036: from_installed_paths makes only {derived} \
             `paths::` calls. Every root has to come from the function that \
             writes it; a literal path here is a second source of truth, and \
             the first one that disagreed cost a user their other editor."
        );
        assert!(
            plan.contains("let Self {"),
            "OMEGA-DELTA-0036: {} no longer destructures UninstallRoots \
             exhaustively in `plan`, so a root can be added to the struct and \
             silently left out of the plan.",
            plan_path.display()
        );

        // The script itself, run for real, refuses a plan it cannot trust.
        #[allow(
            clippy::disallowed_methods,
            reason = "A gate that only reads the script cannot tell whether it \
                      refuses; this one runs it. There is no async runtime here."
        )]
        let output = std::process::Command::new("sh")
            .arg(&script_path)
            .env("OMEGA_UNINSTALL_PRODUCT", "Omega RC")
            .env("OMEGA_UNINSTALL_PATHS", "")
            .output()
            .expect("run the uninstall script");
        assert!(
            !output.status.success(),
            "OMEGA-DELTA-0036: the uninstall script accepted an empty plan. \
             Refusing is the safe direction; every default this file has ever \
             had belonged to somebody else's product."
        );
    }

    /// OMEGA-DELTA-0043. `--uninstall` plans the installation, not one file.
    ///
    /// `0.2.0-rc16` fixed the destructive half of omega#88 and still did not
    /// remove Omega: `main.rs` handed `app.path()` — `Omega.app/Contents/MacOS/
    /// omega` — to a field documented as "the application bundle or executable",
    /// so a completed uninstall left `/Applications/Omega.app` in place with
    /// 130.9 MB, five executables including a `cli` that still carries
    /// `--uninstall`, and a bundled Node runtime (omega#92).
    ///
    /// The behavioural proof is in `crates/cli/src/uninstall.rs`, where the
    /// shipped script runs against a fabricated home whose bundle holds several
    /// executables and every one of them is read back. This asserts the two
    /// structural properties: the call site asks for the installation rather
    /// than the executable, and the constructor normalizes what it is given so
    /// a caller cannot reintroduce the defect from outside.
    #[test]
    fn the_uninstall_plan_names_the_installation_root() {
        let plan_path = repository_path(UNINSTALL_PLAN_PATH);
        let plan = std::fs::read_to_string(&plan_path)
            .unwrap_or_else(|error| panic!("cannot read {}: {error}", plan_path.display()));
        assert!(
            plan.contains("pub fn installation_root("),
            "OMEGA-DELTA-0043: {} no longer derives an installation root, so \
             whatever path a caller happens to be holding is what gets removed.",
            plan_path.display()
        );
        let constructor = plan
            .split("pub fn from_installed_paths")
            .nth(1)
            .and_then(|tail| tail.split("\n    }").next())
            .expect("OMEGA-DELTA-0043: from_installed_paths is gone");
        assert!(
            constructor.contains("installation_root"),
            "OMEGA-DELTA-0043: from_installed_paths takes the caller's path as \
             the installation root. Every macOS caller holds an executable \
             inside the bundle, and removing it is not an uninstall."
        );

        let main_path = repository_path(CLI_MAIN_PATH);
        let main = std::fs::read_to_string(&main_path)
            .unwrap_or_else(|error| panic!("cannot read {}: {error}", main_path.display()));
        let call = main
            .split("from_installed_paths(")
            .nth(1)
            .expect("OMEGA-DELTA-0043: the uninstall call site is gone");
        let call = &call[..call.find(')').unwrap_or(call.len())];
        assert!(
            call.contains("installation_root"),
            "OMEGA-DELTA-0043: {} passes {call:?} to the uninstaller. \
             `path()` is one executable inside `Omega.app`; \
             `installation_root()` is the installation.",
            main_path.display()
        );
        assert!(
            main.contains("fn installation_root(&self) -> PathBuf;"),
            "OMEGA-DELTA-0043: InstalledApp no longer distinguishes the \
             executable from the installation. One method answering both \
             questions is what shipped omega#92."
        );
    }

    /// OMEGA-DELTA-0037. Omega identifies itself to third parties as Omega.
    ///
    /// `X-Title` is displayed to the account holder in their own OpenRouter
    /// dashboard, so it is outbound product identity rather than a wire
    /// contract. Every request Omega made through `0.2.0-rc14` announced a
    /// different editor, and `HTTP-Referer` pointed at that editor's site
    /// (omega#89).
    #[test]
    fn outbound_attribution_names_omega() {
        let path = repository_path(OPEN_ROUTER_PATH);
        let source = std::fs::read_to_string(&path)
            .unwrap_or_else(|error| panic!("cannot read {}: {error}", path.display()));
        let titles = source.matches(".header(\"X-Title\"").count();
        assert!(
            titles >= 2,
            "OMEGA-DELTA-0037: {} sets X-Title on {titles} request paths; both \
             the streaming and the non-streaming call carry it.",
            path.display()
        );
        assert!(
            !source.contains("\"Zed Editor\"") && !source.contains("\"https://zed.dev\""),
            "OMEGA-DELTA-0037: {} identifies Omega to OpenRouter as a different \
             product. The user reads this value in their own dashboard.",
            path.display()
        );
        assert_eq!(
            source.matches("app_identity::PRODUCT_NAME").count(),
            titles,
            "OMEGA-DELTA-0037: every X-Title must come from the identity \
             constant, so a rebase cannot restore a literal on one path only."
        );
    }

    /// OMEGA-DELTA-0038. The packaged gate opens every executable that ships,
    /// and reads help as clap renders it.
    ///
    /// Both halves of this delta are the same failure. `script/bundle-omega-rc`
    /// copies and signs three binaries into `Contents/MacOS`; every packaged
    /// check in the brand gate opened one of them, so the uninstaller inside
    /// `cli` was outside every check that had ever run (omega#88). And every
    /// prose stream reads *source*, while clap builds the sentence a person
    /// reads at run time — joining doc lines, resolving `cfg_attr`, printing
    /// the flag name beside the text — so `--zed <ZED>`, `Run zed in the
    /// foreground` and a `--user-data-dir` line naming the wrong product's data
    /// directory all shipped under a green gate (omega#89).
    #[test]
    fn the_packaged_gate_opens_every_shipped_executable_and_reads_rendered_help() {
        let verifier =
            std::fs::read_to_string(repository_path(BRAND_VERIFIER_PATH)).expect("brand verifier");
        let bundler =
            std::fs::read_to_string(repository_path(RC_BUNDLE_SCRIPT_PATH)).expect("bundle script");

        // Derived, not listed: whatever the packaging script writes into
        // Contents/MacOS is what the gate has to be able to open.
        let marker = "${app_path}/Contents/MacOS/";
        let mut shipped: Vec<&str> = bundler
            .match_indices(marker)
            .filter_map(|(at, _)| {
                bundler[at + marker.len()..]
                    .split(['"', '\'', ' ', '\n'])
                    .next()
                    .filter(|name| !name.is_empty() && !name.contains('/'))
            })
            .collect();
        shipped.sort_unstable();
        shipped.dedup();
        assert!(
            shipped.len() >= 3,
            "OMEGA-DELTA-0038: only {shipped:?} were found being written into \
             Contents/MacOS by {RC_BUNDLE_SCRIPT_PATH}; the parser broke and \
             this check is reporting green about nothing."
        );

        let policy = brand_policy().expect("brand policy parses");
        let floor = policy["packaged"]["minimum_executables"]
            .as_u64()
            .expect("packaged.minimum_executables") as usize;
        assert!(
            floor >= shipped.len(),
            "OMEGA-DELTA-0038: the bundle ships {} executables ({shipped:?}) \
             but the gate's floor is {floor}. A bundle that ships a companion \
             binary and a scan that expects fewer is the shape omega#88 \
             shipped in.",
            shipped.len()
        );

        assert!(
            verifier.contains("def bundle_executables("),
            "OMEGA-DELTA-0038: {BRAND_VERIFIER_PATH} no longer derives the set \
             of executables it opens by walking the bundle."
        );
        // One accessor names the main binary; nothing else may name a binary.
        // The code form is counted, not the prose form, so the docstring that
        // explains why is not itself a violation.
        let hardcoded = verifier.matches("app / \"Contents/MacOS/").count();
        assert_eq!(
            hardcoded, 1,
            "OMEGA-DELTA-0038: {BRAND_VERIFIER_PATH} names a path under \
             Contents/MacOS {hardcoded} times. Exactly one — the `main_binary` \
             accessor — is allowed; every other check reads the derived \
             inventory, because a check that opens one remembered binary is \
             what let a destructive uninstaller ship twice."
        );
        for required in [
            "def check_rendered_help(",
            "def check_packaged_first_party_agent(",
            "def check_packaged_executable_inventory(",
            "check_rendered_help(APP)",
            "check_packaged_first_party_agent(APP)",
            "check_packaged_executable_inventory(APP)",
        ] {
            assert!(
                verifier.contains(required),
                "OMEGA-DELTA-0038: {BRAND_VERIFIER_PATH} is missing {required:?}. \
                 A check that is defined and never called is the state \
                 first_party_agent.phrases was in for four release candidates."
            );
        }
        assert!(
            verifier.contains("\"--version\"") && verifier.contains("\"--help\""),
            "OMEGA-DELTA-0038: the rendered-output gate no longer runs the \
             shipped binaries with --help and --version, so it is reading \
             source again."
        );
    }

    /// OMEGA-DELTA-0031, widened. Lowercase `zed`, and doc comments written the
    /// long way.
    ///
    /// Two structural causes behind omega#89, both of which would have survived
    /// a fix that only edited the offending strings.
    ///
    /// `brand.words` held `Zed` alone, and the reason recorded for excluding
    /// the lowercase spelling — that it is a substring of `authorized` — was
    /// false, because the boundary rule already excludes that. The exclusion is
    /// what hid the rendered `--help` of both shipped binaries.
    ///
    /// And the doc scanner matched `///` and `//!` only, so it never read
    /// `#[cfg_attr(target_os = "macos", doc = "…")]` — which is exactly where
    /// `cli --help` took the wrong product's data directory from.
    #[test]
    fn the_doc_scanner_reads_every_spelling_of_a_doc_comment() {
        let policy = brand_policy().expect("brand policy parses");
        let words: Vec<&str> = policy["brand"]["words"]
            .as_array()
            .expect("brand.words")
            .iter()
            .filter_map(serde_json::Value::as_str)
            .collect();
        assert!(
            words.contains(&"zed"),
            "OMEGA-DELTA-0031: brand.words is {words:?}. The lowercase spelling \
             is what `Run zed in the foreground` is written in, and it is the \
             boundary rule — not the case — that keeps `authorized` out."
        );
        for benign in ["authorized", "normalized", "organized", "customized"] {
            assert!(
                brand_hits(benign, &policy).is_empty(),
                "OMEGA-DELTA-0031: {benign:?} is reported as a brand hit. The \
                 boundary rule has to carry the lowercase word, or the gate \
                 cries wolf and gets deleted."
            );
        }
        assert_eq!(
            brand_hits("Run zed in the foreground", &policy),
            vec!["zed".to_owned()],
            "OMEGA-DELTA-0031: the sentence that shipped in two published \
             prereleases is not reported as a hit"
        );

        assert_eq!(
            doc_comment_body(r#"    #[cfg_attr(target_os = "macos", doc = "`~/Library/x`.")]"#),
            Some("`~/Library/x`."),
            "a cfg_attr doc attribute is a doc comment"
        );
        assert_eq!(
            doc_comment_body(r#"        doc = "the long way""#),
            Some("the long way"),
            "an attribute written across several lines is still a doc comment"
        );
        assert_eq!(
            doc_comment_body(r#"    #[doc = "plain"]"#),
            Some("plain"),
            "a plain doc attribute is a doc comment"
        );
        assert_eq!(doc_comment_body("    /// sugar"), Some(" sugar"));
        assert_eq!(
            doc_comment_body(r#"    let doc = "not documentation";"#),
            None,
            "a Rust binding named `doc` is not documentation"
        );
        assert_eq!(doc_comment_body("    let x = 1;"), None);
    }

    /// OMEGA-DELTA-0039. The installed-proof harness observes what it records.
    ///
    /// Three checks in the harness could not fail (omega#90). The secret
    /// tripwire made a pipe, wrote a fresh random needle into it, closed both
    /// ends in the same function and then searched the disk for it, so `pass`
    /// was guaranteed by construction. Four of its six surfaces resolved under
    /// the data root, where Omega writes no logs, no telemetry and no crash
    /// reports, and recorded `absent` — which did not fail the receipt. And the
    /// `light-theme` / `dark-theme` observations wrote `content_legible: True`
    /// as a literal, with zero OCR calls and zero pixel comparisons, so a
    /// frozen or blank window passed both.
    ///
    /// This asserts the shape of the corrections. The scripts' own
    /// `--self-test` paths carry the behavioural oracles.
    #[test]
    fn the_installed_proof_harness_observes_what_it_records() {
        let tripwires = std::fs::read_to_string(repository_path(INSTALLED_TRIPWIRE_PATH))
            .expect("tripwire collector");
        assert!(
            !tripwires.contains("secrets.token_hex"),
            "OMEGA-DELTA-0039: {INSTALLED_TRIPWIRE_PATH} mints its own needle \
             again. A needle no other process has ever seen cannot be found, \
             so the scan passes whatever the product does."
        );
        assert!(
            tripwires.contains("--needle-fd"),
            "OMEGA-DELTA-0039: {INSTALLED_TRIPWIRE_PATH} no longer takes the \
             needle from the caller through a descriptor."
        );
        for required in ["Library/Logs", "DiagnosticReports"] {
            assert!(
                tripwires.contains(required),
                "OMEGA-DELTA-0039: {INSTALLED_TRIPWIRE_PATH} no longer scans \
                 {required:?}. On macOS `paths::logs_dir()` is \
                 ~/Library/Logs/<slug> and `paths::crashes_dir()` is \
                 ~/Library/Logs/DiagnosticReports; a surface resolved anywhere \
                 else records `absent` about a directory the product never \
                 writes."
            );
        }
        assert!(
            tripwires.contains("blocked"),
            "OMEGA-DELTA-0039: a surface that cannot be observed has to block. \
             'nothing was found there' and 'nobody looked' must not read the \
             same in a receipt."
        );

        let observations = std::fs::read_to_string(repository_path(INSTALLED_OBSERVATION_PATH))
            .expect("observation collector");
        let appearance = observations
            .split("# ---- appearance ---")
            .nth(1)
            .expect("OMEGA-DELTA-0039: the appearance block is gone");
        for required in ["ocr_lines(", "differing_pixels("] {
            assert!(
                appearance.contains(required),
                "OMEGA-DELTA-0039: the appearance block does not call \
                 {required:?}. `content_legible` was a Python literal there \
                 through 0.2.0-rc14: a fact about the host's appearance setting \
                 and about a file existing, filed as a fact about the product."
            );
        }
        assert!(
            !appearance.contains("\"content_legible\": True,"),
            "OMEGA-DELTA-0039: `content_legible` is a constant in the \
             appearance block again."
        );

        let bundler =
            std::fs::read_to_string(repository_path(RC_BUNDLE_SCRIPT_PATH)).expect("bundle script");
        assert!(
            !bundler.contains("\"dirty\": False"),
            "OMEGA-DELTA-0039: the release record states `dirty` as a literal. \
             It is a field that reads like an observation, so it has to be one."
        );
    }

    // ------ OMEGA-DELTA-0042

    /// OMEGA-DELTA-0042. The Exo the lane drives is the agent harness.
    ///
    /// omega#86 was closed for integrating exo labs' cluster-inference
    /// appliance, which shares a name with the harness and nothing else. The
    /// pin therefore carries the repository as a *field*, and this reads it, so
    /// the distinction survives somebody skimming a doc comment.
    #[test]
    fn the_exo_lane_drives_the_harness_exo_and_not_the_cluster_one() {
        let path = repository_path(EXO_LANE_PIN_PATH);
        let source = std::fs::read_to_string(&path)
            .unwrap_or_else(|error| panic!("cannot read {}: {error}", path.display()));
        let upstream = source
            .split_once("upstream: \"")
            .and_then(|(_, rest)| rest.split_once('"'))
            .expect("EXO_PIN names an upstream")
            .0;
        assert!(
            [EXO_HARNESS_UPSTREAM, EXO_HARNESS_MAINTAINED_FORK]
                .iter()
                .any(|repository| upstream.ends_with(repository)),
            "OMEGA-DELTA-0042: the Exo pin names {upstream}, which is neither \
             {EXO_HARNESS_UPSTREAM} nor its reviewed maintained fork \
             {EXO_HARNESS_MAINTAINED_FORK}. omega#86 made this mistake once."
        );

        for relative in [EXO_LANE_LAW_PATH, EXO_LANE_PIN_PATH, EXO_CONNECTION_PATH] {
            let path = repository_path(relative);
            let source = std::fs::read_to_string(&path)
                .unwrap_or_else(|error| panic!("cannot read {}: {error}", path.display()));
            let targeted = named_in_code(&source, EXO_CLUSTER_UPSTREAM);
            assert!(
                !targeted,
                "OMEGA-DELTA-0042: {} names {EXO_CLUSTER_UPSTREAM} outside a \
                 comment, which would make the wrong Exo a target rather than \
                 a warning.",
                path.display()
            );
        }
    }

    /// OMEGA-DELTA-0042. No text from outside Omega can become an Exo flag.
    ///
    /// Read off the written shapes rather than off the builder, because the
    /// shapes are what a reviewer reads. Exo takes its global options after the
    /// subcommand, so a shape that put `<prompt>` before the terminator would
    /// hand the command line to whoever typed the prompt — and at this pin that
    /// failure is *silent*: `--help` as a prompt exits 0 with usage text and no
    /// turn.
    #[test]
    fn the_exo_lane_puts_no_user_text_before_the_argument_terminator() {
        let path = repository_path(EXO_LANE_COMMAND_PATH);
        let source = std::fs::read_to_string(&path)
            .unwrap_or_else(|error| panic!("cannot read {}: {error}", path.display()));
        let table = source
            .split_once("pub const ADMITTED_LANE_ARGV")
            .and_then(|(_, rest)| rest.split_once("\n];"))
            .expect("the admitted argv table is present")
            .0;

        let mut shapes = 0usize;
        for shape in table.split("    (\n").skip(1) {
            shapes += 1;
            let tokens: Vec<&str> = shape
                .match_indices('"')
                .map(|(offset, _)| offset)
                .collect::<Vec<_>>()
                .chunks_exact(2)
                .map(|pair| &shape[pair[0] + 1..pair[1]])
                .collect();
            let terminator = tokens.iter().position(|token| *token == "--");
            for slot in EXO_LANE_USER_TEXT_SLOTS {
                let Some(at) = tokens.iter().position(|token| token == slot) else {
                    continue;
                };
                let Some(terminator) = terminator else {
                    panic!(
                        "OMEGA-DELTA-0042: an admitted Exo command carries {slot} \
                         and emits no argument terminator, so the value is Exo's \
                         command line rather than its input: {tokens:?}"
                    );
                };
                assert!(
                    at > terminator,
                    "OMEGA-DELTA-0042: an admitted Exo command puts {slot} \
                     before the argument terminator: {tokens:?}"
                );
            }
        }
        assert!(
            shapes >= 4,
            "OMEGA-DELTA-0042: the admitted argv table parsed as {shapes} \
             shapes, so this check is reading nothing."
        );
    }

    /// OMEGA-DELTA-0042. Omega never puts Exo on a network.
    ///
    /// Exo's one server has no authentication and full access to its secrets;
    /// loopback is the entire boundary and Exo's own documentation says so.
    /// Tier A needs no address at all, so anything here is a surface that was
    /// added rather than required.
    #[test]
    fn the_exo_lane_exposes_no_endpoint_off_this_machine() {
        for relative in [
            EXO_CONNECTION_PATH,
            EXO_LANE_LAW_PATH,
            EXO_LANE_COMMAND_PATH,
        ] {
            let path = repository_path(relative);
            let source = std::fs::read_to_string(&path)
                .unwrap_or_else(|error| panic!("cannot read {}: {error}", path.display()));
            for (what, token) in EXO_OFF_MACHINE_TOKENS {
                assert!(
                    !named_in_code(&source, token),
                    "OMEGA-DELTA-0042: {} names {what} (`{token}`) outside a \
                     comment. Exo's endpoint is unauthenticated and Omega must \
                     never proxy it off-machine.",
                    path.display()
                );
            }
        }

        // The flags that would redirect Exo, checked where they would have to
        // appear to do any harm: the command lines Omega actually builds.
        let command_path = repository_path(EXO_LANE_COMMAND_PATH);
        let command = std::fs::read_to_string(&command_path)
            .unwrap_or_else(|error| panic!("cannot read {}: {error}", command_path.display()));
        let table = command
            .split_once("pub const ADMITTED_LANE_ARGV")
            .and_then(|(_, rest)| rest.split_once("\n];"))
            .expect("the admitted argv table is present")
            .0;
        for flag in EXO_REDIRECTING_FLAGS {
            assert!(
                !table.contains(flag),
                "OMEGA-DELTA-0042: an admitted Exo command line carries \
                 `{flag}`, which points Exo away from the state root on disk \
                 and at a server with no authentication."
            );
        }

        // And the positive half: the lane refuses an off-loopback endpoint it
        // inherited. Without this the check above is satisfied by a lane that
        // simply never looked.
        let connection_path = repository_path(EXO_CONNECTION_PATH);
        let connection = std::fs::read_to_string(&connection_path)
            .unwrap_or_else(|error| panic!("cannot read {}: {error}", connection_path.display()));
        let body = function_body(&connection, "check_endpoint").unwrap_or_else(|| {
            panic!(
                "OMEGA-DELTA-0042: {} no longer checks where Exo is. \
                 `EXO_EXOHARNESS_URL` in the inherited environment redirects \
                 the lane off-machine with no Omega command line changing.",
                connection_path.display()
            )
        });
        assert!(
            body.contains("EXO_EXOHARNESS_URL") && body.contains("LoopbackEndpoint::parse"),
            "OMEGA-DELTA-0042: `check_endpoint` no longer parses the inherited \
             endpoint through the type that refuses a non-loopback one."
        );
        let turn = function_body(&connection, "observe").expect("the ACP observation exists");
        assert!(
            turn.contains("self.check_endpoint()"),
            "OMEGA-DELTA-0042: the ACP preflight no longer checks where Exo is."
        );
        let prompt = function_body(&connection, "prompt").expect("the ACP prompt path exists");
        assert!(
            prompt.contains("driver.preflight().await") && prompt.contains("acp.prompt(params"),
            "OMEGA-DELTA-0042: the ACP prompt must finish its preflight before it sends."
        );
    }

    /// OMEGA-DELTA-0042, and owner gate 8 behind it.
    ///
    /// Adding an executor lane must not open a fourth model-initiated path into
    /// Full Auto authority. The lane reaches nothing that starts a run: it names
    /// no launch origin, constructs no pin gesture, and never writes a run
    /// reference — a record that carried one would be claiming engine-lane
    /// authority Exo does not have.
    #[test]
    fn the_exo_lane_opens_no_path_into_full_auto_authority() {
        for relative in [EXO_CONNECTION_PATH, EXO_LANE_LAW_PATH] {
            let path = repository_path(relative);
            let source = std::fs::read_to_string(&path)
                .unwrap_or_else(|error| panic!("cannot read {}: {error}", path.display()));
            for token in EXO_FULL_AUTO_TOKENS {
                let named = named_in_code(&source, token);
                assert!(
                    !named,
                    "OMEGA-DELTA-0042: {} names `{token}` in code. An Exo agent \
                     has an unrestricted networked shell; it is exactly the \
                     caller owner gate 8 exists for.",
                    path.display()
                );
            }
        }
    }

    /// OMEGA-DELTA-0042. Every turn is gated before it is sent.
    ///
    /// Three refusals, in order, and the order matters: an agent read after the
    /// send would report a capability the turn already used. Checked on the
    /// function body, because the calls existing somewhere in a file is not the
    /// same claim as the turn path running them.
    #[test]
    fn an_exo_turn_checks_the_pin_and_the_agent_before_it_sends() {
        let path = repository_path(EXO_CONNECTION_PATH);
        let source = std::fs::read_to_string(&path)
            .unwrap_or_else(|error| panic!("cannot read {}: {error}", path.display()));
        let body = function_body(&source, "observe").unwrap_or_else(|| {
            panic!(
                "OMEGA-DELTA-0042: {} has no exact Exo observation, so nothing \
                 gates the streamed turn any more.",
                path.display()
            )
        });
        let pin = body
            .find("self.check_pin()")
            .expect("OMEGA-DELTA-0042: a turn no longer checks which Exo it is driving");
        let agent = body
            .find("ExoCommand::ShowAgent")
            .expect("OMEGA-DELTA-0042: a turn no longer reads the exact Exo agent");
        let conversation = body
            .find("ExoCommand::ShowConversation")
            .expect("OMEGA-DELTA-0042: a turn no longer reads the exact Exo conversation");
        let prompt = function_body(&source, "prompt")
            .expect("OMEGA-DELTA-0042: the streamed ACP prompt path is absent");
        let preflight = prompt
            .find("driver.preflight().await")
            .expect("OMEGA-DELTA-0042: an ACP turn no longer runs its preflight");
        let send = prompt
            .find("acp.prompt(params")
            .expect("OMEGA-DELTA-0042: an ACP turn no longer sends");
        assert!(
            pin < agent && agent < conversation && preflight < send,
            "OMEGA-DELTA-0042: the exact pin, agent, and conversation checks \
             must run before the complete preflight permits the ACP send."
        );
    }

    #[test]
    fn an_exo_turn_streams_cancels_and_requires_exact_one_use_authority() {
        let connection_path = repository_path(EXO_CONNECTION_PATH);
        let connection = std::fs::read_to_string(&connection_path)
            .unwrap_or_else(|error| panic!("cannot read {}: {error}", connection_path.display()));
        let compact_connection = without_whitespace(&connection);
        for token in [
            "AcpConnection::stdio",
            "\"acp\".to_owned()",
            "acp.prompt(params",
            "self.acp.cancel(session_id",
            "grant.consume(&observed.observed, &turn_ref",
            "persist_tier_c_receipt",
        ] {
            assert!(
                compact_connection.contains(&without_whitespace(token)),
                "OMEGA-DELTA-0042: the Exo ACP and authority path lost `{token}`"
            );
        }

        let thread_path = repository_path(THREAD_VIEW_PATH);
        let thread = std::fs::read_to_string(&thread_path)
            .unwrap_or_else(|error| panic!("cannot read {}: {error}", thread_path.display()));
        for token in [
            "omega-exo-authorize-self-modification",
            "Allow this Exo agent to modify itself for one turn?",
            "self_modification_request",
            "confirm_self_modification",
        ] {
            assert!(
                thread.contains(token),
                "OMEGA-DELTA-0042: the visible one-turn Exo confirmation lost `{token}`"
            );
        }
    }

    /// OMEGA-DELTA-0042. The lane is wired, not merely built.
    ///
    /// omega#78 shipped a router nobody constructed, and `OMEGA-DELTA-0035`
    /// exists because of it. The same failure is available here: a lane with a
    /// law, a connection, and tests, reachable by nobody.
    #[test]
    fn the_exo_lane_is_reachable_from_omega_agent() {
        let router_path = repository_path(ROUTER_DISPATCH_PATH);
        let router = std::fs::read_to_string(&router_path)
            .unwrap_or_else(|error| panic!("cannot read {}: {error}", router_path.display()));
        assert!(
            router.contains("omega_exo_connection::connect_configured_lane")
                && router.contains("with_external_acp(exo)"),
            "OMEGA-DELTA-0042: {} no longer registers the Exo lane as the \
             router's external executor, so a pin to it can never be honoured.",
            router_path.display()
        );

        let factory_path = repository_path(AGENT_SERVER_FACTORY_PATH);
        let factory = std::fs::read_to_string(&factory_path)
            .unwrap_or_else(|error| panic!("cannot read {}: {error}", factory_path.display()));
        assert!(
            factory.contains("omega_exo_connection::ExoLaneConfig::data_dir_path()"),
            "OMEGA-DELTA-0042: {} no longer hands the router the Exo lane's \
             configuration path, so the lane is never found.",
            factory_path.display()
        );

        let disclosure_path = repository_path(EXECUTOR_DISCLOSURE_BINDING_PATH);
        let disclosure = std::fs::read_to_string(&disclosure_path)
            .unwrap_or_else(|error| panic!("cannot read {}: {error}", disclosure_path.display()));
        assert!(
            disclosure.contains("downcast::<crate::omega_exo_connection::ExoHarnessConnection>()"),
            "OMEGA-DELTA-0042: {} no longer recognises the Exo connection by \
             its concrete type. `agent_id()` on that connection is derived from \
             what Exo said about itself, so classifying by it would let an Exo \
             install choose its own executor class.",
            disclosure_path.display()
        );
    }

    // ------ OMEGA-DELTA-0046

    /// OMEGA-DELTA-0046. An Exo thread is a usable workspace, not one label.
    ///
    /// The workspace must keep Omega's standard transcript and composer. Its
    /// inspector must project facts from the same preflight that gates a turn.
    /// Its controls must reuse the existing cancel and exact one-turn authority
    /// paths. This source check catches the cheap failure modes: a mock panel,
    /// a second message implementation, or controls that only look active.
    #[test]
    fn an_exo_thread_has_a_live_workspace_and_exact_runtime_inspector() {
        let thread_path = repository_path(THREAD_VIEW_PATH);
        let thread = std::fs::read_to_string(&thread_path)
            .unwrap_or_else(|error| panic!("cannot read {}: {error}", thread_path.display()));
        for token in [
            "omega-exo-workspace-header",
            "omega-exo-inspector",
            "Runtime inspector",
            "render_entries(cx)",
            "render_message_editor(window, cx)",
            "cancel_generation",
            "refresh_exo_inspection",
            "authorize_exo_self_modification",
            "ObservedExoCapabilityState::requested_capabilities",
        ] {
            assert!(
                thread.contains(token),
                "OMEGA-DELTA-0046: the Exo workspace lost `{token}`"
            );
        }

        let connection_path = repository_path(EXO_CONNECTION_PATH);
        let connection = std::fs::read_to_string(&connection_path)
            .unwrap_or_else(|error| panic!("cannot read {}: {error}", connection_path.display()));
        for token in [
            "ExoInspectionSnapshot",
            "ExoTurnPhase",
            "driver.observe().await",
            "driver.preflight().await",
            "acp.prompt(params",
            "meta_value(meta, \"exo.session_id\")",
            "meta_value(meta, \"exo.turn_id\")",
            "meta_value(meta, \"exo.latest_event_id\")",
        ] {
            assert!(
                connection.contains(token),
                "OMEGA-DELTA-0046: the Exo workspace state lost `{token}`"
            );
        }

        let visual_path = repository_path(VISUAL_TEST_RUNNER_PATH);
        let visual = std::fs::read_to_string(&visual_path)
            .unwrap_or_else(|error| panic!("cannot read {}: {error}", visual_path.display()));
        for token in [
            "OMEGA_EXO_VISUAL_ONLY",
            "run_omega_exo_visual_tests",
            "omega_exo_workspace_wide",
            "omega_exo_workspace_narrow",
            "the real Exo visual turn failed",
            "turn.exo_session_id.is_some()",
            "turn.exo_turn_id.is_some()",
            "turn.latest_event_id.is_some()",
        ] {
            assert!(
                visual.contains(token),
                "OMEGA-DELTA-0046: the real Exo visual proof lost `{token}`"
            );
        }
    }

    // ------ OMEGA-DELTA-0047 … 0050 — zero base

    /// OMEGA-DELTA-0047. The mode reader names the command line, and the
    /// shipped defaults contain no zero-base key.
    ///
    /// The cheap failure is a settings value. A settings value is writable by a
    /// project settings file and by anything else that can write settings, so a
    /// mode that hides authority-bearing surfaces would be settable by
    /// something that is not the person at the keyboard. `OMEGA-DELTA-0020`
    /// records the same objection against a composer mode flag, and this check
    /// is the mechanised version of it: the mode's own source must not read a
    /// settings store, an environment variable, or a file, and no key that
    /// could turn it on may appear in the shipped defaults.
    #[test]
    fn zero_base_is_entered_only_from_the_command_line() {
        let mode_path = repository_path(ZERO_BASE_MODE_PATH);
        let mode = std::fs::read_to_string(&mode_path)
            .unwrap_or_else(|error| panic!("cannot read {}: {error}", mode_path.display()));

        assert!(
            mode.contains("pub fn enter_from_command_line()"),
            "OMEGA-DELTA-0047: {} must offer exactly one way in, and it must \
             say in its own name where the mode comes from.",
            mode_path.display()
        );

        for other_source in [
            "SettingsStore",
            "Settings::get",
            "settings::",
            "std::env::var",
            "std::fs::read",
            "paths::",
        ] {
            assert!(
                !mode.contains(other_source),
                "OMEGA-DELTA-0047: {} reads {other_source}. The mode is read \
                 from the process command line, once, and from nowhere else — \
                 a second reader is a second way to turn the mode on that the \
                 person at the keyboard did not use.",
                mode_path.display()
            );
        }

        let startup_path = repository_path(STARTUP_PATH);
        let startup = std::fs::read_to_string(&startup_path)
            .unwrap_or_else(|error| panic!("cannot read {}: {error}", startup_path.display()));
        assert!(
            startup.contains("zero_base: bool"),
            "OMEGA-DELTA-0047: {} no longer declares the zero-base flag on the \
             argument parser, so the shipped binary has no way into the mode.",
            startup_path.display()
        );
        assert!(
            startup.contains("omega_zero_base::enter_from_command_line()"),
            "OMEGA-DELTA-0047: {} no longer enters the mode from the parsed \
             command line.",
            startup_path.display()
        );

        // The whole shipped default settings text, not a parsed subtree: a
        // zero-base key anywhere in it is the failure, wherever someone nested
        // it.
        let defaults_path = repository_path(DEFAULT_SETTINGS_PATH);
        let defaults = std::fs::read_to_string(&defaults_path)
            .unwrap_or_else(|error| panic!("cannot read {}: {error}", defaults_path.display()));
        for key in ["zero_base", "zero-base", "zeroBase"] {
            assert!(
                !defaults.contains(key),
                "OMEGA-DELTA-0047: {} contains {key:?}. Zero base is not a \
                 setting, and a settings key for it would be a second way in \
                 that a project settings file could write.",
                defaults_path.display()
            );
        }
    }

    /// OMEGA-DELTA-0048. Zero base hides by filter and by refusal, and deletes
    /// nothing.
    ///
    /// Two halves, and both are load-bearing. The palette restriction is what a
    /// person sees; the action gate is what makes "not rendered" safe, because
    /// a surface that is only visually absent is still one key press away. And
    /// the deletion that would make either of them unnecessary is the one thing
    /// that cannot be done here: the built-in keymap is loaded and unwrapped at
    /// startup, so a binding naming a missing action kills the process while
    /// `cargo check --workspace` stays green. `0.2.0-rc6` died that way.
    #[test]
    fn zero_base_hides_by_filter_and_refusal_and_deletes_nothing() {
        let ui_path = repository_path(ZERO_BASE_UI_PATH);
        let ui = std::fs::read_to_string(&ui_path)
            .unwrap_or_else(|error| panic!("cannot read {}: {error}", ui_path.display()));
        for token in [
            "filter.restrict_to(ADMITTED_NAMESPACES, ADMITTED_ACTIONS)",
            "cx.set_action_gate(",
            "omega_zero_base::refusal(action_name)",
            "workspace.show_toast(",
        ] {
            assert!(
                ui.contains(token),
                "OMEGA-DELTA-0048: the zero-base surface lost `{token}`. \
                 Without it a hidden action is a silent no-op rather than a \
                 sentence a person can read."
            );
        }

        let filter_path = repository_path(COMMAND_PALETTE_FILTER_PATH);
        let filter = std::fs::read_to_string(&filter_path)
            .unwrap_or_else(|error| panic!("cannot read {}: {error}", filter_path.display()));
        assert!(
            filter.contains("pub fn restrict_to(") && filter.contains("pub fn clear_restriction("),
            "OMEGA-DELTA-0048: {} no longer offers an admitted set, so zero \
             base could only hide by listing every namespace it knows about — \
             and would keep admitting the ones a later crate adds.",
            filter_path.display()
        );

        let dispatch_path = repository_path(ACTION_DISPATCH_PATH);
        let dispatch = std::fs::read_to_string(&dispatch_path)
            .unwrap_or_else(|error| panic!("cannot read {}: {error}", dispatch_path.display()));
        assert!(
            dispatch.contains("if !cx.action_is_admitted(action)"),
            "OMEGA-DELTA-0048: {} no longer consults the action gate before \
             dispatching, so a refused action would still reach its listener \
             through a key binding.",
            dispatch_path.display()
        );

        let panels_path = repository_path(WORKSPACE_INITIALIZATION_PATH);
        let panels = std::fs::read_to_string(&panels_path)
            .unwrap_or_else(|error| panic!("cannot read {}: {error}", panels_path.display()));
        for token in [
            "if omega_zero_base::is_active()",
            "add_panel_when_ready(project_panel",
            "add_panel_when_ready(sarah_workroom_panel",
        ] {
            assert!(
                panels.contains(token),
                "OMEGA-DELTA-0048: {} lost `{token}`. The panels are skipped in \
                 zero base and kept everywhere else; deleting their load calls \
                 would make the mode irreversible inside the window.",
                panels_path.display()
            );
        }

        // Nothing was deleted: every namespace zero base hides is still bound
        // in all three shipped keymaps. `keymaps_name_no_deleted_action` covers
        // the crates Omega removed outright; this covers the ones it only hides.
        let mut missing: Vec<String> = Vec::new();
        for keymap in [
            "assets/keymaps/default-macos.json",
            "assets/keymaps/default-linux.json",
            "assets/keymaps/default-windows.json",
        ] {
            let path = repository_path(keymap);
            let source = std::fs::read_to_string(&path)
                .unwrap_or_else(|error| panic!("cannot read {}: {error}", path.display()));
            for namespace in ZERO_BASE_HIDDEN_KEYMAP_NAMESPACES {
                if !source.contains(namespace) {
                    missing.push(format!("{namespace:?} is no longer bound in {keymap}"));
                }
            }
        }
        assert!(
            missing.is_empty(),
            "OMEGA-DELTA-0048: zero base hides these surfaces and must delete \
             none of them:\n{}",
            missing.join("\n")
        );
    }

    /// OMEGA-DELTA-0049. A zero-base turn still names its executor.
    ///
    /// The disclosure line is `OMEGA-DELTA-0021`, and in the Exo lane it is
    /// also the door: a thread routes to Exo exactly when a person pins
    /// `ExternalAcp` on it. A mode whose entire purpose is subtraction is the
    /// likeliest place for that line to be subtracted by accident, and the
    /// likeliest *shape* for the accident is a zero-base branch inside the
    /// surface that draws it — a second code path that renders something
    /// cheaper. So the check is that no such branch exists, and that the two
    /// zero-base baselines are recorded by the same capture that photographs a
    /// real Exo turn rather than by a second, mocked one.
    #[test]
    fn a_zero_base_turn_still_names_its_executor() {
        // The two functions that draw the line do not know zero base exists.
        //
        // This was first written as "the whole file must not name the mode",
        // on the reasoning that a branch anywhere could reach the line. That
        // was too strong, and it forbade the layout the owner actually asked
        // for: an empty transcript has to claim the vertical space in zero base
        // so the composer sits at the bottom instead of floating at the top,
        // and that is a mode-aware branch in this file by necessity. A rule
        // that forbids the fix it was meant to protect is the wrong rule.
        //
        // So the scope is the disclosure and its pin, which is what the delta
        // is about: no cheaper second rendering of who ran the turn. Layout
        // elsewhere in the file is free to know the mode.
        let thread_path = repository_path(THREAD_VIEW_PATH);
        let thread = std::fs::read_to_string(&thread_path)
            .unwrap_or_else(|error| panic!("cannot read {}: {error}", thread_path.display()));
        // OMEGA-DELTA-0055 removed `render_executor_pin` from this check, along
        // with the function itself, which left one name behind. See the `drawn`
        // list below.
        let drawing = "render_executor_disclosure";
        let body = function_body(&thread, drawing).unwrap_or_else(|| {
            panic!(
                "OMEGA-DELTA-0049: cannot find `{drawing}` in {}",
                thread_path.display()
            )
        });
        assert!(
            body.len() > 40,
            "OMEGA-DELTA-0049: the body read for `{drawing}` is too short to \
             be the real one, so this check would pass without reading it",
        );
        assert!(
            !body.contains("omega_zero_base"),
            "OMEGA-DELTA-0049: `{drawing}` in {} now branches on zero base. A \
             mode-aware branch here is a second way to draw the line, and the \
             cheaper one always wins eventually.",
            thread_path.display()
        );
        // OMEGA-DELTA-0055 removed `self.render_executor_pin(cx)` from this
        // list. The reasoning it carried — "removing the executor line removes
        // the door into the Exo lane" — was true while the pin *was* the door.
        // Routing is automatic now, so the pin is not the door, and the
        // assertion had become a rule protecting a control the owner asked to
        // remove. The other half of 0049 is untouched: the disclosure line is
        // still drawn, and still has no cheaper zero-base-specific path.
        for drawn in [
            "fn render_executor_disclosure",
            "self.render_executor_disclosure(cx)",
        ] {
            assert!(
                thread.contains(drawn),
                "OMEGA-DELTA-0049: {} no longer draws `{drawn}`. Removing the \
                 executor line removes the statement of who ran the turn.",
                thread_path.display()
            );
        }
        assert!(
            !thread.contains("render_executor_pin"),
            "OMEGA-DELTA-0049: {} draws the executor pin again. \
             `OMEGA-DELTA-0055` removed it, and the route it used to set is \
             decided automatically.",
            thread_path.display()
        );

        // The binding that builds the typed record is untouched by the mode.
        let binding_path = repository_path(EXECUTOR_DISCLOSURE_BINDING_PATH);
        let binding = std::fs::read_to_string(&binding_path)
            .unwrap_or_else(|error| panic!("cannot read {}: {error}", binding_path.display()));
        assert!(
            !binding.contains("omega_zero_base"),
            "OMEGA-DELTA-0049: {} now knows about zero base. The record is \
             derived from the connection's concrete type, and a mode is not one \
             of its inputs.",
            binding_path.display()
        );

        // The two scenes exist, and they are recorded by the same capture that
        // runs the real Exo turn. A separate zero-base capture would be free to
        // drift into a mock, which is the failure this pairing prevents.
        let visual_path = repository_path(VISUAL_TEST_RUNNER_PATH);
        let visual = std::fs::read_to_string(&visual_path)
            .unwrap_or_else(|error| panic!("cannot read {}: {error}", visual_path.display()));
        // OMEGA-DELTA-0052 removed `omega_zero_base_ui::install_on_workspace(`
        // from this list. The runner used to call it to add the mode's one
        // status-bar control before capturing, and that control no longer
        // exists — the owner asked for zero base to have no way out. The token
        // is not weakened away: what it protected was "the zero-base scene is
        // built by the shipped surface code, not by a stand-in", and the
        // per-scene assertion the runner now carries protects the same thing
        // more directly by refusing to photograph a scene whose surface does
        // not match the mode's actual state.
        for token in [
            "\"omega_zero_base_wide\"",
            "\"omega_zero_base_narrow\"",
            "ExoSceneSurface::ZeroBase",
            "omega_zero_base::is_active() == (surface == ExoSceneSurface::ZeroBase)",
            "omega_zero_base::enter_from_command_line();",
        ] {
            assert!(
                visual.contains(token),
                "OMEGA-DELTA-0049: the zero-base visual proof lost `{token}`"
            );
        }
        assert_eq!(
            visual.matches("fn run_omega_exo_visual_capture").count(),
            1,
            "OMEGA-DELTA-0049: there is more than one Exo capture in {}. The \
             zero-base scenes photograph a real streamed Exo turn because they \
             go through the same capture as the full-editor scenes; a second \
             capture is where a mocked turn would enter.",
            visual_path.display()
        );

        for scene in ["omega_zero_base_wide", "omega_zero_base_narrow"] {
            let baseline = repository_path(&format!(
                "crates/zed/test_fixtures/visual_tests/{scene}.png"
            ));
            let recorded = std::fs::metadata(&baseline).unwrap_or_else(|error| {
                panic!(
                    "OMEGA-DELTA-0049: {} is missing ({error}). A scene cannot \
                     land without its picture.",
                    baseline.display()
                )
            });
            assert!(
                recorded.len() > 0,
                "OMEGA-DELTA-0049: {} is empty.",
                baseline.display()
            );
        }

        // The suite has to be able to record them again, and this is the defect
        // that held this entry back. The capture waited with `run_until_parked`,
        // which returns only when the scheduler has nothing left to run — and an
        // attached ACP transport reading a live child's stdout is runnable again
        // as soon as it is polled, so it never returned. The capture hung before
        // its turn with the runner spinning on one core. A hang is worse than a
        // failure because it reports nothing, so every wait in this capture
        // spends a budget and then says what it was waiting for.
        assert!(
            visual.contains("fn step_scheduler(") && visual.contains("SCHEDULER_STEP_BUDGET"),
            "OMEGA-DELTA-0049: {} lost its bounded wait. `run_until_parked` \
             does not return while a real `exo acp` child is attached.",
            visual_path.display()
        );
        assert!(
            visual
                .matches("step_scheduler(cx, SCHEDULER_STEP_BUDGET)")
                .count()
                >= 2,
            "OMEGA-DELTA-0049: {} waits on the Exo turn without a budget \
             somewhere. Both the wait for the connected thread and the wait \
             after the turn are unbounded without it.",
            visual_path.display()
        );

        // And the suite ends the process it started rather than waiting for the
        // last `Rc` to go away. Sampled at 100ms across a run without this, a
        // capture's child was still alive as the next capture's child started.
        assert!(
            visual.contains("exo.end_exo_process();"),
            "OMEGA-DELTA-0049: {} no longer ends the `exo acp` process each \
             capture started, and a scene would again depend on a reference \
             graph unwinding in time.",
            visual_path.display()
        );
        let connection_path = repository_path(EXO_CONNECTION_PATH);
        let connection = std::fs::read_to_string(&connection_path)
            .unwrap_or_else(|error| panic!("cannot read {}: {error}", connection_path.display()));
        assert!(
            connection.contains("pub fn end_exo_process(&self)")
                && connection.contains("self.acp.end_agent_server_process();"),
            "OMEGA-DELTA-0049: {} no longer offers a way to end the lane's \
             process by name.",
            connection_path.display()
        );
    }

    /// OMEGA-DELTA-0050. Zero base opens no authority path.
    ///
    /// Owner gate 8 closes the launch origins at four and the pin gestures at
    /// two. A mode that pre-pinned its thread to the Exo lane would need a
    /// third pin gesture, which is an edit to a closed list. This packet makes
    /// no such edit, and this check is what says so in a place a later change
    /// has to argue with.
    #[test]
    fn zero_base_opens_no_authority_path() {
        let gate_path = repository_path(GATE_EIGHT_PATH);
        let gate = std::fs::read_to_string(&gate_path)
            .unwrap_or_else(|error| panic!("cannot read {}: {error}", gate_path.display()));
        let compact = without_whitespace(&gate);
        assert!(
            compact.contains(&without_whitespace(
                "&[
                    Self::NewThreadMenuItem,
                    Self::OpenLauncherAction,
                    Self::RunMonitorNewRun,
                    Self::RunSurfaceNewRun,
                ]"
            )),
            "OMEGA-DELTA-0050: LaunchOrigin::all() changed. Zero base adds no \
             launch origin, so a change here means something else took gate \
             8's list — say which, in its own delta."
        );
        assert!(
            compact.contains(&without_whitespace(
                "&[Self::ExecutorPinMenuItem, Self::ExecutorPinCleared]"
            )),
            "OMEGA-DELTA-0050: PinGesture::all() changed. Zero base pins \
             nothing — the viewer sets the pin with one visible click on the \
             disclosure line — so a third gesture is a separate decision that \
             needs its own reason in this test."
        );
        assert!(
            !gate.contains("omega_zero_base"),
            "OMEGA-DELTA-0050: {} now knows about zero base. Gate 8's lists are \
             about human gestures, and a mode is not one.",
            gate_path.display()
        );

        // Not rendered *and* disabled, in both places, because a surface that
        // is only visually absent is still one dispatch away.
        let mode_path = repository_path(ZERO_BASE_MODE_PATH);
        let mode = std::fs::read_to_string(&mode_path)
            .unwrap_or_else(|error| panic!("cannot read {}: {error}", mode_path.display()));
        assert!(
            !mode.contains("\"full_auto_panel\"") && !mode.contains("\"agent_computer\""),
            "OMEGA-DELTA-0050: {} admits a namespace that reaches Full Auto or \
             the Agent Computer. Neither belongs in the admitted set.",
            mode_path.display()
        );

        let panel_path = repository_path(AGENT_PANEL_PATH);
        let panel = std::fs::read_to_string(&panel_path)
            .unwrap_or_else(|error| panic!("cannot read {}: {error}", panel_path.display()));
        assert!(
            panel.contains(".when(!omega_zero_base::is_active(), |menu|"),
            "OMEGA-DELTA-0050: {} renders the Full Auto entry in zero base.",
            panel_path.display()
        );
        assert!(
            panel.contains("self.refuse_in_zero_base(\"full_auto_panel::OpenLauncher\", cx)")
                && panel.contains("self.refuse_in_zero_base(\"full_auto_panel::ToggleFocus\", cx)"),
            "OMEGA-DELTA-0050: {} no longer refuses the Full Auto surface in \
             zero base. Not rendering it is only half the rule.",
            panel_path.display()
        );

        let full_auto_path = repository_path(FULL_AUTO_PANEL_PATH);
        let full_auto = std::fs::read_to_string(&full_auto_path)
            .unwrap_or_else(|error| panic!("cannot read {}: {error}", full_auto_path.display()));
        assert!(
            full_auto.contains(".when(!omega_zero_base::is_active(), |this|"),
            "OMEGA-DELTA-0050: {} renders the Full Auto start control in zero \
             base.",
            full_auto_path.display()
        );
        assert!(
            full_auto.contains("omega_zero_base::refusal(\"full_auto_panel::StartRun\")"),
            "OMEGA-DELTA-0050: {} no longer refuses `start_run` in zero base. \
             Starting Full Auto is the one act gate 8 says no code path may \
             reach without an explicit human gesture.",
            full_auto_path.display()
        );

        // `OMEGA-DELTA-0040` keeps its order. The zero-base branch waits on the
        // identity gate before it opens and zooms its panel. Without the wait
        // the mode still *works* on a first-ever launch — it simply covers
        // onboarding with a zoomed panel, which is a bypass of an identity gate
        // wearing a layout's clothes. That is the failure this line catches.
        let panels = std::fs::read_to_string(repository_path(WORKSPACE_INITIALIZATION_PATH))
            .expect("the workspace initialization is readable");
        assert!(
            panels.contains("await_identity_ready(app_state, cx).await.log_err();"),
            "OMEGA-DELTA-0050: the zero-base panel branch no longer waits for \
             identity onboarding, so a first-ever launch in zero base would \
             never see the identity gate `OMEGA-DELTA-0040` puts in front of it."
        );

        // No change to the Exo lane: zero base writes no configuration, opens
        // no listener, and proxies no `exo serve`.
        let ui_path = repository_path(ZERO_BASE_UI_PATH);
        let ui = std::fs::read_to_string(&ui_path)
            .unwrap_or_else(|error| panic!("cannot read {}: {error}", ui_path.display()));
        for reaching_the_lane in [
            "ExoLaneConfig",
            "exo serve",
            "write_exo",
            "FullAutoDispatch",
            "LaunchOrigin",
            "PinGesture",
        ] {
            assert!(
                !ui.contains(reaching_the_lane),
                "OMEGA-DELTA-0050: {} reaches {reaching_the_lane}. Zero base is \
                 a subtraction of the editor, not a second code path through \
                 the Exo lane or the launch gate.",
                ui_path.display()
            );
        }
    }

    // ---------------------------------------------------------------------
    // OMEGA-DELTA-0051 — Zero base derives its setup and can finish identity
    // ---------------------------------------------------------------------

    /// OMEGA-DELTA-0051. Zero base asks for the identity and derives the rest,
    /// and the one thing it asks for can actually be finished.
    ///
    /// Two halves, and the second is the one that was broken in the shipped
    /// mode. A page that asks fewer questions is a preference; a page whose
    /// only remaining question cannot be answered is a dead end that no amount
    /// of `cargo check` can see, because the mode is entered by a command-line
    /// flag and no test that does not launch the binary with it runs any of
    /// this code.
    #[test]
    fn zero_base_derives_setup_and_can_finish_identity_onboarding() {
        let page_path = repository_path(ONBOARDING_BASICS_PAGE_PATH);
        let page = std::fs::read_to_string(&page_path)
            .unwrap_or_else(|error| panic!("cannot read {}: {error}", page_path.display()));

        assert!(
            page.contains("if omega_zero_base::is_active() {"),
            "OMEGA-DELTA-0051: {} no longer has a zero-base branch, so the mode \
             renders the whole first-run page again: a theme picker, a keymap \
             picker offering eight other editors, an agent-install grid, import \
             settings and two toggles, in a mode that claims to show one Exo \
             thread and nothing else.",
            page_path.display()
        );

        // The branch is pinned to the identity section specifically, rather
        // than to "some shorter page". Reducing the page to nothing would
        // satisfy a looser check and would skip the identity gate
        // `OMEGA-DELTA-0040` puts in front of a first-ever launch.
        let branch = page
            .split_once("if omega_zero_base::is_active() {")
            .map(|(_, rest)| rest)
            .unwrap_or_default();
        let branch = branch
            .split_once("\n    }")
            .map_or(branch, |(body, _)| body);
        assert!(
            branch.contains("render_identity_section("),
            "OMEGA-DELTA-0051: zero base's branch in {} does not render the \
             identity section. `OMEGA-DELTA-0040` puts identity onboarding in \
             front of a first-ever launch, and a mode that renders the page \
             without it is that gate skipped in a layout's clothes — the same \
             bypass a zoomed panel already committed once.",
            page_path.display()
        );
        for skipped in ONBOARDING_SECTIONS_ZERO_BASE_SKIPS {
            assert!(
                !branch.contains(skipped),
                "OMEGA-DELTA-0051: zero base's branch in {} renders \
                 `{skipped}`. Every section it names has a shipped default, and \
                 the owner asked for the mode to detect rather than ask.",
                page_path.display()
            );
            assert!(
                page.contains(skipped),
                "OMEGA-DELTA-0051: `{skipped}` is gone from {} entirely. Zero \
                 base does not render these sections; it does not delete them, \
                 and without the flag the page is unchanged.",
                page_path.display()
            );
        }

        // The half that was actually broken. `OMEGA-DELTA-0040` parks startup
        // on `await_identity_ready`, and only the first-run branch of
        // `on_finish` releases it. With `onboarding::Finish` refused, a fresh
        // profile in zero base reached the identity page, created an identity,
        // pressed "Finish Setup", and stayed there permanently.
        let mode_path = repository_path(ZERO_BASE_MODE_PATH);
        let mode = std::fs::read_to_string(&mode_path)
            .unwrap_or_else(|error| panic!("cannot read {}: {error}", mode_path.display()));
        // Scoped to the constant, not the file. The same names appear in that
        // crate's own tests as the *refused* list, so a file-wide scan would
        // read a test's denial as an admission and fail on a correct tree.
        let admitted = mode
            .split_once("pub const ADMITTED_ACTIONS: &[&str] = &[")
            .map(|(_, rest)| rest)
            .unwrap_or_default();
        let admitted = admitted.split_once("];").map_or(admitted, |(list, _)| list);
        assert!(
            !admitted.is_empty(),
            "OMEGA-DELTA-0051: no ADMITTED_ACTIONS list found in {}. The check \
             below would be vacuous, so it fails instead.",
            mode_path.display()
        );
        assert!(
            admitted.contains("\"onboarding::Finish\","),
            "OMEGA-DELTA-0051: {} no longer admits `onboarding::Finish`. That \
             action is the only thing that releases `await_identity_ready`, so \
             refusing it makes a fresh profile in zero base a dead end: \
             identity never becomes ready and the mode never reaches a thread, \
             across restarts.",
            mode_path.display()
        );
        for hosted in [
            "\"onboarding::SignIn\"",
            "\"onboarding::OpenAccount\"",
            "\"onboarding::ResetHints\"",
        ] {
            assert!(
                !admitted.contains(hosted),
                "OMEGA-DELTA-0051: {} admits {hosted}. Admitting the action \
                 that finishes the identity gate must not drag the hosted \
                 account path `OMEGA-DELTA-0010` and `OMEGA-DELTA-0011` \
                 removed in with it.",
                mode_path.display()
            );
        }
    }

    // ---------------------------------------------------------------------
    // OMEGA-DELTA-0052 — Zero base is the default, and it has no exit
    // ---------------------------------------------------------------------

    /// OMEGA-DELTA-0052. `omega` opens zero base, `--full-editor` opens the
    /// editor, and a process in zero base cannot leave it.
    ///
    /// Both halves invert something a person can see, and both are invisible to
    /// the compiler for the reason every zero-base defect this week has been:
    /// the mode is decided in the argument parser and read through a process
    /// global, so `cargo check`, `cargo test` and clippy can all be green while
    /// the shipped binary opens the wrong product entirely.
    ///
    /// The removal half needs a check more than the default half does. Deleting
    /// a control is easy to *half* do — hide the button, keep `leave` on the
    /// crate, keep the action in the registry, keep the palette restriction
    /// clearable — and the result reads as removed while remaining one dispatch
    /// away. That is exactly the failure `OMEGA-DELTA-0048` names about every
    /// other hidden surface, so the check is that the way out is *absent*
    /// rather than *unrendered*.
    #[test]
    fn zero_base_is_the_default_and_has_no_way_out() {
        let startup_path = repository_path(STARTUP_PATH);
        let startup = std::fs::read_to_string(&startup_path)
            .unwrap_or_else(|error| panic!("cannot read {}: {error}", startup_path.display()));

        assert!(
            startup.contains("full_editor: bool"),
            "OMEGA-DELTA-0052: {} no longer declares the full-editor flag, so \
             the shipped binary has no way to open the editor at all.",
            startup_path.display()
        );
        assert!(
            startup.contains("if !args.full_editor && !names_something_to_edit {"),
            "OMEGA-DELTA-0052: {} no longer defaults to zero base. `omega` with \
             no arguments must open one Exo thread; the editor is what the flag \
             asks for, not the other way round.",
            startup_path.display()
        );
        // A path argument implies the editor. Without this, `omega src/main.rs`
        // opens a chat thread with no way to reach the file it was given, which
        // is a regression rather than a subtraction.
        for implied in [
            "!args.paths_or_urls.is_empty()",
            "!args.diff.is_empty()",
            "args.dev_container",
            "args.demo_workroom",
        ] {
            assert!(
                startup.contains(implied),
                "OMEGA-DELTA-0052: {} no longer treats `{implied}` as naming \
                 something to edit, so that command line would open a single \
                 chat thread with no editor and nothing to point it at.",
                startup_path.display()
            );
        }

        let mode_path = repository_path(ZERO_BASE_MODE_PATH);
        let mode = std::fs::read_to_string(&mode_path)
            .unwrap_or_else(|error| panic!("cannot read {}: {error}", mode_path.display()));
        assert!(
            mode.contains("pub const FULL_EDITOR_FLAG: &str = \"--full-editor\";"),
            "OMEGA-DELTA-0052: {} no longer names the flag that opens the \
             editor, and the refusal sentence has nothing true to point at.",
            mode_path.display()
        );
        for gone in [
            "pub fn leave()",
            "static LEFT",
            "LEAVE_LABEL",
            "BANNER_LABEL",
        ] {
            assert!(
                !mode.contains(gone),
                "OMEGA-DELTA-0052: {} still carries `{gone}`. A process that \
                 starts in zero base stays in zero base, so leaving is absent \
                 rather than unrendered — the owner asked for no way out, and a \
                 way out that is merely not drawn is still one dispatch away.",
                mode_path.display()
            );
        }
        assert!(
            mode.contains("pub fn is_active() -> bool {\n    ENTERED.load(Ordering::SeqCst)\n}"),
            "OMEGA-DELTA-0052: `is_active` in {} reads something other than the \
             single entry flag. A second input is a second way for the mode to \
             turn off inside a running process.",
            mode_path.display()
        );

        // The admitted namespace that held the way out. `omega_zero_base` had
        // exactly one action in it, `Leave`, and an admitted namespace with
        // nothing in it is a door frame left standing where the room went.
        let namespaces = mode
            .split_once("pub const ADMITTED_NAMESPACES: &[&str] = &[")
            .map(|(_, rest)| rest)
            .unwrap_or_default();
        let namespaces = namespaces
            .split_once("];")
            .map_or(namespaces, |(list, _)| list);
        assert!(
            !namespaces.is_empty(),
            "OMEGA-DELTA-0052: no ADMITTED_NAMESPACES list found in {}. The \
             check below would be vacuous, so it fails instead.",
            mode_path.display()
        );
        assert!(
            !namespaces.contains("\"omega_zero_base\""),
            "OMEGA-DELTA-0052: {} still admits the `omega_zero_base` namespace, \
             which existed only to carry the `Leave` action.",
            mode_path.display()
        );

        let ui_path = repository_path(ZERO_BASE_UI_PATH);
        let ui = std::fs::read_to_string(&ui_path)
            .unwrap_or_else(|error| panic!("cannot read {}: {error}", ui_path.display()));
        for gone in [
            "pub fn install_on_workspace",
            "ZeroBaseStatusItem",
            "clear_restriction()",
            "cx.clear_action_gate()",
            "omega_zero_base::leave()",
        ] {
            assert!(
                !ui.contains(gone),
                "OMEGA-DELTA-0052: {} still carries `{gone}`. The status-bar \
                 control, the action that unwound the mode, and the two calls \
                 that lifted the palette restriction and the action gate at \
                 runtime are all removed together — keeping any one of them \
                 leaves the mode leavable by something other than the button.",
                ui_path.display()
            );
        }
        // The half that stays. Removing the way out must not remove the two
        // mechanisms `OMEGA-DELTA-0048` depends on.
        for kept in [
            "filter.restrict_to(ADMITTED_NAMESPACES, ADMITTED_ACTIONS)",
            "cx.set_action_gate(",
        ] {
            assert!(
                ui.contains(kept),
                "OMEGA-DELTA-0052: {} lost `{kept}` along with the way out. The \
                 refusal gate is what makes an unrendered surface safe, and it \
                 is not what the owner asked to remove.",
                ui_path.display()
            );
        }

        let panels_path = repository_path(WORKSPACE_INITIALIZATION_PATH);
        let panels = std::fs::read_to_string(&panels_path)
            .unwrap_or_else(|error| panic!("cannot read {}: {error}", panels_path.display()));
        assert!(
            !panels.contains("omega_zero_base_ui::install_on_workspace"),
            "OMEGA-DELTA-0052: {} still installs the zero-base status-bar \
             control on the workspace.",
            panels_path.display()
        );

        // The refusal sentence, read from the source rather than called. This
        // crate checks text on purpose: `omega_zero_base` proves the sentence's
        // shape with its own unit test, and depending on it here would make the
        // registry's checks depend on the tree they check.
        let sentence = mode
            .split_once("pub fn refusal(action_name: &str) -> String {")
            .map(|(_, rest)| rest)
            .unwrap_or_default();
        let sentence = sentence
            .split_once("\n}")
            .map_or(sentence, |(body, _)| body);
        assert!(
            sentence.contains("{FULL_EDITOR_FLAG}"),
            "OMEGA-DELTA-0052: the refusal sentence in {} no longer names the \
             flag that opens the editor. A refusal must say how to get the thing \
             it refused, and starting Omega differently is now the only way.",
            mode_path.display()
        );
        assert!(
            !sentence.contains("LEAVE_LABEL"),
            "OMEGA-DELTA-0052: the refusal sentence in {} still offers a control \
             in the window. There is no longer one to find, so the sentence \
             would send a person looking for a button that does not exist.",
            mode_path.display()
        );
    }

    // ---------------------------------------------------------------------
    // OMEGA-DELTA-0053 — A sealed zero base does not render the editor
    // ---------------------------------------------------------------------

    /// OMEGA-DELTA-0053. Zero base subtracts the editor rather than covering
    /// it.
    ///
    /// The mode used to hide the editor by zooming the agent panel over it, and
    /// one press of the sidebar control released the zoom and put Zed's whole
    /// welcome surface on the screen — New File, Open Project, Clone
    /// Repository, Open Command Palette, Open Settings, Customize Keymaps,
    /// Explore Extensions — inside a mode whose premise is that none of them
    /// are present.
    ///
    /// The action gate cannot catch that. It refuses *actions*, and the control
    /// that did it is an ordinary click listener on the title bar that calls a
    /// workspace method. So the answer is structural: once sealed, the
    /// workspace renders no centre pane, no title bar and no status bar, and
    /// the reveal path returns early instead of closing the one panel the
    /// window has.
    ///
    /// The seal is later than the mode on purpose, and the check pins that too.
    /// `OMEGA-DELTA-0040`'s identity onboarding is a centre-pane item, so
    /// sealing at startup would leave a fresh profile with nowhere to answer the
    /// identity gate — a worse dead end than the one `OMEGA-DELTA-0051`
    /// repaired, and the same shape.
    #[test]
    fn a_sealed_zero_base_renders_no_editor() {
        let mode_path = repository_path(ZERO_BASE_MODE_PATH);
        let mode = std::fs::read_to_string(&mode_path)
            .unwrap_or_else(|error| panic!("cannot read {}: {error}", mode_path.display()));
        assert!(
            mode.contains("pub fn seal()") && mode.contains("pub fn is_sealed() -> bool"),
            "OMEGA-DELTA-0053: {} no longer offers the seal. Without it the mode \
             is back to hiding the editor under a zoomed panel, which one \
             sidebar toggle undid.",
            mode_path.display()
        );
        assert!(
            mode.contains("is_active() && SEALED.load(Ordering::SeqCst)"),
            "OMEGA-DELTA-0053: `is_sealed` in {} no longer requires the mode to \
             be on. A build started with the editor flag must not be sealable by \
             a stray call.",
            mode_path.display()
        );

        let workspace_path = repository_path(WORKSPACE_RENDER_PATH);
        let workspace = std::fs::read_to_string(&workspace_path)
            .unwrap_or_else(|error| panic!("cannot read {}: {error}", workspace_path.display()));
        assert!(
            workspace.contains("let zero_base_sealed = omega_zero_base::is_sealed();"),
            "OMEGA-DELTA-0053: {} no longer reads the seal, so its render draws \
             the editor in zero base again.",
            workspace_path.display()
        );
        for structural in [
            // The centre pane group and its tab bar, which is where the welcome
            // surface the owner was shown lives.
            "if zero_base_sealed {",
            // The title bar, whose controls are click listeners the action gate
            // never sees.
            ".filter(|_| !zero_base_sealed)",
            // The status bar.
            "self.status_bar_visible(cx) && !zero_base_sealed",
            // The reveal path that closed the one open dock.
            "if omega_zero_base::is_sealed() {",
        ] {
            assert!(
                workspace.contains(structural),
                "OMEGA-DELTA-0053: {} lost `{structural}`. Each one is a surface \
                 the mode claims is absent, and a surface that is only covered \
                 is one control away from being present.",
                workspace_path.display()
            );
        }

        // Sealing happens after the identity gate, in the one place that opens
        // the thread. A seal anywhere earlier is the dead end described above.
        let panels_path = repository_path(WORKSPACE_INITIALIZATION_PATH);
        let panels = std::fs::read_to_string(&panels_path)
            .unwrap_or_else(|error| panic!("cannot read {}: {error}", panels_path.display()));
        assert_eq!(
            panels.matches("omega_zero_base::seal();").count(),
            1,
            "OMEGA-DELTA-0053: {} must seal the window exactly once. A second \
             caller is a second policy about when the identity gate can still \
             be answered.",
            panels_path.display()
        );
        let before_seal = panels
            .split_once("omega_zero_base::seal();")
            .map(|(before, _)| before)
            .unwrap_or_default();
        assert!(
            before_seal.contains("await_identity_ready(app_state, cx).await.log_err();"),
            "OMEGA-DELTA-0053: {} seals the window before waiting for identity \
             onboarding. Identity onboarding is a centre-pane item and a sealed \
             workspace renders no centre pane, so a fresh profile would have \
             nowhere to answer the gate `OMEGA-DELTA-0040` puts in front of it.",
            panels_path.display()
        );
    }

    // ---------------------------------------------------------------------
    // OMEGA-DELTA-0054 — Zero base opens the directory it was started in
    // ---------------------------------------------------------------------

    /// OMEGA-DELTA-0054. A thread that holds file tools has something to point
    /// them at, or says so in one line.
    ///
    /// Zero base opened no project. `crates/zed/src/zed.rs` said so in its own
    /// comment — "no project is opened, so there is no buffer for them to show"
    /// — and the consequence was not a missing buffer. The workspace had no
    /// worktrees, so `grep`, `find_path`, `list_directory`, `read_file` and
    /// `terminal` had nothing to operate on. Several searches returned nothing
    /// and the agent reported that the workspace appeared to be empty, which was
    /// true of the workspace and false of the owner's code.
    ///
    /// Why no test saw it: the visual baselines photograph a workspace the
    /// runner builds itself, and the runner hands it a lane path. The scenes had
    /// a project while the shipped launch path did not. That is the same class
    /// of gap `OMEGA-DELTA-0049` already records about `install_on_workspace`,
    /// and it has now cost something twice.
    #[test]
    fn zero_base_opens_the_directory_it_was_started_in() {
        let workdir_path = repository_path(WORKDIR_PATH);
        let workdir = std::fs::read_to_string(&workdir_path)
            .unwrap_or_else(|error| panic!("cannot read {}: {error}", workdir_path.display()));
        assert!(
            workdir.contains("pub fn plausible_project_root(")
                && workdir.contains("home: Option<&Path>"),
            "OMEGA-DELTA-0054: {} no longer offers the plausibility test, or no \
             longer takes the home directory as a parameter. Reading the ambient \
             process state would make the rule testable only on a machine that \
             happens to be in the right state, and startup is the one path no \
             test in this repository reaches.",
            workdir_path.display()
        );
        // A marker requirement would refuse a plain folder of files, which is a
        // legitimate thing to point an agent at and the case a person is most
        // likely in. The rule rejects what a launcher hands over and accepts
        // the rest.
        let rule = function_body(&workdir, "plausible_project_root").unwrap_or_else(|| {
            panic!(
                "OMEGA-DELTA-0054: cannot find the rule in {}",
                workdir_path.display()
            )
        });
        assert!(
            rule.len() > 200,
            "OMEGA-DELTA-0054: the body read for the rule is too short to be the \
             real one, so the check below would pass without reading it"
        );
        for marker in ["\".git\"", "Cargo.toml", "package.json"] {
            assert!(
                !rule.contains(marker),
                "OMEGA-DELTA-0054: the rule in {} requires {marker} of a project \
                 root. A plain folder of files is a legitimate thing to open, and \
                 a rule that refused it would refuse the ordinary case.",
                workdir_path.display()
            );
        }

        let startup_path = repository_path(STARTUP_OPEN_PATH);
        let startup = std::fs::read_to_string(&startup_path)
            .unwrap_or_else(|error| panic!("cannot read {}: {error}", startup_path.display()));
        // Order, not spelling. This used to require the exact text
        // `if open_zero_base_project(&app_state, cx).await {`, and
        // `OMEGA-DELTA-0093` had to negate that condition so the driven send
        // could run after either branch rather than only after the fallback.
        // The property the check is for is unchanged and is now asserted
        // directly: the project attempt comes first, and the empty workspace is
        // what happens when it fails.
        let attempt = startup
            .find("open_zero_base_project(&app_state, cx).await")
            .unwrap_or_else(|| {
                panic!(
                    "OMEGA-DELTA-0054: {} no longer tries to open a project, so \
                     zero base opens with no worktrees and every file tool the \
                     thread holds reads nothing.",
                    startup_path.display()
                )
            });
        let fallback = startup[attempt..]
            .find("workspace::open_new(")
            .unwrap_or_else(|| {
                panic!(
                    "OMEGA-DELTA-0054: {} no longer falls back to an empty \
                     workspace after the project attempt.",
                    startup_path.display()
                )
            });
        assert!(
            fallback > 0,
            "OMEGA-DELTA-0054: {} opens an empty workspace before it tries to \
             open the working directory as a project.",
            startup_path.display()
        );
        assert!(
            startup.contains("if !omega_zero_base::is_active() {\n        return false;\n    }"),
            "OMEGA-DELTA-0054: {} opens the working directory outside zero base \
             too. `omega --full-editor` opening whatever directory a shell \
             happened to be in is a change nobody asked for.",
            startup_path.display()
        );

        // The empty case is legible. A person must never again be told that
        // their workspace appears to be empty as if it were a fact about their
        // code.
        let thread_path = repository_path(THREAD_VIEW_PATH);
        let thread = std::fs::read_to_string(&thread_path)
            .unwrap_or_else(|error| panic!("cannot read {}: {error}", thread_path.display()));
        let notice =
            function_body(&thread, "render_zero_base_provider_notice").unwrap_or_else(|| {
                panic!(
                    "OMEGA-DELTA-0054: cannot find the composer notice in {}",
                    thread_path.display()
                )
            });
        assert!(
            notice.len() > 200,
            "OMEGA-DELTA-0054: the body read for the composer notice is too \
             short to be the real one, so this check would pass without reading \
             it"
        );
        assert!(
            notice.contains("No folder open") && notice.contains("visible_worktrees"),
            "OMEGA-DELTA-0054: the composer notice in {} no longer says when \
             there is no folder, or no longer asks the project. A thread that \
             silently searches nothing is how \"the workspace appears to be \
             empty\" gets said to somebody about their own code.",
            thread_path.display()
        );

        // The line the baselines caught. It said "No AI provider configured"
        // directly beneath a turn that had just completed through `exo/basic`,
        // and it offered a way out of the mode that `OMEGA-DELTA-0052` removed.
        assert!(
            !thread.contains("No AI provider configured"),
            "OMEGA-DELTA-0054: {} still says \"No AI provider configured\". That \
             was true of model providers and false of the turn the person had \
             just watched run, and detection now answers the question the line \
             was asking badly.",
            thread_path.display()
        );
        assert!(
            notice.contains("omega_agent_detect::detected()"),
            "OMEGA-DELTA-0054: the composer notice in {} no longer asks what is \
             installed. The settings-keyed predicate it replaced reads a written \
             setting, not a binary that exists, and a fresh profile has no \
             settings however many agents are on the machine.",
            thread_path.display()
        );

        // The control the notice draws is admitted, because a drawn control the
        // gate refuses is the "Close Left Dock" defect in the other direction.
        let mode_path = repository_path(ZERO_BASE_MODE_PATH);
        let mode = std::fs::read_to_string(&mode_path)
            .unwrap_or_else(|error| panic!("cannot read {}: {error}", mode_path.display()));
        let admitted = mode
            .split_once("pub const ADMITTED_ACTIONS: &[&str] = &[")
            .map(|(_, rest)| rest)
            .unwrap_or_default();
        let admitted = admitted.split_once("];").map_or(admitted, |(list, _)| list);
        assert!(
            admitted.contains("\"workspace::Open\","),
            "OMEGA-DELTA-0054: {} does not admit `workspace::Open`, so the \
             composer's Open Folder control is drawn and refused at dispatch — \
             the same failure as a status-bar button that does nothing.",
            mode_path.display()
        );
        assert!(
            !admitted.contains("\"workspace::OpenFiles\""),
            "OMEGA-DELTA-0054: {} admits `workspace::OpenFiles`. Choosing what \
             the thread can see is one control; opening the editor's file \
             surfaces inside a mode that does not render them is another.",
            mode_path.display()
        );
    }

    // ---------------------------------------------------------------------
    // OMEGA-DELTA-0055 — Routing is decided, not selected
    // ---------------------------------------------------------------------

    /// OMEGA-DELTA-0055. The executor pin control is gone and the route it used
    /// to set is decided automatically.
    ///
    /// These are one change, not two. The pin was the only door into the Exo
    /// lane — `OMEGA-DELTA-0049`'s own prose says a thread routes to Exo
    /// exactly when a person pins `ExternalAcp` on it — so deleting the control
    /// on its own would have made the lane unreachable rather than automatic.
    /// The check therefore asserts both halves together, because either alone
    /// is a defect: the control without the routing is jargon shown to a
    /// person, and the routing without the control is fine but the control
    /// coming back is a regression.
    ///
    /// Owner gate 8 is deliberately untouched, and the reason it is untouched
    /// is the reason this is admissible at all. The gate forbids a
    /// model-initiated start of Full Auto authority. An engine lane is that
    /// authority and still needs a pin. An external ACP agent is not — and the
    /// connection is made at startup from what is installed, so nothing a turn
    /// says can attach one.
    #[test]
    fn routing_is_decided_rather_than_selected() {
        let thread_path = repository_path(THREAD_VIEW_PATH);
        let thread = std::fs::read_to_string(&thread_path)
            .unwrap_or_else(|error| panic!("cannot read {}: {error}", thread_path.display()));
        for token in [
            "render_executor_pin",
            "\"omega-executor-pin\"",
            "Pin this thread",
        ] {
            assert!(
                !thread.contains(token),
                "OMEGA-DELTA-0055: {} still carries `{token}`. The control read \
                 `pin: none` and opened a menu of `native_loop`, `external_acp` \
                 and `engine_lane` — wire tokens offered to a person as a \
                 choice. `ExecutorClass::token`'s own documentation says it is \
                 never shown to a user on its own.",
                thread_path.display()
            );
        }

        let law_path = repository_path(ROUTE_DECISION_PATH);
        let law = std::fs::read_to_string(&law_path)
            .unwrap_or_else(|error| panic!("cannot read {}: {error}", law_path.display()));
        let rule = function_body(&law, "route").unwrap_or_else(|| {
            panic!(
                "OMEGA-DELTA-0055: cannot find the routing law in {}",
                law_path.display()
            )
        });
        assert!(
            rule.len() > 400,
            "OMEGA-DELTA-0055: the body read for the routing law is too short \
             to be the real one, so the checks below would pass without reading \
             it"
        );
        // Scoped to the *unpinned* branch, and the scoping is load-bearing.
        // `if inputs.external_acp.is_some() {` also appears in the arm that
        // honours an `ExternalAcp` pin, so a check against the whole function
        // stayed green with the automatic arm deleted — observed directly while
        // falsifying this test. The branch is what the delta is about.
        let unpinned = rule
            .split_once("let Some(pin) = inputs.pin.clone() else {")
            .map(|(_, rest)| rest)
            .unwrap_or_default();
        let unpinned = unpinned
            .split_once("\n    };")
            .map_or(unpinned, |(body, _)| body);
        assert!(
            unpinned.len() > 200 && unpinned.len() < rule.len(),
            "OMEGA-DELTA-0055: the unpinned branch read from {} is not a \
             plausible branch, so the checks below would be testing the whole \
             function or nothing",
            law_path.display()
        );
        assert!(
            unpinned.contains("if inputs.external_acp.is_some() {")
                && unpinned.contains("reason: RouteReason::DetectedExternalAcp"),
            "OMEGA-DELTA-0055: {} no longer routes an unpinned thread to an \
             attached external agent. With the pin control removed, that leaves \
             the Exo lane unreachable by any means.",
            law_path.display()
        );
        assert!(
            !unpinned.contains("ExecutorClass::EngineLane"),
            "OMEGA-DELTA-0055: the unpinned branch in {} names an engine lane. \
             An engine lane is Full Auto authority and owner gate 8 admits only \
             an explicit human action into it, so an unpinned thread must not \
             be able to reach one.",
            law_path.display()
        );
        assert!(
            law.contains("DetectedExternalAcp,"),
            "OMEGA-DELTA-0055: {} no longer declares the reason an automatic \
             route is recorded under. A route the journal cannot name is a \
             route nobody can explain afterwards.",
            law_path.display()
        );

        let mode_path = repository_path(ZERO_BASE_MODE_PATH);
        let mode = std::fs::read_to_string(&mode_path).unwrap_or_default();
        assert!(
            !mode.contains("ExecutorPin") && !mode.contains("PinGesture"),
            "OMEGA-DELTA-0055: {} now knows about pins. Automatic routing is a \
             property of the routing law, not of a mode.",
            mode_path.display()
        );
    }

    /// OMEGA-DELTA-0092. The lane is derived from the install, and the
    /// derivation is admitted for exactly one path.
    ///
    /// Three things, and each is a way the change silently comes undone.
    ///
    /// The **derivation exists** and is reached from the lane's own resolution,
    /// because `OMEGA-DELTA-0055` routes an unpinned thread to whatever
    /// external agent is attached and nothing attached one: the lane needed a
    /// hand-written file. Losing the call puts the hand back.
    ///
    /// The **file still wins**, and it wins on existing rather than on parsing.
    /// A damaged lane file that fell through to derivation would replace an
    /// explicit configuration with a guess about a different `.exo`, which is
    /// the `OMEGA-DELTA-0042` failure arrived at from the other side.
    ///
    /// The **gate is positive**. Derivation is admitted when the path *is* the
    /// data directory's, not when it is absent from a list of harness paths, so
    /// a harness invented tomorrow is excluded by default. `agent_ui` hands a
    /// stateless run a temporary path that does not exist precisely so a
    /// rendering harness never spawns somebody's Exo, and deriving on absence
    /// would have undone that quietly.
    #[test]
    fn the_exo_lane_is_derived_from_the_install_and_only_for_the_product() {
        let detect_path = repository_path(EXO_DETECT_PATH);
        let detect = std::fs::read_to_string(&detect_path)
            .unwrap_or_else(|error| panic!("cannot read {}: {error}", detect_path.display()));
        assert!(
            detect.len() > 2000,
            "OMEGA-DELTA-0092: {} is too short to be the derivation, so the \
             checks below would pass without reading it",
            detect_path.display()
        );
        // Scoped to the derived lane's own declaration, and the scoping is
        // load-bearing. `pub conversation:` also appears on `ExoLaneOverrides`,
        // so a check against the whole file stayed green with the field deleted
        // from the lane — observed directly while falsifying this test.
        let derived = detect
            .split_once("pub struct DerivedExoLane {")
            .and_then(|(_, rest)| rest.split_once("\n}"))
            .map(|(body, _)| body)
            .unwrap_or_else(|| {
                panic!(
                    "OMEGA-DELTA-0092: cannot find the derived lane in {}",
                    detect_path.display()
                )
            });
        assert!(
            derived.len() < detect.len() / 2,
            "OMEGA-DELTA-0092: the body read for the derived lane is not a \
             plausible struct, so the field checks would be reading the whole \
             file — which is how a deleted field went unnoticed once already"
        );
        for field in ["binary", "checkout", "root", "agent", "conversation"] {
            assert!(
                derived.contains(&format!("pub {field}:")),
                "OMEGA-DELTA-0092: {} no longer derives `{field}`. A lane is \
                 all five fields or it is not a lane.",
                detect_path.display()
            );
        }
        assert!(
            detect.contains("pub const HARNESS_DIRECTORY: &str = \"exoharness\";"),
            "OMEGA-DELTA-0092: {} no longer names Exo's harness directory.",
            detect_path.display()
        );
        // Scoped for the same reason, with the test module removed rather than
        // by function: the fixture helper beside this code builds the identical
        // path, so a whole-file check stayed green with the *production* reader
        // moved one level up — also observed while falsifying this test, and
        // the more dangerous of the two, because that is the level the layout
        // trap is at.
        assert!(
            production_source(&detect).contains("root.join(HARNESS_DIRECTORY).join(\"agents\")"),
            "OMEGA-DELTA-0092: {} no longer reads agents from inside Exo's \
             harness directory. Exo's `--root` is not where the records are — \
             the CLI opens the store at `<root>/exoharness` — so a reader one \
             level too high finds nothing on a machine that has agents and \
             reports that Exo has never run.",
            detect_path.display()
        );
        // The correction. A searcher that looks in one place produces a
        // confident false absence on a machine that has the thing — the same
        // shape as reading one level too high, one level further out, and it
        // happened here: `<checkout>/.exo` was the only candidate, it did not
        // exist, and the absence was reported as "Exo has never been run on
        // this machine" while two roots with live agents sat on the same disk.
        let root = function_body(&detect, "state_root").unwrap_or_else(|| {
            panic!(
                "OMEGA-DELTA-0092: cannot find the state root search in {}",
                detect_path.display()
            )
        });
        for candidate in [
            "overrides.root.as_deref()",
            "working_directory",
            "checkout.join(STATE_ROOT_DIRECTORY)",
            "lane_file",
        ] {
            assert!(
                root.contains(candidate),
                "OMEGA-DELTA-0092: the state root search in {} no longer tries \
                 `{candidate}`. Exo's root is wherever `--root` said, and on \
                 the machine this was written against it has never been beside \
                 the checkout.",
                detect_path.display()
            );
        }
        assert!(
            root.contains("record_slugs(&agents_directory(candidate)).is_empty()"),
            "OMEGA-DELTA-0092: the state root search in {} no longer prefers a \
             root that holds an agent. An empty root is the same dead end as no \
             root, so taking one over a working one reintroduces the failure \
             the search exists to remove.",
            detect_path.display()
        );
        assert!(
            detect.contains("searched: Vec<PathBuf>,") && !detect.contains("expected: PathBuf,"),
            "OMEGA-DELTA-0092: {} reports an absent root as one expected path \
             again. A refusal that names one place gets read as a statement \
             about every place, which is exactly how this went wrong.",
            detect_path.display()
        );
        // The order is the policy, so it is written down as prose beside the
        // code and checked against it. A candidate added or removed is a policy
        // change and belongs in the registry entry, not in a list edit.
        let documented = declaration_body(&detect, "pub const ROOT_CANDIDATE_ORDER: &[&str] = &[")
            .unwrap_or_else(|| {
                panic!(
                    "OMEGA-DELTA-0092: {} no longer documents the candidate \
                     order.",
                    detect_path.display()
                )
            });
        assert_eq!(
            documented.matches('"').count() / 2,
            ROOT_CANDIDATE_ORDER_LENGTH,
            "OMEGA-DELTA-0092: {} documents a different number of root \
             candidates than the registry entry describes.",
            detect_path.display()
        );
        for named in [ROOT_ENV_VAR_SPELLING, LANE_FILE_ENV_VAR_SPELLING] {
            assert!(
                documented.contains(named),
                "OMEGA-DELTA-0092: the documented candidate order in {} no \
                 longer mentions {named}.",
                detect_path.display()
            );
        }
        assert!(
            detect.contains("admits_upstream"),
            "OMEGA-DELTA-0092: {} no longer asks the pin whether a checkout is \
             the Exo this lane drives. omega#86 was a day spent on exo labs' \
             `exo-explore/exo`, which shares nothing with ours but a name.",
            detect_path.display()
        );
        // The working-directory candidate has to be a directory somebody chose,
        // and the one place that decides it is `omega_workdir`.
        //
        // macOS hands a Finder or Dock launch a working directory of `/`. Read
        // raw, the candidate the search added becomes `/.exo`: inert in the
        // launch a new person actually makes, and — if such a root ever exists
        // — a path nobody named offered to the search that decides which `.exo`
        // somebody's first message lands in. `omega_workdir` already answers
        // "is this a directory a person chose", for `OMEGA-DELTA-0054` and on
        // this same startup path; a second answer to that question in this file
        // is how the two eventually disagree about the same launch.
        let from_env = function_body(&detect, "derive_lane_from_env").unwrap_or_else(|| {
            panic!(
                "OMEGA-DELTA-0092: cannot find the environment read in {}",
                detect_path.display()
            )
        });
        assert!(
            uncommented(from_env).contains("working_directory: chosen_working_directory("),
            "OMEGA-DELTA-0092: {} reads the working directory raw again. A \
             launcher hands over `/`, so an ungated read offers `/.exo` to the \
             root search on every Finder and Dock launch — the one launch a new \
             person makes.",
            detect_path.display()
        );
        let chosen = function_body(&detect, "chosen_working_directory").unwrap_or_else(|| {
            panic!(
                "OMEGA-DELTA-0092: {} no longer decides which working \
                 directories are candidates in one testable place. It is a \
                 parameter everywhere else in this crate for the same reason: \
                 startup is the path no test here reaches.",
                detect_path.display()
            )
        });
        assert!(
            uncommented(chosen).contains("omega_workdir::plausible_project_root"),
            "OMEGA-DELTA-0092: {} decides for itself whether a working \
             directory is one a person chose. `OMEGA-DELTA-0054` owns that \
             question, and two answers to it disagree about the same launch.",
            detect_path.display()
        );

        let lane_path = repository_path(EXO_LANE_RESOLUTION_PATH);
        let lane = std::fs::read_to_string(&lane_path)
            .unwrap_or_else(|error| panic!("cannot read {}: {error}", lane_path.display()));
        let resolve = function_body(&lane, "resolve").unwrap_or_else(|| {
            panic!(
                "OMEGA-DELTA-0092: cannot find the lane resolution in {}. \
                 Without it a thread reaches Exo only through a file somebody \
                 wrote by hand.",
                lane_path.display()
            )
        });
        assert!(
            resolve.len() > 200,
            "OMEGA-DELTA-0092: the body read for the lane resolution is too \
             short to be the real one, so the checks below would pass without \
             reading it"
        );
        assert!(
            resolve.contains("omega_agent_detect::exo::derive_lane_from_env()"),
            "OMEGA-DELTA-0092: {} no longer derives a lane from the install. \
             `OMEGA-DELTA-0055` routes an unpinned thread to the external agent \
             that is attached; without this, nothing attaches one.",
            lane_path.display()
        );
        assert!(
            resolve.contains("if path.exists() {") && resolve.contains("return Self::load(path);"),
            "OMEGA-DELTA-0092: {} no longer lets an existing lane file settle \
             the question. It must be existence and not parsing: a damaged file \
             that fell through to derivation would replace somebody's explicit \
             configuration with a guess about a different `.exo`.",
            lane_path.display()
        );
        assert!(
            resolve.contains("if path != Self::data_dir_path() {"),
            "OMEGA-DELTA-0092: {} no longer restricts derivation to the \
             product's own lane path. `agent_ui` hands a stateless run a \
             temporary path that does not exist so that a rendering harness \
             never spawns somebody's Exo, and deriving whenever a file is \
             absent undoes that.",
            lane_path.display()
        );
        // The search reads two fields out of a lane file to find a root, so it
        // carries its own copy of the schema string. A guard that silently
        // stopped matching would make it accept a file the product refuses.
        let quoted = |source: &str, name: &str| -> String {
            source
                .split_once(&format!("{name}: &str = \""))
                .and_then(|(_, rest)| rest.split_once('"'))
                .map(|(value, _)| value.to_owned())
                .unwrap_or_else(|| panic!("OMEGA-DELTA-0092: no `{name}` to read"))
        };
        assert_eq!(
            quoted(&lane, "EXO_LANE_SCHEMA"),
            quoted(&detect, "LANE_FILE_SCHEMA"),
            "OMEGA-DELTA-0092: {} and {} disagree about the lane file schema.",
            lane_path.display(),
            detect_path.display()
        );
        assert!(
            lane.contains("ExoLaneConfig::resolve(lane_path)"),
            "OMEGA-DELTA-0092: {} connects the lane without going through the \
             resolution, so the derivation exists and nothing reaches it.",
            lane_path.display()
        );
    }

    /// OMEGA-DELTA-0093. A turn can be driven without a keyboard, over the
    /// send a typed message already uses.
    ///
    /// The flag has to exist, the startup path has to reach the driver, and the
    /// driver has to go through the panel rather than around it. The last is
    /// the one worth a mechanical check: `AcpThread::send` is public and
    /// reachable from `crates/zed`, so the shortest way to make this flag
    /// "work" is to build a prompt and push it at the connection — which would
    /// skip the composer, the mention resolution, the queue and the send
    /// disposition, and would prove nothing about the path a person uses. A
    /// control surface that bypasses the production path proves nothing about
    /// the production path.
    ///
    /// The wait is checked too. A thread is idle for the moment between being
    /// built and the turn starting, so a driver that waited only for idle would
    /// report a completed turn before the first token: a green unattended run
    /// that means nothing, which is exactly what this deliverable exists to
    /// stop being possible.
    #[test]
    fn a_turn_can_be_driven_over_the_send_a_typed_message_uses() {
        let startup_path = repository_path(STARTUP_OPEN_PATH);
        let startup = std::fs::read_to_string(&startup_path)
            .unwrap_or_else(|error| panic!("cannot read {}: {error}", startup_path.display()));
        assert!(
            startup.contains("omega_send: Option<String>"),
            "OMEGA-DELTA-0093: {} no longer declares --omega-send. Synthetic \
             keystrokes are unusable on a busy desktop, and every visual claim \
             about a turn depends on sending one without the window having \
             focus.",
            startup_path.display()
        );
        // Commented out is not called. The first spelling of this check was
        // `startup.contains("drive_omega_send(cx).await;")`, and prefixing the
        // line with `//` left it green — observed directly while falsifying
        // this test.
        assert!(
            uncommented(&startup).contains("drive_omega_send(cx).await;"),
            "OMEGA-DELTA-0093: {} parses the flag and never acts on it.",
            startup_path.display()
        );

        // Both halves of the driver, because they are one path split for
        // readability: the outer function owns the deadline and the exit
        // status, the inner one owns the sequence. Reading only the inner one
        // left a bypass planted in the outer one unnoticed — also observed
        // while falsifying this test.
        let driver = ["drive_omega_send", "run_omega_send"]
            .into_iter()
            .map(|name| {
                function_body(&startup, name).unwrap_or_else(|| {
                    panic!(
                        "OMEGA-DELTA-0093: cannot find `{name}` in {}",
                        startup_path.display()
                    )
                })
            })
            .collect::<Vec<_>>()
            .join("\n");
        assert!(
            driver.len() > 400,
            "OMEGA-DELTA-0093: the body read for the driven send is too short \
             to be the real one, so the checks below would pass without reading \
             it"
        );
        assert!(
            driver.contains("panel.omega_send_first_message("),
            "OMEGA-DELTA-0093: {} no longer sends through the panel.",
            startup_path.display()
        );
        for bypass in [
            "thread.send(",
            "send_raw(",
            "AgentConnection",
            "PromptRequest",
        ] {
            assert!(
                !driver.contains(bypass),
                "OMEGA-DELTA-0093: the driven send in {} reaches `{bypass}` \
                 itself. That is a second way to send, beside the one a typed \
                 message uses, and a control surface that bypasses the \
                 production path proves nothing about the production path.",
                startup_path.display()
            );
        }
        assert!(
            driver.contains("!= acp_thread::ThreadStatus::Idle")
                && driver.contains("== acp_thread::ThreadStatus::Idle"),
            "OMEGA-DELTA-0093: the driven send in {} no longer waits for the \
             turn to start before waiting for it to finish. A thread is idle \
             between being built and the turn starting, so waiting only for \
             idle reports a completed turn before the first token.",
            startup_path.display()
        );

        let panel_path = repository_path(AGENT_PANEL_PATH);
        let panel = std::fs::read_to_string(&panel_path)
            .unwrap_or_else(|error| panic!("cannot read {}: {error}", panel_path.display()));
        let entry = function_body(&panel, "omega_send_first_message").unwrap_or_else(|| {
            panic!(
                "OMEGA-DELTA-0093: cannot find the keyboardless send in {}",
                panel_path.display()
            )
        });
        assert!(
            entry.contains("self.external_thread(") && entry.contains("auto_submit: true"),
            "OMEGA-DELTA-0093: {} no longer submits through `external_thread`, \
             which is the same call the Git panel's review action makes and the \
             path that puts the text in the composer and presses send.",
            panel_path.display()
        );
        assert!(
            entry.contains("has_open_project"),
            "OMEGA-DELTA-0093: {} no longer refuses to open a thread with no \
             project. A thread whose file tools have no worktree is \
             OMEGA-DELTA-0054's failure, and reporting success there reports a \
             turn that did not start.",
            panel_path.display()
        );
    }

    /// OMEGA-DELTA-0100. The composer stays at the bottom and the transcript
    /// grows up into the space above it.
    ///
    /// omega#100's fifth acceptance. Three facts in `thread_view.rs` produce
    /// it, and until now none of them was asserted anywhere — the property was
    /// carried only by four PNG baselines compared at a 0.99 pixel threshold.
    /// That is weak evidence for a layout rule in two ways. A rebase that
    /// restored the upstream branch would move the composer to the top of a
    /// zoomed window, which is a large visual change and would fail the
    /// comparison — but only if somebody regenerates the baselines, and those
    /// four are the ones that need a live Exo state root to produce. On a
    /// machine with no root they cannot be regenerated at all, so the only
    /// check on the rule is one that cannot currently be run.
    ///
    /// The three facts, and why each is separately load-bearing:
    ///
    /// **The composer does not absorb an empty thread.** Upstream gives the
    /// composer the whole panel when there are no messages. In a dock-sized
    /// panel that reads as a roomy new-thread surface; zoomed to the window,
    /// which is what zero base does, it put the text field at the top of the
    /// screen with a field of dead black under it.
    ///
    /// **The empty transcript takes that space instead.** The inverse of the
    /// same rule, in the other branch. Both are needed: with only the first,
    /// nothing claims the space and the composer floats; with only the second,
    /// two elements both expand.
    ///
    /// **The conversation is drawn before the composer.** The order in the
    /// column is the "grows upward" claim itself, and it is the one fact a
    /// reader would assume rather than check.
    #[test]
    fn the_composer_stays_at_the_bottom_and_the_transcript_grows_up_to_it() {
        let path = repository_path(THREAD_VIEW_PATH);
        let source = std::fs::read_to_string(&path)
            .unwrap_or_else(|error| panic!("cannot read {}: {error}", path.display()));
        let source = uncommented(&source);

        assert!(
            source.contains(
                "let composer_fills_panel = !has_messages && !omega_zero_base::is_active();"
            ),
            "OMEGA-DELTA-0100: {} lets the composer absorb an empty thread in \
             zero base again. Upstream's rule reads as a roomy new-thread \
             surface in a dock-sized panel; zoomed to the whole window it puts \
             the input at the top of the screen with dead black beneath it.",
            path.display()
        );

        // Scoped to the branch, not to the file: `flex_1().size_full()` also
        // appears on the branch that has messages, so a whole-file `contains`
        // would stay green with the empty-transcript branch deleted — which is
        // the branch that does the pushing.
        let empty = source
            .split_once("} else if omega_zero_base::is_active() {")
            .map(|(_, rest)| rest)
            .unwrap_or_else(|| {
                panic!(
                    "OMEGA-DELTA-0100: {} no longer has a zero-base branch for \
                     an empty transcript, so nothing claims the space and the \
                     composer floats back up.",
                    path.display()
                )
            });
        let empty = &empty[..empty.len().min(400)];
        assert!(
            empty.contains("this.flex_1().size_full().into_any()"),
            "OMEGA-DELTA-0100: {}'s empty transcript no longer takes the space \
             above the composer in zero base. The composer gave that space up \
             in the check above; if nothing else claims it the input floats to \
             the top, which is the layout the owner asked to be rid of.",
            path.display()
        );

        // The last one, because the three earlier spellings are inside the Exo
        // inspector's own match and are not the column the composer sits in.
        let conversation = source.rfind(".child(conversation)").unwrap_or_else(|| {
            panic!("OMEGA-DELTA-0100: {} draws no conversation", path.display())
        });
        let composer = source
            .find(".child(self.render_message_editor(window, cx))")
            .unwrap_or_else(|| {
                panic!(
                    "OMEGA-DELTA-0100: {} draws no composer in its column",
                    path.display()
                )
            });
        assert!(
            conversation < composer,
            "OMEGA-DELTA-0100: {} draws the composer above the conversation. \
             The order in the column is the whole of \"the transcript grows \
             upward from the composer\", and it is the one fact a reader \
             assumes rather than checks.",
            path.display()
        );
    }

    // ---------------------------------------------------------------------
    // OMEGA-DELTA-0070 — A public Nostr chat skill is in every install
    // ---------------------------------------------------------------------

    /// OMEGA-DELTA-0070. The public NIP-29 chat skill is compiled in, is
    /// registered in the one table the loader reads, and keeps the precedence
    /// that lets a person override it.
    ///
    /// The runtime proof is `public_nostr_chat_is_built_in` in
    /// `crates/agent_skills/`: it runs the real loader and asserts the skill is
    /// in the catalog with source `BuiltIn`. That test cannot live here,
    /// because `agent_skills` depends on gpui and these checks are meant to run
    /// without building the UI framework. This check covers what a text scan
    /// covers better anyway — that the shipped file, the registration, and the
    /// runtime proof all still exist, and that the frontmatter the loader
    /// validates has not drifted out of the limits it validates against.
    #[test]
    fn a_public_nostr_chat_skill_ships_in_the_binary() {
        let skill_path = repository_path(PUBLIC_NOSTR_CHAT_SKILL_PATH);
        let skill = std::fs::read_to_string(&skill_path).unwrap_or_else(|error| {
            panic!(
                "OMEGA-DELTA-0070: cannot read {}: {error}. The skill is \
                 `include_str!`d, so Omega does not build without it — but a \
                 later change could point the include somewhere else and leave \
                 this file behind.",
                skill_path.display()
            )
        });

        let (frontmatter, body) = skill
            .strip_prefix("---\n")
            .and_then(|rest| rest.split_once("\n---"))
            .unwrap_or_else(|| {
                panic!(
                    "OMEGA-DELTA-0070: {} has no YAML frontmatter block, so \
                     the loader cannot parse it and the skill would be \
                     silently absent from every install.",
                    skill_path.display()
                )
            });

        let field = |key: &str| -> String {
            frontmatter
                .lines()
                .find_map(|line| line.strip_prefix(key))
                .unwrap_or_else(|| {
                    panic!(
                        "OMEGA-DELTA-0070: {} declares no `{key}` in its \
                         frontmatter. The loader rejects a skill without one.",
                        skill_path.display()
                    )
                })
                .trim()
                .to_owned()
        };

        assert_eq!(
            field("name:"),
            PUBLIC_NOSTR_CHAT_SKILL_NAME,
            "OMEGA-DELTA-0070: {} names itself something other than the name \
             it is registered under. The catalog entry takes its name from the \
             frontmatter and the embedded body is keyed by the table name, so \
             the two disagreeing gives a skill whose body cannot be fetched.",
            skill_path.display()
        );

        let description = field("description:");
        assert!(
            !description.is_empty() && description.len() <= MAX_SKILL_DESCRIPTION_LEN,
            "OMEGA-DELTA-0070: the description in {} is {} bytes. \
             `validate_description` refuses an empty one and refuses more than \
             {MAX_SKILL_DESCRIPTION_LEN}, and a built-in that fails validation \
             never reaches the catalog.",
            skill_path.display(),
            description.len()
        );

        for term in ["NIP-29", "relayUrl", "groupId"] {
            assert!(
                body.contains(term),
                "OMEGA-DELTA-0070: {} no longer mentions {term:?}, so it is \
                 not the public NIP-29 chat procedure this delta ships.",
                skill_path.display()
            );
        }

        let loader_path = repository_path(BUILTIN_SKILLS_PATH);
        let loader = std::fs::read_to_string(&loader_path)
            .unwrap_or_else(|error| panic!("cannot read {}: {error}", loader_path.display()));

        assert!(
            loader.contains(&format!(
                "pub const MAX_SKILL_DESCRIPTION_LEN: usize = {MAX_SKILL_DESCRIPTION_LEN};"
            )),
            "OMEGA-DELTA-0070: {} no longer declares \
             MAX_SKILL_DESCRIPTION_LEN as {MAX_SKILL_DESCRIPTION_LEN}, so the \
             limit checked above is not the limit the loader enforces.",
            loader_path.display()
        );

        assert!(
            loader.contains("include_str!(\"builtin/public-nostr-chat/SKILL.md\")"),
            "OMEGA-DELTA-0070: {} no longer embeds the skill at compile time. \
             Reading it from a directory would make the skill a file an \
             install may not have.",
            loader_path.display()
        );

        let table = loader
            .split_once("const BUILTIN_SKILL_ENTRIES")
            .and_then(|(_, rest)| rest.split_once("];"))
            .map(|(entries, _)| entries)
            .unwrap_or_else(|| {
                panic!(
                    "OMEGA-DELTA-0070: cannot find the registration table in {}",
                    loader_path.display()
                )
            });
        assert!(
            table.contains(&format!("(\"{PUBLIC_NOSTR_CHAT_SKILL_NAME}\"")),
            "OMEGA-DELTA-0070: {PUBLIC_NOSTR_CHAT_SKILL_NAME} is not in \
             BUILTIN_SKILL_ENTRIES in {}, so it is compiled into the binary \
             and reachable by nothing.",
            loader_path.display()
        );

        let catalog = function_body(&loader, "builtin_skills").unwrap_or_else(|| {
            panic!(
                "OMEGA-DELTA-0070: cannot find builtin_skills in {}",
                loader_path.display()
            )
        });
        assert!(
            catalog.contains("BUILTIN_SKILL_ENTRIES"),
            "OMEGA-DELTA-0070: builtin_skills in {} no longer reads the \
             registration table. It used to name `create-skill` directly while \
             the table was a second list used only to serve bodies; with one \
             entry the two could not disagree, so nothing showed that adding a \
             skill to the table added nothing to the catalog.",
            loader_path.display()
        );
        assert!(
            !catalog.contains("\"create-skill\"") && !catalog.contains("_CONTENT"),
            "OMEGA-DELTA-0070: builtin_skills in {} names an individual skill \
             again. A per-skill line beside the table is the second list this \
             delta removed.",
            loader_path.display()
        );

        let precedence = function_body(&loader, "precedence").unwrap_or_else(|| {
            panic!(
                "OMEGA-DELTA-0070: cannot find SkillSource::precedence in {}",
                loader_path.display()
            )
        });
        assert!(
            precedence.contains("Self::BuiltIn => 0,"),
            "OMEGA-DELTA-0070: a built-in skill no longer has the lowest \
             precedence in {}. Shipping a default that a global or \
             project-local skill of the same name cannot shadow takes a \
             person's override away instead of giving them a capability.",
            loader_path.display()
        );

        assert!(
            loader.contains("fn public_nostr_chat_is_built_in()"),
            "OMEGA-DELTA-0070: the runtime proof is gone from {}. Everything \
             above reads text; only that test runs the loader and asserts the \
             skill reaches the catalog as BuiltIn.",
            loader_path.display()
        );
        assert!(
            loader.contains("fn every_builtin_entry_loads_through_the_loader()"),
            "OMEGA-DELTA-0070: the table-to-catalog proof is gone from {}, so \
             a registered skill that never loads would be caught by nothing.",
            loader_path.display()
        );
    }

    // ---------------------------------------------------------------------
    // OMEGA-DELTA-0041 — Omega Agent served over ACP on a loopback socket
    // ---------------------------------------------------------------------

    /// OMEGA-DELTA-0041. The served ACP surface is off unless the flag says
    /// exactly `1`, and the source says so.
    ///
    /// `omega_acp_server` proves the behaviour with its own unit test. This
    /// checks the *shape* the behaviour depends on, in the file, because a
    /// later edit that made the flag truthy-tolerant — `is_some`,
    /// `unwrap_or("1")`, a `parse::<bool>()` — would keep every one of that
    /// crate's tests passing for the values it happened to list while turning
    /// a listener on for values it did not.
    #[test]
    fn the_served_acp_surface_is_off_unless_the_flag_is_exact() {
        let source = std::fs::read_to_string(repository_path(ACP_SERVER_PATH))
            .expect("the served ACP surface is readable");

        assert!(
            source.contains("None => Enablement::Off(OffReason::FlagUnset)"),
            "OMEGA-DELTA-0041: an unset OMEGA_ACP_SERVER must be off. A              listener that is on by default is a different product."
        );
        assert!(
            source.contains("Some(ENABLE_VALUE) => Enablement::On")
                && source.contains("Some(_) => Enablement::Off(OffReason::FlagNotExactlyOne)"),
            "OMEGA-DELTA-0041: the enable flag must be an exact match on              ENABLE_VALUE with everything else off. A truthy-tolerant flag is              a flag whose default nobody can state."
        );
        for tolerant in [
            "to_lowercase()",
            "eq_ignore_ascii_case",
            "parse::<bool>",
            "unwrap_or(ENABLE_VALUE)",
            "flag.is_some()",
        ] {
            assert!(
                !source.contains(tolerant),
                "OMEGA-DELTA-0041: the enable flag reads {tolerant}, which                  widens the set of values that open an unauthenticated socket."
            );
        }
    }

    /// OMEGA-DELTA-0041. The socket is the supervisor's, and GPUI cannot reach
    /// the crate that opens it.
    ///
    /// omega#82's falsifier is *GPUI owns the socket*. Two things make that
    /// false and both are checked here rather than asserted in prose: the
    /// crate that binds declares no GPUI dependency at all, and the only
    /// production caller of `start_if_enabled` is `crates/omega_effectd`.
    #[test]
    fn only_the_supervisor_opens_the_served_acp_socket() {
        let manifest = std::fs::read_to_string(repository_path(ACP_SERVER_MANIFEST_PATH))
            .expect("the served ACP surface's manifest is readable");
        for reaching_into_the_app in [
            "gpui.workspace",
            "workspace.workspace",
            "agent_ui.workspace",
            "project.workspace",
            "ui.workspace",
            "editor.workspace",
        ] {
            assert!(
                !manifest.contains(reaching_into_the_app),
                "OMEGA-DELTA-0041: crates/omega_acp_server depends on                  {reaching_into_the_app}. The crate that opens the                  unauthenticated loopback socket must not be reachable from                  the UI layer — that is exactly what omega#82's falsifier                  names."
            );
        }

        let crates = repository_path("crates");
        let mut binders: Vec<String> = Vec::new();
        let mut starters: Vec<String> = Vec::new();
        for_each_source_file(&crates, &["rs"], |path, source| {
            let display = path
                .display()
                .to_string()
                .rsplit("crates/")
                .next()
                .unwrap_or_default()
                .to_owned();
            if display.starts_with("omega_deltas/") {
                return;
            }
            for line in source.lines() {
                let trimmed = line.trim();
                if trimmed.starts_with("//") || trimmed.starts_with("///") {
                    continue;
                }
                if trimmed.contains("LoopbackAcpServer::bind(") {
                    binders.push(display.clone());
                }
                if trimmed.contains("start_if_enabled()") && !trimmed.starts_with("pub fn ") {
                    starters.push(display.clone());
                }
            }
        });

        assert!(
            !binders.is_empty() && !starters.is_empty(),
            "OMEGA-DELTA-0041: the scan found no bind and no start, so this              check is vacuous. Either the served surface is gone or the              needles stopped matching."
        );
        for file in &binders {
            assert!(
                file.starts_with("omega_acp_server/"),
                "OMEGA-DELTA-0041: {file} binds the served ACP listener. Only                  crates/omega_acp_server may."
            );
        }
        for file in &starters {
            assert!(
                file.starts_with("omega_effectd/") || file.starts_with("omega_acp_server/"),
                "OMEGA-DELTA-0041: {file} starts the served ACP listener. The                  supervisor layer owns the lifecycle; GPUI never opens its own                  socket."
            );
        }
    }

    /// OMEGA-DELTA-0041. Nothing reachable over the socket can take an
    /// executor pin.
    ///
    /// Owner gate 8 at the socket. An engine lane *is* Full Auto authority, a
    /// pin is the only door to one, and `OMEGA-DELTA-0035` already requires
    /// every pin-setting call to name a literal `PinGesture`. This closes the
    /// other end: the crate serving an unauthenticated surface never names a
    /// pin at all, so there is nothing there for a later edit to reach for.
    #[test]
    fn nothing_over_the_served_acp_surface_can_take_a_pin() {
        let source = std::fs::read_to_string(repository_path(ACP_SERVER_PATH))
            .expect("the served ACP surface is readable");
        // The shipped half only. The test module below it reads the pin ledger
        // on purpose, to prove every pin gesture is classified as unexposed.
        let shipped = source
            .split_once("#[cfg(test)]")
            .map(|(shipped, _)| shipped)
            .expect("the served ACP surface has a test module");
        for line in shipped.lines() {
            let trimmed = line.trim();
            // The doc comments explain *why* there is no pin here, and must be
            // allowed to say the word.
            if trimmed.starts_with("//") {
                continue;
            }
            for pin_reaching in [
                "PinGesture",
                "pin_session(",
                "pin_next_session(",
                "ExecutorPin::",
            ] {
                assert!(
                    !trimmed.contains(pin_reaching),
                    "OMEGA-DELTA-0041: the served ACP surface names                      {pin_reaching}. Nothing an external host can reach may                      set a pin: a pin is the only door to an engine lane and                      an engine lane is Full Auto authority, which owner gate 8                      admits only an explicit human action into."
                );
            }
        }
        assert!(
            source.contains("pin: None,"),
            "OMEGA-DELTA-0041: the served route inputs no longer pin nothing.              If this moved, the check above is watching the wrong thing."
        );
    }

    /// OMEGA-DELTA-0041. The served surface presents the first-party agent's
    /// own identity, not a second one.
    ///
    /// An attached host is told it is talking to Omega Agent. If the served
    /// identity drifted from the one `crates/agent` declares, the served
    /// surface would be disclosing an agent that does not exist — which is the
    /// same defect class as a rendered label in the record.
    #[test]
    fn the_served_surface_presents_the_first_party_agent_id() {
        let identity = std::fs::read_to_string(repository_path(NATIVE_AGENT_IDENTITY_PATH))
            .expect("the native agent is readable");
        let served = std::fs::read_to_string(repository_path(ACP_SERVER_PATH))
            .expect("the served ACP surface is readable");

        let declared = identity
            .lines()
            .find_map(|line| {
                let line = line.trim();
                let rest = line.strip_prefix("pub static OMEGA_AGENT_ID")?;
                let start = rest.find("AgentId::new(\"")? + "AgentId::new(\"".len();
                let tail = &rest[start..];
                let end = tail.find('"')?;
                Some(tail[..end].to_owned())
            })
            .expect("crates/agent declares OMEGA_AGENT_ID as a literal");

        assert!(
            served.contains(&format!(
                "pub const SERVED_AGENT_ID: &str = \"{declared}\";"
            )),
            "OMEGA-DELTA-0041: the served ACP surface presents an identity              other than {declared:?}, which is what crates/agent declares. An              attached host would be disclosed an agent that does not exist."
        );
    }

    /// OMEGA-DELTA-0041. The supervisor's start is not conditional on anything
    /// but the flag.
    ///
    /// A start hidden behind "and the engine is available" would make the
    /// surface's default depend on packaging rather than on the flag, so the
    /// default nobody could state would be back.
    #[test]
    fn the_supervisor_starts_the_served_surface_before_it_resolves_the_engine() {
        let source = std::fs::read_to_string(repository_path(EFFECTD_PATH))
            .expect("the supervisor is readable");
        let start = source
            .find("start_served_acp_surface();")
            .expect("OMEGA-DELTA-0041: the supervisor no longer starts the served surface");
        let resolve = source
            .find("resolve_effectd_command(")
            .expect("the supervisor resolves the packaged component");
        assert!(
            start < resolve,
            "OMEGA-DELTA-0041: the served surface is started after the              packaged component is resolved, so whether it listens depends on              packaging as well as on the flag."
        );
    }

    // ------------------------------------------------------ OMEGA-DELTA-0045

    /// OMEGA-DELTA-0045. A host-authored note is an entry kind, not a caption.
    ///
    /// `AgentThreadEntry` had six variants and every one of them was something
    /// a model or a user said. There was nowhere to put a line the *host*
    /// wrote, so the host had nothing to write into and refused. The variant is
    /// the seam; without it the refusal is the only honest answer, which is
    /// exactly the state rc11 through rc17 shipped.
    ///
    /// `push_system_note` is asserted to return `bool` and to be keyed on the
    /// engine-supplied id. Last-write-wins would let a retry rewrite a
    /// disclosure the owner had already been shown; an unkeyed append would
    /// show it twice.
    #[test]
    fn a_host_authored_note_is_a_thread_entry_kind() {
        let path = repository_path(THREAD_ENTRY_PATH);
        let source = std::fs::read_to_string(&path)
            .unwrap_or_else(|error| panic!("cannot read {}: {error}", path.display()));

        let (_, body, _) = next_enum_body(&source, "AgentThreadEntry").unwrap_or_else(|| {
            panic!(
                "OMEGA-DELTA-0045: no AgentThreadEntry enum found in {}. The \
                 check would be vacuous, so it fails instead.",
                path.display()
            )
        });
        assert!(
            body.lines()
                .map(str::trim)
                .any(|line| line == "SystemNote(SystemNote),"),
            "OMEGA-DELTA-0045: AgentThreadEntry has no SystemNote variant in \
             {}. Without an entry kind a non-model disclosure can be, the host \
             has nowhere to write a provider handoff and the thread the owner \
             reads goes silent — the rc11 defect FA-07 gate 5 forbids.",
            path.display()
        );

        let code = code_of(&source);
        assert!(
            code.contains(
                "pub fn push_system_note(&mut self, note: SystemNote, cx: &mut Context<Self>) -> bool"
            ),
            "OMEGA-DELTA-0045: {} must expose push_system_note returning \
             whether it appended.",
            path.display()
        );
        assert!(
            code.contains("existing.id == note.id"),
            "OMEGA-DELTA-0045: push_system_note in {} no longer keys on the \
             engine-supplied note id. Unkeyed, a retried append shows the owner \
             the same disclosure twice; last-write-wins lets a retry rewrite a \
             disclosure the owner has already read.",
            path.display()
        );
    }

    /// OMEGA-DELTA-0045. The host writes the note instead of refusing it.
    ///
    /// The refusal was typed and honest — better than rc11's silent
    /// `() => {}` — and it was still silence in the place that matters. An
    /// independent reviewer found `SYSTEM_NOTE_REFUSAL` in the shipped bytes of
    /// both `0.2.0-rc15` and `0.2.0-rc16`, so no candidate to date discloses a
    /// cross-provider handoff to the owner reading the thread.
    ///
    /// Two halves, because either alone is passable and useless: the refusal is
    /// gone, and the method reaches `push_system_note` on the thread named by
    /// `threadRef`. A handoff is addressed to the *target* thread; filing it
    /// against the source one would put the disclosure where the owner is no
    /// longer reading.
    #[test]
    fn the_host_appends_a_provider_handoff_note_rather_than_refusing_it() {
        let path = repository_path(HOST_BRIDGE_PATH);
        let source = std::fs::read_to_string(&path)
            .unwrap_or_else(|error| panic!("cannot read {}: {error}", path.display()));
        let code = code_of(&source);

        assert!(
            !code.contains(SYSTEM_NOTE_REFUSAL),
            "OMEGA-DELTA-0045: {} still refuses the system-note method with \
             {SYSTEM_NOTE_REFUSAL:?}. That refusal is the rc11 silence in a \
             typed wrapper: the engine emits a provider-handoff note naming \
             both lanes and the host drops it, so a run that changed which \
             model spends the owner's budget leaves no trace in the transcript.",
            path.display()
        );
        assert!(
            code.contains("HostMethod::AppendSystemNote => append_system_note("),
            "OMEGA-DELTA-0045: {} no longer routes AppendSystemNote to \
             append_system_note.",
            path.display()
        );
        assert!(
            code.contains("thread.push_system_note("),
            "OMEGA-DELTA-0045: append_system_note in {} does not reach \
             push_system_note. A method that validates its params and returns \
             `{{\"appended\": true}}` without writing anything is a refusal \
             that lies rather than a refusal that is honest.",
            path.display()
        );
        assert!(
            code.contains("thread.thread_id.to_key_string() == params.thread_ref"),
            "OMEGA-DELTA-0045: append_system_note in {} no longer resolves the \
             thread named by threadRef. A handoff is addressed to the target \
             thread; filing it against whichever thread is nearest puts the \
             disclosure where the owner has stopped reading.",
            path.display()
        );
    }

    /// OMEGA-DELTA-0045. The thread surface draws the note, unconditionally.
    ///
    /// An entry kind nothing renders satisfies every other check here and
    /// discloses nothing, which is the same failure shape omega#77 pinned for
    /// the executor line. So the call site is pinned as well as the variant.
    ///
    /// And it is pinned as an *unconditional* draw. The gate is owner
    /// visibility; a note behind a disclosure triangle, a hover, or a collapsed
    /// section is a note the rc11 handoff would also have passed. The body is
    /// read for the expansion vocabulary the compaction entry uses, because
    /// that is the nearest thing in this file to copy by accident.
    #[test]
    fn the_thread_surface_draws_a_host_authored_note_unconditionally() {
        let path = repository_path(THREAD_VIEW_PATH);
        let source = std::fs::read_to_string(&path)
            .unwrap_or_else(|error| panic!("cannot read {}: {error}", path.display()));
        let code = code_of(&source);

        assert!(
            code.contains("fn render_system_note("),
            "OMEGA-DELTA-0045: {} must define the host-authored note line.",
            path.display()
        );
        assert!(
            code.contains(
                "AgentThreadEntry::SystemNote(note) => self.render_system_note(entry_ix, note),"
            ),
            "OMEGA-DELTA-0045: {} must draw a SystemNote entry from the entry \
             match. Defining the renderer without dispatching to it discloses \
             nothing, and every other check in this delta would still pass.",
            path.display()
        );

        let start = code
            .find("fn render_system_note(")
            .expect("the renderer was just asserted to exist");
        let body = &code[start..];
        let end = body.find("\n    fn ").expect(
            "OMEGA-DELTA-0045: no method follows render_system_note, so its \
             body cannot be bounded and the scan below would read the rest of \
             the file",
        );
        let body = &body[..end];

        assert!(
            body.contains("Label::new(note.text.clone())"),
            "OMEGA-DELTA-0045: the note line in {} must be drawn from the \
             entry's own text as a Label. Rendering it as Markdown would let \
             provider-supplied content style a host-authored disclosure or pass \
             itself off as one.",
            path.display()
        );
        for hideable in ["is_expanded", "expansion", "toggle", "tooltip", "hover"] {
            assert!(
                !body.contains(hideable),
                "OMEGA-DELTA-0045: the note line in {} names {hideable:?}. The \
                 gate is owner visibility, and anything the owner has to click, \
                 expand, or hover to see is a disclosure the rc11 handoff would \
                 also have passed.",
                path.display()
            );
        }
    }

    /// OMEGA-DELTA-0060. A thread reads only the subagents it spawned.
    ///
    /// The scoping rule is the whole risk of this tool. Three things have to
    /// hold together, and each one is a way the rule could be lost without
    /// anybody noticing:
    ///
    /// 1. The decision is a **total** function over the immediate parent. A new
    ///    outcome, or a `_ =>` arm, would let a future case fall through to
    ///    whatever the last arm happens to be.
    /// 2. The environment reads the caller from **its own** thread. If the
    ///    caller ever became a parameter, the tool would be naming the thread
    ///    whose subagents it may read, and the scope would be the tool's
    ///    discipline rather than the signature's.
    /// 3. The ancestor chain is never walked. This tool is what makes a
    ///    grandchild's session ID visible to a root thread, so transitive
    ///    access would follow from raising `MAX_SUBAGENT_DEPTH` alone.
    #[test]
    fn a_thread_reads_only_the_subagents_it_spawned() {
        let tool_path = repository_path(SUBAGENT_TRANSCRIPT_TOOL_PATH);
        let tool = std::fs::read_to_string(&tool_path)
            .unwrap_or_else(|error| panic!("cannot read {}: {error}", tool_path.display()));

        for variant in SUBAGENT_TRANSCRIPT_ACCESS_VARIANTS {
            assert!(
                tool.contains(&format!("TranscriptAccess::{variant}"))
                    || tool.contains(&format!("{variant} ")),
                "OMEGA-DELTA-0060: {} no longer names the `{variant}` access \
                 outcome. The decision is a closed set; a removed or renamed \
                 outcome is a change to who may read a transcript.",
                tool_path.display()
            );
        }

        // The decision function takes the *immediate* parent, one `Option`, and
        // nothing that could carry a chain. A `Vec`, an iterator or an
        // `ancestors` argument here would be transitive access arriving quietly.
        let compact = without_whitespace(&tool);
        assert!(
            compact.contains(&without_whitespace(
                "pub fn subagent_transcript_access(
                    caller: &acp::SessionId,
                    target: &acp::SessionId,
                    target_parent: Option<&acp::SessionId>,
                ) -> TranscriptAccess"
            )),
            "OMEGA-DELTA-0060: the access decision in {} changed shape. It takes \
             the immediate parent and nothing else on purpose — a chain, a list \
             or an ancestor walk here grants a root thread everything its \
             delegates named.",
            tool_path.display()
        );

        // No catch-all. A total match over two inputs is what makes "there is
        // no case that quietly allows" checkable.
        assert!(
            !compact.contains(&without_whitespace("_ => TranscriptAccess::Granted")),
            "OMEGA-DELTA-0060: {} grants access from a catch-all arm. Every \
             grant must name the case it is granting.",
            tool_path.display()
        );

        // The refusal tells the caller whose thread it is. Reporting a real
        // session as missing sends a caller debugging its own delegation after
        // a bug that is not there, and withholds nothing it did not already
        // have.
        assert!(
            tool.contains("is a subagent of thread {parent}, not of this"),
            "OMEGA-DELTA-0060: {} no longer says whose subagent the refused \
             session is. A refusal that claims the session does not exist is a \
             lie to a caller that already holds the ID.",
            tool_path.display()
        );

        // The environment derives the caller from its own thread. The trait
        // method has no caller parameter, so the tool cannot ask on behalf of
        // anyone else.
        let registration_path = repository_path(SUBAGENT_TRANSCRIPT_REGISTRATION_PATH);
        let registration = std::fs::read_to_string(&registration_path)
            .unwrap_or_else(|error| panic!("cannot read {}: {error}", registration_path.display()));
        let registration_compact = without_whitespace(&registration);
        assert!(
            registration_compact.contains(&without_whitespace(
                "fn read_subagent_transcript(
                    &self,
                    _session_id: acp::SessionId,
                    _window: TranscriptWindowRequest,
                    _cx: &mut App,
                ) -> Result<SubagentTranscript, String>"
            )),
            "OMEGA-DELTA-0060: the transcript read in {} changed shape. It must \
             not take the calling thread as an argument — the environment knows \
             which thread is asking, and that is what keeps the scope out of \
             the tool's hands.",
            registration_path.display()
        );

        let environment_path = repository_path(SUBAGENT_TRANSCRIPT_ENVIRONMENT_PATH);
        let environment = std::fs::read_to_string(&environment_path)
            .unwrap_or_else(|error| panic!("cannot read {}: {error}", environment_path.display()));
        assert!(
            environment.contains("subagent_transcript_access(&caller, target.id(), target.parent_thread_id().as_ref())"),
            "OMEGA-DELTA-0060: {} no longer decides transcript access from the \
             target's stored parent. Any other source for that comparison is a \
             second answer to who may read a transcript.",
            environment_path.display()
        );
        // The refusal returns before any message is read. A shape that builds
        // the transcript first and filters afterwards is one early return away
        // from leaking.
        let refusal_index = environment
            .find("if let Some(refusal) = access.refusal(&session_id)")
            .expect(
                "OMEGA-DELTA-0060: the environment no longer refuses before \
                 reading. Access must be decided before any content is touched.",
            );
        let read_index = environment
            .find("target.transcript_window(window)")
            .expect("OMEGA-DELTA-0060: the environment no longer reads a bounded window");
        assert!(
            refusal_index < read_index,
            "OMEGA-DELTA-0060: {} reads transcript content before refusing. \
             Decide access first; a filter after the read is a leak waiting for \
             an early return.",
            environment_path.display()
        );
    }

    /// OMEGA-DELTA-0060. Every bound on the transcript is visible in it.
    ///
    /// The tool exists to hand a parent work it deliberately kept out of its
    /// context, so it must be bounded. But a bound that fires silently is worse
    /// than no bound: a reader who cannot see the cut concludes the subagent
    /// never did the thing, and re-delegates work that was already done. So
    /// every truncation has to say so, and the caps have to stay declared
    /// rather than drifting into magic numbers at the call site.
    #[test]
    fn a_truncated_transcript_says_that_it_was_truncated() {
        let tool_path = repository_path(SUBAGENT_TRANSCRIPT_TOOL_PATH);
        let tool = std::fs::read_to_string(&tool_path)
            .unwrap_or_else(|error| panic!("cannot read {}: {error}", tool_path.display()));

        for declared in [
            "pub const DEFAULT_MESSAGE_LIMIT: usize",
            "pub const MAX_MESSAGE_LIMIT: usize",
            "pub const FULL_BLOCK_BYTE_LIMIT: usize",
            "pub const OUTLINE_BLOCK_BYTE_LIMIT: usize",
            "pub const MAX_TRANSCRIPT_BYTES: usize",
        ] {
            assert!(
                tool.contains(declared),
                "OMEGA-DELTA-0060: {} no longer declares `{declared}`. An \
                 unbounded transcript hands the parent back the context that \
                 delegating was meant to save.",
                tool_path.display()
            );
        }

        // Both truncations announce themselves, and the whole-response one
        // names an offset that actually resumes the reading.
        assert!(
            tool.contains("block truncated: {} of {} bytes shown"),
            "OMEGA-DELTA-0060: {} clips a block without saying it clipped it.",
            tool_path.display()
        );
        assert!(
            tool.contains("transcript truncated at the {byte_cap}-byte cap"),
            "OMEGA-DELTA-0060: {} stops rendering at the byte cap without \
             marking it. Silent truncation reads as absence.",
            tool_path.display()
        );
        assert!(
            tool.contains("message(s) of this window were not rendered"),
            "OMEGA-DELTA-0060: {} no longer says how many messages the cap \
             dropped.",
            tool_path.display()
        );
        assert!(
            tool.contains("Ask again with offset="),
            "OMEGA-DELTA-0060: {} truncates without telling the reader how to \
             see the rest. A bound with no way past it is a dead end.",
            tool_path.display()
        );

        // The marker's own room is reserved out of the cap, so the cap can
        // never be the reason the reader is not told about the cap.
        assert!(
            tool.contains("byte_cap.saturating_sub(TRUNCATION_MARKER_RESERVE)"),
            "OMEGA-DELTA-0060: {} no longer reserves room for the truncation \
             marker. A marker that does not fit is a silent truncation.",
            tool_path.display()
        );
    }

    /// OMEGA-DELTA-0060. The tool actually reaches the model.
    ///
    /// Three gates drop a tool without failing to compile: the `tools!` macro,
    /// the profile allowlists in the shipped defaults, and the permission-UI
    /// lists. A tool that is written and registered but missing from a profile
    /// is invisible to the model, and the only symptom is that the model never
    /// calls it.
    #[test]
    fn the_transcript_tool_reaches_the_model() {
        let registration_path = repository_path(SUBAGENT_TRANSCRIPT_REGISTRATION_PATH);
        let registration = std::fs::read_to_string(&registration_path)
            .unwrap_or_else(|error| panic!("cannot read {}: {error}", registration_path.display()));
        // Registered under the same depth gate as spawning: a thread that
        // cannot spawn subagents has none to read.
        let compact = without_whitespace(&registration);
        assert!(
            compact.contains(&without_whitespace(
                "if self.depth() < MAX_SUBAGENT_DEPTH {
                    self.add_tool(SpawnAgentTool::new(environment.clone()));"
            )) && compact.contains(&without_whitespace(
                "ReadSubagentTranscriptTool::new(environment"
            )),
            "OMEGA-DELTA-0060: {} no longer registers the transcript tool \
             alongside spawning. It is gated with the spawn tool so a thread \
             with no subagents does not carry a tool that always refuses.",
            registration_path.display()
        );

        let settings = default_settings().expect("default settings must parse");
        let profiles = settings
            .get("agent")
            .and_then(|agent| agent.get("profiles"))
            .expect("the agent profiles must exist");
        for profile in ["write", "ask"] {
            let enabled = profiles
                .get(profile)
                .and_then(|profile| profile.get("tools"))
                .and_then(|tools| tools.get(SUBAGENT_TRANSCRIPT_TOOL_NAME))
                .and_then(serde_json::Value::as_bool)
                .unwrap_or(false);
            assert!(
                enabled,
                "OMEGA-DELTA-0060: the `{profile}` profile in {} does not enable \
                 `{SUBAGENT_TRANSCRIPT_TOOL_NAME}`. `Thread::enabled_tools` \
                 filters on this allowlist, so the tool would never reach the \
                 model and nothing would fail to compile.",
                DEFAULT_SETTINGS_PATH
            );
        }
    }

    /// OMEGA-DELTA-0080. A tool call's result body opens at a ceiling, and the
    /// reader is told what the ceiling hides.
    ///
    /// Four seams, because reverting any one of them restores the full-height
    /// body without a compile error:
    ///
    /// 1. the ceiling exists and is the Omega value, not the upstream one;
    /// 2. every terminal the agent panel creates opens with it applied;
    /// 3. the terminal honours it before it falls back to a scroll region,
    ///    otherwise a long result escapes the ceiling by being long;
    /// 4. the thread draws the control that lifts it, otherwise the ceiling is
    ///    a truncation with no way out.
    #[test]
    fn a_tool_result_opens_at_a_ceiling_the_reader_can_lift() {
        let ceiling_path = repository_path(TOOL_OUTPUT_CEILING_PATH);
        let ceiling_source = std::fs::read_to_string(&ceiling_path)
            .unwrap_or_else(|error| panic!("cannot read {}: {error}", ceiling_path.display()));

        // 1. The value. Read from the declaration, so a changed number fails
        // here rather than passing because the name is still spelled the same.
        let declared = ceiling_source
            .split_once("const COLLAPSED_TOOL_OUTPUT_LINES: usize =")
            .and_then(|(_, rest)| rest.split_once(';'))
            .map(|(value, _)| value.trim().replace('_', ""))
            .unwrap_or_else(|| {
                panic!(
                    "OMEGA-DELTA-0080: {} declares no \
                     `COLLAPSED_TOOL_OUTPUT_LINES`. Without it a tool result \
                     renders at its natural height again, which is upstream \
                     Zed's behaviour and the defect the owner reported.",
                    ceiling_path.display()
                )
            });
        assert_eq!(
            declared.parse::<usize>().ok(),
            Some(TOOL_OUTPUT_CEILING_LINES),
            "OMEGA-DELTA-0080: the ceiling in {} is `{declared}`, not \
             {TOOL_OUTPUT_CEILING_LINES}. Upstream's only ceiling on the same \
             body is {UPSTREAM_TOOL_OUTPUT_CEILING_LINES} lines. The value is \
             also restated in the `terminal_view` test \
             `embedded_ceiling_only_binds_on_a_long_result`, which fails with \
             this one if the two drift apart.",
            ceiling_path.display()
        );

        // 2. Applied at creation, so a result is capped before anybody reads
        // it. A ceiling applied on first render would flash the full height.
        //
        // Comment lines are dropped first. A commented-out call is the exact
        // shape a rebase leaves behind, and it reads as present to a plain
        // substring search.
        let compact_ceiling = without_whitespace(
            &ceiling_source
                .lines()
                .filter(|line| !line.trim_start().starts_with("//"))
                .collect::<Vec<_>>()
                .join("\n"),
        );
        assert!(
            compact_ceiling.contains(&without_whitespace(
                "view.set_embedded_max_lines(Some(COLLAPSED_TOOL_OUTPUT_LINES), cx);"
            )),
            "OMEGA-DELTA-0080: `create_terminal` in {} no longer applies the \
             ceiling to the terminal it builds. Every tool result the agent \
             panel shows comes from this one constructor, so the ceiling is \
             absent everywhere if it is absent here.",
            ceiling_path.display()
        );

        // 3. Honoured ahead of the scrollable fallback. Without the guard, a
        // result over `MAX_EMBEDDED_LINES` takes the fallback and ignores the
        // ceiling — the longest results would be the ones that escape it.
        let terminal_path = repository_path(TOOL_OUTPUT_CEILING_TERMINAL_PATH);
        let terminal_source = std::fs::read_to_string(&terminal_path)
            .unwrap_or_else(|error| panic!("cannot read {}: {error}", terminal_path.display()));
        let compact_terminal = without_whitespace(production_source(&terminal_source));
        assert!(
            compact_terminal.contains(&without_whitespace(
                "if max_lines.is_none() && total_lines > Self::MAX_EMBEDDED_LINES {"
            )),
            "OMEGA-DELTA-0080: `content_mode` in {} no longer checks the \
             ceiling before the `MAX_EMBEDDED_LINES` fallback, so a result \
             longer than {UPSTREAM_TOOL_OUTPUT_CEILING_LINES} lines escapes \
             the ceiling by being long.",
            terminal_path.display()
        );
        assert!(
            compact_terminal.contains(&without_whitespace("pub fn embedded_displayed_lines(")),
            "OMEGA-DELTA-0080: {} no longer exposes `embedded_displayed_lines`. \
             The decision moved back inside a method that needs a window, and \
             the ceiling has no test that can run without one.",
            terminal_path.display()
        );

        // 4. The way out. A ceiling with no control is a truncation.
        let render_path = repository_path(TOOL_OUTPUT_CEILING_RENDER_PATH);
        let render_source = std::fs::read_to_string(&render_path)
            .unwrap_or_else(|error| panic!("cannot read {}: {error}", render_path.display()));
        // `production_source` is no use here: this file opens a `#[cfg(test)]`
        // block near the top, so it would cut away everything being asserted.
        assert!(
            render_source.contains("fn render_terminal_output_ceiling_toggle(")
                && render_source.contains("self.render_terminal_output_ceiling_toggle("),
            "OMEGA-DELTA-0080: {} declares the ceiling control but never calls \
             it, or no longer has it. A reader would have no way to see the \
             lines the ceiling hides.",
            render_path.display()
        );
        // The control is a sibling of the capped body, never inside it.
        // `OMEGA-DELTA-0060`'s rule: the bound cannot be the reason the reader
        // is not told about the bound. A control placed inside the region the
        // ceiling clips would be cut by the ceiling it describes.
        let compact_render = without_whitespace(&render_source);
        assert!(
            compact_render.contains(&without_whitespace(
                ".child(element)
                 .children(ceiling_toggle)
                 .into_any_element()"
            )),
            "OMEGA-DELTA-0080: in {} the ceiling control is no longer the last \
             sibling of the capped body. Nested inside it, the control is \
             clipped by the ceiling it exists to announce.",
            render_path.display()
        );
        assert!(
            compact_render.contains(&without_whitespace(
                "div().h_72().child(terminal_view).into_any_element()"
            )),
            "OMEGA-DELTA-0080: in {} the bounded branch no longer holds the \
             terminal alone. Anything else put inside it is subject to the \
             same height limit, which is how the control ends up clipped.",
            render_path.display()
        );

        assert!(
            render_source.contains("tool_output_ceiling_label(total_lines"),
            "OMEGA-DELTA-0080: {} no longer takes the control's label from \
             `tool_output_ceiling_label`. The label states the count of hidden \
             lines, and a hand-written \"Show more\" tells a reader nothing \
             about what opening it costs.",
            render_path.display()
        );
    }

    // ------ OMEGA-DELTA-0090 — Fork and snapshot as episode reset

    /// OMEGA-DELTA-0090. The episode crate cannot reach the working tree.
    ///
    /// This is the load-bearing one. The manual falsification loop it replaces
    /// reverted a mutation with `git checkout --` and wiped uncommitted work in
    /// two files. A rule saying "do not do that" is a rule somebody forgets at
    /// 3am; the crate's answer is that it has no filesystem, no process, and no
    /// path type anywhere in it, so no version of that mistake compiles.
    ///
    /// Scanned over production source rather than tests, for the same reason
    /// `OMEGA-DELTA-0042` does it: the tests beside a refusal must be free to
    /// write the refused thing down.
    #[test]
    fn the_episode_crate_cannot_reach_the_working_tree() {
        let mut scanned = 0usize;
        for relative in EPISODE_CRATE_SOURCES {
            let path = repository_path(relative);
            let source = std::fs::read_to_string(&path)
                .unwrap_or_else(|error| panic!("cannot read {}: {error}", path.display()));
            scanned += 1;
            for token in EPISODE_FORBIDDEN_REACH {
                assert!(
                    !named_in_code(&source, token),
                    "OMEGA-DELTA-0090: {} names `{token}` in code. The episode crate \
                     is a leaf law with no filesystem and no process; the whole \
                     reason a forked episode cannot destroy uncommitted work is \
                     that nothing in it can reach a file.",
                    path.display()
                );
            }
        }
        assert_eq!(
            scanned,
            EPISODE_CRATE_SOURCES.len(),
            "OMEGA-DELTA-0090: the scan read {scanned} files, so it is reading less \
             than the crate."
        );
        assert!(
            scanned >= 7,
            "OMEGA-DELTA-0090: {scanned} files is not the episode crate. A module \
             added to the crate must be added to EPISODE_CRATE_SOURCES, or it goes \
             unscanned."
        );
    }

    /// OMEGA-DELTA-0090. An episode sends queries, one fork, and one restore.
    ///
    /// `exo serve` is unauthenticated and answers the whole protocol, including
    /// `get_secret` and `delete_agent`. Loopback keeps the endpoint off the
    /// network; this keeps the authority small once you are on it. Read off the
    /// `request_type` table, because that table is what actually goes on the
    /// wire.
    #[test]
    fn an_episode_sends_no_write_or_secret_request() {
        let path = repository_path(EPISODE_REQUEST_PATH);
        let source = std::fs::read_to_string(&path)
            .unwrap_or_else(|error| panic!("cannot read {}: {error}", path.display()));
        // `OMEGA-DELTA-0102`. The table names variants of the one enumeration
        // rather than spelling Exo's wire strings for itself, so this reads
        // identifiers and resolves them through the crate.
        let table = source
            .split_once("pub const fn kind(&self)")
            .and_then(|(_, rest)| rest.split_once("\n    }"))
            .expect("OMEGA-DELTA-0090: the request kind table is present")
            .0;
        let emitted: Vec<String> = kinds_named_in(table);
        assert_eq!(
            emitted.len(),
            4,
            "OMEGA-DELTA-0090: the request kind table parsed as {emitted:?}, which is \
             not the closed set of four this delta describes."
        );

        let families = repository_path(EPISODE_FAMILY_PATH);
        let family_source = std::fs::read_to_string(&families)
            .unwrap_or_else(|error| panic!("cannot read {}: {error}", families.display()));
        let classification = uncommented(&family_source);
        for variant in &emitted {
            let row = classification
                .lines()
                .find(|line| {
                    line.contains(&format!("ExoRequestKind::{variant} => RequestFamily::"))
                })
                .unwrap_or_else(|| {
                    panic!(
                        "OMEGA-DELTA-0090: the episode client emits `{variant}` and \
                         the family decision does not classify it, so nothing decided \
                         it was admitted."
                    )
                });
            assert!(
                EPISODE_ADMITTED_FAMILIES
                    .iter()
                    .any(|family| row.contains(&format!("RequestFamily::{family}"))),
                "OMEGA-DELTA-0090: the episode client emits `{variant}`, which is \
                 classified `{}`. Admitted families are {EPISODE_ADMITTED_FAMILIES:?}.",
                row.trim()
            );
            let kind = kind_named(variant).unwrap_or_else(|| {
                panic!("OMEGA-DELTA-0090: `{variant}` is not a request type Exo has")
            });
            assert!(
                omega_exo_episode::is_admitted(kind),
                "OMEGA-DELTA-0090: the episode client emits `{kind}`, which the \
                 compiled family decision refuses."
            );
        }

        let sent: Vec<&'static str> = emitted
            .iter()
            .filter_map(|variant| kind_named(variant))
            .map(omega_exo_lane::ExoRequestKind::wire)
            .collect();
        assert_eq!(sent.len(), 4);
        for refused in EPISODE_REFUSED_REQUEST_TYPES {
            assert!(
                !sent.contains(refused),
                "OMEGA-DELTA-0090: the episode client emits `{refused}`, which appends \
                 to, deletes from, or reads the secrets of somebody else's Exo."
            );
        }
        for family in EPISODE_REFUSED_FAMILIES {
            assert!(
                family_source.contains(&format!("RequestFamily::{family}")),
                "OMEGA-DELTA-0090: the family partition no longer has a `{family}` \
                 family, so nothing is refused by construction any more."
            );
        }
    }

    /// OMEGA-DELTA-0090. Two forks are compared, and the comparison ignores
    /// only what a fork rewrites.
    ///
    /// `conversation_fork` re-mints every event id, sets the fork's own
    /// conversation id, and recomputes `created_at`. Nothing else changes, so
    /// those three are identity and everything else is content. The dangerous
    /// direction is growth: an exclusion set that grew would make more and more
    /// episodes compare equal, and the acceptance condition "two forks start
    /// identical" would go green by ignoring more rather than by matching more.
    #[test]
    fn the_episode_comparison_ignores_only_what_a_fork_rewrites() {
        let path = repository_path(EPISODE_STATE_PATH);
        let source = std::fs::read_to_string(&path)
            .unwrap_or_else(|error| panic!("cannot read {}: {error}", path.display()));
        let declared = source
            .split_once("pub const IDENTITY_FIELDS: &[&str] = &[")
            .and_then(|(_, rest)| rest.split_once("];"))
            .expect("OMEGA-DELTA-0090: IDENTITY_FIELDS is declared")
            .0;
        let fields: Vec<&str> = declared
            .match_indices('"')
            .map(|(offset, _)| offset)
            .collect::<Vec<_>>()
            .chunks_exact(2)
            .map(|pair| &declared[pair[0] + 1..pair[1]])
            .collect();
        assert_eq!(
            fields, EPISODE_IDENTITY_FIELDS,
            "OMEGA-DELTA-0090: the episode comparison ignores {fields:?}. Exactly \
             {EPISODE_IDENTITY_FIELDS:?} are the fields `fork` rewrites; anything \
             more is a real divergence being hidden, and anything less makes every \
             correct fork compare unequal."
        );
        assert!(
            !fields.contains(&"data"),
            "OMEGA-DELTA-0090: the comparison ignores the event payload, so every \
             pair of episodes compares equal and the check tests nothing."
        );
        for content in ["session_id", "turn_id"] {
            assert!(
                !fields.contains(&content),
                "OMEGA-DELTA-0090: the comparison ignores `{content}`, which a fork \
                 copies verbatim. Ignoring it hides a divergence that is real."
            );
        }
    }

    /// OMEGA-DELTA-0090. The loop forks before it mutates, and probes before it
    /// reads the check.
    ///
    /// Both orderings are omega#103's own falsifiers. Fork after the mutation
    /// and the sibling carries it, so the fork point proves nothing. Read the
    /// check outcome before proving the mutation applied and a check that never
    /// ran against anything reports green — which happened here, more than once,
    /// on 2026-07-26.
    #[test]
    fn the_falsification_loop_forks_first_and_probes_before_it_checks() {
        let path = repository_path(EPISODE_RESET_PATH);
        let source = std::fs::read_to_string(&path)
            .unwrap_or_else(|error| panic!("cannot read {}: {error}", path.display()));
        let loop_source = source
            .split_once("pub const FALSIFICATION_LOOP: &[Step] = &[")
            .and_then(|(_, rest)| rest.split_once("];"))
            .expect("OMEGA-DELTA-0090: FALSIFICATION_LOOP is declared")
            .0;
        let steps: Vec<&str> = loop_source
            .lines()
            .filter_map(|line| line.trim().strip_prefix("Step::"))
            .filter_map(|line| line.split(',').next())
            .collect();
        assert!(
            steps.len() >= 10,
            "OMEGA-DELTA-0090: the loop parsed as {steps:?}, so this check is \
             reading nothing."
        );
        let at = |step: &str| {
            steps
                .iter()
                .position(|candidate| *candidate == step)
                .unwrap_or_else(|| {
                    panic!("OMEGA-DELTA-0090: `{step}` is not a step of the loop: {steps:?}")
                })
        };
        assert!(
            at("ForkCandidate") < at("ApplyMutationInCandidate"),
            "OMEGA-DELTA-0090: the loop mutates before it forks, so the sibling \
             carries the mutation and the fork point proves nothing: {steps:?}"
        );
        assert!(
            at("ForkControl") < at("ApplyMutationInCandidate"),
            "OMEGA-DELTA-0090: the control fork is taken after the mutation: {steps:?}"
        );
        assert!(
            at("ProbeMutationApplied") < at("RunNamedCheck"),
            "OMEGA-DELTA-0090: the loop runs the check before it proves the mutation \
             applied. An edit that silently did not apply produces a check that \
             passes while testing nothing: {steps:?}"
        );
        assert!(
            at("CompareStartingStates") < at("ApplyMutationInCandidate"),
            "OMEGA-DELTA-0090: the starting states are compared after something \
             already moved one of them: {steps:?}"
        );
        assert!(
            at("RunNamedCheck") < at("ReadVerdict"),
            "OMEGA-DELTA-0090: the verdict is read before the check runs: {steps:?}"
        );
    }

    /// OMEGA-DELTA-0090. The crate records that Exo's fork does not carry
    /// snapshots.
    ///
    /// omega#103 and the teardown both say fork plus `start_sandbox` is a
    /// complete episode reset needing no upstream change. The conversation half
    /// is; the filesystem half is not, because `fork` copies four prefixes and
    /// `snapshots` is not one of them. If somebody quietly adds `snapshots` to
    /// the copied list here to make a refusal go away, the refusals stop
    /// matching Exo and the episode reports a reset it did not perform.
    #[test]
    fn the_episode_reset_records_that_a_fork_does_not_carry_snapshots() {
        let path = repository_path(EPISODE_RESET_PATH);
        let source = std::fs::read_to_string(&path)
            .unwrap_or_else(|error| panic!("cannot read {}: {error}", path.display()));
        let copied = source
            .split_once("pub const FORK_COPIES_PREFIXES: &[&str] = &[")
            .and_then(|(_, rest)| rest.split_once("];"))
            .expect("OMEGA-DELTA-0090: FORK_COPIES_PREFIXES is declared")
            .0;
        let prefixes: Vec<&str> = copied
            .match_indices('"')
            .map(|(offset, _)| offset)
            .collect::<Vec<_>>()
            .chunks_exact(2)
            .map(|pair| &copied[pair[0] + 1..pair[1]])
            .collect();
        assert_eq!(
            prefixes, EPISODE_FORK_COPIES_PREFIXES,
            "OMEGA-DELTA-0090: the crate says `fork` copies {prefixes:?}. Upstream's \
             `BasicConversationHandle::fork` makes exactly four `copy_prefix` calls, \
             for {EPISODE_FORK_COPIES_PREFIXES:?}."
        );
        assert!(
            !prefixes.contains(&"snapshots"),
            "OMEGA-DELTA-0090: the crate now believes a fork carries snapshots. It \
             does not, at the pinned Exo, and `start_sandbox` inside a fork fails \
             loading the manifest."
        );
        assert!(
            source.contains("SnapshotLostByFork") && source.contains("SiblingsShareOneSandbox"),
            "OMEGA-DELTA-0090: the two reset refusals are gone from {}. Removing them \
             does not make the filesystem reset work; it makes the failure silent.",
            path.display()
        );
    }

    // ------ OMEGA-DELTA-0102 — One enumeration of Exo's protocol, two decisions

    /// Every `Self::Variant => "wire",` pair in a `match`, as it is written.
    ///
    /// Used to read the single enumeration out of its source rather than to
    /// trust a copy of it. Comments are stripped first: a commented-out arm is
    /// exactly how a list stays "complete" across the change that shortens it.
    fn match_arms_to_literals(body: &str) -> Vec<(String, String)> {
        uncommented(body)
            .lines()
            .filter_map(|line| {
                let (left, right) = line.trim().split_once("=>")?;
                let variant = left.trim().strip_prefix("Self::")?.trim();
                let literal = right.trim().strip_prefix('"')?;
                let literal = literal.split('"').next()?;
                Some((variant.to_owned(), literal.to_owned()))
            })
            .collect()
    }

    /// The `ExoRequestKind::Variant` identifiers a decision body names.
    fn kinds_named_in(body: &str) -> Vec<String> {
        let body = uncommented(body);
        let mut named = Vec::new();
        for (offset, _) in body.match_indices("ExoRequestKind::") {
            let rest = &body[offset + "ExoRequestKind::".len()..];
            let end = rest
                .find(|character: char| !character.is_alphanumeric() && character != '_')
                .unwrap_or(rest.len());
            let variant = &rest[..end];
            if !variant.is_empty() && !named.iter().any(|seen| seen == variant) {
                named.push(variant.to_owned());
            }
        }
        named
    }

    /// The enum value a source-level variant identifier stands for.
    ///
    /// Resolved through the crate rather than through a table here, so a
    /// variant this registry can name is a variant that exists.
    fn kind_named(variant: &str) -> Option<omega_exo_lane::ExoRequestKind> {
        omega_exo_lane::ExoRequestKind::ALL
            .into_iter()
            .find(|kind| format!("{kind:?}") == variant)
    }

    fn exo_source(relative: &str) -> String {
        let path = repository_path(relative);
        std::fs::read_to_string(&path)
            .unwrap_or_else(|error| panic!("cannot read {}: {error}", path.display()))
    }

    /// OMEGA-DELTA-0102. Exo's protocol is written down once.
    ///
    /// omega#103 and omega#104 each landed a full, correct, independent
    /// transcription of the same 52 request variants — one to partition them
    /// into families, one to pick eight reads out of them. Both lanes said the
    /// enumeration should live in one place and neither could move it
    /// mid-flight. It lives in `omega_exo_lane::protocol` now, and this is the
    /// check that stops a third copy appearing: no file that *decides* about
    /// the protocol may spell one of its request kinds as a string literal.
    ///
    /// Whole literals, not substrings, for the reason `OMEGA-DELTA-0091` found:
    /// Exo's event tag `conversation_forked` contains its request kind
    /// `conversation_fork`, and a decoder must stay free to name the event.
    #[test]
    fn exos_request_protocol_is_transcribed_in_exactly_one_place() {
        let protocol = exo_source(EXO_PROTOCOL_PATH);
        let wire = function_body(&protocol, "wire").expect(
            "OMEGA-DELTA-0102: the enumeration no longer maps its variants to Exo's \
             wire strings",
        );
        let transcribed = match_arms_to_literals(wire);
        assert_eq!(
            transcribed.len(),
            omega_exo_lane::EXO_REQUEST_KIND_COUNT,
            "OMEGA-DELTA-0102: the enumeration in {EXO_PROTOCOL_PATH} parsed as {} \
             rows and Exo's protocol has {} request types. Either the parse is \
             reading less than the file or the transcription is short.",
            transcribed.len(),
            omega_exo_lane::EXO_REQUEST_KIND_COUNT
        );

        // The parse and the compiled crate agree, so a green run here is not a
        // green run against a file this check misread.
        for (variant, literal) in &transcribed {
            let kind = kind_named(variant).unwrap_or_else(|| {
                panic!(
                    "OMEGA-DELTA-0102: {EXO_PROTOCOL_PATH} names a variant \
                     `{variant}` that is not in ExoRequestKind::ALL, so ALL is \
                     shorter than the enum"
                )
            });
            assert_eq!(
                kind.wire(),
                literal,
                "OMEGA-DELTA-0102: {variant} is written with wire `{literal}` and \
                 compiles to `{}`",
                kind.wire()
            );
        }

        let spellings: Vec<&str> = omega_exo_lane::ExoRequestKind::ALL
            .iter()
            .map(|kind| kind.wire())
            .collect();
        let mut scanned = 0usize;
        for relative in EXO_PROTOCOL_CONSUMER_SOURCES {
            let source = exo_source(relative);
            scanned += 1;
            for literal in string_literals(&source) {
                assert!(
                    !spellings.contains(&literal.as_str()),
                    "OMEGA-DELTA-0102: {relative} spells Exo's `{literal}` request \
                     kind as a string literal. Exo's protocol is enumerated once, \
                     in {EXO_PROTOCOL_PATH}; a second spelling is a second list, \
                     and the whole cost of the first duplication was that the next \
                     upstream variant would have had to be noticed twice."
                );
            }
        }
        assert_eq!(
            scanned,
            EXO_PROTOCOL_CONSUMER_SOURCES.len(),
            "OMEGA-DELTA-0102: the scan read {scanned} files, so it is reading less \
             than the crates that decide about the protocol."
        );
        assert!(
            scanned >= 13,
            "OMEGA-DELTA-0102: {scanned} files is not both Exo law crates. A module \
             added to either must be added to EXO_PROTOCOL_CONSUMER_SOURCES, or it \
             goes unscanned and may start a second list."
        );
    }

    /// OMEGA-DELTA-0102. A 53rd variant cannot pass unclassified, in either
    /// decision.
    ///
    /// Both decisions are `match`es over `ExoRequestKind` with no wildcard arm,
    /// so upstream's next variant is a build failure on the person adding it
    /// rather than a runtime discovery by whoever sent it. Two things are
    /// checked, because either alone is satisfiable while testing nothing: that
    /// each decision *names* all 52 variants, and that neither has a wildcard
    /// that would make naming them optional.
    ///
    /// Comments are stripped before the scan. A commented-out match arm reads
    /// as a named variant to `contains`, which is the shape that produced a
    /// false green in both prior lanes.
    #[test]
    fn both_decisions_are_total_over_the_one_enumeration() {
        let every: Vec<String> = omega_exo_lane::ExoRequestKind::ALL
            .iter()
            .map(|kind| format!("{kind:?}"))
            .collect();
        assert_eq!(every.len(), omega_exo_lane::EXO_REQUEST_KIND_COUNT);

        for (relative, function) in EXO_PROTOCOL_DECISIONS {
            let source = exo_source(relative);
            let body = function_body(&source, function).unwrap_or_else(|| {
                panic!(
                    "OMEGA-DELTA-0102: {relative} no longer has a `{function}` \
                     decision over Exo's protocol"
                )
            });
            let named = kinds_named_in(body);
            for variant in &every {
                assert!(
                    named.iter().any(|seen| seen == variant),
                    "OMEGA-DELTA-0102: {relative}'s `{function}` does not classify \
                     `ExoRequestKind::{variant}`. Every decision over Exo's protocol \
                     is total, so an unclassified variant is a request nobody \
                     decided about."
                );
            }
            for variant in &named {
                assert!(
                    kind_named(variant).is_some(),
                    "OMEGA-DELTA-0102: {relative}'s `{function}` classifies \
                     `ExoRequestKind::{variant}`, which Exo does not have at the pin."
                );
            }
            let stripped = uncommented(body);
            for wildcard in EXO_DECISION_WILDCARD_ARMS {
                assert!(
                    !stripped.contains(wildcard),
                    "OMEGA-DELTA-0102: {relative}'s `{function}` has a `{wildcard}` \
                     arm. A wildcard makes the decision a default: upstream's next \
                     variant compiles, is classified by nobody, and the build stays \
                     green. The totality is the whole mechanism."
                );
            }
        }

        // And the compiled decisions answer for every variant, which is what the
        // source scan above is a proxy for.
        let mut episode_admitted = 0usize;
        let mut log_admitted = 0usize;
        for kind in omega_exo_lane::ExoRequestKind::ALL {
            if omega_exo_episode::is_admitted(kind) {
                episode_admitted += 1;
            }
            if omega_exo_log::is_admitted_read(kind) {
                log_admitted += 1;
            }
        }
        assert_eq!(
            episode_admitted, 20,
            "OMEGA-DELTA-0102: the episode law admits {episode_admitted} of Exo's \
             request types. If that moved, a family was reclassified, and the \
             reclassification is the change worth reviewing."
        );
        assert_eq!(
            log_admitted, 8,
            "OMEGA-DELTA-0102: the read-only client admits {log_admitted} request \
             types. Eight is the closed set omega#104 scoped it to."
        );
    }

    /// OMEGA-DELTA-0102. The two decisions disagree, and are meant to.
    ///
    /// This is the check that stops the duplication being "fixed" the wrong
    /// way. One enumeration, two decision tables — not one decision shared by
    /// two callers. The episode law holds fork and restore authority because
    /// forking *is* its mechanism; the log client is read-only and omega#104
    /// says forking is omega#103's and is scoped there. A merged decision would
    /// give one of the two something nobody granted it.
    ///
    /// The ten reads are the other half of the same argument, from the quieter
    /// direction: the episode law classifies `list_agents` as a query because it
    /// changes nothing, and the log client refuses it because omega#104 scoped
    /// that client to one conversation's own record. Both are right.
    #[test]
    fn the_two_decisions_over_exos_protocol_are_not_merged() {
        use omega_exo_lane::ExoRequestKind;

        for held_only_by_the_episode_law in [
            ExoRequestKind::ConversationFork,
            ExoRequestKind::StartSandbox,
        ] {
            assert!(
                omega_exo_episode::is_admitted(held_only_by_the_episode_law),
                "OMEGA-DELTA-0102: the episode law refuses \
                 `{held_only_by_the_episode_law}`, which is its own mechanism."
            );
            assert!(
                !omega_exo_log::is_admitted_read(held_only_by_the_episode_law),
                "OMEGA-DELTA-0102: the read-only log client admits \
                 `{held_only_by_the_episode_law}`. That is a write, it is \
                 omega#103's authority, and a read-only client holding it is \
                 exactly what merging the two decisions would produce."
            );
        }

        for read_but_host_wide in [
            ExoRequestKind::ListAgents,
            ExoRequestKind::ListConversations,
            ExoRequestKind::ListBindings,
            ExoRequestKind::GetBinding,
            ExoRequestKind::AgentListBindings,
            ExoRequestKind::AgentGetBinding,
            ExoRequestKind::ConversationListBindings,
            ExoRequestKind::ConversationGetBinding,
            ExoRequestKind::GetSandboxProcessEvents,
            ExoRequestKind::WaitSandboxProcess,
        ] {
            assert_eq!(
                omega_exo_episode::family_of(read_but_host_wide),
                omega_exo_episode::RequestFamily::Query,
                "OMEGA-DELTA-0102: `{read_but_host_wide}` stopped being a query in \
                 the episode law's partition."
            );
            assert!(
                !omega_exo_log::is_admitted_read(read_but_host_wide),
                "OMEGA-DELTA-0102: the log client admits `{read_but_host_wide}`. It \
                 reads, and reading is not the boundary: omega#104 scoped that \
                 client to a conversation's own record, and a list of every agent \
                 on the host is not that."
            );
        }

        let episode: Vec<ExoRequestKind> = ExoRequestKind::ALL
            .into_iter()
            .filter(|kind| omega_exo_episode::is_admitted(*kind))
            .collect();
        let log: Vec<ExoRequestKind> = ExoRequestKind::ALL
            .into_iter()
            .filter(|kind| omega_exo_log::is_admitted_read(*kind))
            .collect();
        assert_ne!(
            episode, log,
            "OMEGA-DELTA-0102: the two decisions over Exo's protocol now admit the \
             same set. They are two because they answer different questions; one \
             decision behind both callers is the merge this check exists to refuse."
        );
        for admitted_by_the_reader in &log {
            assert!(
                episode.contains(admitted_by_the_reader),
                "OMEGA-DELTA-0102: the read-only client admits \
                 `{admitted_by_the_reader}` and the episode law does not. The reader \
                 is meant to be the smaller authority of the two."
            );
        }
    }

    // ------ OMEGA-DELTA-0091

    /// OMEGA-DELTA-0091. Every string literal in a source file, outside
    /// comments and outside the test module beside it.
    ///
    /// Exact literals rather than substrings, because Exo's event tags overlap
    /// its request kinds: the event `conversation_forked` contains the request
    /// `conversation_fork`, and a `contains` check would refuse a decoder for
    /// naming an event it must decode. An equality check on whole literals has
    /// no such collision.
    fn string_literals(source: &str) -> Vec<String> {
        let production = production_source(source);
        let characters: Vec<char> = production.chars().collect();
        let mut literals = Vec::new();
        let mut index = 0usize;
        while index < characters.len() {
            match characters[index] {
                '/' if characters.get(index + 1) == Some(&'/') => {
                    while index < characters.len() && characters[index] != '\n' {
                        index += 1;
                    }
                }
                '/' if characters.get(index + 1) == Some(&'*') => {
                    index += 2;
                    while index + 1 < characters.len()
                        && !(characters[index] == '*' && characters[index + 1] == '/')
                    {
                        index += 1;
                    }
                    index += 2;
                }
                '"' => {
                    index += 1;
                    let mut literal = String::new();
                    while index < characters.len() && characters[index] != '"' {
                        if characters[index] == '\\' {
                            index += 1;
                        }
                        if index < characters.len() {
                            literal.push(characters[index]);
                            index += 1;
                        }
                    }
                    index += 1;
                    literals.push(literal);
                }
                _ => index += 1,
            }
        }
        literals
    }

    fn exo_log_source(relative: &str) -> String {
        let path = repository_path(relative);
        std::fs::read_to_string(&path)
            .unwrap_or_else(|error| panic!("cannot read {}: {error}", path.display()))
    }

    /// OMEGA-DELTA-0091. The client cannot name a request it may not send.
    ///
    /// The issue asked for a shape where the wrong call cannot be expressed
    /// rather than one that refuses at runtime, so this reads the whole crate
    /// for the forty-four unadmitted kinds and for the eight admitted ones. A
    /// denylist consulted at a call site would pass this check while a caller
    /// that forgot to consult it sent a fork; a closed enum that never spells
    /// the kind cannot.
    #[test]
    fn the_exo_log_client_can_name_only_the_eight_read_variants() {
        // `OMEGA-DELTA-0102`. The unadmitted forty-four are derived from the one
        // enumeration and the crate's own decision, not transcribed here. This
        // registry keeps only its independent statement of the admitted eight,
        // and holds the crate to it.
        let admitted: Vec<&str> = omega_exo_log::exo_admitted_read_kinds();
        let unadmitted: Vec<&'static str> = omega_exo_log::unadmitted_kinds()
            .into_iter()
            .map(omega_exo_lane::ExoRequestKind::wire)
            .collect();
        assert_eq!(
            admitted.len() + unadmitted.len(),
            omega_exo_lane::EXO_REQUEST_KIND_COUNT,
            "OMEGA-DELTA-0091: the two halves no longer partition Exo's \
             protocol, so one of them stopped being a partition of anything."
        );
        let mut declared: Vec<&str> = EXO_LOG_ADMITTED_KINDS.to_vec();
        declared.sort_unstable();
        let mut answered: Vec<&str> = admitted.clone();
        answered.sort_unstable();
        assert_eq!(
            answered, declared,
            "OMEGA-DELTA-0091: the crate admits a different eight than the \
             registry does. One of the two changed without the other, and the \
             registry is where the decision is supposed to be legible."
        );
        for kind in &admitted {
            assert!(
                !unadmitted.contains(kind),
                "OMEGA-DELTA-0091: `{kind}` is on both sides of the partition."
            );
        }

        for relative in EXO_LOG_SOURCE_PATHS {
            let source = exo_log_source(relative);
            for literal in string_literals(&source) {
                if let Some(kind) = unadmitted.iter().find(|kind| **kind == literal) {
                    panic!(
                        "OMEGA-DELTA-0091: {relative} names Exo's `{kind}` \
                         request. This crate is read-only and reaches eight \
                         query variants; a ninth is a policy change, not a \
                         convenience."
                    );
                }
            }
        }

        // The positive half is read off the closed type and nowhere else. An
        // earlier version of this check scanned the whole crate for the eight
        // admitted kinds, which the crate's own published table satisfied on its
        // own — so a variant could stop sending a read and the check would still
        // pass, reading the list rather than the code.
        //
        // `OMEGA-DELTA-0102`: the variant-to-kind map names enum variants now,
        // so this reads identifiers and resolves them through the crate. Whole
        // identifiers, for the same reason whole literals mattered before —
        // `ConversationGetEvent` is a prefix of `ConversationGetEvents`, and
        // `kinds_named_in` stops at the end of the identifier rather than at a
        // substring match.
        let query = exo_log_source("crates/omega_exo_log/src/query.rs");
        let body = function_body(&query, "kind")
            .expect("OMEGA-DELTA-0091: the query type no longer maps variants to request kinds");
        let mut sent: Vec<&str> = kinds_named_in(body)
            .iter()
            .map(|variant| {
                kind_named(variant)
                    .unwrap_or_else(|| {
                        panic!("OMEGA-DELTA-0091: `{variant}` is not a request type Exo has")
                    })
                    .wire()
            })
            .collect();
        sent.sort_unstable();
        sent.dedup();
        assert_eq!(
            sent, declared,
            "OMEGA-DELTA-0091: the request kinds the query type can produce are \
             no longer exactly the eight admitted reads. A kind that vanished is \
             a read the workspace can no longer perform; a kind that appeared is \
             a capability nobody granted."
        );

        // And the decision beside it agrees, in the crate that publishes it.
        let admission = exo_log_source(EXO_LOG_ADMISSION_PATH);
        let decision = function_body(&admission, "is_admitted_read")
            .expect("OMEGA-DELTA-0091: the crate no longer decides its own admitted set");
        let admits: Vec<&str> = decision
            .split("=> true")
            .next()
            .map(|arm| {
                kinds_named_in(arm)
                    .iter()
                    .filter_map(|variant| kind_named(variant))
                    .map(omega_exo_lane::ExoRequestKind::wire)
                    .collect()
            })
            .unwrap_or_default();
        let mut published = admits;
        published.sort_unstable();
        published.dedup();
        assert_eq!(
            published, declared,
            "OMEGA-DELTA-0091: the crate's own admission decision and the \
             registry disagree."
        );
    }

    /// OMEGA-DELTA-0091. Exo stays on this machine, and the refusal is a
    /// sentence.
    ///
    /// Two checks in one because they fail together. `exo serve` has no
    /// authentication at all, so the address is the boundary — and `localhost`
    /// is a *name*, which means the parse is not the end of it. The resolved
    /// socket address is checked again before the connection opens, so a hosts
    /// file that points `localhost` at a Tailnet address gets a refusal rather
    /// than a connection.
    #[test]
    fn the_exo_log_client_reaches_exo_only_on_this_machine() {
        let client = exo_log_source("crates/omega_exo_log/src/client.rs");

        let open = function_body(&client, "open")
            .expect("OMEGA-DELTA-0091: the client no longer has one construction path");
        assert!(
            open.contains("LoopbackEndpoint::parse(address)"),
            "OMEGA-DELTA-0091: the client no longer parses *its own argument* \
             through the type that refuses a non-loopback one. An earlier \
             version of this check asked only that the call appear somewhere in \
             the constructor, which a constructor that parsed a hardcoded \
             loopback string and ignored `address` satisfies."
        );
        // `-> Self {` is a return type followed by a body brace, not a struct
        // literal, and the builders that adjust a timeout return one.
        let production = production_source(&client);
        let built = production.match_indices("Self {").count()
            - production.match_indices("-> Self {").count();
        assert_eq!(
            built, 1,
            "OMEGA-DELTA-0091: the client is constructed in {built} places. One \
             checked constructor is the whole argument for the type; a second \
             is a way to hold an address nobody parsed."
        );

        let resolve = function_body(&client, "resolve").expect(
            "OMEGA-DELTA-0091: the client no longer resolves before it connects. \
             `localhost` is a name, and a name is not a guarantee.",
        );
        assert!(
            resolve.contains("is_loopback"),
            "OMEGA-DELTA-0091: the resolved address is no longer checked. A \
             hosts file is enough to move `localhost` off this machine."
        );

        let post = function_body(&client, "post")
            .expect("OMEGA-DELTA-0091: the client no longer has one send path");
        let resolved_at = post
            .find("self.resolve()")
            .expect("OMEGA-DELTA-0091: the send path no longer resolves through the check");
        let connected_at = post
            .find("TcpStream::connect")
            .expect("OMEGA-DELTA-0091: the send path no longer opens a connection");
        assert!(
            resolved_at < connected_at,
            "OMEGA-DELTA-0091: the client connects before it checks where it is \
             connecting."
        );

        for relative in EXO_LOG_SOURCE_PATHS {
            let source = exo_log_source(relative);
            for token in EXO_LOG_FALSE_AUTH_TOKENS {
                assert!(
                    !named_in_code(&source, token),
                    "OMEGA-DELTA-0091: {relative} names `{token}` in code. Exo \
                     accepts a bearer token and never checks it, so sending one \
                     asserts a protection that does not exist."
                );
            }
            for (what, token) in EXO_OFF_MACHINE_TOKENS {
                assert!(
                    !named_in_code(&source, token),
                    "OMEGA-DELTA-0091: {relative} names {what} (`{token}`) in \
                     code. Omega must never carry Exo's unauthenticated \
                     endpoint off this machine."
                );
            }
        }
    }

    /// OMEGA-DELTA-0091. What Exo says a turn cost is what Exo says.
    ///
    /// Exo never makes the model call through an attested path; its own cost
    /// design document calls the numbers "agent-reported telemetry, not an
    /// attested ledger". So the type is named for its provenance, the rendering
    /// says it, and there is no conversion out of it — a `From` impl is exactly
    /// how a harness number reaches a ledger without anybody deciding it should.
    #[test]
    fn exo_reported_usage_is_never_accounting_truth() {
        let record = exo_log_source("crates/omega_exo_log/src/record.rs");
        assert!(
            record.contains(&format!("pub struct {EXO_LOG_USAGE_TYPE}")),
            "OMEGA-DELTA-0091: Exo's usage record is no longer typed as \
             `{EXO_LOG_USAGE_TYPE}`. A name that does not say `harness` is a \
             number that reads as a measurement."
        );
        assert!(
            record.contains(&format!("\"{EXO_LOG_USAGE_PROVENANCE}\"")),
            "OMEGA-DELTA-0091: the usage record no longer carries its \
             provenance string."
        );
        for relative in EXO_LOG_SOURCE_PATHS {
            let source = exo_log_source(relative);
            assert!(
                !source.contains(&format!("impl From<{EXO_LOG_USAGE_TYPE}>")),
                "OMEGA-DELTA-0091: {relative} converts {EXO_LOG_USAGE_TYPE} into \
                 another type. Harness telemetry must not become an Omega usage, \
                 cost, or credit value by a conversion nobody had to ask for."
            );
        }

        let history = exo_log_source("crates/omega_exo_log/src/history.rs");
        let render = function_body(&history, "to_text")
            .expect("OMEGA-DELTA-0091: the history no longer renders");
        assert!(
            render.contains(&format!("{EXO_LOG_USAGE_TYPE}::PROVENANCE")),
            "OMEGA-DELTA-0091: the rendering prints usage without its \
             provenance, so a token count sits beside a message reading as a \
             measurement Omega made."
        );
    }

    /// OMEGA-DELTA-0091. A history missing its artifacts says so.
    ///
    /// The falsifier for this issue: Exo's event log names artifacts and never
    /// contains them, so a history built without artifact reads keeps every
    /// name and loses every body. The render must make that visible rather than
    /// producing an empty tool result, which is indistinguishable from a tool
    /// that returned nothing.
    #[test]
    fn an_exo_history_without_its_artifacts_says_what_is_missing() {
        let history = exo_log_source("crates/omega_exo_log/src/history.rs");
        for required in ["NotRead", "unread_artifact_rows", "unresolved_artifact_ids"] {
            assert!(
                history.contains(required),
                "OMEGA-DELTA-0091: the history no longer declares `{required}`. \
                 Without it an unread artifact renders as a tool result with no \
                 body, which reads as a tool that returned nothing."
            );
        }
        let resolve = function_body(&history, "resolve")
            .expect("OMEGA-DELTA-0091: the history no longer resolves artifact references");
        assert!(
            resolve.contains("Self::NotRead"),
            "OMEGA-DELTA-0091: an artifact that was not read no longer produces \
             the variant that says so."
        );
        assert!(
            history.contains("fn without_the_artifact_read_the_history_loses_its_tool_results"),
            "OMEGA-DELTA-0091: the falsifier — remove the artifact read, and the \
             tool results must go with it — is no longer run as a test."
        );
    }

    /// OMEGA-DELTA-0061. A named executor is honoured or refused by name.
    ///
    /// This is the whole risk of per-spawn executors. The parent asks for
    /// Codex; it must get Codex or an error saying it could not. What must
    /// never happen is the third thing — running on the parent's own model and
    /// reporting success — because the parent then believes an independent
    /// agent looked at the problem when the same agent looked at it twice.
    /// That is the same defect class as an undisclosed provider handoff, and it
    /// is invisible at exactly the moment it matters.
    ///
    /// So the resolver has two outcomes and no fallthrough, and the silence
    /// case is pinned in both directions: omitting the field must still inherit
    /// the parent, or every existing spawn changes behaviour.
    #[test]
    fn a_named_executor_is_honoured_or_refused_by_name() {
        let path = repository_path(SUBAGENT_EXECUTOR_PATH);
        let source = std::fs::read_to_string(&path)
            .unwrap_or_else(|error| panic!("cannot read {}: {error}", path.display()));
        let compact = without_whitespace(&source);

        for outcome in SUBAGENT_EXECUTOR_OUTCOMES {
            assert!(
                source.contains(&format!("ExecutorResolution::{outcome}")),
                "OMEGA-DELTA-0061: {} no longer names the `{outcome}` outcome. \
                 Resolving an executor has exactly two answers; a third would \
                 be the silent fallback this delta exists to prevent.",
                path.display()
            );
        }

        // Refusal carries a sentence, not a unit. A refusal the model cannot
        // read is a failure it cannot act on.
        assert!(
            compact.contains(&without_whitespace("Refused(String)")),
            "OMEGA-DELTA-0061: {} no longer carries a reason on refusal.",
            path.display()
        );

        // The two refusal paths must name what was asked for, and the
        // not-installed one must say it did not fall back.
        assert!(
            source.contains("Cannot spawn a `{}` subagent")
                && source.contains("Cannot spawn a `{requested}` subagent"),
            "OMEGA-DELTA-0061: {} no longer names the requested executor when \
             it refuses. \"That agent is unavailable\" does not tell the parent \
             which of its three subagents failed.",
            path.display()
        );
        assert!(
            source.contains("will not silently run this on the parent's own model"),
            "OMEGA-DELTA-0061: {} no longer states that it refused rather than \
             falling back. The promise not to substitute is the delta.",
            path.display()
        );

        // An unrecognised name refuses. Reading "or a model for the native
        // loop" as "anything unknown is a model" makes the typo `codex-acpp` a
        // silent inherit.
        assert!(
            source.contains("not an executor Omega knows"),
            "OMEGA-DELTA-0061: {} no longer refuses unrecognised executors. An \
             unknown name must fail, not be guessed as a model.",
            path.display()
        );
        assert!(
            !compact.contains(&without_whitespace("_ => ExecutorResolution::Resolved")),
            "OMEGA-DELTA-0061: {} resolves from a catch-all arm. Every \
             resolution must name the case it is resolving.",
            path.display()
        );

        // Every path that inherits the parent, counted.
        //
        // Asserting that the refusal *strings* exist proves nothing about which
        // branch runs: a fallback added above them leaves every string in place
        // and the file still reads correctly. What cannot be faked is the
        // number of ways the function can answer "inherit". There are exactly
        // two legitimate ones — the field was omitted, and the field was blank
        // — and a third is a request for a named executor being answered with
        // something else.
        let production = source
            .split("#[cfg(test)]")
            .next()
            .expect("the file must have production code");
        let inherit_paths = production
            .matches("ExecutorResolution::Resolved(SubagentExecutor::InheritParent)")
            .count();
        assert_eq!(
            inherit_paths,
            2,
            "OMEGA-DELTA-0061: {} has {inherit_paths} paths that inherit the \
             parent, not 2. Inheriting is only ever correct when nothing was \
             asked for (omitted or blank). A third path means a request for a \
             named executor is being answered with the parent's own model, \
             which is the substitution this delta forbids.",
            path.display()
        );

        // Silence still means today's behaviour.
        assert!(
            compact.contains(&without_whitespace(
                "let Some(requested) = requested else {
                    return ExecutorResolution::Resolved(SubagentExecutor::InheritParent);"
            )),
            "OMEGA-DELTA-0061: {} no longer inherits the parent when the field \
             is omitted. Every spawn that does not ask for an executor must \
             behave exactly as it did before this delta.",
            path.display()
        );

        // Resuming cannot quietly drop a requested executor either — the same
        // fallback arriving by a different door.
        let tool_path = repository_path(SUBAGENT_SPAWN_TOOL_PATH);
        let tool = std::fs::read_to_string(&tool_path)
            .unwrap_or_else(|error| panic!("cannot read {}: {error}", tool_path.display()));
        assert!(
            tool.contains("Cannot set `executor` when continuing an existing"),
            "OMEGA-DELTA-0061: {} accepts `executor` alongside `session_id` \
             without refusing. A resumed session runs on whatever created it, \
             so honouring the request is impossible and accepting it silently \
             drops it.",
            tool_path.display()
        );
    }

    /// OMEGA-DELTA-0061. Only agents actually installed here can be asked for.
    ///
    /// `AllAgentServersSettings` records what is *configured*, which is not
    /// evidence of presence — a fresh `--user-data-dir` has no settings written
    /// whatever is on disk, so a settings-based check offers nothing on exactly
    /// the machine a new person is using, and offers a missing agent on one
    /// where the settings outlived the binary. Presence comes from the `PATH`
    /// probe in `omega_agent_detect`.
    ///
    /// Presence is also decided *once*. Re-checking it at connect time from a
    /// different source is how two answers to "is Codex installed" get to
    /// disagree.
    #[test]
    fn only_detected_agents_are_offered() {
        let path = repository_path(SUBAGENT_EXECUTOR_PATH);
        let source = std::fs::read_to_string(&path)
            .unwrap_or_else(|error| panic!("cannot read {}: {error}", path.display()));

        assert!(
            source.contains("omega_agent_detect::detected()"),
            "OMEGA-DELTA-0061: {} no longer takes the installed set from \
             `omega_agent_detect`. Presence must come from the PATH probe.",
            path.display()
        );
        // Checked against code only. The doc comments in that file *name*
        // `AllAgentServersSettings` to explain why it is the wrong source, and
        // a check that cannot tell an explanation from a use would forbid
        // writing the reason down.
        let code_only: String = source
            .lines()
            .filter(|line| !line.trim_start().starts_with("//"))
            .collect::<Vec<_>>()
            .join("\n");
        assert!(
            !code_only.contains("AllAgentServersSettings"),
            "OMEGA-DELTA-0061: {} decides availability from agent-server \
             settings. Settings record configuration, not presence.",
            path.display()
        );

        // Known and installed are separate inputs, so "not installed here" and
        // "no such agent" stay different sentences.
        assert!(
            without_whitespace(&source).contains(&without_whitespace(
                "pub fn resolve_subagent_executor(
                    requested: Option<&str>,
                    known: &[InstalledAgent],
                    installed: &[InstalledAgent],
                ) -> ExecutorResolution"
            )),
            "OMEGA-DELTA-0061: the resolver in {} changed shape. It takes the \
             known set and the installed set separately so that a real agent \
             that is merely absent gets a different sentence from a name that \
             does not exist.",
            path.display()
        );

        let handle_path = repository_path(SUBAGENT_EXTERNAL_HANDLE_PATH);
        let handle = std::fs::read_to_string(&handle_path)
            .unwrap_or_else(|error| panic!("cannot read {}: {error}", handle_path.display()));
        assert!(
            handle.contains("Presence is *not* rechecked here."),
            "OMEGA-DELTA-0061: {} no longer records that presence was already \
             decided. A second presence check from another source is a second \
             answer to the same question.",
            handle_path.display()
        );
    }

    /// OMEGA-DELTA-0061. Every subagent result names the executor that produced it.
    ///
    /// A mixed fan-out the parent cannot attribute is not finished. Three
    /// results come back from one turn; if they are anonymous, the parent
    /// cannot tell Codex's answer from Claude's from its own, and the entire
    /// reason for spawning different executors is gone.
    ///
    /// The label is asked of the **handle**, not of the request. A label taken
    /// from what was asked for would still read "Codex" on a subagent that
    /// silently ran as something else — it would report the intention rather
    /// than the fact, which is precisely the failure the other check forbids.
    #[test]
    fn every_subagent_result_names_its_executor() {
        let registration_path = repository_path(SUBAGENT_TRANSCRIPT_REGISTRATION_PATH);
        let registration = std::fs::read_to_string(&registration_path)
            .unwrap_or_else(|error| panic!("cannot read {}: {error}", registration_path.display()));
        assert!(
            registration.contains("fn executor_label(&self) -> String;"),
            "OMEGA-DELTA-0061: {} no longer requires a subagent handle to say \
             what ran it. Without it a mixed fan-out is three anonymous \
             answers.",
            registration_path.display()
        );

        let tool_path = repository_path(SUBAGENT_SPAWN_TOOL_PATH);
        let tool = std::fs::read_to_string(&tool_path)
            .unwrap_or_else(|error| panic!("cannot read {}: {error}", tool_path.display()));

        // Taken from the handle, so it reports the fact and not the request.
        assert!(
            tool.contains("subagent.executor_label()"),
            "OMEGA-DELTA-0061: {} no longer asks the handle what ran the \
             subagent. A label derived from the request reports the intention, \
             not what happened.",
            tool_path.display()
        );
        // And it reaches the model, which is the only reader that can act on it.
        // Both arms. A success that is attributed and a failure that is not
        // leaves the parent unable to tell which of three subagents died.
        let attributed = tool.matches(r#""executor": executor"#).count();
        assert_eq!(
            attributed,
            2,
            "OMEGA-DELTA-0061: {} puts the executor in {attributed} of the 2 \
             results the model reads. Attribution the parent cannot see is not \
             attribution, and an unattributed *failure* is the case that \
             matters most in a mixed fan-out.",
            tool_path.display()
        );

        let handle_path = repository_path(SUBAGENT_EXTERNAL_HANDLE_PATH);
        let handle = std::fs::read_to_string(&handle_path)
            .unwrap_or_else(|error| panic!("cannot read {}: {error}", handle_path.display()));
        // The two handles must not report the same thing, or attribution is
        // uniform and therefore useless.
        assert!(
            handle.contains(r#""Omega (native loop, inherited from parent)""#),
            "OMEGA-DELTA-0061: {} no longer distinguishes an inherited subagent \
             in its own label.",
            handle_path.display()
        );
        // Whitespace-insensitive: `cargo fmt` wraps this `format!`, and a check
        // pinned to the single-line form would fail on formatting rather than
        // on behaviour.
        assert!(
            without_whitespace(&handle).contains(&without_whitespace(
                r#"format!("{} ({}, external ACP agent)", self.agent_name, self.agent_id)"#
            )),
            "OMEGA-DELTA-0061: {} no longer names the external agent in its own \
             label. Two subagents given different executors must not report the \
             same thing.",
            handle_path.display()
        );
    }
    // ------------------------------------------------------ OMEGA-DELTA-0094

    /// OMEGA-DELTA-0094. Local is reachable with no account, no relay and no
    /// network.
    ///
    /// The property omega#107 puts above everything else, and the one omega#108
    /// is forbidden from weakening. It is checked on the dependency graph
    /// rather than on the code, because that is where it is decidable: a crate
    /// that cannot reach a socket cannot be made to reach one by a later edit
    /// that reads as a small convenience.
    ///
    /// A closed list, not a denylist. See `AUDIENCE_ALLOWED_DEPENDENCIES`.
    #[test]
    fn local_needs_no_network_no_relay_and_no_account() {
        let path = repository_path(AUDIENCE_MANIFEST_PATH);
        let manifest = std::fs::read_to_string(&path)
            .unwrap_or_else(|error| panic!("cannot read {}: {error}", path.display()));

        let dependencies = manifest
            .split_once("\n[dependencies]")
            .map(|(_, rest)| rest.split("\n[").next().unwrap_or_default())
            .unwrap_or_else(|| {
                panic!(
                    "OMEGA-DELTA-0094: {} declares no `[dependencies]` section, so \
                     this check cannot see what Local depends on.",
                    path.display()
                )
            });

        let declared: Vec<String> = dependencies
            .lines()
            .map(str::trim)
            .filter(|line| !line.is_empty() && !line.starts_with('#'))
            .map(|line| {
                line.split(['=', '.'])
                    .next()
                    .unwrap_or_default()
                    .trim()
                    .to_string()
            })
            .collect();

        for dependency in &declared {
            assert!(
                AUDIENCE_ALLOWED_DEPENDENCIES.contains(&dependency.as_str()),
                "OMEGA-DELTA-0094: `omega_audience` now depends on `{dependency}`. \
                 Local is the audience that works when everything else is down, \
                 and it is that only for as long as the crate holding its rules \
                 cannot reach an account, a relay, or a socket. Admitted: \
                 {AUDIENCE_ALLOWED_DEPENDENCIES:?}."
            );
        }

        // A local thread costs no durable write either. `record_thread_opening`
        // returns before it reaches the store, because an absent record already
        // means Local. Without this the path that has to keep working when
        // everything else is down would carry a write per thread, and adding
        // one is what broke `test_select_agent_action_updates_visible_draft`.
        let control = read_repository_file(AUDIENCE_CONTROL_PATH);
        let record = body_of(&control, "record_thread_opening");
        assert!(
            record.contains("if audience.is_local() {"),
            "OMEGA-DELTA-0094: {} now writes a durable record for a local \
             thread. An absent record already means Local, so the row changes \
             no answer and costs the one path that must never need anything.",
            repository_path(AUDIENCE_CONTROL_PATH).display()
        );

        let source = read_repository_file(AUDIENCE_PATH);
        assert!(
            source.contains("pub fn local() -> Self"),
            "OMEGA-DELTA-0094: {} must construct the local audience. A local \
             audience that is configured rather than constructed can be \
             configured into a shared one.",
            repository_path(AUDIENCE_PATH).display()
        );
        assert!(
            source.contains("reach: Reach::ThisComputer,"),
            "OMEGA-DELTA-0094: the local audience no longer declares \
             `Reach::ThisComputer` in {}.",
            repository_path(AUDIENCE_PATH).display()
        );
        assert!(
            source.contains("pub const fn is_empty(&self) -> bool {")
                && source.contains("        false"),
            "OMEGA-DELTA-0094: `AudienceRoster::is_empty` in {} is no longer \
             constantly false. omega#107 acceptance 4 is that a profile which \
             has joined nothing still sees Local and does not read as broken, \
             and a roster that can report itself empty invites a caller to draw \
             nothing.",
            repository_path(AUDIENCE_PATH).display()
        );
    }

    /// OMEGA-DELTA-0094. The composer reads the thread, never the selection.
    ///
    /// omega#107's first falsifier, mechanically: "remove the workspace from
    /// the thread record and infer it from the current selection". The label a
    /// person reads is produced by one function, and that function is held to
    /// taking a thread and consulting no selection. `AudienceBook::audience_of`
    /// has no selection parameter at all, so the only way to reintroduce the
    /// defect is to call the selection here — which is what this reads for.
    #[test]
    fn the_composer_reads_the_audience_from_the_thread() {
        let path = repository_path(AUDIENCE_CONTROL_PATH);
        let source = read_repository_file(AUDIENCE_CONTROL_PATH);

        assert!(
            source.contains("pub fn thread_audience_label(thread_id: ThreadId, cx: &mut App)"),
            "OMEGA-DELTA-0094: {} must produce the composer's label from a \
             thread identity. A label function that takes a selection can only \
             be correct by accident.",
            path.display()
        );

        let body = body_of(&source, "thread_audience_label");
        assert!(
            body.contains("thread_audience(thread_id, cx)"),
            "OMEGA-DELTA-0094: the composer's label in {} is no longer read \
             from the thread's own audience.",
            path.display()
        );
        assert!(
            !body.contains("selected"),
            "OMEGA-DELTA-0094: the composer's label in {} consults the \
             selection. This is omega#107's falsifier: switching audience then \
             repaints every thread already on screen, so a private \
             conversation from last week renders as a public one, nothing is \
             published, and the person has no way to find out.",
            path.display()
        );

        let rules = read_repository_file(AUDIENCE_PATH);
        let audience_of = body_of(&rules, "audience_of");
        assert!(
            audience_of.contains("unwrap_or_else(AudienceId::local)"),
            "OMEGA-DELTA-0094: a thread with no recorded audience no longer \
             resolves to Local in {}. The threads with no record are the ones \
             written before this existed, which are the ones somebody held in \
             private.",
            repository_path(AUDIENCE_PATH).display()
        );
    }

    /// OMEGA-DELTA-0094. The current audience is on the face of the control.
    ///
    /// omega#107's third falsifier is "drop the visible indicator: it must
    /// become impossible to tell a private thread from a public one without
    /// opening a menu". So the control is required in the composer row a person
    /// is already looking at, and it is required to be the composer row rather
    /// than a settings page — the owner has rejected a modal setup screen
    /// repeatedly.
    #[test]
    fn the_composer_shows_the_audience_without_opening_a_menu() {
        let path = repository_path(THREAD_VIEW_PATH);
        let source = read_repository_file(THREAD_VIEW_PATH);

        let bar = body_of(&source, "render_zero_base_executor_bar");
        assert!(
            bar.contains("omega_audience_control::render_audience_control("),
            "OMEGA-DELTA-0094: zero base's composer bar in {} no longer draws \
             the audience control. It is the row a person reads before typing, \
             and without it there is no way to tell a private thread from a \
             public one without opening something.",
            path.display()
        );
        assert!(
            bar.contains("self.root_thread_id"),
            "OMEGA-DELTA-0094: the composer bar in {} no longer passes the \
             thread's own identity to the audience control.",
            path.display()
        );

        let control = read_repository_file(AUDIENCE_CONTROL_PATH);
        let render = body_of(&control, "render_audience_control");
        assert!(
            render.contains("thread_audience_label(thread_id, cx)"),
            "OMEGA-DELTA-0094: the control in {} no longer puts the audience on \
             its own face. A control that only names the audience once its menu \
             is open is the indicator omega#107 forbids dropping.",
            repository_path(AUDIENCE_CONTROL_PATH).display()
        );
        assert!(
            render.contains("Button::new(\"omega-audience-selector\", label)"),
            "OMEGA-DELTA-0094: the control in {} no longer labels its trigger \
             with the audience.",
            repository_path(AUDIENCE_CONTROL_PATH).display()
        );
    }

    /// OMEGA-DELTA-0094. Switching does not move a thread that already exists.
    ///
    /// Two halves, because either alone is enough to lose the property. The
    /// rule refuses a rebinding, and the only caller returns before it would
    /// attempt one — a caller that called `bind` and discarded the `Result`
    /// would satisfy the first half while doing exactly what it forbids.
    #[test]
    fn a_thread_keeps_the_audience_it_was_started_in() {
        let rules = read_repository_file(AUDIENCE_PATH);
        let bind = body_of(&rules, "bind");
        assert!(
            bind.contains("Err(RebindRefused {"),
            "OMEGA-DELTA-0094: `AudienceBook::bind` in {} no longer refuses to \
             move a bound thread. omega#107 deliverable 5 is that a thread keeps \
             the audience it was started in.",
            repository_path(AUDIENCE_PATH).display()
        );

        let control = read_repository_file(AUDIENCE_CONTROL_PATH);
        let record = body_of(&control, "record_thread_opening");
        assert!(
            record.contains("if loaded.book.recorded(&key).is_some() {")
                && record.contains("return;"),
            "OMEGA-DELTA-0094: `record_thread_opening` in {} no longer returns \
             early on a thread that already has an audience. Calling `bind` and \
             discarding the refusal would leave the rule green while the caller \
             tried to move threads on every open.",
            repository_path(AUDIENCE_CONTROL_PATH).display()
        );

        let select = body_of(&control, "select_audience");
        assert!(
            !select.contains("thread_id") && !select.contains("book"),
            "OMEGA-DELTA-0094: `select_audience` in {} now touches a thread. \
             Choosing an audience is for the next thread; a chooser that can \
             reach the record is one edit away from rewriting an old thread's \
             audience.",
            repository_path(AUDIENCE_CONTROL_PATH).display()
        );
    }

    /// OMEGA-DELTA-0094. The audience is recorded where a thread starts.
    ///
    /// The distinction between a thread that did not exist a moment ago and one
    /// being opened again is only available at construction, where
    /// `resume_session_id` and `thread_id` are both `Option`. The draw-time
    /// substitute is `AcpThread::is_draft_thread`, which is
    /// `entries().is_empty()` and therefore also true of a resumed thread whose
    /// entries have not loaded — so a slow disk would hand a community audience
    /// to somebody's old private conversation. This pins the call to the
    /// constructor and pins the two signals it reads.
    #[test]
    fn the_audience_is_recorded_where_a_thread_starts() {
        let path = repository_path(CONVERSATION_VIEW_PATH);
        let source = read_repository_file(CONVERSATION_VIEW_PATH);

        assert!(
            source.contains("omega_audience_control::record_thread_opening("),
            "OMEGA-DELTA-0094: {} no longer records a thread's audience when \
             the thread is created. Without it every thread is unrecorded, \
             every thread reads as Local, and selecting an audience does \
             nothing at all.",
            path.display()
        );
        assert!(
            source.contains("omega_audience::ThreadOpening::Resumed")
                && source.contains("omega_audience::ThreadOpening::Started"),
            "OMEGA-DELTA-0094: {} no longer distinguishes a started thread from \
             a resumed one. Treating a resumed thread as started makes opening \
             last month's private conversation adopt today's selection.",
            path.display()
        );
        assert!(
            source.contains("reattached_to_a_persisted_record || resume_session_id.is_some()"),
            "OMEGA-DELTA-0094: {} no longer reads both signals for a resumed \
             thread. A thread reattached to a persisted record and a thread \
             resuming a session are both threads that already existed.",
            path.display()
        );

        let control = read_repository_file(AUDIENCE_CONTROL_PATH);
        assert!(
            !control.contains(".is_draft_thread()"),
            "OMEGA-DELTA-0094: {} decides an audience from `is_draft_thread`, \
             which is `entries().is_empty()`. That is also true of a resumed \
             thread whose entries have not loaded yet.",
            repository_path(AUDIENCE_CONTROL_PATH).display()
        );
    }

    /// [`function_body`], but a miss is a failure rather than an empty string.
    ///
    /// OMEGA-DELTA-0090's rule, applied to the checks themselves: a check that
    /// cannot find its subject must fail. Returning `unwrap_or_default` on a
    /// renamed function turns every assertion about its body into an assertion
    /// about nothing, and it turns green.
    fn body_of<'a>(source: &'a str, name: &str) -> &'a str {
        function_body(source, name).unwrap_or_else(|| {
            panic!(
                "OMEGA-DELTA-0094: `fn {name}` is gone. A check that cannot find \
                 what it is about must fail rather than pass."
            )
        })
    }

    /// Read a repository source file, or say which one could not be read.
    fn read_repository_file(relative: &str) -> String {
        let path = repository_path(relative);
        std::fs::read_to_string(&path)
            .unwrap_or_else(|error| panic!("cannot read {}: {error}", path.display()))
    }

    /// OMEGA-DELTA-0095. The coding agent that is installed runs the turn.
    ///
    /// omega#106. Detection, the onboarding grid, ACP hosting for `codex-acp`
    /// and `claude-acp`, and auto-routing to the attached external agent all
    /// existed already. Nothing attached one, so a machine with Codex on
    /// `PATH` ran every turn on the native loop and the disclosure named
    /// `native_loop`. This is the same failure shape as `OMEGA-DELTA-0035` and
    /// `OMEGA-DELTA-0042`: a complete mechanism reachable by nobody, which
    /// compiles, passes its own tests, and does nothing.
    ///
    /// The four parts checked here are the four ways it can revert to that:
    /// the attach can stop being called, the choice can stop being made from
    /// presence, the failure can start degrading to the native loop, or the
    /// ACP servers can stop being configured so nothing can be hosted.
    #[test]
    fn the_installed_coding_agent_is_attached_as_the_thread_executor() {
        let attach_path = repository_path(DETECTED_EXECUTOR_ATTACH_PATH);
        let attach = std::fs::read_to_string(&attach_path)
            .unwrap_or_else(|error| panic!("cannot read {}: {error}", attach_path.display()));
        // The tests in that file name several of these tokens while showing
        // why the production path does not use them, so the scan reads the
        // production half only.
        let production = code_of(
            attach
                .split("#[cfg(test)]")
                .next()
                .expect("splitting always yields a first part"),
        );
        let compact = without_whitespace(&production);

        for step in DETECTED_EXECUTOR_CONNECT_STEPS {
            assert!(
                compact.contains(&without_whitespace(step)),
                "OMEGA-DELTA-0095: {} lost `{step}` from the connect sequence. \
                 That is the path `agent_connection_store` already uses, and \
                 the only bridge from a registered external agent to an \
                 `Rc<dyn AgentConnection>`; a second way of starting the same \
                 agent would diverge from it silently.",
                attach_path.display()
            );
        }

        for token in DETECTED_EXECUTOR_DRIVABLE_TOKENS {
            assert!(
                compact.contains(&without_whitespace(token)),
                "OMEGA-DELTA-0095: {} no longer names `{token}` among the \
                 agents it will attach, so that agent can never run a turn.",
                attach_path.display()
            );
        }

        for signature in [
            "pub fn choose_executor(detected: &[DetectedAgent])",
            "detected: &[DetectedAgent],",
        ] {
            assert!(
                compact.contains(&without_whitespace(signature)),
                "OMEGA-DELTA-0095: {} no longer takes `{signature}`, so it no \
                 longer decides from what detection found on disk.",
                attach_path.display()
            );
        }
        assert!(
            !production.contains("AllAgentServersSettings"),
            "OMEGA-DELTA-0095: {} reads the configured-agents map. That records \
             what is configured, not what is present — Omega ships `codex-acp` \
             in its own defaults — so attaching from it would attach Codex on a \
             machine that has never had it, and the failure would arrive as a \
             thread that reports one executor and runs another.",
            attach_path.display()
        );

        assert_eq!(
            production.matches("Ok(None)").count(),
            1,
            "OMEGA-DELTA-0095: {} returns `Ok(None)` from somewhere other than \
             the one place that means \"nothing drivable is installed\". Every \
             other way of not reaching a chosen agent must be an error naming \
             it: a second `Ok(None)` is a silent fallback to the native loop on \
             a machine whose owner believes Codex is running.",
            attach_path.display()
        );

        let router_path = repository_path(ROUTER_DISPATCH_PATH);
        let router = std::fs::read_to_string(&router_path)
            .unwrap_or_else(|error| panic!("cannot read {}: {error}", router_path.display()));
        assert!(
            router.contains("omega_agent_attach::connect_detected_executor")
                && router.contains("with_external_acp(installed)"),
            "OMEGA-DELTA-0095: {} no longer registers the installed coding \
             agent as the router's external executor, so an unpinned thread \
             falls to the native loop on a machine with Codex.",
            router_path.display()
        );

        let factory_path = repository_path(AGENT_SERVER_FACTORY_PATH);
        let factory = std::fs::read_to_string(&factory_path)
            .unwrap_or_else(|error| panic!("cannot read {}: {error}", factory_path.display()));
        assert!(
            without_whitespace(&factory).contains(&without_whitespace(
                "if std::env::var(\"ZED_STATELESS\").is_ok()
                    || cfg!(any(test, feature = \"test-support\"))
                {
                    Vec::new()
                } else {
                    omega_agent_detect::detected().to_vec()
                },"
            )),
            "OMEGA-DELTA-0095: {} no longer hands the router the detected \
             agents, or no longer empties that set for a run that is not a \
             person's session. The first leaves the attach unreachable; the \
             second lets a rendering harness or a test spawn somebody's real \
             Codex, which is the rule the Exo lane path beside it already \
             follows.",
            factory_path.display()
        );

        let settings = default_settings().expect("default settings parse");
        for key in DETECTED_EXECUTOR_SETTINGS_KEYS {
            let entry = default_setting(&settings, key).unwrap_or_else(|| {
                panic!(
                    "OMEGA-DELTA-0095: {key} is absent from the shipped \
                     defaults, so its ACP server never registers and a machine \
                     with that agent installed cannot run a turn on it."
                )
            });
            assert_eq!(
                entry.get("type").and_then(serde_json::Value::as_str),
                Some("registry"),
                "OMEGA-DELTA-0095: {key} must resolve from the ACP registry, \
                 which is the source the endpoint allow-list approved."
            );
        }
    }

    /// OMEGA-DELTA-0095, amended by omega#106's close-out. A failed attach is
    /// re-driven when the adapter registers, and that is what makes failing
    /// hard the honest choice rather than a dead end.
    ///
    /// The amendment argued that a chosen agent Omega cannot reach must stay an
    /// error rather than degrade to the native loop, and the argument turns
    /// entirely on this seam. `ConversationView` subscribes to the
    /// agent-server store; the ACP registry finishing its load rebuilds that
    /// store and emits `AgentServersUpdated`; and a view sitting in
    /// `ServerState::LoadError` resets and connects again. So a slow registry
    /// costs a few seconds of a named error and then heals itself, and the
    /// attach never has to guess — at the one moment it must decide — whether
    /// an absent adapter is three seconds late or permanently gone.
    ///
    /// Degrading would remove exactly this. A degraded router connects
    /// successfully, so the view reaches `Connected` with no thread error,
    /// `should_retry` is false for it, and nothing re-drives the attach when
    /// the adapter finally arrives. The person would spend the rest of the
    /// session on the native loop while their installed Codex sat there
    /// working — a thread running one executor while the reader believes
    /// another, arrived at from the opposite direction.
    ///
    /// Which is why this is checked rather than assumed. Delete the retry and
    /// the attach's failure stops being recoverable, and the argument recorded
    /// in `OMEGA-DELTA-0095` stops being true — silently, in a file the attach
    /// does not own.
    #[test]
    fn a_failed_attach_is_retried_when_the_adapter_registers() {
        let path = repository_path(CONNECTION_RETRY_PATH);
        let source = read_repository_file(CONNECTION_RETRY_PATH);
        let compact = without_whitespace(&code_of(&source));

        assert!(
            compact.contains(&without_whitespace(
                "cx.subscribe_in(
                    &agent_server_store,
                    window,
                    Self::handle_agent_servers_updated,
                )"
            )),
            "OMEGA-DELTA-0095: {} no longer subscribes to the agent-server \
             store. Nothing then tells a view that the ACP registry finished \
             loading, so an attach that failed while the registry was still \
             in flight stays failed for the life of the view.",
            path.display()
        );

        let handler = body_of(&source, "handle_agent_servers_updated");
        let handler = without_whitespace(&code_of(handler));
        assert!(
            handler.contains(&without_whitespace("ServerState::LoadError { .. } => true")),
            "OMEGA-DELTA-0095: `handle_agent_servers_updated` in {} no longer \
             re-drives a view that failed to load. `omega_agent_attach` fails \
             hard on a chosen agent it cannot reach *because* this recovers: \
             without it, a registry that was two seconds late becomes a \
             permanently unusable agent panel, and the case for failing over \
             degrading collapses.",
            path.display()
        );
        assert!(
            handler.contains(&without_whitespace("self.reset(window, cx)")),
            "OMEGA-DELTA-0095: `handle_agent_servers_updated` in {} decides to \
             retry and then does not connect again. A retry that reaches no \
             `connect` is the same dead end with an extra branch.",
            path.display()
        );
    }

    /// OMEGA-DELTA-0095, amended by omega#106's close-out. An attach failure
    /// names what Omega could not reach, not the reader's installation.
    ///
    /// `omega_agent_detect` proves the `codex` binary is on `PATH`. What the
    /// attach spawns is `codex-acp`, a different artifact that Omega resolves
    /// from the ACP registry. When that resolution fails the reader's Codex is
    /// fine and Omega's own supply chain is not, so a sentence of the shape
    /// "Codex is installed at /usr/local/bin/codex, but …" hands the reader a
    /// false statement about their machine and sends them to debug the one
    /// thing that is working.
    ///
    /// The rule is the disclosure rule, applied to a failure instead of a
    /// turn: say what actually happened, and do not attribute it to something
    /// that did not do it.
    #[test]
    fn an_attach_failure_names_what_omega_could_not_reach() {
        let attach_path = repository_path(DETECTED_EXECUTOR_ATTACH_PATH);
        let attach = read_repository_file(DETECTED_EXECUTOR_ATTACH_PATH);
        let production = attach
            .split("#[cfg(test)]")
            .next()
            .expect("splitting always yields a first part");

        for phrase in DETECTED_EXECUTOR_REGISTRATION_FAILURE_PHRASES {
            assert!(
                production.contains(phrase),
                "OMEGA-DELTA-0095: {} no longer says `{phrase}` when a chosen \
                 agent's ACP adapter never registers. The reader is left to \
                 conclude their own installation is broken, and to go and \
                 debug the one part of this that is working.",
                attach_path.display()
            );
        }

        assert!(
            !production.contains("but Omega registered no ACP server under"),
            "OMEGA-DELTA-0095: {} is back to the sentence that opened with the \
             reader's binary and its path and then reported a failure. What \
             failed is the ACP registry resolution of a separate adapter; \
             naming the binary there is a false claim about their machine.",
            attach_path.display()
        );
    }

    // ------------------------------------------------------ OMEGA-DELTA-0105

    /// A source in which [`body_of`] can also find a free function.
    ///
    /// `function_body` searches for `" fn name("`, and a `fn` declared at the
    /// top level of a module has a newline in front of it rather than a space,
    /// so it is invisible. That is not a hypothetical: three of the functions
    /// this delta is about — `loaded` and `preview_audience` in the control,
    /// and `bounded` in the rules — are free functions, and every assertion
    /// about them found nothing on the first run. `body_of` turned that into a
    /// failure rather than a green test, which is what `OMEGA-DELTA-0090`'s
    /// rule is for.
    fn with_free_functions_indented(source: &str) -> String {
        source.replace("\nfn ", "\n fn ")
    }

    /// OMEGA-DELTA-0105. Local is what a profile with nothing written *loads*
    /// into, not only what an unrecorded thread resolves to.
    ///
    /// `OMEGA-DELTA-0094` pinned `AudienceBook::audience_of`'s fallback and
    /// stopped there. The other fallback on the fresh-profile path is in
    /// `loaded`, which hydrates the selection from the key-value store, and
    /// nothing held it: changing `unwrap_or_else(AudienceId::local)` there to
    /// name any other audience left all five of that delta's checks green
    /// while making a machine that has never chosen anything start its threads
    /// somewhere else.
    ///
    /// That is the same shape as the gap `OMEGA-DELTA-0094` found in its own
    /// unit tests — every one of them bound a thread first, so the branch that
    /// runs on every thread on a fresh machine had nothing holding it. The
    /// fresh-profile path is the one every install is on; it needs the checks,
    /// not the exercised one.
    #[test]
    fn local_is_what_a_fresh_profile_loads_into() {
        let path = repository_path(AUDIENCE_CONTROL_PATH);
        let source = with_free_functions_indented(&uncommented(&read_repository_file(
            AUDIENCE_CONTROL_PATH,
        )));
        let hydrate = body_of(&source, "loaded");

        assert!(
            hydrate.contains("unwrap_or_else(AudienceId::local)"),
            "OMEGA-DELTA-0105: the selection in {} no longer falls back to \
             Local. A profile that has never chosen an audience is every \
             profile on a fresh machine, and it must load into the one that \
             needs no account, no relay and no network.",
            path.display()
        );
        assert!(
            hydrate.contains("unwrap_or_default()"),
            "OMEGA-DELTA-0105: the audience book in {} no longer falls back to \
             an empty book. A book that failed to decode must leave every \
             thread unrecorded — and so Local — rather than carrying whatever \
             a partial parse produced.",
            path.display()
        );
        assert!(
            hydrate.contains("roster.resolve(id).is_some()"),
            "OMEGA-DELTA-0105: {} no longer discards a stored selection the \
             roster cannot resolve. Leaving a community and finding the \
             composer still offering to start threads in it is a control \
             describing a door that is not there.",
            path.display()
        );
    }

    /// OMEGA-DELTA-0105. The menu's wording lives in one place.
    ///
    /// The lane that wrote these three sentences named them as its own biggest
    /// guess: a person picks a different audience, the menu closes, and the
    /// button reads exactly what it read before, because choosing applies to
    /// the next thread. That is correct, and it is also what a broken dropdown
    /// looks like; these sentences are the only thing between the two
    /// readings.
    ///
    /// Nothing here can check whether they land. What it can do is make the
    /// change cheap: the strings are constants in `omega_audience` beside the
    /// rules they describe, and a literal reappearing in the control fails.
    #[test]
    fn the_menus_sentences_are_written_once() {
        let rules_path = repository_path(AUDIENCE_PATH);
        let rules = read_repository_file(AUDIENCE_PATH);
        let control_path = repository_path(AUDIENCE_CONTROL_PATH);
        let control = uncommented(&read_repository_file(AUDIENCE_CONTROL_PATH));

        for sentence in AUDIENCE_MENU_SENTENCES {
            assert!(
                rules.contains(sentence),
                "OMEGA-DELTA-0105: {} no longer declares {sentence:?}. The \
                 menu's wording is written beside the rule it describes, with \
                 the guess and its falsifier, so that changing it is one edit.",
                rules_path.display()
            );
            assert!(
                !control.contains(sentence),
                "OMEGA-DELTA-0105: {} writes {sentence:?} as a literal. It is \
                 already a constant in `omega_audience`; two copies means the \
                 next person changes one of them.",
                control_path.display()
            );
        }

        for name in [
            "SELECTION_MENU_HEADER",
            "SWITCHING_DOES_NOT_MOVE_A_THREAD",
            "THREAD_IS_NOT_IN_THE_SELECTION",
        ] {
            assert!(
                control.contains(name),
                "OMEGA-DELTA-0105: {} no longer renders `{name}`. A sentence \
                 that is declared and never drawn is not the sentence a person \
                 reads.",
                control_path.display()
            );
        }
    }

    /// OMEGA-DELTA-0105. The name on the control's face is bounded, and the
    /// row it sits in can still wrap.
    ///
    /// The audience button was added to a `flex_wrap` row already carrying the
    /// executor disclosure, a turn-phase dot, a provider notice, the model
    /// selector and Send. Nothing here renders a pixel, so none of this says
    /// the row fits a narrow dock. What it does say is that the row's width is
    /// a function of bounded things: the only unbounded text in it is an
    /// audience name, which this repository does not choose — omega#108's come
    /// from a Forge repository, and `OMEGA_AUDIENCE_PREVIEW` puts an arbitrary
    /// environment string there today.
    ///
    /// The label beside it is the `OMEGA-DELTA-0021` executor disclosure,
    /// which is `.truncate()`d, so it is the one that gives way. An unbounded
    /// audience name does not merely look wide: it takes room from the
    /// mandatory attribution of which executor ran the turn.
    #[test]
    fn the_audience_on_the_composer_is_bounded_and_the_row_can_wrap() {
        let rules_path = repository_path(AUDIENCE_PATH);
        let rules = read_repository_file(AUDIENCE_PATH);

        let label = body_of(&rules, "label");
        assert!(
            label.contains("bounded(audience.name())"),
            "OMEGA-DELTA-0105: `ThreadAudience::label` in {} no longer bounds \
             the name it returns. It is the only function that produces the \
             face of the composer's audience control, and an audience name is \
             somebody else's text.",
            rules_path.display()
        );
        let indented = with_free_functions_indented(&rules);
        let bounded = body_of(&indented, "bounded");
        assert!(
            bounded.contains("MAX_LABEL_CHARS") && bounded.contains("chars()"),
            "OMEGA-DELTA-0105: `bounded` in {} no longer counts characters \
             against `MAX_LABEL_CHARS`. Counting bytes panics on a name in any \
             script that is not ASCII.",
            rules_path.display()
        );

        let view_path = repository_path(THREAD_VIEW_PATH);
        let view = read_repository_file(THREAD_VIEW_PATH);
        let bar = body_of(&view, "render_zero_base_executor_bar");
        // The outer row's own modifiers, which are everything before its first
        // child. Asserting on the whole body would pass on the `.flex_wrap()`
        // of the right-hand group, and it is the outer row that decides
        // whether a narrow dock wraps or clips.
        let outer = bar.split(".child(").next().unwrap_or_default();
        assert!(
            outer.contains(".flex_wrap()"),
            "OMEGA-DELTA-0105: the outer row of zero base's composer bar in {} \
             no longer wraps. It carries six controls; without wrapping, a \
             narrow dock clips the one at the end, which is Send.",
            view_path.display()
        );
        assert!(
            bar.contains(".truncate()"),
            "OMEGA-DELTA-0105: the executor disclosure in {} no longer \
             truncates. It is the label that gives way when the row is tight, \
             and `OMEGA-DELTA-0021` requires it to be rendered on every draw.",
            view_path.display()
        );
    }

    /// OMEGA-DELTA-0105. The preview roster entry is a fixture, and the three
    /// things `OMEGA-DELTA-0094` claimed about it are now true of the code.
    ///
    /// That delta recorded that the fixture "publishes nothing, its identity
    /// is `preview:` prefixed so it cannot be mistaken for a Forge coordinate,
    /// and it cannot become the default". Nothing held any of the three, and
    /// the first was false at the only gate that matters. The fixture carries
    /// `Reach::Shared` — deliberately, because it exists to make the
    /// not-private case observable — and `may_publish`, which omega#108 is
    /// told to ask *before* an effect, answers on reach. So it returned `Ok`.
    /// Nothing publishes today, so nothing left any machine; the first
    /// transport wired behind that gate would have been authorised, on any
    /// machine with the variable set, to publish into an audience that does
    /// not exist.
    ///
    /// The prefix is now reserved in both directions, exactly as `local` is,
    /// so "cannot collide with a Forge coordinate" is a constructor's refusal
    /// rather than a naming convention.
    #[test]
    fn the_preview_audience_is_a_fixture_and_not_a_place() {
        let rules_path = repository_path(AUDIENCE_PATH);
        let rules = read_repository_file(AUDIENCE_PATH);

        let publish = body_of(&rules, "may_publish");
        // Two arms, counted rather than merely present. A first draft of this
        // assertion asked whether the body contained `is_preview()` at all,
        // and it stayed green with the resolved-fixture arm deleted — the
        // unresolved one alone satisfied it, and the unresolved one is not the
        // arm a machine with the variable set goes through. Watching that
        // mutation survive is what found this.
        assert_eq!(
            publish.matches("AudienceIsAFixture").count(),
            2,
            "OMEGA-DELTA-0105: `may_publish` in {} no longer refuses a fixture \
             on both paths. A fixture the roster resolves and a record that \
             outlived `OMEGA_AUDIENCE_PREVIEW` are both fixtures, and the \
             first is the one a machine with the variable set reaches.",
            rules_path.display()
        );
        let fixture_arm = publish
            .find("is_preview()")
            .expect("`may_publish` must ask whether the audience is a fixture");
        let reach_arm = publish
            .find("Reach::Shared")
            .expect("`may_publish` must still admit a shared audience");
        assert!(
            fixture_arm < reach_arm,
            "OMEGA-DELTA-0105: `may_publish` in {} consults reach before it \
             asks whether the audience is a fixture. The fixture is \
             `Reach::Shared` on purpose, so reach answers first and answers \
             `Ok`.",
            rules_path.display()
        );

        let joined = body_of(&rules, "joined");
        assert!(
            joined.contains("PREVIEW_PREFIX") && joined.contains("ReservedPreviewPrefix"),
            "OMEGA-DELTA-0105: `AudienceId::joined` in {} no longer reserves \
             the fixture prefix. omega#108's coordinates arrive through this \
             constructor, and a Forge coordinate wearing `preview:` would be \
             refused at `may_publish` as a fixture — a real membership that \
             silently stops being able to publish.",
            rules_path.display()
        );

        let control_path = repository_path(AUDIENCE_CONTROL_PATH);
        let control = uncommented(&read_repository_file(AUDIENCE_CONTROL_PATH));
        let indented = with_free_functions_indented(&control);
        let preview = body_of(&indented, "preview_audience");
        assert!(
            preview.contains("omega_audience::preview_audience("),
            "OMEGA-DELTA-0105: {} decides what the fixture variable means \
             instead of delegating. The rule belongs where it can be tested on \
             a machine that is not in the right state; this module's job is \
             reading the variable.",
            control_path.display()
        );
        assert!(
            !control.contains("Audience::preview("),
            "OMEGA-DELTA-0105: {} mints a fixture itself. A fixture is a thing \
             `omega_audience` constructs and never a thing that arrives.",
            control_path.display()
        );
        assert!(
            preview.contains("std::env::var("),
            "OMEGA-DELTA-0105: the fixture in {} is no longer gated on the \
             environment. It must be absent on every machine that has not \
             deliberately asked for it.",
            control_path.display()
        );
    }

    /// OMEGA-DELTA-0105. Nothing can declare a dependency where
    /// `OMEGA-DELTA-0094`'s allowlist cannot see it.
    ///
    /// That check reads the manifest text from `\n[dependencies]` to the next
    /// `\n[`, which makes `[dependencies.tokio]`, `[build-dependencies]` and
    /// `[target.'cfg(unix)'.dependencies]` invisible to it — three ordinary
    /// spellings, any of which would put a socket behind Local while the
    /// allowlist stayed green. Rather than teach that parser every spelling,
    /// the manifest is held to the two sections it can read.
    #[test]
    fn nothing_can_declare_a_dependency_the_local_allowlist_cannot_see() {
        let path = repository_path(AUDIENCE_MANIFEST_PATH);
        let manifest = read_repository_file(AUDIENCE_MANIFEST_PATH);

        for line in manifest.lines().map(str::trim) {
            if !line.starts_with('[') || !line.contains("dependencies") {
                continue;
            }
            assert!(
                AUDIENCE_ALLOWED_MANIFEST_SECTIONS.contains(&line),
                "OMEGA-DELTA-0105: {} declares dependencies under `{line}`, \
                 which `local_needs_no_network_no_relay_and_no_account` cannot \
                 see — it reads `[dependencies]` and stops at the next `[`. \
                 Local's allowlist would stay green over anything declared \
                 there. Admitted sections: \
                 {AUDIENCE_ALLOWED_MANIFEST_SECTIONS:?}.",
                path.display()
            );
        }

        for required in AUDIENCE_ALLOWED_MANIFEST_SECTIONS {
            assert!(
                manifest.lines().any(|line| line.trim() == *required),
                "OMEGA-DELTA-0105: {} no longer declares `{required}`. A check \
                 that cannot find what it is about must fail rather than pass.",
                path.display()
            );
        }
    }
}
