use serde::Deserialize;

#[derive(Debug, Deserialize)]
struct CompatibilityAllowList {
    version: u32,
    owner: String,
    policy: String,
    allowed_dispositions: Vec<String>,
    entries: Vec<CompatibilityEntry>,
}

#[derive(Debug, Deserialize)]
struct CompatibilityEntry {
    #[serde(rename = "match")]
    match_text: String,
    path: Option<String>,
    reason: String,
    owner: String,
    disposition: String,
    expiry: String,
}

fn load_allowlist() -> CompatibilityAllowList {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("fixtures/compatibility_allowlist.json");
    let text = std::fs::read_to_string(&path)
        .unwrap_or_else(|error| panic!("read {}: {error}", path.display()));
    serde_json::from_str(&text).unwrap_or_else(|error| panic!("parse allowlist: {error}"))
}

#[test]
fn compatibility_allowlist_has_required_fields() {
    let allowlist = load_allowlist();
    assert_eq!(allowlist.version, 1);
    assert_eq!(allowlist.owner, "OpenAgents");
    assert!(!allowlist.policy.is_empty());
    assert!(
        allowlist
            .allowed_dispositions
            .iter()
            .any(|disposition| disposition == "approved_compatibility")
    );
    assert!(
        allowlist
            .allowed_dispositions
            .iter()
            .any(|disposition| disposition == "blocked")
    );
    assert!(!allowlist.entries.is_empty());

    for entry in &allowlist.entries {
        assert!(
            !entry.match_text.is_empty(),
            "entry match must be non-empty"
        );
        assert!(!entry.reason.is_empty(), "entry reason must be non-empty");
        assert_eq!(entry.owner, "OpenAgents");
        assert!(!entry.expiry.is_empty(), "entry expiry must be non-empty");
        assert!(
            allowlist
                .allowed_dispositions
                .iter()
                .any(|disposition| disposition == &entry.disposition),
            "unknown disposition {} for {}",
            entry.disposition,
            entry.match_text
        );
        if let Some(path) = &entry.path {
            assert!(!path.is_empty(), "entry path must be non-empty when set");
        }
    }
}

#[test]
fn high_risk_public_files_forbid_zed_product_phrases() {
    let workspace_root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let paths = [
        "crates/onboarding/src/basics_page.rs",
        "crates/onboarding/src/multibuffer_hint.rs",
        "crates/ai_onboarding/src/ai_onboarding.rs",
        "crates/ai_onboarding/src/plan_definitions.rs",
        "crates/ai_onboarding/src/young_account_banner.rs",
        "crates/settings_ui/src/settings_ui.rs",
        "crates/settings_ui/src/page_data.rs",
        "crates/workspace/src/notifications.rs",
        "crates/workspace/src/workspace_error.rs",
        "crates/title_bar/src/title_bar.rs",
        "crates/title_bar/src/collab.rs",
        "crates/extensions_ui/src/extensions_ui.rs",
        "crates/auto_update_helper/src/dialog.rs",
        "crates/zed/src/zed/move_to_applications.rs",
        "crates/edit_prediction/src/edit_prediction.rs",
        "crates/edit_prediction/src/zeta.rs",
    ];

    let forbidden = [
        "Welcome to Zed",
        "Welcome to Zed AI",
        "Updating Zed",
        "About Zed",
        "Move Zed to Applications",
        "Installing Zed",
        "Help improve Zed",
        "Help fix Zed",
        "Zed Agent",
        "Try Zed Pro",
        "Zed — Settings",
        "Restart to update Zed",
        "Please update Zed",
        "Please restart Zed",
        "built-in to Zed",
        "version of Zed",
        "Update Zed",
        "A new version of Zed",
        "Zed needs an xdg-desktop-portal",
        "continue using Zed AI",
        "billing-support@zed.dev",
        "https://zed.dev/releases",
        "https://zed.dev/docs/multibuffers",
        "https://zed.dev/docs/linux",
    ];

    for relative in paths {
        let path = workspace_root.join(relative);
        let source = std::fs::read_to_string(&path)
            .unwrap_or_else(|error| panic!("read {}: {error}", path.display()));
        for phrase in forbidden {
            assert!(
                !source.contains(phrase),
                "{relative} still contains forbidden public phrase: {phrase}"
            );
        }
    }
}

#[test]
fn omega_hosted_ai_and_external_agent_copy_is_honest() {
    let workspace_root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let ai_onboarding =
        std::fs::read_to_string(workspace_root.join("crates/ai_onboarding/src/ai_onboarding.rs"))
            .expect("read ai_onboarding");
    assert!(ai_onboarding.contains("not available in Omega"));
    assert!(!ai_onboarding.contains("Welcome to Zed AI"));
    assert!(!ai_onboarding.contains("Try Zed Pro for Free"));

    let basics =
        std::fs::read_to_string(workspace_root.join("crates/onboarding/src/basics_page.rs"))
            .expect("read basics_page");
    assert!(basics.contains("Install external agents to start a thread"));
    assert!(basics.contains("Codex uses its own login and configuration"));
    assert!(!basics.contains(".name(\"Zed Agent\")"));
}
