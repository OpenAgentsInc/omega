use std::{
    fs,
    io::{self, Read, Write},
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};

use nostr::{FromBech32, ToBech32, nips::nip49::EncryptedSecretKey};
use sha2::{Digest, Sha256};
use thiserror::Error;

use crate::{CandidateRef, RecoveryCandidate};

pub(crate) const MAX_RECOVERY_ARTIFACT_BYTES: u64 = 4_096;
static TEMPORARY_ARTIFACT_COUNTER: AtomicU64 = AtomicU64::new(0);

pub(crate) fn discover(path: PathBuf) -> Result<RecoveryCandidate, RecoveryArtifactError> {
    let metadata = secure_metadata(&path)?;
    let path_hash = hex::encode(Sha256::digest(path.to_string_lossy().as_bytes()));
    let candidate_ref = CandidateRef::new(format!("recovery-artifact-{path_hash}"))
        .map_err(|_| RecoveryArtifactError::InvalidArtifact)?;
    Ok(RecoveryCandidate::artifact(
        candidate_ref,
        path,
        metadata.len(),
    ))
}

pub(crate) fn read_encrypted(
    candidate: &RecoveryCandidate,
) -> Result<EncryptedSecretKey, RecoveryArtifactError> {
    let file = secure_open(candidate.path())?;
    let metadata = file.metadata()?;
    validate_metadata(candidate.path(), &metadata)?;
    if metadata.len() != candidate.byte_length() {
        return Err(RecoveryArtifactError::CandidateChanged);
    }

    let capacity =
        usize::try_from(metadata.len()).map_err(|_| RecoveryArtifactError::ArtifactTooLarge)?;
    let mut bytes = Vec::with_capacity(capacity);
    file.take(MAX_RECOVERY_ARTIFACT_BYTES + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() as u64 > MAX_RECOVERY_ARTIFACT_BYTES {
        return Err(RecoveryArtifactError::ArtifactTooLarge);
    }
    let contents =
        std::str::from_utf8(&bytes).map_err(|_| RecoveryArtifactError::InvalidArtifact)?;
    let token = contents
        .strip_suffix("\r\n")
        .or_else(|| contents.strip_suffix('\n'))
        .unwrap_or(&contents);
    if token.is_empty()
        || token
            .bytes()
            .any(|byte| byte.is_ascii_whitespace() || byte == 0)
    {
        return Err(RecoveryArtifactError::InvalidArtifact);
    }
    let encrypted = EncryptedSecretKey::from_bech32(token)
        .map_err(|_| RecoveryArtifactError::InvalidArtifact)?;
    if !(16..=18).contains(&encrypted.log_n()) {
        return Err(RecoveryArtifactError::UnsupportedWorkFactor);
    }
    Ok(encrypted)
}

pub(crate) fn write_encrypted(
    path: &Path,
    encrypted_secret: &EncryptedSecretKey,
) -> Result<u64, RecoveryArtifactError> {
    match fs::symlink_metadata(path) {
        Ok(_) => return Err(RecoveryArtifactError::DestinationExists),
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(error) => return Err(error.into()),
    }
    let parent = path
        .parent()
        .ok_or(RecoveryArtifactError::InvalidDestination)?;
    if parent.as_os_str().is_empty() {
        return Err(RecoveryArtifactError::InvalidDestination);
    }
    let parent_metadata = fs::symlink_metadata(parent)?;
    if !parent_metadata.is_dir() || parent_metadata.file_type().is_symlink() {
        return Err(RecoveryArtifactError::InvalidDestination);
    }

    let encoded = encrypted_secret
        .to_bech32()
        .map_err(|_| RecoveryArtifactError::EncryptionFailed)?;
    let byte_length = encoded.len() as u64 + 1;
    if byte_length > MAX_RECOVERY_ARTIFACT_BYTES {
        return Err(RecoveryArtifactError::ArtifactTooLarge);
    }

    let (temporary_path, mut file) = create_temporary_artifact(parent, path)?;
    let write_result = (|| {
        file.write_all(encoded.as_bytes())?;
        file.write_all(b"\n")?;
        file.sync_all()?;
        validate_metadata(&temporary_path, &file.metadata()?)
    })();
    if let Err(error) = write_result {
        fs::remove_file(&temporary_path)?;
        return Err(error);
    }
    drop(file);

    if let Err(error) = fs::hard_link(&temporary_path, path) {
        fs::remove_file(&temporary_path)?;
        return if error.kind() == io::ErrorKind::AlreadyExists {
            Err(RecoveryArtifactError::DestinationExists)
        } else {
            Err(error.into())
        };
    }
    fs::remove_file(&temporary_path)?;
    validate_metadata(path, &fs::symlink_metadata(path)?)?;
    Ok(byte_length)
}

fn create_temporary_artifact(
    parent: &Path,
    destination: &Path,
) -> Result<(PathBuf, fs::File), RecoveryArtifactError> {
    let file_name = destination
        .file_name()
        .ok_or(RecoveryArtifactError::InvalidDestination)?
        .to_string_lossy();
    for _ in 0..32 {
        let counter = TEMPORARY_ARTIFACT_COUNTER.fetch_add(1, Ordering::Relaxed);
        let temporary_path = parent.join(format!(
            ".{file_name}.omega-recovery-{}-{counter}.tmp",
            std::process::id()
        ));
        let mut options = fs::OpenOptions::new();
        options.create_new(true).write(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt as _;

            options.mode(0o600).custom_flags(libc::O_CLOEXEC);
        }
        match options.open(&temporary_path) {
            Ok(file) => return Ok((temporary_path, file)),
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
            Err(error) => return Err(error.into()),
        }
    }
    Err(RecoveryArtifactError::TemporaryFileUnavailable)
}

fn secure_metadata(path: &Path) -> Result<fs::Metadata, RecoveryArtifactError> {
    let metadata = fs::symlink_metadata(path)?;
    validate_metadata(path, &metadata)?;
    Ok(metadata)
}

fn secure_open(path: &Path) -> Result<fs::File, RecoveryArtifactError> {
    let mut options = fs::OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt as _;
        options.custom_flags(libc::O_CLOEXEC | libc::O_NOFOLLOW);
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt as _;
        use windows_sys::Win32::Storage::FileSystem::FILE_FLAG_OPEN_REPARSE_POINT;

        options.custom_flags(FILE_FLAG_OPEN_REPARSE_POINT);
    }
    Ok(options.open(path)?)
}

fn validate_metadata(path: &Path, metadata: &fs::Metadata) -> Result<(), RecoveryArtifactError> {
    if !metadata.is_file() || metadata.file_type().is_symlink() {
        return Err(RecoveryArtifactError::UnsafeArtifact);
    }
    if metadata.len() == 0 {
        return Err(RecoveryArtifactError::InvalidArtifact);
    }
    if metadata.len() > MAX_RECOVERY_ARTIFACT_BYTES {
        return Err(RecoveryArtifactError::ArtifactTooLarge);
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::{MetadataExt as _, PermissionsExt as _};

        let user_id = unsafe { libc::geteuid() };
        if metadata.permissions().mode() & 0o077 != 0
            || metadata.uid() != user_id
            || metadata.nlink() != 1
        {
            return Err(RecoveryArtifactError::WeakPermissions);
        }
        let path_metadata = fs::symlink_metadata(path)?;
        if path_metadata.dev() != metadata.dev() || path_metadata.ino() != metadata.ino() {
            return Err(RecoveryArtifactError::CandidateChanged);
        }
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt as _;
        use windows_sys::Win32::Storage::FileSystem::FILE_ATTRIBUTE_REPARSE_POINT;

        if metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
            return Err(RecoveryArtifactError::UnsafeArtifact);
        }
    }
    Ok(())
}

#[derive(Debug, Error)]
pub(crate) enum RecoveryArtifactError {
    #[error("the recovery artifact is not a regular protected file")]
    UnsafeArtifact,
    #[error("the recovery artifact has weak permissions")]
    WeakPermissions,
    #[error("the recovery artifact exceeds the size limit")]
    ArtifactTooLarge,
    #[error("the recovery artifact uses an unsupported password work factor")]
    UnsupportedWorkFactor,
    #[error("the recovery artifact is invalid")]
    InvalidArtifact,
    #[error("the recovery artifact changed after discovery")]
    CandidateChanged,
    #[error("the recovery artifact destination already exists")]
    DestinationExists,
    #[error("the recovery artifact destination is invalid")]
    InvalidDestination,
    #[error("a protected temporary recovery artifact could not be created")]
    TemporaryFileUnavailable,
    #[error("recovery artifact encryption failed")]
    EncryptionFailed,
    #[error("recovery artifact I/O failed")]
    Io(#[from] io::Error),
}

#[cfg(test)]
mod tests {
    use super::*;

    const NIP49_VECTOR: &str = "ncryptsec1qgg9947rlpvqu76pj5ecreduf9jxhselq2nae2kghhvd5g7dgjtcxfqtd67p9m0w57lspw8gsq6yphnm8623nsl8xn9j4jdzz84zm3frztj3z7s35vpzmqf6ksu8r89qk5z2zxfmu5gv8th8wclt0h4p";

    fn write_protected(path: &Path, contents: &[u8]) {
        fs::write(path, contents).expect("write recovery artifact fixture");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            fs::set_permissions(path, fs::Permissions::from_mode(0o600))
                .expect("protect recovery artifact fixture");
        }
    }

    #[test]
    fn official_vector_is_bounded_and_canonical() {
        let temporary_directory = tempfile::tempdir().expect("create temporary directory");
        let artifact_path = temporary_directory.path().join("recovery.ncryptsec");
        write_protected(&artifact_path, format!("{NIP49_VECTOR}\n").as_bytes());

        let candidate = discover(artifact_path).expect("discover official recovery vector");
        let encrypted = read_encrypted(&candidate).expect("read official recovery vector");
        assert_eq!(encrypted.log_n(), 16);
        assert_eq!(
            encrypted
                .to_bech32()
                .expect("encode official recovery vector"),
            NIP49_VECTOR
        );
    }

    #[test]
    fn unsafe_or_resource_dangerous_artifacts_are_rejected_before_decryption() {
        let temporary_directory = tempfile::tempdir().expect("create temporary directory");
        let weak_path = temporary_directory.path().join("weak.ncryptsec");
        fs::write(&weak_path, NIP49_VECTOR).expect("write weak artifact fixture");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            fs::set_permissions(&weak_path, fs::Permissions::from_mode(0o644))
                .expect("set weak permissions");
            assert!(matches!(
                discover(weak_path),
                Err(RecoveryArtifactError::WeakPermissions)
            ));
        }

        let expensive_path = temporary_directory.path().join("expensive.ncryptsec");
        let encrypted =
            EncryptedSecretKey::from_bech32(NIP49_VECTOR).expect("parse official recovery vector");
        let mut bytes = encrypted.as_vec();
        bytes[1] = 19;
        let expensive = EncryptedSecretKey::from_slice(&bytes)
            .expect("construct unsupported work-factor fixture")
            .to_bech32()
            .expect("encode unsupported work-factor fixture");
        write_protected(&expensive_path, expensive.as_bytes());
        let candidate = discover(expensive_path).expect("discover expensive artifact");
        assert!(matches!(
            read_encrypted(&candidate),
            Err(RecoveryArtifactError::UnsupportedWorkFactor)
        ));

        let multiline_path = temporary_directory.path().join("multiline.ncryptsec");
        write_protected(
            &multiline_path,
            format!("{NIP49_VECTOR}\n{NIP49_VECTOR}\n").as_bytes(),
        );
        let candidate = discover(multiline_path).expect("discover multiline artifact");
        assert!(matches!(
            read_encrypted(&candidate),
            Err(RecoveryArtifactError::InvalidArtifact)
        ));
    }

    #[cfg(unix)]
    #[test]
    fn symlink_artifacts_are_rejected_without_following_the_target() {
        use std::os::unix::fs::symlink;

        let temporary_directory = tempfile::tempdir().expect("create temporary directory");
        let target_path = temporary_directory.path().join("target.ncryptsec");
        let link_path = temporary_directory.path().join("link.ncryptsec");
        write_protected(&target_path, NIP49_VECTOR.as_bytes());
        symlink(&target_path, &link_path).expect("create recovery artifact symlink");

        assert!(matches!(
            discover(link_path),
            Err(RecoveryArtifactError::UnsafeArtifact)
        ));
        assert_eq!(
            fs::read_to_string(target_path).expect("read unchanged symlink target"),
            NIP49_VECTOR
        );
    }
}
