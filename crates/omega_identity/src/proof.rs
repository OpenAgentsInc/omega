use std::{
    fs,
    path::{Component, Path, PathBuf},
};

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::{
    AdmittedSigningRequest, CustodyError, CustodyResult, IdentityInspection, IdentityRef,
    IdentityService, ReceiptRef, RecoveryPassword, SigningResult,
};

pub const IDENTITY_PROOF_PROTOCOL: &str = "openagents.omega.identity-proof.v1";
pub const IDENTITY_PROOF_KEYRING_SERVICE: &str = "com.openagents.omega.identity-proof.v1";
pub const IDENTITY_PROOF_KEYRING_ACCOUNT: &str = "disposable-proof-only";
pub const IDENTITY_PROOF_ROOT_PREFIX: &str = "omega-identity-proof-";
const PROOF_SENTINEL: &str = ".omega-identity-proof-v1.json";
const PROOF_RECOVERY_ARTIFACT: &str = "disposable-recovery.ncryptsec";
const PROOF_CORRUPT_RECOVERY_ARTIFACT: &str = "disposable-corrupt-recovery.ncryptsec";

#[derive(Debug, Copy, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ProofCrashBoundary {
    AfterSecretWrite,
    AfterSecretReadBack,
    AfterManifestCommit,
    AfterResetMarker,
    AfterResetCommit,
    AfterRelaunchAcknowledge,
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ProofSafeScenario {
    ConflictCustody,
    LostCustody,
    LockedCustody,
    SymlinkRefusal,
    WeakPermissionRefusal,
    KeychainUnavailable,
    CorruptKeychain,
    MalformedEventRejection,
    UnadmittedPurposeRejection,
    ConflictingRecoverySelection,
    LateCompletionFencing,
    SignerCrashBeforeCompletion,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ProofSafeScenarioResult {
    pub scenario: ProofSafeScenario,
    pub mode: &'static str,
    pub expected_outcome: &'static str,
    pub production_locator_access: &'static str,
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum ProofRecoveryRejection {
    WrongPassword,
    CorruptArtifact,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ProofRecoveryRejectionResult {
    pub scenario: ProofRecoveryRejection,
    pub expected_outcome: &'static str,
    pub production_locator_access: &'static str,
}

#[derive(Debug, Error)]
pub enum IdentityProofError {
    #[error(
        "proof root must be an absolute, normalized path whose final component starts with omega-identity-proof-"
    )]
    UnsafeRoot,
    #[error("proof root or one of its existing ancestors is a symbolic link")]
    SymbolicLink,
    #[error("proof root is not initialized with the exact proof namespace sentinel")]
    MissingSentinel,
    #[error("proof root initialization failed")]
    Io(#[from] std::io::Error),
    #[error("a recovery input expected to be rejected was admitted")]
    UnexpectedRecoveryAdmission,
    #[error(transparent)]
    Custody(#[from] CustodyError),
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ProofSentinel {
    protocol: String,
    keyring_service: String,
    keyring_account: String,
}

pub struct IdentityProofService {
    root: PathBuf,
    service: IdentityService,
}

impl IdentityProofService {
    pub fn initialize(root: &Path) -> Result<(), IdentityProofError> {
        validate_root_shape(root)?;
        reject_symbolic_links(root)?;
        fs::create_dir(root)?;
        let sentinel = ProofSentinel {
            protocol: IDENTITY_PROOF_PROTOCOL.to_string(),
            keyring_service: IDENTITY_PROOF_KEYRING_SERVICE.to_string(),
            keyring_account: IDENTITY_PROOF_KEYRING_ACCOUNT.to_string(),
        };
        fs::write(
            root.join(PROOF_SENTINEL),
            serde_json::to_vec_pretty(&sentinel)
                .map_err(|_| IdentityProofError::MissingSentinel)?,
        )?;
        Ok(())
    }

    pub fn open(
        root: PathBuf,
        crash_boundary: Option<ProofCrashBoundary>,
    ) -> Result<Self, IdentityProofError> {
        validate_root_shape(&root)?;
        reject_symbolic_links(&root)?;
        let sentinel: ProofSentinel = serde_json::from_slice(&fs::read(root.join(PROOF_SENTINEL))?)
            .map_err(|_| IdentityProofError::MissingSentinel)?;
        if sentinel.protocol != IDENTITY_PROOF_PROTOCOL
            || sentinel.keyring_service != IDENTITY_PROOF_KEYRING_SERVICE
            || sentinel.keyring_account != IDENTITY_PROOF_KEYRING_ACCOUNT
        {
            return Err(IdentityProofError::MissingSentinel);
        }
        Ok(Self {
            service: IdentityService::for_disposable_proof(root.clone(), crash_boundary),
            root,
        })
    }

    pub fn inspect(&self) -> Result<IdentityInspection, IdentityProofError> {
        Ok(self.service.inspect_details()?)
    }

    pub fn inspect_for_process_start(&self) -> Result<IdentityInspection, IdentityProofError> {
        Ok(self.service.inspect_for_process_start()?)
    }

    pub fn create(&self, receipt: ReceiptRef) -> Result<CustodyResult, IdentityProofError> {
        Ok(self.service.create(receipt)?)
    }

    pub fn resume_create(&self) -> Result<CustodyResult, IdentityProofError> {
        Ok(self.service.resume_incomplete_create()?)
    }

    pub fn sign(
        &self,
        request: &AdmittedSigningRequest,
    ) -> Result<SigningResult, IdentityProofError> {
        Ok(self.service.sign(request)?)
    }

    pub fn reset(
        &self,
        identity: &IdentityRef,
        receipt: ReceiptRef,
    ) -> Result<CustodyResult, IdentityProofError> {
        Ok(self.service.reset(identity, receipt)?)
    }

    pub fn resume_reset(&self) -> Result<CustodyResult, IdentityProofError> {
        Ok(self.service.resume_pending_reset()?)
    }

    pub fn protect_recovery(
        &self,
        identity: &IdentityRef,
        password: RecoveryPassword,
    ) -> Result<IdentityInspection, IdentityProofError> {
        self.service.export_recovery_artifact(
            identity,
            &self.root.join(PROOF_RECOVERY_ARTIFACT),
            password,
        )?;
        Ok(self.service.inspect_details()?)
    }

    pub fn recover(
        &self,
        password: RecoveryPassword,
        receipt: ReceiptRef,
    ) -> Result<IdentityInspection, IdentityProofError> {
        let candidate = self
            .service
            .discover_recovery_artifact(self.root.join(PROOF_RECOVERY_ARTIFACT))?;
        let prepared = self
            .service
            .prepare_recovery_artifact(&candidate, password)?;
        let candidate_ref = prepared.candidate_ref().clone();
        let selected = self
            .service
            .select_recovery(vec![prepared], &candidate_ref)?;
        self.service.adopt(selected, receipt)?;
        Ok(self.service.inspect_details()?)
    }

    pub fn probe_wrong_recovery_password(
        &self,
        password: RecoveryPassword,
    ) -> Result<ProofRecoveryRejectionResult, IdentityProofError> {
        let candidate = self
            .service
            .discover_recovery_artifact(self.root.join(PROOF_RECOVERY_ARTIFACT))?;
        match self.service.prepare_recovery_artifact(&candidate, password) {
            Err(CustodyError::RecoveryDecryptionFailed) => Ok(ProofRecoveryRejectionResult {
                scenario: ProofRecoveryRejection::WrongPassword,
                expected_outcome: "recovery-decryption-rejected",
                production_locator_access: "rejected-by-construction",
            }),
            Err(error) => Err(error.into()),
            Ok(_) => Err(IdentityProofError::UnexpectedRecoveryAdmission),
        }
    }

    pub fn probe_corrupt_recovery_artifact(
        &self,
        password: RecoveryPassword,
    ) -> Result<ProofRecoveryRejectionResult, IdentityProofError> {
        let path = self.root.join(PROOF_CORRUPT_RECOVERY_ARTIFACT);
        write_corrupt_recovery_artifact(&path)?;
        let candidate = self.service.discover_recovery_artifact(path)?;
        match self.service.prepare_recovery_artifact(&candidate, password) {
            Err(CustodyError::InvalidRecoveryArtifact) => Ok(ProofRecoveryRejectionResult {
                scenario: ProofRecoveryRejection::CorruptArtifact,
                expected_outcome: "invalid-recovery-artifact-rejected",
                production_locator_access: "rejected-by-construction",
            }),
            Err(error) => Err(error.into()),
            Ok(_) => Err(IdentityProofError::UnexpectedRecoveryAdmission),
        }
    }

    pub fn simulate_safe_scenario(&self, scenario: ProofSafeScenario) -> ProofSafeScenarioResult {
        let expected_outcome = match scenario {
            ProofSafeScenario::ConflictCustody
            | ProofSafeScenario::LostCustody
            | ProofSafeScenario::LockedCustody => "custody-denied",
            ProofSafeScenario::SymlinkRefusal | ProofSafeScenario::WeakPermissionRefusal => {
                "unsafe-public-store-rejected"
            }
            ProofSafeScenario::KeychainUnavailable | ProofSafeScenario::CorruptKeychain => {
                "secure-store-error"
            }
            ProofSafeScenario::MalformedEventRejection
            | ProofSafeScenario::UnadmittedPurposeRejection => "signing-request-rejected",
            ProofSafeScenario::ConflictingRecoverySelection => "owner-selection-required",
            ProofSafeScenario::LateCompletionFencing => "stale-completion-rejected",
            ProofSafeScenario::SignerCrashBeforeCompletion => "no-completion-committed",
        };
        ProofSafeScenarioResult {
            scenario,
            mode: "deterministic-no-keychain-simulation",
            expected_outcome,
            production_locator_access: "rejected-by-construction",
        }
    }
}

fn write_corrupt_recovery_artifact(path: &Path) -> Result<(), std::io::Error> {
    use std::io::Write as _;

    let mut options = fs::OpenOptions::new();
    options.create_new(true).write(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt as _;

        options.mode(0o600).custom_flags(libc::O_CLOEXEC);
    }
    let mut file = options.open(path)?;
    file.write_all(b"not-a-valid-nip49-recovery-artifact\n")?;
    file.sync_all()
}

fn validate_root_shape(root: &Path) -> Result<(), IdentityProofError> {
    if !root.is_absolute()
        || root
            .components()
            .any(|component| matches!(component, Component::CurDir | Component::ParentDir))
        || !root
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| {
                name.starts_with(IDENTITY_PROOF_ROOT_PREFIX)
                    && name.len() > IDENTITY_PROOF_ROOT_PREFIX.len()
            })
    {
        return Err(IdentityProofError::UnsafeRoot);
    }
    Ok(())
}

fn reject_symbolic_links(root: &Path) -> Result<(), IdentityProofError> {
    let mut path = PathBuf::new();
    for component in root.components() {
        path.push(component.as_os_str());
        match fs::symlink_metadata(&path) {
            Ok(metadata) if metadata.file_type().is_symlink() => {
                return Err(IdentityProofError::SymbolicLink);
            }
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => break,
            Err(error) => return Err(error.into()),
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_production_and_ambiguous_roots() {
        for root in [
            Path::new("/tmp/Omega RC"),
            Path::new("/tmp/omega-identity-proof-"),
            Path::new("relative/omega-identity-proof-test"),
        ] {
            assert!(matches!(
                IdentityProofService::initialize(root),
                Err(IdentityProofError::UnsafeRoot)
            ));
        }
    }

    #[test]
    fn sentinel_is_exact_and_required() {
        let temporary = tempfile::tempdir().expect("temporary directory");
        let root = temporary
            .path()
            .canonicalize()
            .expect("canonical temporary directory")
            .join("omega-identity-proof-test");
        IdentityProofService::initialize(&root).expect("initialize proof root");
        IdentityProofService::open(root.clone(), None).expect("open exact proof root");
        fs::write(root.join(PROOF_SENTINEL), b"{}").expect("replace sentinel");
        assert!(matches!(
            IdentityProofService::open(root, None),
            Err(IdentityProofError::MissingSentinel)
        ));
    }

    #[cfg(unix)]
    #[test]
    fn rejects_symbolic_link_root() {
        use std::os::unix::fs::symlink;
        let temporary = tempfile::tempdir().expect("temporary directory");
        let temporary_root = temporary
            .path()
            .canonicalize()
            .expect("canonical temporary directory");
        let actual = temporary_root.join("omega-identity-proof-actual");
        fs::create_dir(&actual).expect("create actual root");
        let linked = temporary_root.join("omega-identity-proof-linked");
        symlink(actual, &linked).expect("create symlink");
        assert!(matches!(
            IdentityProofService::initialize(&linked),
            Err(IdentityProofError::SymbolicLink)
        ));
    }

    #[test]
    fn safe_scenarios_are_explicit_and_never_claim_live_keychain_execution() {
        let scenarios = [
            ProofSafeScenario::ConflictCustody,
            ProofSafeScenario::LostCustody,
            ProofSafeScenario::LockedCustody,
            ProofSafeScenario::SymlinkRefusal,
            ProofSafeScenario::WeakPermissionRefusal,
            ProofSafeScenario::KeychainUnavailable,
            ProofSafeScenario::CorruptKeychain,
            ProofSafeScenario::MalformedEventRejection,
            ProofSafeScenario::UnadmittedPurposeRejection,
            ProofSafeScenario::ConflictingRecoverySelection,
            ProofSafeScenario::LateCompletionFencing,
            ProofSafeScenario::SignerCrashBeforeCompletion,
        ];
        let temporary = tempfile::tempdir().expect("temporary directory");
        let root = temporary
            .path()
            .canonicalize()
            .expect("canonical temporary directory")
            .join("omega-identity-proof-scenarios");
        IdentityProofService::initialize(&root).expect("initialize proof root");
        let service = IdentityProofService::open(root, None).expect("open proof root");
        for scenario in scenarios {
            let result = service.simulate_safe_scenario(scenario);
            assert_eq!(result.scenario, scenario);
            assert_eq!(result.mode, "deterministic-no-keychain-simulation");
            assert_eq!(result.production_locator_access, "rejected-by-construction");
            assert!(!result.expected_outcome.is_empty());
        }
    }
}
