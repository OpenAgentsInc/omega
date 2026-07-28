use std::{collections::BTreeMap, fs, path::PathBuf};

use serde::Deserialize;
use sha2::{Digest, Sha256};

const PINNED_PNG_SHA256: &str = "1f80f8d36d459e2bf62c3d1fcb05f42cf6c2a26b84aeb320eb89bedbb489d551";
const PINNED_ICNS_SHA256: &str = "f6d748c1765161ec785f55137afd548cf1ff23de12d2de14383e8a01c697654d";

#[derive(Debug, Deserialize)]
struct Manifest {
    channel_badges: String,
    pinned_source: PinnedSource,
    outputs: BTreeMap<String, OutputRecord>,
}

#[derive(Debug, Deserialize)]
struct PinnedSource {
    png_path: String,
    png_sha256: String,
    png_pixels: [u32; 2],
    icns_path: String,
    icns_sha256: String,
}

#[derive(Debug, Deserialize)]
struct OutputRecord {
    sha256: String,
    format: String,
    #[serde(default)]
    pixels: Option<[u32; 2]>,
    #[serde(default)]
    sizes: Option<Vec<u32>>,
    derived_from: String,
}

fn resources_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../zed/resources")
}

fn sha256_file(path: &std::path::Path) -> String {
    let bytes = fs::read(path).unwrap_or_else(|error| panic!("read {}: {error}", path.display()));
    let mut hasher = Sha256::new();
    hasher.update(&bytes);
    format!("{:x}", hasher.finalize())
}

fn png_dimensions(path: &std::path::Path) -> [u32; 2] {
    let bytes = fs::read(path).unwrap_or_else(|error| panic!("read {}: {error}", path.display()));
    assert!(
        bytes.starts_with(b"\x89PNG\r\n\x1a\n"),
        "{} is not a PNG",
        path.display()
    );
    let width = u32::from_be_bytes(bytes[16..20].try_into().expect("png width"));
    let height = u32::from_be_bytes(bytes[20..24].try_into().expect("png height"));
    [width, height]
}

fn load_manifest() -> (PathBuf, Manifest) {
    let root = resources_root();
    let manifest_path = root.join("icon_family/manifest.json");
    let text = fs::read_to_string(&manifest_path)
        .unwrap_or_else(|error| panic!("read {}: {error}", manifest_path.display()));
    let manifest: Manifest = serde_json::from_str(&text)
        .unwrap_or_else(|error| panic!("parse {}: {error}", manifest_path.display()));
    (root, manifest)
}

#[test]
fn pinned_openagents_icon_sources_match_release_digests() {
    let (root, manifest) = load_manifest();

    assert_eq!(manifest.channel_badges, "deferred");
    assert_eq!(manifest.pinned_source.png_sha256, PINNED_PNG_SHA256);
    assert_eq!(manifest.pinned_source.icns_sha256, PINNED_ICNS_SHA256);
    assert_eq!(manifest.pinned_source.png_pixels, [1024, 1024]);

    let png_path = root.join(&manifest.pinned_source.png_path);
    let icns_path = root.join(&manifest.pinned_source.icns_path);
    assert_eq!(sha256_file(&png_path), PINNED_PNG_SHA256);
    assert_eq!(sha256_file(&icns_path), PINNED_ICNS_SHA256);
    assert_eq!(png_dimensions(&png_path), [1024, 1024]);
}

#[test]
fn generated_package_icons_match_manifest_digests_and_dimensions() {
    let (root, manifest) = load_manifest();

    let expected_outputs = [
        "Document.icns",
        "app-icon-dev.png",
        "app-icon-dev@2x.png",
        "app-icon-nightly.png",
        "app-icon-nightly@2x.png",
        "app-icon-preview.png",
        "app-icon-preview@2x.png",
        "app-icon.png",
        "app-icon@2x.png",
        "windows/app-icon-dev.ico",
        "windows/app-icon-nightly.ico",
        "windows/app-icon-preview.ico",
        "windows/app-icon.ico",
    ];
    assert_eq!(
        manifest.outputs.keys().cloned().collect::<Vec<_>>(),
        expected_outputs
    );

    for (relative_path, record) in &manifest.outputs {
        let path = root.join(relative_path);
        assert_eq!(
            sha256_file(&path),
            record.sha256,
            "digest mismatch for {relative_path}"
        );

        match record.format.as_str() {
            "png" => {
                let pixels = record.pixels.expect("png outputs record pixels");
                assert_eq!(png_dimensions(&path), pixels, "{relative_path} pixels");
                assert!(
                    record.derived_from.ends_with("openagents-icon.png"),
                    "{relative_path} must derive from the pinned PNG"
                );
            }
            "icns" => {
                let bytes = fs::read(&path).expect("read icns");
                assert!(
                    bytes.starts_with(b"icns"),
                    "{relative_path} missing icns magic"
                );
                assert!(
                    record.derived_from.ends_with("openagents-icon.icns"),
                    "{relative_path} must derive from the pinned ICNS"
                );
            }
            "ico" => {
                let bytes = fs::read(&path).expect("read ico");
                assert_eq!(
                    &bytes[0..4],
                    &[0, 0, 1, 0],
                    "{relative_path} missing ico magic"
                );
                let sizes = record.sizes.as_ref().expect("ico outputs record sizes");
                assert_eq!(sizes, &vec![16, 32, 48, 256]);
                assert!(
                    record.derived_from.ends_with("openagents-icon.png"),
                    "{relative_path} must derive from the pinned PNG"
                );
            }
            other => panic!("unexpected icon format {other} for {relative_path}"),
        }
    }
}
