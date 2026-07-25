use serde::Deserialize;

#[derive(Debug, Deserialize)]
struct ReleaseRecord {
    schema_version: u32,
    product: String,
    channel: String,
    version: String,
    artifact_name: String,
    volume_name: String,
    bundle_identifier: String,
    team_id: String,
    signing_identity: String,
    source: SourceSection,
    toolchains: ToolchainsSection,
    digests: DigestsSection,
    notarization: NotarizationSection,
    legal: LegalSection,
    publication: PublicationSection,
}

#[derive(Debug, Deserialize)]
struct SourceSection {
    commit: String,
    #[serde(default)]
    #[allow(dead_code)]
    upstream_commit: Option<String>,
    dirty: bool,
}

#[derive(Debug, Deserialize)]
struct ToolchainsSection {
    rustc: String,
    host: String,
    target: String,
}

#[derive(Debug, Deserialize)]
struct DigestsSection {
    cargo_lock_sha256: String,
    icon_family_manifest_sha256: String,
    #[serde(default)]
    package_sha256: Option<String>,
}

#[derive(Debug, Deserialize)]
struct NotarizationSection {
    attempted: bool,
    stapled: bool,
    /// OMEGA-DELTA-0023. Whether the ticket is stapled to `Omega.app` itself.
    ///
    /// Kept separate from `stapled`, which covers the disk image. A DMG ticket
    /// does not travel with the application that ends up in `/Applications`,
    /// so conflating the two is how a candidate could claim Gatekeeper
    /// acceptance while `stapler validate /Applications/Omega.app` reported no
    /// ticket and offline first start was unprovable.
    #[serde(default)]
    app_stapled: bool,
    status: String,
}

#[derive(Debug, Deserialize)]
struct LegalSection {
    commercial_terms_attached: bool,
}

#[derive(Debug, Deserialize)]
struct PublicationSection {
    repository: String,
    tag: String,
    prerelease: bool,
    latest: bool,
}

fn load_fixture() -> ReleaseRecord {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("fixtures/release_record_v1.json");
    let text = std::fs::read_to_string(&path)
        .unwrap_or_else(|error| panic!("read {}: {error}", path.display()));
    serde_json::from_str(&text).unwrap_or_else(|error| panic!("parse fixture: {error}"))
}

fn assert_sha256_hex(label: &str, value: &str) {
    assert_eq!(value.len(), 64, "{label} must be 64 hex chars");
    assert!(
        value.chars().all(|character| character.is_ascii_hexdigit()),
        "{label} must be hexadecimal"
    );
}

#[test]
fn release_record_fixture_matches_rc_contract() {
    let record = load_fixture();

    assert_eq!(record.schema_version, 1);
    assert_eq!(record.product, "Omega");
    assert_eq!(record.channel, "rc");
    assert_eq!(record.version, "0.2.0-rc2");
    assert_eq!(record.artifact_name, "Omega-v0.2.0-rc2-macos-arm64.dmg");
    assert_eq!(record.volume_name, "Omega RC");
    assert_eq!(record.bundle_identifier, "com.openagents.omega.rc");
    assert_eq!(record.team_id, "HQWSG26L43");
    assert!(
        record
            .signing_identity
            .contains("OpenAgents, Inc. (HQWSG26L43)")
    );
    assert!(!record.source.commit.is_empty());
    assert!(!record.source.dirty);
    assert!(!record.toolchains.rustc.is_empty());
    assert_eq!(record.toolchains.host, "aarch64-apple-darwin");
    assert_eq!(record.toolchains.target, "aarch64-apple-darwin");
    assert_sha256_hex(
        "digests.cargo_lock_sha256",
        &record.digests.cargo_lock_sha256,
    );
    assert_sha256_hex(
        "digests.icon_family_manifest_sha256",
        &record.digests.icon_family_manifest_sha256,
    );
    assert!(record.digests.package_sha256.is_none());
    assert!(!record.legal.commercial_terms_attached);
    assert!(!record.notarization.attempted);
    assert!(!record.notarization.stapled);
    assert!(!record.notarization.app_stapled);
    assert_eq!(record.notarization.status, "not_attempted");
    assert_eq!(record.publication.repository, "OpenAgentsInc/omega");
    assert_eq!(record.publication.tag, "v0.2.0-rc2");
    assert!(record.publication.prerelease);
    assert!(!record.publication.latest);
}

#[test]
fn release_record_publication_must_be_prerelease_not_latest() {
    let record = load_fixture();
    assert!(
        record.publication.prerelease && !record.publication.latest,
        "RC candidates must publish as prerelease and never as latest"
    );
    assert!(!record.legal.commercial_terms_attached);
}
