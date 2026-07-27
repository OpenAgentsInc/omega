use std::{
    fmt, fs,
    io::{self, Write as _},
    path::PathBuf,
};

use atomic_write_file::AtomicWriteFile;
use nostr::{Keys, SecretKey};
use thiserror::Error;
use zeroize::Zeroizing;

use crate::{ContractError, IdentityRef, KeyringLocator, PublicIdentity};

pub struct ImportedSecret(Zeroizing<String>);

impl ImportedSecret {
    pub fn new(secret: String) -> Result<Self, InvalidImportedSecret> {
        let secret = Zeroizing::new(secret);
        Keys::parse(secret.as_str()).map_err(|_| InvalidImportedSecret)?;
        Ok(Self(secret))
    }
}

impl fmt::Debug for ImportedSecret {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("ImportedSecret([REDACTED])")
    }
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, Error)]
#[error("the imported identity secret is invalid")]
pub struct InvalidImportedSecret;

pub(crate) struct SecretKeyMaterial(Zeroizing<[u8; 32]>);

impl SecretKeyMaterial {
    pub(crate) fn generate() -> Self {
        let keys = Keys::generate();
        Self(Zeroizing::new(keys.secret_key().to_secret_bytes()))
    }

    pub(crate) fn from_imported(secret: ImportedSecret) -> Result<Self, SecretMaterialError> {
        let keys = Keys::parse(secret.0.as_str()).map_err(|_| SecretMaterialError)?;
        Ok(Self(Zeroizing::new(keys.secret_key().to_secret_bytes())))
    }

    pub(crate) fn from_bytes(bytes: Zeroizing<[u8; 32]>) -> Result<Self, SecretMaterialError> {
        SecretKey::from_slice(bytes.as_ref()).map_err(|_| SecretMaterialError)?;
        Ok(Self(bytes))
    }

    pub(crate) fn from_secret_key(secret_key: SecretKey) -> Self {
        Self(Zeroizing::new(secret_key.to_secret_bytes()))
    }

    #[cfg(test)]
    pub(crate) fn duplicate(&self) -> Self {
        Self(Zeroizing::new(*self.0))
    }

    pub(crate) fn public_identity(&self) -> Result<PublicIdentity, SecretMaterialError> {
        let keys = self.keys()?;
        let public_key_hex = keys.public_key().to_hex();
        let identity_ref = IdentityRef::new(format!("omega-nostr-{public_key_hex}"))
            .map_err(SecretMaterialError::from)?;
        PublicIdentity::from_public_key_hex(identity_ref, public_key_hex)
            .map_err(SecretMaterialError::from)
    }

    pub(crate) fn keys(&self) -> Result<Keys, SecretMaterialError> {
        let secret_key =
            SecretKey::from_slice(self.0.as_ref()).map_err(|_| SecretMaterialError::invalid())?;
        Ok(Keys::new(secret_key))
    }
}

impl fmt::Debug for SecretKeyMaterial {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("SecretKeyMaterial([REDACTED])")
    }
}

#[derive(Debug, Error)]
#[error("secure identity material is invalid")]
pub(crate) struct SecretMaterialError;

impl SecretMaterialError {
    fn invalid() -> Self {
        Self
    }
}

impl From<ContractError> for SecretMaterialError {
    fn from(_error: ContractError) -> Self {
        Self
    }
}

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub(crate) enum StoreError {
    Locked,
    Unavailable,
    Corrupt,
}

pub(crate) trait SecretStore: Send + Sync {
    fn read(&self, locator: &KeyringLocator) -> Result<Option<SecretKeyMaterial>, StoreError>;
    fn write(&self, locator: &KeyringLocator, secret: &SecretKeyMaterial)
    -> Result<(), StoreError>;
    fn delete(&self, locator: &KeyringLocator) -> Result<(), StoreError>;
}

pub(crate) struct FileSecretStore {
    path: PathBuf,
}

impl FileSecretStore {
    pub(crate) fn new(path: PathBuf) -> Self {
        Self { path }
    }

    fn classify_io_error(error: io::Error) -> StoreError {
        match error.kind() {
            io::ErrorKind::PermissionDenied => StoreError::Locked,
            io::ErrorKind::InvalidData => StoreError::Corrupt,
            _ => StoreError::Unavailable,
        }
    }
}

impl SecretStore for FileSecretStore {
    fn read(&self, _locator: &KeyringLocator) -> Result<Option<SecretKeyMaterial>, StoreError> {
        let bytes = match fs::read(&self.path) {
            Ok(bytes) => Zeroizing::new(bytes),
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(Self::classify_io_error(error)),
        };
        let secret_bytes: [u8; 32] = bytes
            .as_slice()
            .try_into()
            .map_err(|_| StoreError::Corrupt)?;
        SecretKeyMaterial::from_bytes(Zeroizing::new(secret_bytes))
            .map(Some)
            .map_err(|_| StoreError::Corrupt)
    }

    fn write(
        &self,
        _locator: &KeyringLocator,
        secret: &SecretKeyMaterial,
    ) -> Result<(), StoreError> {
        if let Some(parent) = self.path.parent() {
            fs::create_dir_all(parent).map_err(Self::classify_io_error)?;
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt as _;

                fs::set_permissions(parent, fs::Permissions::from_mode(0o700))
                    .map_err(Self::classify_io_error)?;
            }
        }
        let mut file = AtomicWriteFile::open(&self.path).map_err(Self::classify_io_error)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;

            file.as_file()
                .set_permissions(fs::Permissions::from_mode(0o600))
                .map_err(Self::classify_io_error)?;
        }
        file.write_all(secret.0.as_ref())
            .map_err(Self::classify_io_error)?;
        file.commit().map_err(Self::classify_io_error)
    }

    fn delete(&self, _locator: &KeyringLocator) -> Result<(), StoreError> {
        match fs::remove_file(&self.path) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(Self::classify_io_error(error)),
        }
    }
}

#[cfg(test)]
mod tests {
    use tempfile::TempDir;

    use super::*;

    #[test]
    fn secret_debug_output_is_redacted() {
        let secret =
            SecretKeyMaterial::from_bytes(Zeroizing::new([42; 32])).expect("valid test secret");
        assert_eq!(format!("{secret:?}"), "SecretKeyMaterial([REDACTED])");
        assert!(!format!("{secret:?}").contains("42"));
    }

    #[test]
    fn imported_secret_debug_output_is_redacted() {
        let imported = ImportedSecret::new(
            "0000000000000000000000000000000000000000000000000000000000000001".to_string(),
        )
        .expect("valid imported test secret");
        assert_eq!(format!("{imported:?}"), "ImportedSecret([REDACTED])");
    }

    #[test]
    fn file_store_round_trips_and_deletes_a_secret() {
        let temporary_directory = TempDir::new().expect("create temporary directory");
        let path = temporary_directory.path().join("identity");
        let store = FileSecretStore::new(path.clone());
        let locator = KeyringLocator::for_channel(app_identity::AppChannel::Dev);
        let secret =
            SecretKeyMaterial::from_bytes(Zeroizing::new([42; 32])).expect("valid test secret");

        assert!(store.read(&locator).expect("read absent secret").is_none());
        store.write(&locator, &secret).expect("write secret");
        let restored = store
            .read(&locator)
            .expect("read secret")
            .expect("stored secret");
        assert_eq!(
            restored
                .public_identity()
                .expect("derive restored public identity"),
            secret
                .public_identity()
                .expect("derive original public identity")
        );

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;

            let mode = fs::metadata(&path)
                .expect("read secret metadata")
                .permissions()
                .mode()
                & 0o777;
            assert_eq!(mode, 0o600);
        }

        store.delete(&locator).expect("delete secret");
        assert!(store.read(&locator).expect("read deleted secret").is_none());
    }
}
