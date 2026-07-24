use std::{
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};

use app_identity::AppChannel;
use nostr::{
    JsonUtil,
    nips::nip49::{EncryptedSecretKey, KeySecurity},
};
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::{
    AdmittedSigningRequest, CompletionRecord, ContractError, CustodyResult, CustodyState,
    IdentityManifest, IdentityRef, ImportedSecret, KeyringLocator, PublicIdentity,
    PublicStoreError, ReceiptRef, SigningResult,
    mutation_lock::{IdentityMutationGuard, MutationLockError},
    public_store::{
        read_completion_record, read_identity_manifest, read_json_document, remove_public_document,
        write_completion_record, write_identity_manifest, write_json_document,
    },
    recovery::{
        CandidateKind, CandidateRef, PreparedRecovery, RecoveryArtifactReceipt, RecoveryCandidate,
        RecoveryPassword, RecoveryResolution, SelectedRecovery, reconcile_prepared,
    },
    recovery_artifact::{self, RecoveryArtifactError},
    secret::{SecretKeyMaterial, SecretStore, StoreError, SystemKeyringStore},
};

const IDENTITY_TRANSACTION_SCHEMA: &str = "openagents.omega.identity-transaction.v1";
const RESET_MARKER_SCHEMA: &str = "openagents.omega.identity-reset.v1";
const NIP49_LOG_N: u8 = 16;
static RECOVERY_KDF_LOCK: Mutex<()> = Mutex::new(());

pub struct IdentityService {
    channel: AppChannel,
    locator: KeyringLocator,
    paths: CustodyPaths,
    store: Arc<dyn SecretStore>,
    generator: Arc<dyn SecretGenerator>,
}

impl IdentityService {
    pub fn system(channel: AppChannel) -> Self {
        Self::new(
            channel,
            CustodyPaths::for_data_root(paths::data_dir().join("identity")),
            Arc::new(SystemKeyringStore),
            Arc::new(SystemSecretGenerator),
        )
    }

    pub fn inspect(&self) -> Result<CustodyResult, CustodyError> {
        let _mutation_guard = IdentityMutationGuard::acquire(&self.locator)?;
        if let Some(result) = self.resume_reset_if_pending_locked() {
            return Ok(result);
        }
        Ok(self.resolve_locked().result)
    }

    pub fn open(&self, expected_identity: &IdentityRef) -> Result<CustodyResult, CustodyError> {
        let _mutation_guard = IdentityMutationGuard::acquire(&self.locator)?;
        self.require_no_reset_locked()?;
        let mut result = self.resolve_locked().result;
        if let Some(identity) = &result.identity
            && identity.identity_ref() != expected_identity
        {
            result.state = CustodyState::Conflict;
        }
        Ok(result)
    }

    pub fn create(&self, receipt_ref: ReceiptRef) -> Result<CustodyResult, CustodyError> {
        let _mutation_guard = IdentityMutationGuard::acquire(&self.locator)?;
        self.require_no_reset_locked()?;
        if let Some(result) = self.ready_idempotent_result_locked(&receipt_ref, None)? {
            return Ok(result);
        }

        let transaction = self.begin_or_resume_transaction_locked(
            TransactionOperation::Create,
            receipt_ref,
            None,
            None,
        )?;
        let secret = match self
            .store
            .read(&self.locator)
            .map_err(custody_error_from_store)?
        {
            Some(secret) => secret,
            None => self.generator.generate(),
        };
        self.commit_secret_locked(secret, transaction)
    }

    pub fn import(
        &self,
        imported_secret: ImportedSecret,
        receipt_ref: ReceiptRef,
    ) -> Result<CustodyResult, CustodyError> {
        let prepared = self.prepare_import(imported_secret)?;
        let candidate_ref = prepared.candidate_ref().clone();
        let selected = self.select_recovery(vec![prepared], &candidate_ref)?;
        self.adopt(selected, receipt_ref)
    }

    pub fn prepare_import(
        &self,
        imported_secret: ImportedSecret,
    ) -> Result<PreparedRecovery, CustodyError> {
        let secret = SecretKeyMaterial::from_imported(imported_secret)
            .map_err(|_| CustodyError::InvalidImportedSecret)?;
        let candidate_ref = CandidateRef::new("advanced-nostr-import".to_string())
            .map_err(|_| CustodyError::InvalidRecoveryCandidate)?;
        PreparedRecovery::new(candidate_ref, CandidateKind::AdvancedNostrImport, secret)
    }

    pub fn discover_recovery_artifact(
        &self,
        path: PathBuf,
    ) -> Result<RecoveryCandidate, CustodyError> {
        recovery_artifact::discover(path).map_err(CustodyError::from)
    }

    pub fn prepare_recovery_artifact(
        &self,
        candidate: &RecoveryCandidate,
        password: RecoveryPassword,
    ) -> Result<PreparedRecovery, CustodyError> {
        let encrypted = recovery_artifact::read_encrypted(candidate)?;
        let _kdf_guard = RECOVERY_KDF_LOCK
            .lock()
            .map_err(|_| CustodyError::RecoveryDecryptionFailed)?;
        let secret_key = encrypted
            .decrypt(password.as_str())
            .map_err(|_| CustodyError::RecoveryDecryptionFailed)?;
        PreparedRecovery::new(
            candidate.candidate_ref().clone(),
            CandidateKind::EncryptedRecoveryArtifact,
            SecretKeyMaterial::from_secret_key(secret_key),
        )
    }

    pub fn reconcile_recoveries(&self, candidates: &[&PreparedRecovery]) -> RecoveryResolution {
        reconcile_prepared(candidates)
    }

    pub fn select_recovery(
        &self,
        mut candidates: Vec<PreparedRecovery>,
        selected_candidate_ref: &CandidateRef,
    ) -> Result<SelectedRecovery, CustodyError> {
        if candidates.is_empty() {
            return Err(CustodyError::InvalidRecoveryCandidate);
        }
        for (index, candidate) in candidates.iter().enumerate() {
            if candidates[..index]
                .iter()
                .any(|previous| previous.candidate_ref() == candidate.candidate_ref())
            {
                return Err(CustodyError::InvalidRecoveryCandidate);
            }
        }
        let selected_index = candidates
            .iter()
            .position(|candidate| candidate.candidate_ref() == selected_candidate_ref)
            .ok_or(CustodyError::InvalidRecoveryCandidate)?;
        let selected = candidates.swap_remove(selected_index);
        let resolution = reconcile_prepared(
            &std::iter::once(&selected)
                .chain(candidates.iter())
                .collect::<Vec<_>>(),
        );
        if resolution.candidates.is_empty() {
            return Err(CustodyError::InvalidRecoveryCandidate);
        }
        Ok(SelectedRecovery::new(selected))
    }

    pub fn adopt(
        &self,
        selected: SelectedRecovery,
        receipt_ref: ReceiptRef,
    ) -> Result<CustodyResult, CustodyError> {
        let prepared = selected.into_prepared();
        let _mutation_guard = IdentityMutationGuard::acquire(&self.locator)?;
        self.require_no_reset_locked()?;
        if let Some(result) =
            self.ready_idempotent_result_locked(&receipt_ref, Some(prepared.identity()))?
        {
            return Ok(result);
        }

        let transaction = self.begin_or_resume_transaction_locked(
            TransactionOperation::Import,
            receipt_ref,
            Some(prepared.candidate_ref().clone()),
            Some(prepared.identity()),
        )?;
        if let Some(expected_identity) = &transaction.expected_identity
            && expected_identity != prepared.identity()
        {
            return Err(CustodyError::CustodyDenied(CustodyState::Conflict));
        }
        self.commit_secret_locked(prepared.secret, transaction)
    }

    pub fn export_recovery_artifact(
        &self,
        expected_identity: &IdentityRef,
        path: &Path,
        password: RecoveryPassword,
    ) -> Result<RecoveryArtifactReceipt, CustodyError> {
        let _mutation_guard = IdentityMutationGuard::acquire(&self.locator)?;
        self.require_no_reset_locked()?;
        let resolved = self.resolve_locked();
        if resolved.result.state != CustodyState::Ready {
            return Err(CustodyError::CustodyDenied(resolved.result.state));
        }
        let identity = resolved
            .result
            .identity
            .ok_or(CustodyError::CustodyDenied(CustodyState::Incomplete))?;
        if identity.identity_ref() != expected_identity {
            return Err(CustodyError::CustodyDenied(CustodyState::Conflict));
        }
        let secret = resolved
            .secret
            .ok_or(CustodyError::CustodyDenied(CustodyState::Incomplete))?;
        let keys = secret
            .keys()
            .map_err(|_| CustodyError::CustodyDenied(CustodyState::Incomplete))?;
        let _kdf_guard = RECOVERY_KDF_LOCK
            .lock()
            .map_err(|_| CustodyError::RecoveryEncryptionFailed)?;
        let encrypted = EncryptedSecretKey::new(
            keys.secret_key(),
            password.as_str(),
            NIP49_LOG_N,
            KeySecurity::Unknown,
        )
        .map_err(|_| CustodyError::RecoveryEncryptionFailed)?;
        let byte_length = recovery_artifact::write_encrypted(path, &encrypted)?;
        Ok(RecoveryArtifactReceipt::new(
            path.to_path_buf(),
            identity,
            byte_length,
        ))
    }

    pub fn sign(&self, request: &AdmittedSigningRequest) -> Result<SigningResult, CustodyError> {
        let _mutation_guard = IdentityMutationGuard::acquire(&self.locator)?;
        self.require_no_reset_locked()?;
        let resolved = self.resolve_locked();
        if resolved.result.state != CustodyState::Ready {
            return Err(CustodyError::CustodyDenied(resolved.result.state));
        }
        let identity = resolved
            .result
            .identity
            .ok_or(CustodyError::CustodyDenied(CustodyState::Incomplete))?;
        let secret = resolved
            .secret
            .ok_or(CustodyError::CustodyDenied(CustodyState::Incomplete))?;
        let unsigned_event = request.unsigned_event(&identity)?;
        let event = unsigned_event
            .sign_with_keys(
                &secret
                    .keys()
                    .map_err(|_| CustodyError::CustodyDenied(CustodyState::Incomplete))?,
            )
            .map_err(|_| CustodyError::SigningFailed)?;
        let signed_event_json = event
            .try_as_json()
            .map_err(|_| CustodyError::SigningFailed)?;

        Ok(SigningResult {
            request_ref: request.request_ref.clone(),
            identity,
            event_id: event.id.to_hex(),
            signature: event.sig.to_string(),
            signed_event_json,
        })
    }

    pub fn reset(
        &self,
        expected_identity: &IdentityRef,
        authorization_ref: ReceiptRef,
    ) -> Result<CustodyResult, CustodyError> {
        let _mutation_guard = IdentityMutationGuard::acquire(&self.locator)?;
        if let Some(marker) = self.read_reset_marker_locked()? {
            if marker.expected_identity != *expected_identity
                || marker.authorization_ref != authorization_ref
            {
                return Err(CustodyError::CustodyDenied(CustodyState::Conflict));
            }
            return Ok(marker.result());
        }
        let resolved = self.resolve_locked();
        if let Some(identity) = &resolved.result.identity {
            if identity.identity_ref() != expected_identity {
                return Err(CustodyError::CustodyDenied(CustodyState::Conflict));
            }
        } else {
            return Err(CustodyError::CustodyDenied(resolved.result.state));
        }

        let marker = ResetMarker::pending(expected_identity.clone(), authorization_ref);
        write_json_document(&self.paths.reset_path, &marker)?;
        Ok(marker.result())
    }

    pub fn resume_pending_reset(&self) -> Result<CustodyResult, CustodyError> {
        let _mutation_guard = IdentityMutationGuard::acquire(&self.locator)?;
        self.resume_reset_if_pending_locked()
            .ok_or(CustodyError::CustodyDenied(CustodyState::Absent))
    }

    pub fn acknowledge_relaunch(&self) -> Result<CustodyResult, CustodyError> {
        let _mutation_guard = IdentityMutationGuard::acquire(&self.locator)?;
        let marker = self
            .read_reset_marker_locked()?
            .ok_or(CustodyError::CustodyDenied(CustodyState::Absent))?;
        if marker.status != ResetStatus::Complete
            || self
                .store
                .read(&self.locator)
                .map_err(custody_error_from_store)?
                .is_some()
            || read_identity_manifest(&self.paths.manifest_path, self.channel)?.is_some()
            || self
                .paths
                .completion_path
                .try_exists()
                .map_err(|_| CustodyError::ResetFailed)?
            || self
                .paths
                .transaction_path
                .try_exists()
                .map_err(|_| CustodyError::ResetFailed)?
        {
            return Err(CustodyError::ResetFailed);
        }
        remove_public_document(&self.paths.reset_path)?;
        Ok(CustodyResult {
            state: CustodyState::Absent,
            identity: None,
            receipt_ref: Some(marker.authorization_ref),
        })
    }

    fn new(
        channel: AppChannel,
        paths: CustodyPaths,
        store: Arc<dyn SecretStore>,
        generator: Arc<dyn SecretGenerator>,
    ) -> Self {
        Self {
            channel,
            locator: KeyringLocator::for_channel(channel),
            paths,
            store,
            generator,
        }
    }

    fn require_no_reset_locked(&self) -> Result<(), CustodyError> {
        if let Some(marker) = self.read_reset_marker_locked()? {
            return Err(CustodyError::CustodyDenied(marker.result().state));
        }
        Ok(())
    }

    fn ready_idempotent_result_locked(
        &self,
        receipt_ref: &ReceiptRef,
        expected_identity: Option<&PublicIdentity>,
    ) -> Result<Option<CustodyResult>, CustodyError> {
        let result = self.resolve_locked().result;
        if result.state != CustodyState::Ready {
            return Ok(None);
        }
        if let Some(expected_identity) = expected_identity {
            if result.identity.as_ref() != Some(expected_identity) {
                return Err(CustodyError::CustodyDenied(CustodyState::Conflict));
            }
            if result.receipt_ref.as_ref() != Some(receipt_ref) {
                return Err(CustodyError::CustodyDenied(CustodyState::Conflict));
            }
            if let Some(transaction) = self.read_transaction_locked()? {
                if transaction.expected_identity.as_ref() != result.identity.as_ref() {
                    return Err(CustodyError::CustodyDenied(CustodyState::Conflict));
                }
                remove_public_document(&self.paths.transaction_path)?;
            }
            return Ok(Some(result));
        }
        if result.receipt_ref.as_ref() == Some(receipt_ref) {
            if let Some(transaction) = self.read_transaction_locked()? {
                if transaction.operation != TransactionOperation::Create
                    || &transaction.receipt_ref != receipt_ref
                    || transaction.expected_identity.as_ref() != result.identity.as_ref()
                {
                    return Err(CustodyError::CustodyDenied(CustodyState::Conflict));
                }
                remove_public_document(&self.paths.transaction_path)?;
            }
            return Ok(Some(result));
        }
        Ok(None)
    }

    fn begin_or_resume_transaction_locked(
        &self,
        operation: TransactionOperation,
        receipt_ref: ReceiptRef,
        candidate_ref: Option<CandidateRef>,
        expected_identity: Option<&PublicIdentity>,
    ) -> Result<IdentityTransaction, CustodyError> {
        if let Some(transaction) = self.read_transaction_locked()? {
            if transaction.operation != operation
                || transaction.receipt_ref != receipt_ref
                || transaction.candidate_ref != candidate_ref
            {
                return Err(CustodyError::CustodyDenied(CustodyState::Conflict));
            }
            return Ok(transaction);
        }

        let result = self.resolve_locked().result;
        let can_recover = operation == TransactionOperation::Import
            && matches!(result.state, CustodyState::Lost | CustodyState::Incomplete)
            && result.identity.as_ref() == expected_identity;
        if result.state != CustodyState::Absent && !can_recover {
            return Err(CustodyError::CustodyDenied(result.state));
        }
        let mut transaction = IdentityTransaction::new(operation, receipt_ref, candidate_ref);
        if can_recover {
            transaction.expected_identity = expected_identity.cloned();
        }
        write_json_document(&self.paths.transaction_path, &transaction)?;
        Ok(transaction)
    }

    fn commit_secret_locked(
        &self,
        secret: SecretKeyMaterial,
        mut transaction: IdentityTransaction,
    ) -> Result<CustodyResult, CustodyError> {
        let expected_identity = secret
            .public_identity()
            .map_err(|_| CustodyError::InvalidImportedSecret)?;
        if let Some(transaction_identity) = &transaction.expected_identity
            && transaction_identity != &expected_identity
        {
            return Err(CustodyError::CustodyDenied(CustodyState::Conflict));
        }
        let rollback_on_failure = transaction.expected_identity.is_none();
        let mut write_attempted = false;
        let mut resumed_existing_secret = false;

        let result = (|| {
            match self
                .store
                .read(&self.locator)
                .map_err(custody_error_from_store)?
            {
                Some(stored_secret) => {
                    resumed_existing_secret = true;
                    let stored_identity = stored_secret
                        .public_identity()
                        .map_err(|_| CustodyError::ReadBackMismatch)?;
                    if stored_identity != expected_identity {
                        return Err(CustodyError::CustodyDenied(CustodyState::Conflict));
                    }
                }
                None => {
                    write_attempted = true;
                    self.store
                        .write(&self.locator, &secret)
                        .map_err(custody_error_from_store)?;
                }
            }

            let read_back = self
                .store
                .read(&self.locator)
                .map_err(custody_error_from_store)?
                .ok_or(CustodyError::ReadBackMismatch)?;
            let read_back_identity = read_back
                .public_identity()
                .map_err(|_| CustodyError::ReadBackMismatch)?;
            if read_back_identity != expected_identity {
                return Err(CustodyError::ReadBackMismatch);
            }

            transaction.expected_identity = Some(expected_identity.clone());
            write_json_document(&self.paths.transaction_path, &transaction)?;
            let manifest = IdentityManifest::new(
                expected_identity.clone(),
                self.locator.clone(),
                vec![transaction.receipt_ref.clone()],
            );
            write_identity_manifest(&self.paths.manifest_path, &manifest, self.channel)?;
            let completion =
                CompletionRecord::new(&manifest, transaction.receipt_ref.clone(), self.channel)?;
            write_completion_record(
                &self.paths.completion_path,
                &completion,
                &manifest,
                self.channel,
            )?;

            Ok(CustodyResult {
                state: CustodyState::Ready,
                identity: Some(expected_identity),
                receipt_ref: Some(transaction.receipt_ref.clone()),
            })
        })();

        match result {
            Ok(result) => {
                remove_public_document(&self.paths.transaction_path)?;
                Ok(result)
            }
            Err(error) if rollback_on_failure && write_attempted => {
                self.rollback_transaction_locked()?;
                Err(error)
            }
            Err(error) if rollback_on_failure && resumed_existing_secret => Err(error),
            Err(error) if rollback_on_failure => {
                remove_public_document(&self.paths.transaction_path)?;
                Err(error)
            }
            Err(error) => Err(error),
        }
    }

    fn rollback_transaction_locked(&self) -> Result<(), CustodyError> {
        self.store
            .delete(&self.locator)
            .map_err(|_| CustodyError::TransactionIncomplete)?;
        match self.store.read(&self.locator) {
            Ok(None) => {}
            Ok(Some(_)) | Err(_) => return Err(CustodyError::TransactionIncomplete),
        }
        remove_public_document(&self.paths.completion_path)
            .map_err(|_| CustodyError::TransactionIncomplete)?;
        remove_public_document(&self.paths.manifest_path)
            .map_err(|_| CustodyError::TransactionIncomplete)?;
        remove_public_document(&self.paths.transaction_path)
            .map_err(|_| CustodyError::TransactionIncomplete)
    }

    fn read_transaction_locked(&self) -> Result<Option<IdentityTransaction>, CustodyError> {
        let transaction: Option<IdentityTransaction> =
            read_json_document(&self.paths.transaction_path)
                .map_err(|_| CustodyError::TransactionIncomplete)?;
        if transaction
            .as_ref()
            .is_some_and(|transaction| !transaction.is_valid())
        {
            return Err(CustodyError::TransactionIncomplete);
        }
        Ok(transaction)
    }

    fn read_reset_marker_locked(&self) -> Result<Option<ResetMarker>, CustodyError> {
        let marker: Option<ResetMarker> =
            read_json_document(&self.paths.reset_path).map_err(|_| CustodyError::ResetFailed)?;
        if marker.as_ref().is_some_and(|marker| !marker.is_valid()) {
            return Err(CustodyError::ResetFailed);
        }
        Ok(marker)
    }

    fn resume_reset_if_pending_locked(&self) -> Option<CustodyResult> {
        let marker = match self.read_reset_marker_locked() {
            Ok(Some(marker)) => marker,
            Ok(None) => return None,
            Err(_) => return Some(CustodyResult::for_state(CustodyState::ResetFailed)),
        };
        if marker.status == ResetStatus::Complete {
            return Some(marker.result());
        }
        Some(match self.complete_reset_locked(marker.clone()) {
            Ok(()) => ResetMarker {
                status: ResetStatus::Complete,
                ..marker
            }
            .result(),
            Err(_) => {
                let failed_marker = ResetMarker {
                    status: ResetStatus::Failed,
                    ..marker
                };
                if write_json_document(&self.paths.reset_path, &failed_marker).is_err() {
                    return Some(CustodyResult::for_state(CustodyState::ResetFailed));
                }
                CustodyResult::for_state(CustodyState::ResetFailed)
            }
        })
    }

    fn complete_reset_locked(&self, mut marker: ResetMarker) -> Result<(), CustodyState> {
        match self.store.read(&self.locator) {
            Ok(Some(secret)) => {
                let identity = secret
                    .public_identity()
                    .map_err(|_| CustodyState::ResetFailed)?;
                if identity.identity_ref() != &marker.expected_identity {
                    return Err(CustodyState::Conflict);
                }
            }
            Ok(None) => {}
            Err(_) => return Err(CustodyState::ResetFailed),
        }
        self.store
            .delete(&self.locator)
            .map_err(|_| CustodyState::ResetFailed)?;
        match self.store.read(&self.locator) {
            Ok(None) => {}
            Ok(Some(_)) | Err(_) => return Err(CustodyState::ResetFailed),
        }
        remove_public_document(&self.paths.completion_path)
            .map_err(|_| CustodyState::ResetFailed)?;
        remove_public_document(&self.paths.manifest_path).map_err(|_| CustodyState::ResetFailed)?;
        remove_public_document(&self.paths.transaction_path)
            .map_err(|_| CustodyState::ResetFailed)?;
        marker.status = ResetStatus::Complete;
        write_json_document(&self.paths.reset_path, &marker).map_err(|_| CustodyState::ResetFailed)
    }

    fn resolve_locked(&self) -> ResolvedCustody {
        let secret = match self.store.read(&self.locator) {
            Ok(secret) => secret,
            Err(StoreError::Locked | StoreError::Unavailable) => {
                return ResolvedCustody::without_secret(
                    CustodyState::Locked,
                    self.best_effort_manifest_identity(),
                );
            }
            Err(StoreError::Conflict) => {
                return ResolvedCustody::without_secret(
                    CustodyState::Conflict,
                    self.best_effort_manifest_identity(),
                );
            }
            Err(StoreError::Corrupt | StoreError::Configuration) => {
                return ResolvedCustody::without_secret(
                    CustodyState::Incomplete,
                    self.best_effort_manifest_identity(),
                );
            }
        };
        let transaction = match self.read_transaction_locked() {
            Ok(transaction) => transaction,
            Err(_) => {
                return ResolvedCustody::without_secret(CustodyState::Incomplete, None);
            }
        };
        let manifest = match read_identity_manifest(&self.paths.manifest_path, self.channel) {
            Ok(manifest) => manifest,
            Err(_) => {
                return ResolvedCustody::without_secret(CustodyState::Incomplete, None);
            }
        };

        match (manifest, secret) {
            (None, None) => match transaction {
                Some(transaction) => ResolvedCustody::without_secret(
                    CustodyState::Incomplete,
                    transaction.expected_identity,
                ),
                None => ResolvedCustody::without_secret(CustodyState::Absent, None),
            },
            (Some(manifest), None) => {
                let state = if transaction.as_ref().is_some_and(|transaction| {
                    transaction
                        .expected_identity
                        .as_ref()
                        .is_some_and(|identity| identity != manifest.identity())
                }) {
                    CustodyState::Conflict
                } else {
                    CustodyState::Lost
                };
                ResolvedCustody::without_secret(state, Some(manifest.identity().clone()))
            }
            (None, Some(secret)) => match secret.public_identity() {
                Ok(identity) => ResolvedCustody {
                    result: CustodyResult {
                        state: if transaction.as_ref().is_some_and(|transaction| {
                            transaction
                                .expected_identity
                                .as_ref()
                                .is_some_and(|expected| expected != &identity)
                        }) {
                            CustodyState::Conflict
                        } else {
                            CustodyState::Incomplete
                        },
                        identity: Some(identity),
                        receipt_ref: None,
                    },
                    secret: Some(secret),
                },
                Err(_) => ResolvedCustody::without_secret(CustodyState::Incomplete, None),
            },
            (Some(manifest), Some(secret)) => {
                let identity = match secret.public_identity() {
                    Ok(identity) => identity,
                    Err(_) => {
                        return ResolvedCustody::without_secret(
                            CustodyState::Incomplete,
                            Some(manifest.identity().clone()),
                        );
                    }
                };
                if &identity != manifest.identity() {
                    return ResolvedCustody::without_secret(
                        CustodyState::Conflict,
                        Some(manifest.identity().clone()),
                    );
                }
                if transaction.as_ref().is_some_and(|transaction| {
                    transaction.expected_identity.as_ref() != Some(&identity)
                }) {
                    return ResolvedCustody::without_secret(CustodyState::Conflict, Some(identity));
                }

                match read_completion_record(&self.paths.completion_path, &manifest, self.channel) {
                    Ok(Some(completion)) => ResolvedCustody {
                        result: CustodyResult {
                            state: CustodyState::Ready,
                            identity: Some(identity),
                            receipt_ref: Some(completion.receipt_ref().clone()),
                        },
                        secret: Some(secret),
                    },
                    Ok(None) | Err(_) => ResolvedCustody {
                        result: CustodyResult {
                            state: CustodyState::Incomplete,
                            identity: Some(identity),
                            receipt_ref: None,
                        },
                        secret: Some(secret),
                    },
                }
            }
        }
    }

    fn best_effort_manifest_identity(&self) -> Option<PublicIdentity> {
        read_identity_manifest(&self.paths.manifest_path, self.channel)
            .ok()
            .flatten()
            .map(|manifest| manifest.identity().clone())
    }
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
enum TransactionOperation {
    Create,
    Import,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct IdentityTransaction {
    schema: String,
    operation: TransactionOperation,
    receipt_ref: ReceiptRef,
    candidate_ref: Option<CandidateRef>,
    expected_identity: Option<PublicIdentity>,
}

impl IdentityTransaction {
    fn new(
        operation: TransactionOperation,
        receipt_ref: ReceiptRef,
        candidate_ref: Option<CandidateRef>,
    ) -> Self {
        Self {
            schema: IDENTITY_TRANSACTION_SCHEMA.to_string(),
            operation,
            receipt_ref,
            candidate_ref,
            expected_identity: None,
        }
    }

    fn is_valid(&self) -> bool {
        self.schema == IDENTITY_TRANSACTION_SCHEMA
            && matches!(
                (self.operation, self.candidate_ref.is_some()),
                (TransactionOperation::Create, false) | (TransactionOperation::Import, true)
            )
            && self
                .expected_identity
                .as_ref()
                .is_none_or(|identity| identity.validate().is_ok())
    }
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
enum ResetStatus {
    Pending,
    Failed,
    Complete,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ResetMarker {
    schema: String,
    expected_identity: IdentityRef,
    authorization_ref: ReceiptRef,
    status: ResetStatus,
}

impl ResetMarker {
    fn pending(expected_identity: IdentityRef, authorization_ref: ReceiptRef) -> Self {
        Self {
            schema: RESET_MARKER_SCHEMA.to_string(),
            expected_identity,
            authorization_ref,
            status: ResetStatus::Pending,
        }
    }

    fn is_valid(&self) -> bool {
        self.schema == RESET_MARKER_SCHEMA
    }

    fn result(&self) -> CustodyResult {
        CustodyResult {
            state: match self.status {
                ResetStatus::Failed => CustodyState::ResetFailed,
                ResetStatus::Pending | ResetStatus::Complete => CustodyState::RelaunchRequired,
            },
            identity: None,
            receipt_ref: Some(self.authorization_ref.clone()),
        }
    }
}

impl CustodyResult {
    fn for_state(state: CustodyState) -> Self {
        Self {
            state,
            identity: None,
            receipt_ref: None,
        }
    }
}

struct ResolvedCustody {
    result: CustodyResult,
    secret: Option<SecretKeyMaterial>,
}

impl ResolvedCustody {
    fn without_secret(state: CustodyState, identity: Option<PublicIdentity>) -> Self {
        Self {
            result: CustodyResult {
                state,
                identity,
                receipt_ref: None,
            },
            secret: None,
        }
    }
}

#[derive(Clone)]
struct CustodyPaths {
    manifest_path: PathBuf,
    completion_path: PathBuf,
    transaction_path: PathBuf,
    reset_path: PathBuf,
}

impl CustodyPaths {
    fn for_data_root(root: PathBuf) -> Self {
        Self {
            manifest_path: root.join("identity.json"),
            completion_path: root.join("identity.complete.json"),
            transaction_path: root.join("identity.transaction.json"),
            reset_path: root.join("identity.reset.json"),
        }
    }
}

trait SecretGenerator: Send + Sync {
    fn generate(&self) -> SecretKeyMaterial;
}

struct SystemSecretGenerator;

impl SecretGenerator for SystemSecretGenerator {
    fn generate(&self) -> SecretKeyMaterial {
        SecretKeyMaterial::generate()
    }
}

#[derive(Debug, Error)]
pub enum CustodyError {
    #[error("identity custody is unavailable in state {0:?}")]
    CustodyDenied(CustodyState),
    #[error("secure identity storage is unavailable")]
    SecureStoreUnavailable,
    #[error("secure identity read-back verification failed")]
    ReadBackMismatch,
    #[error("the imported identity secret is invalid")]
    InvalidImportedSecret,
    #[error("the recovery candidate is invalid")]
    InvalidRecoveryCandidate,
    #[error("the encrypted recovery artifact is invalid or unsafe")]
    InvalidRecoveryArtifact,
    #[error("the recovery artifact destination already exists")]
    RecoveryArtifactExists,
    #[error("recovery artifact storage is unavailable")]
    RecoveryArtifactUnavailable,
    #[error("recovery artifact encryption failed")]
    RecoveryEncryptionFailed,
    #[error("the recovery password or artifact is invalid")]
    RecoveryDecryptionFailed,
    #[error("identity signing failed")]
    SigningFailed,
    #[error("the identity transaction is incomplete")]
    TransactionIncomplete,
    #[error("identity reset could not be verified")]
    ResetFailed,
    #[error("identity mutation serialization is unavailable")]
    MutationLock,
    #[error("identity contract validation failed")]
    Contract(#[from] ContractError),
    #[error("public identity state could not be committed")]
    PublicStore(#[from] PublicStoreError),
}

impl From<MutationLockError> for CustodyError {
    fn from(_: MutationLockError) -> Self {
        Self::MutationLock
    }
}

impl From<RecoveryArtifactError> for CustodyError {
    fn from(error: RecoveryArtifactError) -> Self {
        match error {
            RecoveryArtifactError::DestinationExists => Self::RecoveryArtifactExists,
            RecoveryArtifactError::Io(_) | RecoveryArtifactError::TemporaryFileUnavailable => {
                Self::RecoveryArtifactUnavailable
            }
            RecoveryArtifactError::UnsafeArtifact
            | RecoveryArtifactError::WeakPermissions
            | RecoveryArtifactError::ArtifactTooLarge
            | RecoveryArtifactError::UnsupportedWorkFactor
            | RecoveryArtifactError::InvalidArtifact
            | RecoveryArtifactError::CandidateChanged
            | RecoveryArtifactError::InvalidDestination
            | RecoveryArtifactError::EncryptionFailed => Self::InvalidRecoveryArtifact,
        }
    }
}

fn custody_error_from_store(error: StoreError) -> CustodyError {
    match error {
        StoreError::Locked | StoreError::Unavailable => {
            CustodyError::CustodyDenied(CustodyState::Locked)
        }
        StoreError::Conflict => CustodyError::CustodyDenied(CustodyState::Conflict),
        StoreError::Corrupt | StoreError::Configuration => CustodyError::SecureStoreUnavailable,
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use nostr::{Event, JsonUtil};
    use zeroize::Zeroizing;

    use super::*;
    use crate::{SigningPurpose, UnsignedEventTemplate};

    struct FakeStore {
        state: Mutex<FakeStoreState>,
    }

    struct FakeStoreState {
        secret: Option<SecretKeyMaterial>,
        read_mode: FakeReadMode,
        mode_after_write: Option<FakeReadMode>,
        writes: usize,
        deletes: usize,
        fail_delete: bool,
    }

    #[derive(Copy, Clone)]
    enum FakeReadMode {
        Normal,
        Missing,
        Locked,
        Substitute([u8; 32]),
    }

    impl FakeStore {
        fn empty() -> Arc<Self> {
            Arc::new(Self {
                state: Mutex::new(FakeStoreState {
                    secret: None,
                    read_mode: FakeReadMode::Normal,
                    mode_after_write: None,
                    writes: 0,
                    deletes: 0,
                    fail_delete: false,
                }),
            })
        }

        fn set_read_mode(&self, mode: FakeReadMode) {
            self.state.lock().expect("lock fake store").read_mode = mode;
        }

        fn set_mode_after_write(&self, mode: FakeReadMode) {
            self.state.lock().expect("lock fake store").mode_after_write = Some(mode);
        }

        fn set_fail_delete(&self, fail_delete: bool) {
            self.state.lock().expect("lock fake store").fail_delete = fail_delete;
        }

        fn lose_secret(&self) {
            let mut state = self.state.lock().expect("lock fake store");
            state.secret = None;
            state.read_mode = FakeReadMode::Normal;
        }
    }

    impl SecretStore for FakeStore {
        fn read(&self, _locator: &KeyringLocator) -> Result<Option<SecretKeyMaterial>, StoreError> {
            let state = self.state.lock().expect("lock fake store");
            match state.read_mode {
                FakeReadMode::Normal => Ok(state.secret.as_ref().map(SecretKeyMaterial::duplicate)),
                FakeReadMode::Missing => Ok(None),
                FakeReadMode::Locked => Err(StoreError::Locked),
                FakeReadMode::Substitute(bytes) => {
                    SecretKeyMaterial::from_bytes(Zeroizing::new(bytes))
                        .map(Some)
                        .map_err(|_| StoreError::Corrupt)
                }
            }
        }

        fn write(
            &self,
            _locator: &KeyringLocator,
            secret: &SecretKeyMaterial,
        ) -> Result<(), StoreError> {
            let mut state = self.state.lock().expect("lock fake store");
            state.secret = Some(secret.duplicate());
            state.writes += 1;
            if let Some(mode) = state.mode_after_write.take() {
                state.read_mode = mode;
            }
            Ok(())
        }

        fn delete(&self, _locator: &KeyringLocator) -> Result<(), StoreError> {
            let mut state = self.state.lock().expect("lock fake store");
            state.deletes += 1;
            if state.fail_delete {
                return Err(StoreError::Unavailable);
            }
            state.secret = None;
            state.read_mode = FakeReadMode::Normal;
            Ok(())
        }
    }

    struct FixedGenerator([u8; 32]);

    impl SecretGenerator for FixedGenerator {
        fn generate(&self) -> SecretKeyMaterial {
            SecretKeyMaterial::from_bytes(Zeroizing::new(self.0))
                .expect("valid fixed generator secret")
        }
    }

    struct CountingGenerator {
        bytes: [u8; 32],
        calls: Arc<std::sync::atomic::AtomicUsize>,
    }

    impl SecretGenerator for CountingGenerator {
        fn generate(&self) -> SecretKeyMaterial {
            self.calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            SecretKeyMaterial::from_bytes(Zeroizing::new(self.bytes))
                .expect("valid counting generator secret")
        }
    }

    fn service(store: Arc<FakeStore>, data_root: PathBuf) -> IdentityService {
        IdentityService::new(
            AppChannel::Dev,
            CustodyPaths::for_data_root(data_root),
            store,
            Arc::new(FixedGenerator([1; 32])),
        )
    }

    fn counting_service(
        store: Arc<FakeStore>,
        data_root: PathBuf,
        calls: Arc<std::sync::atomic::AtomicUsize>,
    ) -> IdentityService {
        IdentityService::new(
            AppChannel::Dev,
            CustodyPaths::for_data_root(data_root),
            store,
            Arc::new(CountingGenerator {
                bytes: [1; 32],
                calls,
            }),
        )
    }

    fn receipt() -> ReceiptRef {
        ReceiptRef::new("owner-action-1").expect("valid test receipt")
    }

    fn select_one(service: &IdentityService, prepared: PreparedRecovery) -> SelectedRecovery {
        let candidate_ref = prepared.candidate_ref().clone();
        service
            .select_recovery(vec![prepared], &candidate_ref)
            .expect("select prepared recovery")
    }

    fn signing_request(identity: &PublicIdentity) -> AdmittedSigningRequest {
        AdmittedSigningRequest {
            request_ref: ReceiptRef::new("signing-action-1").expect("valid signing receipt"),
            identity_ref: identity.identity_ref().clone(),
            purpose: SigningPurpose::NostrEvent,
            event: UnsignedEventTemplate {
                created_at: 1_700_000_000,
                kind: 1,
                tags: vec![vec!["t".to_string(), "omega".to_string()]],
                content: "Omega custody conformance".to_string(),
            },
        }
    }

    #[test]
    fn restart_returns_the_same_public_identity() {
        let temporary_directory = tempfile::tempdir().expect("create temporary directory");
        let store = FakeStore::empty();
        let first_service = service(store.clone(), temporary_directory.path().to_path_buf());
        let created = first_service.create(receipt()).expect("create identity");
        let created_identity = created.identity.expect("created public identity");

        let restarted_service = service(store.clone(), temporary_directory.path().to_path_buf());
        let restarted = restarted_service.inspect().expect("inspect after restart");
        assert_eq!(restarted.state, CustodyState::Ready);
        assert_eq!(restarted.identity, Some(created_identity));
        assert_eq!(store.state.lock().expect("lock fake store").writes, 1);
    }

    #[test]
    fn inspection_never_generates_and_same_create_is_idempotent() {
        let temporary_directory = tempfile::tempdir().expect("create temporary directory");
        let store = FakeStore::empty();
        let generator_calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let service = counting_service(
            store.clone(),
            temporary_directory.path().to_path_buf(),
            generator_calls.clone(),
        );

        assert_eq!(
            service.inspect().expect("inspect absent identity").state,
            CustodyState::Absent
        );
        assert_eq!(generator_calls.load(std::sync::atomic::Ordering::SeqCst), 0);

        let first = service.create(receipt()).expect("create identity");
        let second = service.create(receipt()).expect("repeat same create");
        assert_eq!(first, second);
        assert_eq!(generator_calls.load(std::sync::atomic::Ordering::SeqCst), 1);
        assert_eq!(store.state.lock().expect("lock fake store").writes, 1);
    }

    #[test]
    fn concurrent_same_create_generates_and_writes_once() {
        let temporary_directory = tempfile::tempdir().expect("create temporary directory");
        let store = FakeStore::empty();
        let generator_calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let service = Arc::new(counting_service(
            store.clone(),
            temporary_directory.path().to_path_buf(),
            generator_calls.clone(),
        ));
        let first = std::thread::spawn({
            let service = service.clone();
            move || service.create(receipt()).expect("first concurrent create")
        });
        let second = std::thread::spawn({
            let service = service;
            move || service.create(receipt()).expect("second concurrent create")
        });

        assert_eq!(
            first.join().expect("join first create"),
            second.join().expect("join second create")
        );
        assert_eq!(generator_calls.load(std::sync::atomic::Ordering::SeqCst), 1);
        assert_eq!(store.state.lock().expect("lock fake store").writes, 1);
    }

    #[test]
    fn pending_create_resumes_the_stored_identity_without_generation() {
        let temporary_directory = tempfile::tempdir().expect("create temporary directory");
        let store = FakeStore::empty();
        let generator_calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let service = counting_service(
            store.clone(),
            temporary_directory.path().to_path_buf(),
            generator_calls.clone(),
        );
        let secret = SecretKeyMaterial::from_bytes(Zeroizing::new([4; 32]))
            .expect("valid pending identity secret");
        let identity = secret
            .public_identity()
            .expect("derive pending public identity");
        store
            .write(&service.locator, &secret)
            .expect("write pending identity secret");
        let mut transaction =
            IdentityTransaction::new(TransactionOperation::Create, receipt(), None);
        transaction.expected_identity = Some(identity.clone());
        write_json_document(&service.paths.transaction_path, &transaction)
            .expect("write pending transaction");

        let resumed = service.create(receipt()).expect("resume pending create");
        assert_eq!(resumed.identity, Some(identity));
        assert_eq!(generator_calls.load(std::sync::atomic::Ordering::SeqCst), 0);
        assert!(!service.paths.transaction_path.exists());
    }

    #[test]
    fn resumed_create_failure_keeps_the_journal_and_stored_identity() {
        let temporary_directory = tempfile::tempdir().expect("create temporary directory");
        let store = FakeStore::empty();
        let generator_calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let service = counting_service(
            store.clone(),
            temporary_directory.path().to_path_buf(),
            generator_calls.clone(),
        );
        let secret = SecretKeyMaterial::from_bytes(Zeroizing::new([4; 32]))
            .expect("valid pending identity secret");
        store
            .write(&service.locator, &secret)
            .expect("write pending identity secret");
        write_json_document(
            &service.paths.transaction_path,
            &IdentityTransaction::new(TransactionOperation::Create, receipt(), None),
        )
        .expect("write pending transaction");
        std::fs::create_dir(&service.paths.manifest_path)
            .expect("create manifest write obstruction");

        assert!(matches!(
            service.create(receipt()),
            Err(CustodyError::PublicStore(_))
        ));
        assert!(service.paths.transaction_path.exists());
        assert!(
            store
                .read(&service.locator)
                .expect("read preserved pending identity")
                .is_some()
        );
        assert_eq!(generator_calls.load(std::sync::atomic::Ordering::SeqCst), 0);
    }

    #[test]
    fn locked_custody_precedes_corrupt_public_transaction() {
        let temporary_directory = tempfile::tempdir().expect("create temporary directory");
        let store = FakeStore::empty();
        let service = service(store.clone(), temporary_directory.path().to_path_buf());
        std::fs::create_dir_all(
            service
                .paths
                .transaction_path
                .parent()
                .expect("transaction parent"),
        )
        .expect("create transaction directory");
        std::fs::write(&service.paths.transaction_path, b"not-json")
            .expect("write corrupt transaction");
        store.set_read_mode(FakeReadMode::Locked);

        assert_eq!(
            service
                .inspect()
                .expect("inspect locked custody with corrupt transaction")
                .state,
            CustodyState::Locked
        );
    }

    #[test]
    fn corrupt_transaction_blocks_create_without_generation() {
        let temporary_directory = tempfile::tempdir().expect("create temporary directory");
        let store = FakeStore::empty();
        let generator_calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let service = counting_service(
            store,
            temporary_directory.path().to_path_buf(),
            generator_calls.clone(),
        );
        std::fs::create_dir_all(
            service
                .paths
                .transaction_path
                .parent()
                .expect("transaction parent"),
        )
        .expect("create transaction directory");
        std::fs::write(
            &service.paths.transaction_path,
            br#"{"schema":"untrusted"}"#,
        )
        .expect("write corrupt transaction");

        assert!(matches!(
            service.create(receipt()),
            Err(CustodyError::TransactionIncomplete)
        ));
        assert_eq!(generator_calls.load(std::sync::atomic::Ordering::SeqCst), 0);
    }

    #[test]
    fn create_refuses_to_replace_existing_custody() {
        let temporary_directory = tempfile::tempdir().expect("create temporary directory");
        let store = FakeStore::empty();
        let service = service(store.clone(), temporary_directory.path().to_path_buf());
        let created = service.create(receipt()).expect("create identity");

        assert!(matches!(
            service.create(ReceiptRef::new("owner-action-2").expect("valid test receipt")),
            Err(CustodyError::CustodyDenied(CustodyState::Ready))
        ));
        assert_eq!(
            service.inspect().expect("inspect identity").identity,
            created.identity
        );
        assert_eq!(store.state.lock().expect("lock fake store").writes, 1);
    }

    #[test]
    fn import_commits_the_supplied_identity_without_exposing_its_secret() {
        let temporary_directory = tempfile::tempdir().expect("create temporary directory");
        let store = FakeStore::empty();
        let service = service(store, temporary_directory.path().to_path_buf());
        let imported_secret =
            ImportedSecret::new(hex::encode([3; 32])).expect("valid imported secret");

        let imported = service
            .import(imported_secret, receipt())
            .expect("import identity");
        let expected = SecretKeyMaterial::from_bytes(Zeroizing::new([3; 32]))
            .expect("valid expected secret")
            .public_identity()
            .expect("derive expected public identity");

        assert_eq!(imported.state, CustodyState::Ready);
        assert_eq!(imported.identity, Some(expected));
        assert!(!format!("{imported:?}").contains(&hex::encode([3; 32])));
    }

    #[test]
    fn lost_identity_can_adopt_only_the_matching_recovery() {
        let temporary_directory = tempfile::tempdir().expect("create temporary directory");
        let store = FakeStore::empty();
        let service = service(store.clone(), temporary_directory.path().to_path_buf());
        let created = service.create(receipt()).expect("create identity");
        let identity = created.identity.expect("created public identity");
        store.lose_secret();
        assert_eq!(
            service.inspect().expect("inspect lost identity").state,
            CustodyState::Lost
        );

        let wrong = service
            .prepare_import(
                ImportedSecret::new(hex::encode([2; 32])).expect("valid different import"),
            )
            .expect("prepare different import");
        assert!(matches!(
            service.adopt(
                select_one(&service, wrong),
                ReceiptRef::new("recovery-action-1").expect("valid recovery receipt")
            ),
            Err(CustodyError::CustodyDenied(CustodyState::Lost))
        ));

        let matching = service
            .prepare_import(
                ImportedSecret::new(hex::encode([1; 32])).expect("valid matching import"),
            )
            .expect("prepare matching import");
        let recovered = service
            .adopt(
                select_one(&service, matching),
                ReceiptRef::new("recovery-action-2").expect("valid recovery receipt"),
            )
            .expect("adopt matching recovery");
        assert_eq!(recovered.state, CustodyState::Ready);
        assert_eq!(recovered.identity, Some(identity));
    }

    #[test]
    fn recovery_reconciliation_deduplicates_identity_and_stops_on_conflict() {
        let first = PreparedRecovery::new(
            CandidateRef::new("candidate-one".to_string()).expect("valid candidate"),
            CandidateKind::EncryptedRecoveryArtifact,
            SecretKeyMaterial::from_bytes(Zeroizing::new([2; 32])).expect("valid first candidate"),
        )
        .expect("prepare first candidate");
        let duplicate = PreparedRecovery::new(
            CandidateRef::new("candidate-two".to_string()).expect("valid candidate"),
            CandidateKind::EncryptedRecoveryArtifact,
            SecretKeyMaterial::from_bytes(Zeroizing::new([2; 32]))
                .expect("valid duplicate candidate"),
        )
        .expect("prepare duplicate candidate");
        let conflicting = PreparedRecovery::new(
            CandidateRef::new("candidate-three".to_string()).expect("valid candidate"),
            CandidateKind::EncryptedRecoveryArtifact,
            SecretKeyMaterial::from_bytes(Zeroizing::new([3; 32]))
                .expect("valid conflicting candidate"),
        )
        .expect("prepare conflicting candidate");

        let deduplicated = reconcile_prepared(&[&first, &duplicate]);
        assert_eq!(
            deduplicated.state,
            crate::RecoveryResolutionState::CandidateSelected
        );
        assert_eq!(deduplicated.candidates.len(), 1);

        let conflict = reconcile_prepared(&[&first, &duplicate, &conflicting]);
        assert_eq!(
            conflict.state,
            crate::RecoveryResolutionState::OwnerSelectionRequired
        );
        assert_eq!(conflict.candidates.len(), 2);
        assert!(conflict.selected_candidate_ref.is_none());

        let selected_candidate_ref = conflicting.candidate_ref().clone();
        let selection_directory = tempfile::tempdir().expect("create selection directory");
        let selection_service =
            service(FakeStore::empty(), selection_directory.path().to_path_buf());
        let selected = selection_service
            .select_recovery(vec![first, conflicting], &selected_candidate_ref)
            .expect("select conflicting recovery explicitly");
        assert_eq!(selected.identity(), &conflict.candidates[1].identity);
    }

    #[test]
    fn read_back_mismatch_or_missing_value_never_completes() {
        for mode in [FakeReadMode::Substitute([2; 32]), FakeReadMode::Missing] {
            let temporary_directory = tempfile::tempdir().expect("create temporary directory");
            let store = FakeStore::empty();
            store.set_mode_after_write(mode);
            let service = service(store, temporary_directory.path().to_path_buf());

            assert!(matches!(
                service.create(receipt()),
                Err(CustodyError::ReadBackMismatch)
            ));
            assert!(
                !temporary_directory
                    .path()
                    .join("identity.complete.json")
                    .exists()
            );
            assert_eq!(
                service.inspect().expect("inspect rolled back create").state,
                CustodyState::Absent
            );
        }
    }

    #[test]
    fn rollback_failure_stays_incomplete_and_blocks_replacement() {
        let temporary_directory = tempfile::tempdir().expect("create temporary directory");
        let store = FakeStore::empty();
        store.set_mode_after_write(FakeReadMode::Missing);
        store.set_fail_delete(true);
        let service = service(store.clone(), temporary_directory.path().to_path_buf());

        assert!(matches!(
            service.create(receipt()),
            Err(CustodyError::TransactionIncomplete)
        ));
        assert!(service.paths.transaction_path.exists());
        assert!(matches!(
            service.create(ReceiptRef::new("owner-action-2").expect("valid second receipt")),
            Err(CustodyError::CustodyDenied(CustodyState::Conflict))
        ));
        assert_eq!(store.state.lock().expect("lock fake store").writes, 1);
    }

    #[test]
    fn locked_and_lost_custody_deny_signing() {
        let temporary_directory = tempfile::tempdir().expect("create temporary directory");
        let store = FakeStore::empty();
        let service = service(store.clone(), temporary_directory.path().to_path_buf());
        let created = service.create(receipt()).expect("create identity");
        let identity = created.identity.expect("created public identity");
        let request = signing_request(&identity);

        store.set_read_mode(FakeReadMode::Locked);
        assert_eq!(
            service.inspect().expect("inspect locked custody").state,
            CustodyState::Locked
        );
        assert!(matches!(
            service.sign(&request),
            Err(CustodyError::CustodyDenied(CustodyState::Locked))
        ));

        store.set_read_mode(FakeReadMode::Missing);
        assert_eq!(
            service.inspect().expect("inspect lost custody").state,
            CustodyState::Lost
        );
        assert!(matches!(
            service.sign(&request),
            Err(CustodyError::CustodyDenied(CustodyState::Lost))
        ));
    }

    #[test]
    fn admitted_signing_returns_a_verified_public_event() {
        let temporary_directory = tempfile::tempdir().expect("create temporary directory");
        let store = FakeStore::empty();
        let service = service(store, temporary_directory.path().to_path_buf());
        let created = service.create(receipt()).expect("create identity");
        let identity = created.identity.expect("created public identity");
        let request = signing_request(&identity);

        let signed = service.sign(&request).expect("sign admitted request");
        let event = Event::from_json(&signed.signed_event_json).expect("parse signed event");
        event.verify().expect("verify signed event");
        assert_eq!(signed.identity, identity);
        assert_eq!(signed.event_id, event.id.to_hex());
        assert_eq!(signed.signature, event.sig.to_string());
    }

    #[test]
    fn encrypted_recovery_artifact_round_trips_without_mutating_preview() {
        let temporary_directory = tempfile::tempdir().expect("create temporary directory");
        let source_store = FakeStore::empty();
        let source_service = service(
            source_store.clone(),
            temporary_directory.path().join("source"),
        );
        let created = source_service
            .create(receipt())
            .expect("create source identity");
        let identity = created.identity.expect("created source identity");
        let artifact_path = temporary_directory.path().join("omega-recovery.ncryptsec");

        let artifact = source_service
            .export_recovery_artifact(
                identity.identity_ref(),
                &artifact_path,
                RecoveryPassword::new("correct horse battery staple".to_string())
                    .expect("valid export password"),
            )
            .expect("export recovery artifact");
        let artifact_bytes = std::fs::read(&artifact_path).expect("read recovery artifact");
        let secret_hex = hex::encode([1; 32]);
        assert_eq!(artifact.byte_length(), artifact_bytes.len() as u64);
        assert!(artifact_bytes.starts_with(b"ncryptsec1"));
        assert!(artifact_bytes.ends_with(b"\n"));
        assert!(!String::from_utf8_lossy(&artifact_bytes).contains(&secret_hex));
        assert!(!String::from_utf8_lossy(&artifact_bytes).contains(identity.npub().as_str()));
        assert!(
            std::fs::read_dir(temporary_directory.path())
                .expect("read artifact directory")
                .all(|entry| {
                    !entry
                        .expect("read artifact directory entry")
                        .file_name()
                        .to_string_lossy()
                        .contains(".omega-recovery-")
                })
        );

        let candidate = source_service
            .discover_recovery_artifact(artifact_path.clone())
            .expect("discover recovery artifact");
        let writes_before_preview = source_store.state.lock().expect("lock fake store").writes;
        assert!(matches!(
            source_service.prepare_recovery_artifact(
                &candidate,
                RecoveryPassword::new("wrong password".to_string()).expect("valid wrong password")
            ),
            Err(CustodyError::RecoveryDecryptionFailed)
        ));
        let prepared = source_service
            .prepare_recovery_artifact(
                &candidate,
                RecoveryPassword::new("correct horse battery staple".to_string())
                    .expect("valid import password"),
            )
            .expect("prepare recovery artifact");
        assert_eq!(prepared.identity(), &identity);
        assert_eq!(
            source_store.state.lock().expect("lock fake store").writes,
            writes_before_preview
        );

        let destination = service(
            FakeStore::empty(),
            temporary_directory.path().join("destination"),
        );
        let adopted = destination
            .adopt(
                select_one(&destination, prepared),
                ReceiptRef::new("artifact-recovery-1").expect("valid recovery receipt"),
            )
            .expect("adopt recovery artifact");
        assert_eq!(adopted.identity.as_ref(), Some(&identity));

        let before = std::fs::read(&artifact_path).expect("read preserved artifact");
        assert!(matches!(
            source_service.export_recovery_artifact(
                identity.identity_ref(),
                &artifact_path,
                RecoveryPassword::new("another strong password".to_string())
                    .expect("valid replacement password")
            ),
            Err(CustodyError::RecoveryArtifactExists)
        ));
        assert_eq!(
            std::fs::read(&artifact_path).expect("read unchanged artifact"),
            before
        );

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            assert_eq!(
                std::fs::metadata(&artifact_path)
                    .expect("read artifact metadata")
                    .permissions()
                    .mode()
                    & 0o777,
                0o600
            );
        }
    }

    #[test]
    fn reset_requires_the_expected_identity_and_verifies_deletion() {
        let temporary_directory = tempfile::tempdir().expect("create temporary directory");
        let store = FakeStore::empty();
        let service = service(store, temporary_directory.path().to_path_buf());
        let created = service.create(receipt()).expect("create identity");
        let identity = created.identity.expect("created public identity");
        let wrong_identity = SecretKeyMaterial::from_bytes(Zeroizing::new([2; 32]))
            .expect("valid different secret")
            .public_identity()
            .expect("derive different identity");

        assert!(matches!(
            service.reset(
                wrong_identity.identity_ref(),
                ReceiptRef::new("reset-action-1").expect("valid reset receipt")
            ),
            Err(CustodyError::CustodyDenied(CustodyState::Conflict))
        ));
        assert_eq!(
            service
                .inspect()
                .expect("inspect after refused reset")
                .state,
            CustodyState::Ready
        );

        let reset = service
            .reset(
                identity.identity_ref(),
                ReceiptRef::new("reset-action-2").expect("valid reset receipt"),
            )
            .expect("reset identity");
        assert_eq!(reset.state, CustodyState::RelaunchRequired);
        assert_eq!(
            service
                .inspect()
                .expect("resume reset after relaunch")
                .state,
            CustodyState::RelaunchRequired
        );
        assert_eq!(
            service
                .acknowledge_relaunch()
                .expect("acknowledge completed reset")
                .state,
            CustodyState::Absent
        );
    }

    #[test]
    fn failed_reset_stays_durable_and_resumes_without_exposing_absent() {
        let temporary_directory = tempfile::tempdir().expect("create temporary directory");
        let store = FakeStore::empty();
        let service = service(store.clone(), temporary_directory.path().to_path_buf());
        let created = service.create(receipt()).expect("create identity");
        let identity = created.identity.expect("created public identity");
        let request = signing_request(&identity);
        store.set_fail_delete(true);

        assert_eq!(
            service
                .reset(
                    identity.identity_ref(),
                    ReceiptRef::new("reset-action-3").expect("valid reset receipt"),
                )
                .expect("record reset intent")
                .state,
            CustodyState::RelaunchRequired
        );
        assert_eq!(
            service.inspect().expect("attempt pending reset").state,
            CustodyState::ResetFailed
        );
        assert!(matches!(
            service.sign(&request),
            Err(CustodyError::CustodyDenied(CustodyState::ResetFailed))
        ));
        assert!(service.paths.reset_path.exists());
        assert!(service.paths.manifest_path.exists());

        store.set_fail_delete(false);
        assert_eq!(
            service
                .resume_pending_reset()
                .expect("resume failed reset")
                .state,
            CustodyState::RelaunchRequired
        );
        assert_eq!(
            service.inspect().expect("inspect completed reset").state,
            CustodyState::RelaunchRequired
        );
        assert_eq!(
            service
                .acknowledge_relaunch()
                .expect("acknowledge completed reset")
                .state,
            CustodyState::Absent
        );
    }

    #[test]
    fn reset_identity_mismatch_is_reset_failed_and_never_deletes() {
        let temporary_directory = tempfile::tempdir().expect("create temporary directory");
        let store = FakeStore::empty();
        let service = service(store.clone(), temporary_directory.path().to_path_buf());
        let created = service.create(receipt()).expect("create identity");
        let identity = created.identity.expect("created public identity");
        service
            .reset(
                identity.identity_ref(),
                ReceiptRef::new("reset-action-4").expect("valid reset receipt"),
            )
            .expect("record reset intent");
        let deletes_before = store.state.lock().expect("lock fake store").deletes;
        store.set_read_mode(FakeReadMode::Substitute([2; 32]));

        assert_eq!(
            service.inspect().expect("inspect reset mismatch").state,
            CustodyState::ResetFailed
        );
        assert_eq!(
            store.state.lock().expect("lock fake store").deletes,
            deletes_before
        );
    }

    #[test]
    fn public_outputs_and_app_data_do_not_contain_the_secret() {
        let temporary_directory = tempfile::tempdir().expect("create temporary directory");
        let store = FakeStore::empty();
        let service = service(store, temporary_directory.path().to_path_buf());
        let result = service.create(receipt()).expect("create identity");
        let secret_hex = hex::encode([1; 32]);

        assert!(!format!("{result:?}").contains(&secret_hex));
        for entry in
            std::fs::read_dir(temporary_directory.path()).expect("read public identity directory")
        {
            let entry = entry.expect("read public identity entry");
            if entry.file_type().expect("read entry type").is_file() {
                let contents =
                    std::fs::read_to_string(entry.path()).expect("read public identity document");
                assert!(!contents.contains(&secret_hex));
                assert!(!contents.contains("nsec"));
            }
        }
    }
}
