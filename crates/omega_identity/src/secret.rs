use std::fmt;

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
    Conflict,
    Configuration,
}

pub(crate) trait SecretStore: Send + Sync {
    fn read(&self, locator: &KeyringLocator) -> Result<Option<SecretKeyMaterial>, StoreError>;
    fn write(&self, locator: &KeyringLocator, secret: &SecretKeyMaterial)
    -> Result<(), StoreError>;
    fn delete(&self, locator: &KeyringLocator) -> Result<(), StoreError>;
}

pub(crate) struct SystemKeyringStore;

impl SecretStore for SystemKeyringStore {
    fn read(&self, locator: &KeyringLocator) -> Result<Option<SecretKeyMaterial>, StoreError> {
        let entry = keyring_entry(locator)?;
        let bytes = match entry.get_secret() {
            Ok(bytes) => Zeroizing::new(bytes),
            Err(keyring::Error::NoEntry) => return Ok(None),
            Err(error) => return Err(classify_keyring_error(error)),
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
        locator: &KeyringLocator,
        secret: &SecretKeyMaterial,
    ) -> Result<(), StoreError> {
        keyring_entry(locator)?
            .set_secret(secret.0.as_ref())
            .map_err(classify_keyring_error)
    }

    fn delete(&self, locator: &KeyringLocator) -> Result<(), StoreError> {
        match keyring_entry(locator)?.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(error) => Err(classify_keyring_error(error)),
        }
    }
}

fn keyring_entry(locator: &KeyringLocator) -> Result<keyring::Entry, StoreError> {
    keyring::Entry::new(locator.service(), locator.account()).map_err(classify_keyring_error)
}

fn classify_keyring_error(error: keyring::Error) -> StoreError {
    match error {
        keyring::Error::NoStorageAccess(_) => StoreError::Locked,
        keyring::Error::PlatformFailure(_) => StoreError::Unavailable,
        keyring::Error::NoEntry => StoreError::Unavailable,
        keyring::Error::BadEncoding(_) => StoreError::Corrupt,
        keyring::Error::Ambiguous(_) => StoreError::Conflict,
        keyring::Error::TooLong(_, _) | keyring::Error::Invalid(_, _) => StoreError::Configuration,
        _ => StoreError::Unavailable,
    }
}

#[cfg(test)]
mod tests {
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
}
