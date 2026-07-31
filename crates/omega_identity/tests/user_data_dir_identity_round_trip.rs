//! `--user-data-dir` has to carry the identity registry, not just the rest of
//! the profile.
//!
//! Startup provisions through `IdentityService::system`, which resolves
//! `paths::data_dir()` and therefore honours the override. The hosted Nostr
//! sign-in path reads through `AccountRegistryService::for_channel`, which used
//! to compute the platform application-support directory itself. An install
//! launched on a non-default data directory wrote its identity to one place and
//! signed in against another, so hosted sign-in and the Sarah admission page
//! both reported the identity `Unavailable` on a profile that had one.
//!
//! This lives in its own integration binary on purpose: the override is a
//! process-global `OnceLock` and the platform default is derived from
//! environment the test has to own, so both must be set exactly once, before
//! anything else in the process resolves a path.

use std::path::{Path, PathBuf};

use app_identity::AppChannel;
use omega_identity::{AccountRegistryService, CustodyState, IdentityService, ReceiptRef};

fn platform_default_root(home: &Path, channel: AppChannel) -> PathBuf {
    #[cfg(target_os = "macos")]
    {
        home.join("Library/Application Support")
            .join(channel.display_name())
    }
    #[cfg(any(target_os = "linux", target_os = "freebsd"))]
    {
        home.join(".local/share").join(channel.storage_slug())
    }
    #[cfg(target_os = "windows")]
    {
        home.join("AppData")
            .join("Local")
            .join(channel.display_name())
    }
    #[cfg(not(any(
        target_os = "macos",
        target_os = "linux",
        target_os = "freebsd",
        target_os = "windows"
    )))]
    {
        home.join(".config").join(channel.storage_slug())
    }
}

#[test]
fn hosted_sign_in_resolves_the_identity_written_under_an_overridden_user_data_dir() {
    let profile_parent = tempfile::tempdir().expect("temporary directory");
    let home = profile_parent.path().join("home");
    std::fs::create_dir_all(&home).expect("create the stand-in home directory");

    // Own every input the platform default is derived from, so the "nothing
    // reached the default root" assertion below is about a directory this test
    // created rather than about whichever profile the machine really has.
    // SAFETY: single-threaded, before the first path resolution in this process.
    unsafe {
        std::env::set_var("HOME", &home);
        std::env::set_var("XDG_DATA_HOME", home.join(".local/share"));
        std::env::set_var("LOCALAPPDATA", home.join("AppData").join("Local"));
    }

    let channel = *app_identity::CHANNEL;
    let default_root = platform_default_root(&home, channel);

    let data_root = paths::set_custom_data_dir(
        profile_parent
            .path()
            .join("profile")
            .to_str()
            .expect("the temporary profile path is valid UTF-8"),
    )
    .clone();

    // Launch on the overridden profile: the startup path resolves its data root
    // through `paths` and provisions an identity there.
    let provisioned = IdentityService::for_channel_data_root(channel, paths::data_dir().clone())
        .provision_for_process_start(
            ReceiptRef::new("user-data-dir-round-trip").expect("receipt reference"),
        )
        .expect("provision an identity under the overridden data root");
    assert_eq!(provisioned.state, CustodyState::Ready);
    let provisioned_identity = provisioned
        .identity
        .expect("provisioning yields a public identity");

    // Sign in: this is the accessor the hosted Nostr auth path uses, and the
    // one that used to read the platform default instead.
    let registry = AccountRegistryService::for_channel(channel);
    let selection = registry
        .selection_token()
        .expect("the overridden profile has an active account selection");
    assert_eq!(selection.identity, provisioned_identity);

    let dashboard = registry
        .inspect()
        .expect("the overridden profile has a readable account registry");
    let active = dashboard
        .accounts
        .iter()
        .find(|account| account.account_ref == selection.account_ref)
        .expect("the selected account is present in the registry it was written to");
    assert_eq!(active.identity, provisioned_identity);

    // The custody sibling resolves through the same accessor and must agree.
    let custody = IdentityService::for_channel(channel)
        .inspect()
        .expect("custody resolves on the overridden profile");
    assert_eq!(custody.state, CustodyState::Ready);
    assert_eq!(custody.identity, Some(provisioned_identity));

    assert!(
        data_root.join("identity").is_dir(),
        "the identity was written below the overridden data root {}",
        data_root.display()
    );
    assert!(
        !default_root.exists(),
        "the overridden profile touched the platform default root {}",
        default_root.display()
    );
}
