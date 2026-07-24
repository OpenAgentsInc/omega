use std::{fmt, path::PathBuf};

use serde::{Deserialize, Deserializer, Serialize, de::Error as _};
use zeroize::Zeroizing;

use crate::{PublicIdentity, custody::CustodyError, secret::SecretKeyMaterial};

pub const RECOVERY_PROTECTION_SCHEMA: &str = "openagents.omega.recovery-protection.v1";

#[derive(Clone, PartialEq, Eq, Hash, Serialize)]
#[serde(transparent)]
pub struct CandidateRef(String);

impl CandidateRef {
    pub(crate) fn new(value: String) -> Result<Self, InvalidCandidateRef> {
        if value.is_empty()
            || value.len() > 128
            || !value
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'-' | b'_'))
        {
            return Err(InvalidCandidateRef);
        }
        Ok(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for CandidateRef {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_tuple("CandidateRef")
            .field(&self.0)
            .finish()
    }
}

impl<'de> Deserialize<'de> for CandidateRef {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        Self::new(String::deserialize(deserializer)?).map_err(D::Error::custom)
    }
}

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub(crate) struct InvalidCandidateRef;

impl fmt::Display for InvalidCandidateRef {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("candidate reference must be a non-empty portable identifier")
    }
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum CandidateKind {
    EncryptedRecoveryArtifact,
    AdvancedNostrImport,
}

#[derive(Clone, PartialEq, Eq)]
pub struct RecoveryCandidate {
    candidate_ref: CandidateRef,
    kind: CandidateKind,
    path: PathBuf,
    byte_length: u64,
}

impl fmt::Debug for RecoveryCandidate {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("RecoveryCandidate")
            .field("candidate_ref", &self.candidate_ref)
            .field("kind", &self.kind)
            .field("path", &"[SELECTED PATH]")
            .field("byte_length", &self.byte_length)
            .finish()
    }
}

impl RecoveryCandidate {
    pub(crate) fn artifact(candidate_ref: CandidateRef, path: PathBuf, byte_length: u64) -> Self {
        Self {
            candidate_ref,
            kind: CandidateKind::EncryptedRecoveryArtifact,
            path,
            byte_length,
        }
    }

    pub fn candidate_ref(&self) -> &CandidateRef {
        &self.candidate_ref
    }

    pub fn kind(&self) -> CandidateKind {
        self.kind
    }

    pub fn path(&self) -> &std::path::Path {
        &self.path
    }

    pub fn byte_length(&self) -> u64 {
        self.byte_length
    }
}

pub struct RecoveryPassword(Zeroizing<String>);

impl RecoveryPassword {
    pub fn new(password: String) -> Result<Self, InvalidRecoveryPassword> {
        let password = Zeroizing::new(password);
        if password.is_empty() || password.len() > 1_024 {
            return Err(InvalidRecoveryPassword);
        }
        Ok(Self(password))
    }

    pub(crate) fn as_str(&self) -> &str {
        self.0.as_str()
    }
}

impl fmt::Debug for RecoveryPassword {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("RecoveryPassword([REDACTED])")
    }
}

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub struct InvalidRecoveryPassword;

impl fmt::Display for InvalidRecoveryPassword {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("recovery password must contain between 1 and 1024 bytes")
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RecoveryProtectionRecord {
    schema: String,
    identity: PublicIdentity,
    artifact_digest: String,
    byte_length: u64,
}

impl RecoveryProtectionRecord {
    pub(crate) fn new(
        identity: PublicIdentity,
        artifact_digest: String,
        byte_length: u64,
    ) -> Result<Self, CustodyError> {
        let record = Self {
            schema: RECOVERY_PROTECTION_SCHEMA.to_string(),
            identity,
            artifact_digest,
            byte_length,
        };
        record.validate()?;
        Ok(record)
    }

    pub fn validate(&self) -> Result<(), CustodyError> {
        if self.schema != RECOVERY_PROTECTION_SCHEMA
            || self.byte_length == 0
            || self.artifact_digest.len() != 64
            || !self
                .artifact_digest
                .bytes()
                .all(|byte| byte.is_ascii_digit() || matches!(byte, b'a'..=b'f'))
            || self.identity.validate().is_err()
        {
            return Err(CustodyError::InvalidRecoveryProtection);
        }
        Ok(())
    }

    pub fn identity(&self) -> &PublicIdentity {
        &self.identity
    }

    pub fn artifact_digest(&self) -> &str {
        &self.artifact_digest
    }

    pub fn byte_length(&self) -> u64 {
        self.byte_length
    }
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum RecoveryProtectionState {
    NotApplicable,
    Needed,
    Protected,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RecoveryProtectionStatus {
    pub state: RecoveryProtectionState,
    pub record: Option<RecoveryProtectionRecord>,
}

impl RecoveryProtectionStatus {
    pub(crate) fn not_applicable() -> Self {
        Self {
            state: RecoveryProtectionState::NotApplicable,
            record: None,
        }
    }

    pub(crate) fn needed() -> Self {
        Self {
            state: RecoveryProtectionState::Needed,
            record: None,
        }
    }

    pub(crate) fn protected(record: RecoveryProtectionRecord) -> Self {
        Self {
            state: RecoveryProtectionState::Protected,
            record: Some(record),
        }
    }
}

pub struct PreparedRecovery {
    candidate_ref: CandidateRef,
    kind: CandidateKind,
    identity: PublicIdentity,
    pub(crate) secret: SecretKeyMaterial,
}

pub struct SelectedRecovery(PreparedRecovery);

impl SelectedRecovery {
    pub(crate) fn new(prepared: PreparedRecovery) -> Self {
        Self(prepared)
    }

    pub fn identity(&self) -> &PublicIdentity {
        self.0.identity()
    }

    pub(crate) fn into_prepared(self) -> PreparedRecovery {
        self.0
    }
}

impl fmt::Debug for SelectedRecovery {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("SelectedRecovery")
            .field("candidate_ref", self.0.candidate_ref())
            .field("identity", self.0.identity())
            .field("secret", &"[REDACTED]")
            .finish()
    }
}

impl PreparedRecovery {
    pub(crate) fn new(
        candidate_ref: CandidateRef,
        kind: CandidateKind,
        secret: SecretKeyMaterial,
    ) -> Result<Self, CustodyError> {
        let identity = secret
            .public_identity()
            .map_err(|_| CustodyError::InvalidImportedSecret)?;
        Ok(Self {
            candidate_ref,
            kind,
            identity,
            secret,
        })
    }

    pub fn candidate_ref(&self) -> &CandidateRef {
        &self.candidate_ref
    }

    pub fn kind(&self) -> CandidateKind {
        self.kind
    }

    pub fn identity(&self) -> &PublicIdentity {
        &self.identity
    }
}

impl fmt::Debug for PreparedRecovery {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("PreparedRecovery")
            .field("candidate_ref", &self.candidate_ref)
            .field("kind", &self.kind)
            .field("identity", &self.identity)
            .field("secret", &"[REDACTED]")
            .finish()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RecoveryCandidateSummary {
    pub candidate_ref: CandidateRef,
    pub kind: CandidateKind,
    pub identity: PublicIdentity,
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum RecoveryResolutionState {
    NoCandidates,
    CandidateSelected,
    OwnerSelectionRequired,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RecoveryResolution {
    pub state: RecoveryResolutionState,
    pub candidates: Vec<RecoveryCandidateSummary>,
    pub selected_candidate_ref: Option<CandidateRef>,
}

pub(crate) fn reconcile_prepared(candidates: &[&PreparedRecovery]) -> RecoveryResolution {
    let mut unique: Vec<RecoveryCandidateSummary> = Vec::new();
    for candidate in candidates {
        if unique
            .iter()
            .any(|existing| existing.identity == candidate.identity)
        {
            continue;
        }
        unique.push(RecoveryCandidateSummary {
            candidate_ref: candidate.candidate_ref.clone(),
            kind: candidate.kind,
            identity: candidate.identity.clone(),
        });
    }

    match unique.as_slice() {
        [] => RecoveryResolution {
            state: RecoveryResolutionState::NoCandidates,
            candidates: unique,
            selected_candidate_ref: None,
        },
        [candidate] => RecoveryResolution {
            state: RecoveryResolutionState::CandidateSelected,
            selected_candidate_ref: Some(candidate.candidate_ref.clone()),
            candidates: unique,
        },
        _ => RecoveryResolution {
            state: RecoveryResolutionState::OwnerSelectionRequired,
            candidates: unique,
            selected_candidate_ref: None,
        },
    }
}

#[derive(Clone, PartialEq, Eq)]
pub struct RecoveryArtifactReceipt {
    path: PathBuf,
    identity: PublicIdentity,
    byte_length: u64,
}

impl fmt::Debug for RecoveryArtifactReceipt {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("RecoveryArtifactReceipt")
            .field("path", &"[SELECTED PATH]")
            .field("identity", &self.identity)
            .field("byte_length", &self.byte_length)
            .finish()
    }
}

impl RecoveryArtifactReceipt {
    pub(crate) fn new(path: PathBuf, identity: PublicIdentity, byte_length: u64) -> Self {
        Self {
            path,
            identity,
            byte_length,
        }
    }

    pub fn path(&self) -> &std::path::Path {
        &self.path
    }

    pub fn identity(&self) -> &PublicIdentity {
        &self.identity
    }

    pub fn byte_length(&self) -> u64 {
        self.byte_length
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn passwords_and_selected_paths_are_redacted_from_debug_output() {
        let password = RecoveryPassword::new("sensitive recovery password".to_string())
            .expect("valid recovery password");
        assert_eq!(format!("{password:?}"), "RecoveryPassword([REDACTED])");

        let candidate = RecoveryCandidate::artifact(
            CandidateRef::new("candidate-one".to_string()).expect("valid candidate"),
            PathBuf::from("/Users/example/private/recovery.ncryptsec"),
            163,
        );
        let debug = format!("{candidate:?}");
        assert!(!debug.contains("/Users/example"));
        assert!(debug.contains("[SELECTED PATH]"));
    }
}
