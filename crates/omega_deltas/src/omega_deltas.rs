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
    "OMEGA-DELTA-0010",
    "OMEGA-DELTA-0011",
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
    // OMEGA-DELTA-0010
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
    ("OMEGA-DELTA-0010", "collab_panel::"),
    ("OMEGA-DELTA-0010", "channel_modal::"),
];

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

    /// OMEGA-DELTA-0011. The agent ships reachable.
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
                "OMEGA-DELTA-0011: {key} must default to true. With it false the \
                 agent is not reachable from the command palette or the panel, \
                 and no Settings control turns it back on."
            );
        }
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
}
