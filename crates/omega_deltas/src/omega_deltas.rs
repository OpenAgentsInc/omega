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
        .filter(|line| !line.trim_start().starts_with("//"))
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

fn next_enum_body<'a>(source: &'a str, name: &str) -> Option<(usize, &'a str, usize)> {
    let header = format!("pub enum {name} {{\n");
    let start = source.find(&header)?;
    let body_start = start + header.len();
    let end = source[body_start..].find("\n}")? + body_start;
    Some((start, &source[body_start..end], end))
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
