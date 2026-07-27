//! Which key opens the lane's root. `OMEGA-DELTA-0126`, omega#112.
//!
//! Exo encrypts the provider credentials in its state root, and the key that
//! opens them is a file whose path Exo learns from its environment.
//!
//! Before this existed, `connect_configured_lane` built its child command with
//! `env: None` and the `exo acp` child inherited whatever Omega itself was
//! launched with. A shell launch after `export EXO_SECRET_BACKEND=file …`
//! worked; the same machine launched from the Dock has neither variable, and
//! the turn failed with `failed to decrypt secret payload` — **after** the
//! person had typed and sent their first message, and with a sentence that
//! reads like a corrupt state root rather than a missing environment.
//!
//! # What this type is, and is not
//!
//! It is a *name for a key*, in the same sense that [`ExoRoot`] is a name for a
//! directory. It has no method that reads a key or touches
//! the file it names. What it produces is [`ExoSecretStore::env`] — the two
//! variables Exo's own CLI documents (`EXO_SECRET_BACKEND`,
//! `EXO_MASTER_KEY_PATH`) — and handing an environment to a child process is
//! the whole of what Omega does with it.
//!
//! That boundary is the same one [`crate::command`] draws: Omega names Exo's
//! storage on a command line and never opens it. A secret store that could
//! *read* would put a provider credential inside Omega's address space, and
//! then "Omega never holds Exo's credentials" would stop being true.
//!
//! [`ExoRoot`]: crate::ExoRoot

use std::path::{Path, PathBuf};

/// The environment variable Exo reads its secret backend from.
pub const SECRET_BACKEND_ENV_VAR: &str = "EXO_SECRET_BACKEND";

/// The environment variable Exo reads its master key path from.
pub const MASTER_KEY_PATH_ENV_VAR: &str = "EXO_MASTER_KEY_PATH";

/// The value `EXO_SECRET_BACKEND` takes for a file-backed root.
pub const FILE_BACKEND: &str = "file";

/// Where the key that opens a root's secrets is kept.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ExoSecretStore {
    /// A master key file.
    ///
    /// `master_key` is `None` when the lane leaves the path to Exo's own
    /// default (`$XDG_CONFIG_HOME/exo/master.key`, else
    /// `$HOME/.config/exo/master.key`). Naming it is not required and stating
    /// it is not wrong: a lane that names the path Exo would have picked
    /// anyway keeps working if Exo's default ever moves.
    File { master_key: Option<PathBuf> },
}

impl ExoSecretStore {
    /// The backend name Exo's CLI accepts, for `EXO_SECRET_BACKEND`.
    #[must_use]
    pub const fn backend_name(&self) -> &'static str {
        FILE_BACKEND
    }

    /// The store this backend name and key path stand for.
    ///
    /// `None` for a backend name Exo does not have. A key path without a
    /// backend name is a file-backed store.
    #[must_use]
    pub fn parse(backend: Option<&str>, master_key: Option<&Path>) -> Option<Self> {
        match backend.map(str::trim) {
            Some(FILE_BACKEND) => Some(Self::File {
                master_key: master_key.map(Path::to_path_buf),
            }),
            Some(_) => None,
            None => master_key.map(|path| Self::File {
                master_key: Some(path.to_path_buf()),
            }),
        }
    }

    /// The variables the `exo` child is launched with.
    ///
    /// Additive: these are set on top of the environment the child would have
    /// inherited, so a lane that names a store overrides an ambient one. Never
    /// a value read out of a key file — only the path Exo uses to find it.
    #[must_use]
    pub fn env(&self) -> Vec<(String, String)> {
        let mut env = vec![(
            SECRET_BACKEND_ENV_VAR.to_owned(),
            self.backend_name().to_owned(),
        )];
        if let Self::File {
            master_key: Some(path),
        } = self
        {
            env.push((
                MASTER_KEY_PATH_ENV_VAR.to_owned(),
                path.to_string_lossy().into_owned(),
            ));
        }
        env
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_file_backed_store_names_the_backend_and_the_key() {
        let store = ExoSecretStore::File {
            master_key: Some(PathBuf::from("/home/someone/.config/exo/master.key")),
        };

        assert_eq!(
            store.env(),
            vec![
                ("EXO_SECRET_BACKEND".to_owned(), "file".to_owned()),
                (
                    "EXO_MASTER_KEY_PATH".to_owned(),
                    "/home/someone/.config/exo/master.key".to_owned()
                ),
            ]
        );
    }

    /// `EXO_SECRET_BACKEND=file` alone is a working configuration — Exo finds
    /// the key at its own default path — so a lane that names no path must not
    /// invent one.
    #[test]
    fn a_file_backed_store_with_no_named_key_sets_only_the_backend() {
        let store = ExoSecretStore::File { master_key: None };

        assert_eq!(
            store.env(),
            vec![("EXO_SECRET_BACKEND".to_owned(), "file".to_owned())]
        );
    }

    #[test]
    fn a_backend_exo_does_not_have_is_refused_rather_than_forwarded() {
        assert_eq!(ExoSecretStore::parse(Some("vault"), None), None);
        assert_eq!(ExoSecretStore::parse(Some("apple-keychain"), None), None);
        assert_eq!(ExoSecretStore::parse(Some(""), None), None);
    }

    #[test]
    fn a_named_key_with_no_backend_is_file_backed() {
        assert_eq!(
            ExoSecretStore::parse(None, Some(Path::new("/k"))),
            Some(ExoSecretStore::File {
                master_key: Some(PathBuf::from("/k"))
            })
        );
    }

    #[test]
    fn naming_neither_is_naming_no_store() {
        assert_eq!(ExoSecretStore::parse(None, None), None);
    }

    /// The type's whole claim. If this ever fails, Omega is holding somebody's
    /// provider credential.
    #[test]
    fn a_secret_store_is_a_name_and_never_a_key() {
        let source = include_str!("secret_store.rs");
        let implementation = source
            .split_once("#[cfg(test)]")
            .expect("the tests are separated from the implementation")
            .0;
        for reading in [
            "fs::",
            "File::",
            "read_to_string",
            "OpenOptions",
            "Entry::new",
        ] {
            assert!(
                !implementation.contains(reading),
                "ExoSecretStore grew a way to read the key it names: {reading}"
            );
        }
    }
}
