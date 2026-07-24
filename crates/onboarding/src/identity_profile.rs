use std::{
    fs,
    io::{self, Read as _, Write as _},
    path::PathBuf,
};

use omega_identity::IdentityRef;
use serde::{Deserialize, Deserializer, Serialize, de::Error as _};
use thiserror::Error;
use uuid::Uuid;

const LOCAL_IDENTITY_PROFILE_VERSION: u32 = 1;
const DISPLAY_NAME_MAX_CHARACTERS: usize = 80;
const LOCAL_AVATAR_PREFIX: &str = "local-avatar:";
const LOCAL_AVATAR_TOKEN_MAX_BYTES: usize = 128;
const LOCAL_AVATAR_MAX_BYTES: u64 = 8 * 1024 * 1024;
const KVP_KEY_PREFIX: &str = "omega.identity-profile.v1";

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct LocalIdentityProfile {
    version: u32,
    identity_ref: IdentityRef,
    #[serde(skip_serializing_if = "Option::is_none")]
    display_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    avatar_reference: Option<String>,
}

impl LocalIdentityProfile {
    pub(crate) fn new(identity_ref: IdentityRef) -> Self {
        Self {
            version: LOCAL_IDENTITY_PROFILE_VERSION,
            identity_ref,
            display_name: None,
            avatar_reference: None,
        }
    }

    pub(crate) fn identity_ref(&self) -> &IdentityRef {
        &self.identity_ref
    }

    pub(crate) fn display_name(&self) -> Option<&str> {
        self.display_name.as_deref()
    }

    pub(crate) fn avatar_reference(&self) -> Option<&str> {
        self.avatar_reference.as_deref()
    }

    pub(crate) fn set_display_name(
        &mut self,
        display_name: Option<String>,
    ) -> Result<(), LocalIdentityProfileError> {
        validate_display_name(display_name.as_deref())?;
        self.display_name = display_name;
        Ok(())
    }

    pub(crate) fn set_avatar_reference(
        &mut self,
        avatar_reference: Option<String>,
    ) -> Result<(), LocalIdentityProfileError> {
        validate_avatar_reference(avatar_reference.as_deref())?;
        self.avatar_reference = avatar_reference;
        Ok(())
    }

    pub(crate) fn canonical_json(&self) -> Result<String, LocalIdentityProfileError> {
        Ok(serde_json::to_string(self)?)
    }

    pub(crate) fn from_canonical_json(
        json: &str,
        expected_identity_ref: &IdentityRef,
    ) -> Result<Self, LocalIdentityProfileError> {
        let profile: Self = serde_json::from_str(json)?;
        if profile.identity_ref() != expected_identity_ref {
            return Err(LocalIdentityProfileError::IdentityMismatch);
        }
        Ok(profile)
    }

    pub(crate) fn kvp_key(identity_ref: &IdentityRef) -> String {
        format!("{KVP_KEY_PREFIX}.{}", identity_ref.as_str())
    }
}

pub(crate) fn install_local_avatar(source: PathBuf) -> Result<String, LocalIdentityProfileError> {
    let metadata = fs::symlink_metadata(&source)?;
    if !metadata.is_file()
        || metadata.file_type().is_symlink()
        || metadata.len() == 0
        || metadata.len() > LOCAL_AVATAR_MAX_BYTES
    {
        return Err(LocalIdentityProfileError::InvalidAvatarFile);
    }
    let extension = source
        .extension()
        .and_then(|extension| extension.to_str())
        .map(str::to_ascii_lowercase)
        .filter(|extension| matches!(extension.as_str(), "png" | "jpg" | "jpeg" | "webp"))
        .ok_or(LocalIdentityProfileError::InvalidAvatarFile)?;
    let token = format!("{}.{}", Uuid::new_v4().simple(), extension);
    let directory = paths::data_dir().join("identity").join("profile-avatars");
    fs::create_dir_all(&directory)?;
    let destination = directory.join(&token);

    let mut source_file = fs::File::open(source)?;
    let mut destination_file = fs::OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(&destination)?;
    let copy_result = (|| {
        let mut limited = (&mut source_file).take(LOCAL_AVATAR_MAX_BYTES + 1);
        let byte_length = io::copy(&mut limited, &mut destination_file)?;
        if byte_length == 0 || byte_length > LOCAL_AVATAR_MAX_BYTES {
            return Err(LocalIdentityProfileError::InvalidAvatarFile);
        }
        destination_file.flush()?;
        destination_file.sync_all()?;
        Ok(())
    })();
    if let Err(error) = copy_result {
        if let Err(remove_error) = fs::remove_file(&destination) {
            zlog::error!(
                "failed to remove incomplete local avatar {}: {remove_error}",
                destination.display()
            );
        }
        return Err(error);
    }
    Ok(format!("{LOCAL_AVATAR_PREFIX}{token}"))
}

impl<'de> Deserialize<'de> for LocalIdentityProfile {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct LocalIdentityProfileWire {
            version: u32,
            identity_ref: IdentityRef,
            #[serde(default)]
            display_name: Option<String>,
            #[serde(default)]
            avatar_reference: Option<String>,
        }

        let wire = LocalIdentityProfileWire::deserialize(deserializer)?;
        if wire.version != LOCAL_IDENTITY_PROFILE_VERSION {
            return Err(D::Error::custom(
                "unsupported local identity profile version",
            ));
        }
        validate_display_name(wire.display_name.as_deref()).map_err(D::Error::custom)?;
        validate_avatar_reference(wire.avatar_reference.as_deref()).map_err(D::Error::custom)?;
        Ok(Self {
            version: wire.version,
            identity_ref: wire.identity_ref,
            display_name: wire.display_name,
            avatar_reference: wire.avatar_reference,
        })
    }
}

fn validate_display_name(display_name: Option<&str>) -> Result<(), LocalIdentityProfileError> {
    let Some(display_name) = display_name else {
        return Ok(());
    };
    if display_name.is_empty()
        || display_name.trim() != display_name
        || display_name.chars().count() > DISPLAY_NAME_MAX_CHARACTERS
        || display_name.chars().any(char::is_control)
    {
        return Err(LocalIdentityProfileError::InvalidDisplayName);
    }
    Ok(())
}

fn validate_avatar_reference(
    avatar_reference: Option<&str>,
) -> Result<(), LocalIdentityProfileError> {
    let Some(avatar_reference) = avatar_reference else {
        return Ok(());
    };
    let Some(token) = avatar_reference.strip_prefix(LOCAL_AVATAR_PREFIX) else {
        return Err(LocalIdentityProfileError::InvalidAvatarReference);
    };
    if token.is_empty()
        || token.len() > LOCAL_AVATAR_TOKEN_MAX_BYTES
        || !token
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
    {
        return Err(LocalIdentityProfileError::InvalidAvatarReference);
    }
    Ok(())
}

#[derive(Debug, Error)]
pub(crate) enum LocalIdentityProfileError {
    #[error("the local display name is invalid")]
    InvalidDisplayName,
    #[error("the local avatar reference is invalid")]
    InvalidAvatarReference,
    #[error("the local profile belongs to another identity")]
    IdentityMismatch,
    #[error("the selected local avatar is invalid")]
    InvalidAvatarFile,
    #[error("local avatar storage is unavailable")]
    Io(#[from] io::Error),
    #[error("the local identity profile is invalid")]
    Serialization(#[from] serde_json::Error),
}

#[cfg(test)]
mod tests {
    use serde_json::Value;

    use super::*;

    fn identity_ref(value: &str) -> IdentityRef {
        IdentityRef::new(value).expect("valid identity reference")
    }

    #[test]
    fn canonical_profile_round_trips() {
        let identity_ref = identity_ref("omega-test-identity");
        let mut profile = LocalIdentityProfile::new(identity_ref.clone());
        profile
            .set_display_name(Some("Ada Lovelace".to_string()))
            .expect("valid display name");
        profile
            .set_avatar_reference(Some("local-avatar:ada_01".to_string()))
            .expect("valid avatar reference");

        let json = profile.canonical_json().expect("serialize local profile");
        assert_eq!(
            json,
            r#"{"version":1,"identity_ref":"omega-test-identity","display_name":"Ada Lovelace","avatar_reference":"local-avatar:ada_01"}"#
        );
        assert_eq!(
            LocalIdentityProfile::from_canonical_json(&json, &identity_ref)
                .expect("parse identity-bound profile"),
            profile
        );
    }

    #[test]
    fn profile_and_kvp_key_are_bound_to_the_public_identity() {
        let first_identity = identity_ref("omega-first-identity");
        let second_identity = identity_ref("omega-second-identity");
        let profile = LocalIdentityProfile::new(first_identity.clone());
        let json = profile.canonical_json().expect("serialize local profile");

        assert!(matches!(
            LocalIdentityProfile::from_canonical_json(&json, &second_identity),
            Err(LocalIdentityProfileError::IdentityMismatch)
        ));
        assert_ne!(
            LocalIdentityProfile::kvp_key(&first_identity),
            LocalIdentityProfile::kvp_key(&second_identity)
        );
        assert_eq!(
            LocalIdentityProfile::kvp_key(&first_identity),
            "omega.identity-profile.v1.omega-first-identity"
        );
    }

    #[test]
    fn display_name_enforces_canonical_bounds_and_control_character_rules() {
        let mut profile = LocalIdentityProfile::new(identity_ref("omega-test-identity"));
        profile
            .set_display_name(Some("a".repeat(DISPLAY_NAME_MAX_CHARACTERS)))
            .expect("maximum display name is accepted");

        for invalid in [
            String::new(),
            " leading".to_string(),
            "trailing ".to_string(),
            "line\nbreak".to_string(),
            "a".repeat(DISPLAY_NAME_MAX_CHARACTERS + 1),
        ] {
            assert!(matches!(
                profile.set_display_name(Some(invalid)),
                Err(LocalIdentityProfileError::InvalidDisplayName)
            ));
        }
    }

    #[test]
    fn avatar_reference_is_an_opaque_bounded_local_token() {
        let mut profile = LocalIdentityProfile::new(identity_ref("omega-test-identity"));
        profile
            .set_avatar_reference(Some(format!(
                "{LOCAL_AVATAR_PREFIX}{}",
                "a".repeat(LOCAL_AVATAR_TOKEN_MAX_BYTES)
            )))
            .expect("maximum local avatar token is accepted");

        for invalid in [
            "https://example.com/avatar.png".to_string(),
            "local-avatar:".to_string(),
            "local-avatar:../avatar.png".to_string(),
            "local-avatar:line\nbreak".to_string(),
            format!(
                "{LOCAL_AVATAR_PREFIX}{}",
                "a".repeat(LOCAL_AVATAR_TOKEN_MAX_BYTES + 1)
            ),
        ] {
            assert!(matches!(
                profile.set_avatar_reference(Some(invalid)),
                Err(LocalIdentityProfileError::InvalidAvatarReference)
            ));
        }
    }

    #[test]
    fn serialized_profile_contains_only_local_public_presentation_fields() {
        let mut profile = LocalIdentityProfile::new(identity_ref("omega-test-identity"));
        profile
            .set_display_name(Some("Ada".to_string()))
            .expect("valid display name");
        profile
            .set_avatar_reference(Some("local-avatar:ada".to_string()))
            .expect("valid avatar reference");
        let value: Value =
            serde_json::from_str(&profile.canonical_json().expect("serialize local profile"))
                .expect("parse local profile JSON");
        let fields = value.as_object().expect("profile is a JSON object");

        assert_eq!(
            fields.keys().map(String::as_str).collect::<Vec<_>>(),
            [
                "version",
                "identity_ref",
                "display_name",
                "avatar_reference"
            ]
        );
        for forbidden in [
            "kind",
            "kind_0",
            "relay",
            "signing",
            "signature",
            "secret",
            "nsec",
            "private_key",
            "seed",
            "mnemonic",
        ] {
            assert!(!fields.contains_key(forbidden));
        }
    }

    #[test]
    fn deserialization_rejects_unknown_or_invalid_fields() {
        let identity_ref = identity_ref("omega-test-identity");
        for json in [
            r#"{"version":2,"identity_ref":"omega-test-identity"}"#,
            r#"{"version":1,"identity_ref":"omega-test-identity","relay":"wss://example.com"}"#,
            r#"{"version":1,"identity_ref":"omega-test-identity","display_name":"line\nbreak"}"#,
            r#"{"version":1,"identity_ref":"omega-test-identity","avatar_reference":"https://example.com/avatar.png"}"#,
        ] {
            assert!(LocalIdentityProfile::from_canonical_json(json, &identity_ref).is_err());
        }
    }
}
