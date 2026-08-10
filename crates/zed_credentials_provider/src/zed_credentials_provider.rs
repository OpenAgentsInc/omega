use std::{
    collections::HashMap,
    fs,
    future::Future,
    io::{self, Write as _},
    path::PathBuf,
    pin::Pin,
    sync::{Arc, Mutex},
};

use anyhow::{Context as _, Result};
use atomic_write_file::AtomicWriteFile;
use credentials_provider::CredentialsProvider;
use futures::FutureExt as _;
use gpui::{App, AsyncApp, Global};
use release_channel::ReleaseChannel;

pub struct ZedCredentialsProvider(pub Arc<dyn CredentialsProvider>);

impl Global for ZedCredentialsProvider {}

/// Returns the global [`CredentialsProvider`].
pub fn init_global(cx: &mut App) {
    // The `CredentialsProvider` trait has `Send + Sync` bounds on it, so it
    // seems like this is a false positive from Clippy.
    #[allow(clippy::arc_with_non_send_sync)]
    let provider = new(cx);
    cx.set_global(ZedCredentialsProvider(provider));
}

pub fn global(cx: &App) -> Arc<dyn CredentialsProvider> {
    cx.try_global::<ZedCredentialsProvider>()
        .map(|provider| provider.0.clone())
        .unwrap_or_else(|| new(cx))
}

/// Returns a channel-namespaced private local credentials provider.
pub fn local_credentials(cx: &App) -> Arc<dyn CredentialsProvider> {
    new(cx)
}

/// Returns the channel-namespaced platform store used only for agent-wallet
/// custody. Callers must keep ordinary runtime credentials on [`global`].
pub fn agent_wallet_credentials(cx: &App) -> Arc<dyn CredentialsProvider> {
    let release_channel =
        ReleaseChannel::try_global(cx).unwrap_or(*release_channel::RELEASE_CHANNEL);
    Arc::new(NamespacedCredentialsProvider {
        namespace: release_channel.credential_namespace(),
        inner: Arc::new(PlatformCredentialsProvider),
    })
}

fn new(cx: &App) -> Arc<dyn CredentialsProvider> {
    let release_channel =
        ReleaseChannel::try_global(cx).unwrap_or(*release_channel::RELEASE_CHANNEL);
    let inner: Arc<dyn CredentialsProvider> = Arc::new(LocalCredentialsProvider::new());

    Arc::new(NamespacedCredentialsProvider {
        namespace: release_channel.credential_namespace(),
        inner,
    })
}

fn namespaced_credential_key(namespace: &str, url: &str) -> String {
    format!("{namespace}:{url}")
}

struct NamespacedCredentialsProvider {
    namespace: &'static str,
    inner: Arc<dyn CredentialsProvider>,
}

impl CredentialsProvider for NamespacedCredentialsProvider {
    fn read_credentials<'a>(
        &'a self,
        url: &'a str,
        cx: &'a AsyncApp,
    ) -> Pin<Box<dyn Future<Output = Result<Option<(String, Vec<u8>)>>> + 'a>> {
        async move {
            let key = namespaced_credential_key(self.namespace, url);
            self.inner.read_credentials(&key, cx).await
        }
        .boxed_local()
    }

    fn write_credentials<'a>(
        &'a self,
        url: &'a str,
        username: &'a str,
        password: &'a [u8],
        cx: &'a AsyncApp,
    ) -> Pin<Box<dyn Future<Output = Result<()>> + 'a>> {
        async move {
            let key = namespaced_credential_key(self.namespace, url);
            self.inner
                .write_credentials(&key, username, password, cx)
                .await
        }
        .boxed_local()
    }

    fn delete_credentials<'a>(
        &'a self,
        url: &'a str,
        cx: &'a AsyncApp,
    ) -> Pin<Box<dyn Future<Output = Result<()>> + 'a>> {
        async move {
            let key = namespaced_credential_key(self.namespace, url);
            self.inner.delete_credentials(&key, cx).await
        }
        .boxed_local()
    }
}

struct PlatformCredentialsProvider;

impl CredentialsProvider for PlatformCredentialsProvider {
    fn read_credentials<'a>(
        &'a self,
        url: &'a str,
        cx: &'a AsyncApp,
    ) -> Pin<Box<dyn Future<Output = Result<Option<(String, Vec<u8>)>>> + 'a>> {
        async move { cx.update(|cx| cx.read_credentials(url)).await }.boxed_local()
    }

    fn write_credentials<'a>(
        &'a self,
        url: &'a str,
        username: &'a str,
        password: &'a [u8],
        cx: &'a AsyncApp,
    ) -> Pin<Box<dyn Future<Output = Result<()>> + 'a>> {
        async move {
            cx.update(move |cx| cx.write_credentials(url, username, password))
                .await
        }
        .boxed_local()
    }

    fn delete_credentials<'a>(
        &'a self,
        url: &'a str,
        cx: &'a AsyncApp,
    ) -> Pin<Box<dyn Future<Output = Result<()>> + 'a>> {
        async move { cx.update(move |cx| cx.delete_credentials(url)).await }.boxed_local()
    }
}

struct LocalCredentialsProvider {
    path: PathBuf,
    access: Mutex<()>,
}

impl LocalCredentialsProvider {
    fn new() -> Self {
        Self::new_at(
            paths::data_dir()
                .join("credentials")
                .join("credentials.json"),
        )
    }

    fn new_at(path: PathBuf) -> Self {
        Self {
            path,
            access: Mutex::new(()),
        }
    }

    fn load_credentials(&self) -> Result<HashMap<String, (String, Vec<u8>)>> {
        let json = match fs::read(&self.path) {
            Ok(json) => json,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(HashMap::new()),
            Err(error) => {
                return Err(error)
                    .with_context(|| format!("failed to read {}", self.path.display()));
            }
        };
        let credentials: HashMap<String, (String, Vec<u8>)> = serde_json::from_slice(&json)?;

        Ok(credentials)
    }

    fn save_credentials(&self, credentials: &HashMap<String, (String, Vec<u8>)>) -> Result<()> {
        let parent = self
            .path
            .parent()
            .context("local credentials path has no parent directory")?;
        fs::create_dir_all(parent)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;

            fs::set_permissions(parent, fs::Permissions::from_mode(0o700))?;
        }

        let json = serde_json::to_vec(credentials)?;
        let mut file = AtomicWriteFile::open(&self.path)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;

            file.as_file()
                .set_permissions(fs::Permissions::from_mode(0o600))?;
        }
        file.write_all(&json)?;
        file.commit()?;

        Ok(())
    }
}

impl CredentialsProvider for LocalCredentialsProvider {
    fn read_credentials<'a>(
        &'a self,
        url: &'a str,
        _cx: &'a AsyncApp,
    ) -> Pin<Box<dyn Future<Output = Result<Option<(String, Vec<u8>)>>> + 'a>> {
        async move {
            let _access = self
                .access
                .lock()
                .map_err(|_| anyhow::anyhow!("local credentials lock is poisoned"))?;
            Ok(self.load_credentials()?.get(url).cloned())
        }
        .boxed_local()
    }

    fn write_credentials<'a>(
        &'a self,
        url: &'a str,
        username: &'a str,
        password: &'a [u8],
        _cx: &'a AsyncApp,
    ) -> Pin<Box<dyn Future<Output = Result<()>> + 'a>> {
        async move {
            let _access = self
                .access
                .lock()
                .map_err(|_| anyhow::anyhow!("local credentials lock is poisoned"))?;
            let mut credentials = self.load_credentials()?;
            credentials.insert(url.to_string(), (username.to_string(), password.to_vec()));

            self.save_credentials(&credentials)
        }
        .boxed_local()
    }

    fn delete_credentials<'a>(
        &'a self,
        url: &'a str,
        _cx: &'a AsyncApp,
    ) -> Pin<Box<dyn Future<Output = Result<()>> + 'a>> {
        async move {
            let _access = self
                .access
                .lock()
                .map_err(|_| anyhow::anyhow!("local credentials lock is poisoned"))?;
            let mut credentials = self.load_credentials()?;
            credentials.remove(url);

            self.save_credentials(&credentials)
        }
        .boxed_local()
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use super::*;
    use tempfile::TempDir;

    #[test]
    fn credential_keys_are_isolated_by_release_channel() {
        let keys = ReleaseChannel::ALL.map(|release_channel| {
            namespaced_credential_key(
                release_channel.credential_namespace(),
                "https://example.com",
            )
        });

        assert_eq!(keys.iter().collect::<HashSet<_>>().len(), 4);
        assert!(
            keys.iter()
                .all(|key| key.starts_with("com.openagents.omega"))
        );
    }

    #[test]
    fn local_credentials_are_private_and_round_trip() {
        let temporary_directory = TempDir::new().expect("create temporary directory");
        let path = temporary_directory
            .path()
            .join("credentials")
            .join("credentials.json");
        let provider = LocalCredentialsProvider::new_at(path.clone());
        let mut credentials = HashMap::new();
        credentials.insert(
            "com.openagents.omega.dev:https://example.com".to_owned(),
            ("account".to_owned(), b"secret".to_vec()),
        );

        provider
            .save_credentials(&credentials)
            .expect("save local credentials");
        assert_eq!(
            provider.load_credentials().expect("load local credentials"),
            credentials
        );

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;

            let directory_mode = fs::metadata(path.parent().expect("credentials parent"))
                .expect("read credentials directory metadata")
                .permissions()
                .mode()
                & 0o777;
            let file_mode = fs::metadata(path)
                .expect("read credentials file metadata")
                .permissions()
                .mode()
                & 0o777;
            assert_eq!(directory_mode, 0o700);
            assert_eq!(file_mode, 0o600);
        }
    }

    #[test]
    fn corrupt_local_credentials_are_not_treated_as_empty() {
        let temporary_directory = TempDir::new().expect("create temporary directory");
        let path = temporary_directory.path().join("credentials.json");
        fs::write(&path, b"not json").expect("write corrupt credentials");
        let provider = LocalCredentialsProvider::new_at(path);

        assert!(provider.load_credentials().is_err());
    }
}
