use std::{
    fs,
    io::{self, Write},
    path::Path,
};

use app_identity::AppChannel;
use atomic_write_file::AtomicWriteFile;
use serde::de::DeserializeOwned;
use thiserror::Error;

use crate::{CompletionRecord, ContractError, IdentityManifest};

#[derive(Debug, Error)]
pub enum PublicStoreError {
    #[error("public identity document violates the Omega contract")]
    Contract(#[from] ContractError),
    #[error("public identity document serialization failed")]
    Serialization(#[from] serde_json::Error),
    #[error("public identity document write failed")]
    Io(#[from] io::Error),
}

pub fn write_identity_manifest(
    path: impl AsRef<Path>,
    manifest: &IdentityManifest,
    channel: AppChannel,
) -> Result<(), PublicStoreError> {
    manifest.validate(channel)?;
    write_public_document(path.as_ref(), manifest)
}

pub fn write_completion_record(
    path: impl AsRef<Path>,
    record: &CompletionRecord,
    manifest: &IdentityManifest,
    channel: AppChannel,
) -> Result<(), PublicStoreError> {
    record.validate_against(manifest, channel)?;
    write_public_document(path.as_ref(), record)
}

pub fn read_identity_manifest(
    path: impl AsRef<Path>,
    channel: AppChannel,
) -> Result<Option<IdentityManifest>, PublicStoreError> {
    let Some(bytes) = read_public_document(path.as_ref())? else {
        return Ok(None);
    };
    let manifest: IdentityManifest = serde_json::from_slice(&bytes)?;
    manifest.validate(channel)?;
    Ok(Some(manifest))
}

pub fn read_completion_record(
    path: impl AsRef<Path>,
    manifest: &IdentityManifest,
    channel: AppChannel,
) -> Result<Option<CompletionRecord>, PublicStoreError> {
    let Some(bytes) = read_public_document(path.as_ref())? else {
        return Ok(None);
    };
    let record: CompletionRecord = serde_json::from_slice(&bytes)?;
    record.validate_against(manifest, channel)?;
    Ok(Some(record))
}

pub(crate) fn read_json_document<T: DeserializeOwned>(
    path: &Path,
) -> Result<Option<T>, PublicStoreError> {
    let Some(bytes) = read_public_document(path)? else {
        return Ok(None);
    };
    Ok(Some(serde_json::from_slice(&bytes)?))
}

pub(crate) fn write_json_document(
    path: &Path,
    value: &impl serde::Serialize,
) -> Result<(), PublicStoreError> {
    write_public_document(path, value)
}

fn read_public_document(path: &Path) -> Result<Option<Vec<u8>>, PublicStoreError> {
    match fs::read(path) {
        Ok(bytes) => Ok(Some(bytes)),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error.into()),
    }
}

pub(crate) fn remove_public_document(path: &Path) -> Result<(), PublicStoreError> {
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error.into()),
    }
}

fn write_public_document(
    path: &Path,
    value: &impl serde::Serialize,
) -> Result<(), PublicStoreError> {
    let serialized = serde_json::to_vec_pretty(value)?;
    if let Some(parent) = path.parent()
        && !parent.as_os_str().is_empty()
    {
        fs::create_dir_all(parent)?;
    }
    let mut file = AtomicWriteFile::open(path)?;
    file.write_all(&serialized)?;
    file.write_all(b"\n")?;
    file.commit()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use serde::ser::Error as _;

    use app_identity::AppChannel;

    use super::*;
    use crate::{IdentityRef, KeyringLocator, PublicIdentity, ReceiptRef};

    const PUBLIC_KEY_HEX: &str = "f86c44a2de95d9149b51c6a29afeabba264c18e2fa7c49de93424a0c56947785";

    #[test]
    fn public_documents_are_written_atomically_and_round_trip() {
        let temporary_directory = tempfile::tempdir().expect("create temporary directory");
        let identity = PublicIdentity::from_public_key_hex(
            IdentityRef::new("omega-test-identity").expect("valid fixture reference"),
            PUBLIC_KEY_HEX,
        )
        .expect("valid fixture public key");
        let receipt_ref = ReceiptRef::new("creation-receipt").expect("valid fixture reference");
        let manifest = IdentityManifest::new(
            identity,
            KeyringLocator::for_channel(AppChannel::Dev),
            vec![receipt_ref.clone()],
        );
        let completion = crate::CompletionRecord::new(&manifest, receipt_ref, AppChannel::Dev)
            .expect("completion record");

        let manifest_path = temporary_directory.path().join("identity.json");
        let completion_path = temporary_directory.path().join("identity.complete.json");
        write_identity_manifest(&manifest_path, &manifest, AppChannel::Dev)
            .expect("write manifest");
        write_completion_record(&completion_path, &completion, &manifest, AppChannel::Dev)
            .expect("write completion");

        let stored_manifest: IdentityManifest =
            serde_json::from_slice(&fs::read(&manifest_path).expect("read stored manifest"))
                .expect("parse stored manifest");
        stored_manifest
            .validate(AppChannel::Dev)
            .expect("stored manifest remains valid");

        let stored_completion: crate::CompletionRecord =
            serde_json::from_slice(&fs::read(&completion_path).expect("read stored completion"))
                .expect("parse stored completion");
        assert_eq!(stored_completion, completion);
        assert_eq!(
            read_identity_manifest(&manifest_path, AppChannel::Dev).expect("read public manifest"),
            Some(manifest.clone())
        );
        assert_eq!(
            read_completion_record(&completion_path, &manifest, AppChannel::Dev)
                .expect("read completion record"),
            Some(completion)
        );
    }

    #[test]
    fn serialization_failure_preserves_existing_document() {
        struct AlwaysFails;

        impl serde::Serialize for AlwaysFails {
            fn serialize<S>(&self, _serializer: S) -> Result<S::Ok, S::Error>
            where
                S: serde::Serializer,
            {
                Err(S::Error::custom("intentional fixture failure"))
            }
        }

        let temporary_directory = tempfile::tempdir().expect("create temporary directory");
        let document_path = temporary_directory.path().join("identity.json");
        fs::write(&document_path, b"existing public document")
            .expect("write existing public document");

        assert!(write_public_document(&document_path, &AlwaysFails).is_err());
        assert_eq!(
            fs::read(document_path).expect("read preserved public document"),
            b"existing public document"
        );
    }
}
