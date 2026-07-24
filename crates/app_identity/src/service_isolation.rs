use serde::Deserialize;

#[derive(Debug, Deserialize)]
struct AllowList {
    version: u32,
    owner: String,
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
    compatibility_override: Option<String>,
}

fn load_allowlist() -> AllowList {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("fixtures/endpoint_allowlist.json");
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
}

#[test]
fn default_settings_do_not_point_at_zed_production() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../assets/settings/default.json");
    let text = std::fs::read_to_string(&path)
        .unwrap_or_else(|error| panic!("read {}: {error}", path.display()));

    assert!(text.contains("\"server_url\": \"https://services.openagents.invalid\""));
    assert!(text.contains("\"diagnostics\": false"));
    assert!(text.contains("\"metrics\": false"));
    assert!(text.contains("\"auto_update\": false"));
    assert!(text.contains("\"disable_ai\": true"));
    assert!(text.contains("\"provider\": \"none\""));
    assert!(text.contains("\"auto_install_extensions\": {}"));
    assert!(!text.contains("\"server_url\": \"https://zed.dev\""));
    assert!(
        text.contains("\"provider\": \"ollama\""),
        "default agent model should not use the Zed cloud provider"
    );
}
