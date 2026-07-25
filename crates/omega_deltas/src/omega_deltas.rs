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
/// Adding a check without adding its ID here, or an ID without a registry
/// entry, fails `every_enforced_delta_is_registered`.
pub const ENFORCED_DELTAS: &[&str] = &[
    "OMEGA-DELTA-0001",
    "OMEGA-DELTA-0002",
    "OMEGA-DELTA-0003",
    "OMEGA-DELTA-0004",
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

    /// Every delta needs a registry entry, so a check cannot outlive its reason.
    #[test]
    fn every_enforced_delta_is_registered() {
        let path = repository_path(DELTA_REGISTRY_PATH);
        let registry = std::fs::read_to_string(&path).expect("delta registry is readable");
        for delta in ENFORCED_DELTAS {
            assert!(
                registry.contains(delta),
                "{delta} is enforced by a test but missing from {}",
                path.display()
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
                "trailing": [1, 2,],
            }"#,
        ))
        .expect("normalized JSONC parses");
        assert_eq!(parsed["url"], "https://example.com/a//b");
        assert_eq!(parsed["text"], "a, b");
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
