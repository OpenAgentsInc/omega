use std::fmt;

use zeroize::Zeroizing;

use crate::KeyringLocator;

#[allow(dead_code)]
pub(crate) struct SecretKeyMaterial(Zeroizing<[u8; 32]>);

#[allow(dead_code)]
impl SecretKeyMaterial {
    pub(crate) fn new(bytes: Zeroizing<[u8; 32]>) -> Self {
        Self(bytes)
    }
}

impl fmt::Debug for SecretKeyMaterial {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("SecretKeyMaterial([REDACTED])")
    }
}

#[allow(dead_code)]
pub(crate) fn keyring_entry(locator: &KeyringLocator) -> Result<keyring::Entry, keyring::Error> {
    keyring::Entry::new(locator.service(), locator.account())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn secret_debug_output_is_redacted() {
        let secret = SecretKeyMaterial::new(Zeroizing::new([42; 32]));
        assert_eq!(format!("{secret:?}"), "SecretKeyMaterial([REDACTED])");
        assert!(!format!("{secret:?}").contains("42"));
    }
}
