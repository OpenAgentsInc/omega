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
        assert!(
            disclosure.contains("\"omega-executor-pin\""),
            "OMEGA-DELTA-0035: {} no longer renders the executor pin control. \
             A pin is the only way a thread reaches anything but the native \
             loop, so with no control there is no route to honour.",
            disclosure_path.display()
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
}
