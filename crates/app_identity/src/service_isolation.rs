use serde::Deserialize;

#[derive(Debug, Deserialize)]
struct AllowList {
    version: u32,
    owner: String,
    enforcement: String,
    normal_start_forbidden_hosts: Vec<String>,
    entries: Vec<AllowListEntry>,
}

#[derive(Debug, Deserialize)]
struct AllowListEntry {
    host: String,
    purpose: String,
    owner: String,
    disposition: String,
    expiry: String,
    #[serde(default)]
    paths: Vec<String>,
    #[serde(default)]
    compatibility_override: Option<String>,
}

fn load_allowlist() -> AllowList {
    let path =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures/endpoint_allowlist.json");
    let text = std::fs::read_to_string(&path)
        .unwrap_or_else(|error| panic!("read {}: {error}", path.display()));
    serde_json::from_str(&text).unwrap_or_else(|error| panic!("parse allowlist: {error}"))
}

#[test]
fn zed_production_services_are_disabled_by_default() {
    assert!(!super::ZED_PRODUCTION_SERVICES_ENABLED);
    // The env override is opt-in and not set during tests.
    assert!(!super::zed_production_services_enabled());
}

#[test]
fn endpoint_allowlist_blocks_zed_hosts_for_normal_start() {
    let allowlist = load_allowlist();
    assert_eq!(allowlist.version, 1);
    assert_eq!(allowlist.owner, "OpenAgents");
    assert_eq!(allowlist.enforcement, "release-proof-evidence-only");

    for host in [
        "zed.dev",
        "api.zed.dev",
        "cloud.zed.dev",
        "collab.zed.dev",
        "status.zed.dev",
    ] {
        assert!(
            allowlist
                .normal_start_forbidden_hosts
                .iter()
                .any(|forbidden| forbidden == host),
            "missing forbidden host {host}"
        );
    }

    for entry in &allowlist.entries {
        assert!(!entry.purpose.is_empty());
        assert_eq!(entry.owner, "OpenAgents");
        assert!(!entry.expiry.is_empty());
        if entry.host.contains("zed.") {
            assert_eq!(entry.disposition, "blocked-by-default");
            assert_eq!(
                entry.compatibility_override.as_deref(),
                Some("OMEGA_ALLOW_ZED_SERVICES=1")
            );
        }
    }

    for (host, required_path, purpose) in [
        (
            "cdn.agentclientprotocol.com",
            "/registry/v1/",
            "ACP registry",
        ),
        ("nodejs.org", "/dist/", "Node.js and npm"),
        ("registry.npmjs.org", "/", "npm metadata and tarballs"),
    ] {
        assert!(allowlist.entries.iter().any(|entry| {
            entry.host == host
                && entry.purpose.contains(purpose)
                && entry.disposition == "approved"
                && entry.paths.iter().any(|path| path == required_path)
        }));
    }
}

#[test]
fn default_settings_enable_registry_acp_without_enabling_zed_production() {
    let path =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/settings/default.json");
    let text = std::fs::read_to_string(&path)
        .unwrap_or_else(|error| panic!("read {}: {error}", path.display()));
    let settings: serde_json::Value = settings_json::parse_json_with_comments(&text)
        .unwrap_or_else(|error| panic!("parse {}: {error}", path.display()));

    assert_eq!(settings["disable_ai"], false);
    assert_eq!(settings["agent_servers"]["codex-acp"]["type"], "registry");
    assert_eq!(
        settings["server_url"],
        "https://services.openagents.invalid"
    );
    assert_eq!(settings["telemetry"]["diagnostics"], false);
    assert_eq!(settings["telemetry"]["metrics"], false);
    assert_eq!(settings["auto_update"], false);
    assert_eq!(settings["edit_predictions"]["provider"], "none");
    assert_eq!(settings["auto_install_extensions"], serde_json::json!({}));
    assert!(!text.contains("\"server_url\": \"https://zed.dev\""));
    // A direct provider, not a Zed-hosted one. What this assertion protects is
    // that the default never points at a Zed service; it is not a claim about
    // which direct provider the owner prefers.
    assert_eq!(settings["agent"]["default_model"]["provider"], "google");
    assert!(!super::zed_production_services_enabled());
}
