use super::{
    PRODUCT_BUG_REPORT_URL, PRODUCT_DOCS_URL, PRODUCT_FEATURE_REQUEST_URL, PRODUCT_NAME,
    PRODUCT_REPOSITORY_URL, PRODUCT_TAGLINE, PUBLISHER_NAME,
};

fn assert_no_zed_product_text(value: &str) {
    assert!(
        !value.to_lowercase().contains("zed"),
        "shell branding string unexpectedly names Zed: {value}"
    );
}

#[test]
fn shell_branding_strings_are_omega_and_openagents() {
    assert_eq!(PRODUCT_NAME, "Omega");
    assert_eq!(PUBLISHER_NAME, "OpenAgents");
    assert_eq!(PRODUCT_TAGLINE, "Your last IDE.");
    assert_eq!(
        PRODUCT_REPOSITORY_URL,
        "https://github.com/OpenAgentsInc/omega"
    );
    assert!(PRODUCT_DOCS_URL.contains("OpenAgentsInc/omega"));
    assert!(PRODUCT_BUG_REPORT_URL.contains("OpenAgentsInc/omega"));
    assert!(PRODUCT_FEATURE_REQUEST_URL.contains("OpenAgentsInc/omega"));

    for value in [
        PRODUCT_NAME,
        PUBLISHER_NAME,
        PRODUCT_TAGLINE,
        PRODUCT_REPOSITORY_URL,
        PRODUCT_DOCS_URL,
        PRODUCT_BUG_REPORT_URL,
        PRODUCT_FEATURE_REQUEST_URL,
        "Welcome to Omega",
        "Welcome back to Omega",
        "About Omega",
        "Hide Omega",
        "Quit Omega",
        "Omega failed to launch",
        "Omega, published by OpenAgents",
    ] {
        assert_no_zed_product_text(value);
    }
}

#[test]
fn shell_source_files_have_no_zed_product_copy() {
    let workspace_root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let paths = [
        "crates/workspace/src/welcome.rs",
        "crates/omega/src/zed/app_menus.rs",
    ];

    for relative in paths {
        let path = workspace_root.join(relative);
        let source = std::fs::read_to_string(&path)
            .unwrap_or_else(|error| panic!("read {}: {error}", path.display()));
        for forbidden in [
            "Welcome to Zed",
            "Welcome back to Zed",
            "About Zed",
            "Hide Zed",
            "Quit Zed",
            "Zed Repository",
            "Zed Twitter",
            "Join the Team",
            "https://zed.dev/docs",
            "https://twitter.com/zeddotdev",
            "https://zed.dev/jobs",
            "VectorName::ZedLogo",
            "The editor for what's next",
        ] {
            assert!(
                !source.contains(forbidden),
                "{relative} still contains forbidden shell copy: {forbidden}"
            );
        }
        assert!(
            source.contains("Omega") || relative.ends_with("app_menus.rs"),
            "{relative} should present Omega branding"
        );
    }
}
