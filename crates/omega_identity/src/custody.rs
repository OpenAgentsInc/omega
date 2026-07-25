use std::{
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};

use app_identity::AppChannel;
use nostr::{
    Event, EventBuilder, JsonUtil, Kind,
    nips::{
        nip44::{self, Version as Nip44Version},
        nip49::{EncryptedSecretKey, KeySecurity},
        nip59,
    },
    secp256k1::Message,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};
use thiserror::Error;

use crate::{
    AdmittedSigningRequest, CompletionRecord, ContractError, CustodyConflict,
    CustodyConflictReason, CustodyResult, CustodyState, GiftWrappedPrivateMessage,
    IdentityInspection, IdentityManifest, IdentityRef, ImportedSecret, KeyringLocator,
    NostrPublicKeyHex, OwnerAttestationRequest, OwnerAttestationResult, PendingIdentityOperation,
    PendingIdentityTransaction, PrivateMessageRequest, PublicIdentity, PublicStoreError,
    ReceiptRef, SigningResult, UnwrappedPrivateMessage,
    mutation_lock::{IdentityMutationGuard, MutationLockError},
    proof::{IDENTITY_PROOF_KEYRING_ACCOUNT, IDENTITY_PROOF_KEYRING_SERVICE, ProofCrashBoundary},
    public_store::{
        read_completion_record, read_completion_record_for_locator, read_identity_manifest,
        read_identity_manifest_for_locator, read_json_document, remove_public_document,
        write_completion_record, write_completion_record_for_locator, write_identity_manifest,
        write_identity_manifest_for_locator, write_json_document,
    },
    recovery::{
        CandidateKind, CandidateRef, PreparedRecovery, RecoveryArtifactReceipt, RecoveryCandidate,
        RecoveryPassword, RecoveryProtectionRecord, RecoveryProtectionStatus, RecoveryResolution,
        SelectedRecovery, reconcile_prepared,
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
    proof_crash_boundary: Option<ProofCrashBoundary>,
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

    /// Build custody against the standard data root for `channel`.
    ///
    /// Unlike [`Self::system`], this does not depend on the compile-time
    /// `app_identity::CHANNEL` path resolution, so an operator CLI can target
    /// `rc` while built from a `dev` tree.
    pub fn for_channel(channel: AppChannel) -> Self {
        Self::for_channel_data_root(channel, channel_data_root(channel))
    }

    /// Build custody against an explicit channel data root (parent of `identity/`).
    pub fn for_channel_data_root(channel: AppChannel, data_root: PathBuf) -> Self {
        Self::new(
            channel,
            CustodyPaths::for_data_root(data_root.join("identity")),
            Arc::new(SystemKeyringStore),
            Arc::new(SystemSecretGenerator),
        )
    }

    pub(crate) fn for_disposable_proof(
        proof_root: PathBuf,
        crash_boundary: Option<ProofCrashBoundary>,
    ) -> Self {
        Self {
            channel: AppChannel::Rc,
            locator: KeyringLocator::proof(
                IDENTITY_PROOF_KEYRING_SERVICE,
                IDENTITY_PROOF_KEYRING_ACCOUNT,
            ),
            paths: CustodyPaths::for_data_root(proof_root.join("identity")),
            store: Arc::new(SystemKeyringStore),
            generator: Arc::new(SystemSecretGenerator),
            proof_crash_boundary: crash_boundary,
        }
    }

    pub fn inspect(&self) -> Result<CustodyResult, CustodyError> {
        let _mutation_guard = IdentityMutationGuard::acquire(&self.locator)?;
        if let Some(result) = self.resume_reset_if_pending_locked() {
            return Ok(result);
        }
        Ok(self.resolve_locked().result)
    }

    pub fn inspect_details(&self) -> Result<IdentityInspection, CustodyError> {
        let _mutation_guard = IdentityMutationGuard::acquire(&self.locator)?;
        self.inspect_details_locked()
    }

    pub fn inspect_for_process_start(&self) -> Result<IdentityInspection, CustodyError> {
        let _mutation_guard = IdentityMutationGuard::acquire(&self.locator)?;
        if let Some(marker) = self.read_reset_marker_locked()?
            && marker.status == ResetStatus::Complete
        {
            self.acknowledge_relaunch_locked(marker)?;
        }
        self.inspect_details_locked()
    }

    fn inspect_details_locked(&self) -> Result<IdentityInspection, CustodyError> {
        if let Some(custody) = self.resume_reset_if_pending_locked() {
            return Ok(IdentityInspection {
                recovery_protection: self.recovery_protection_status_locked(&custody)?,
                custody,
                pending_transaction: None,
                conflict: None,
            });
        }

        let resolved = self.resolve_locked();
        let pending_transaction = self
            .read_transaction_locked()
            .ok()
            .flatten()
            .map(IdentityTransaction::public_facts);
        let recovery_protection = self.recovery_protection_status_locked(&resolved.result)?;
        Ok(IdentityInspection {
            custody: resolved.result,
            pending_transaction,
            conflict: resolved.conflict,
            recovery_protection,
        })
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

    pub fn resume_incomplete_create(&self) -> Result<CustodyResult, CustodyError> {
        let _mutation_guard = IdentityMutationGuard::acquire(&self.locator)?;
        self.require_no_reset_locked()?;
        let transaction = self
            .read_transaction_locked()?
            .ok_or(CustodyError::CustodyDenied(CustodyState::Absent))?;
        if transaction.operation != TransactionOperation::Create {
            return Err(CustodyError::CustodyDenied(CustodyState::Incomplete));
        }
        let resolved = self.resolve_locked();
        if resolved.result.state != CustodyState::Incomplete {
            return Err(CustodyError::CustodyDenied(resolved.result.state));
        }
        let secret = match resolved.secret {
            Some(secret) => secret,
            None if transaction.expected_identity.is_none() => self.generator.generate(),
            None => return Err(CustodyError::TransactionIncomplete),
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

    pub fn resolve_conflict(
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

        let transaction = match self.read_transaction_locked()? {
            Some(transaction) => {
                if transaction.operation != TransactionOperation::ResolveConflict
                    || transaction.receipt_ref != receipt_ref
                    || transaction.candidate_ref.as_ref() != Some(prepared.candidate_ref())
                    || transaction.expected_identity.as_ref() != Some(prepared.identity())
                {
                    return Err(CustodyError::CustodyDenied(CustodyState::Conflict));
                }
                transaction
            }
            None => {
                let resolved = self.resolve_locked();
                let conflict = resolved
                    .conflict
                    .ok_or(CustodyError::CustodyDenied(resolved.result.state))?;
                if conflict.reason == CustodyConflictReason::AmbiguousSecureStore
                    || !conflict
                        .identities
                        .iter()
                        .any(|identity| identity == prepared.identity())
                {
                    return Err(CustodyError::CustodyDenied(CustodyState::Conflict));
                }
                let mut transaction = IdentityTransaction::new(
                    TransactionOperation::ResolveConflict,
                    receipt_ref,
                    Some(prepared.candidate_ref().clone()),
                );
                transaction.expected_identity = Some(prepared.identity().clone());
                transaction.conflict_identities = conflict.identities;
                write_json_document(&self.paths.transaction_path, &transaction)?;
                transaction
            }
        };

        if let Some(stored_secret) = self
            .store
            .read(&self.locator)
            .map_err(custody_error_from_store)?
        {
            let stored_identity = stored_secret
                .public_identity()
                .map_err(|_| CustodyError::ReadBackMismatch)?;
            if !transaction
                .conflict_identities
                .iter()
                .any(|identity| identity == &stored_identity)
            {
                return Err(CustodyError::CustodyDenied(CustodyState::Conflict));
            }
            if &stored_identity != prepared.identity() {
                self.store
                    .delete(&self.locator)
                    .map_err(|_| CustodyError::TransactionIncomplete)?;
                match self.store.read(&self.locator) {
                    Ok(None) => {}
                    Ok(Some(_)) | Err(_) => return Err(CustodyError::TransactionIncomplete),
                }
            }
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
        let artifact_write = recovery_artifact::write_encrypted(path, &encrypted)?;
        let protection = RecoveryProtectionRecord::new(
            identity.clone(),
            artifact_write.digest,
            artifact_write.byte_length,
        )?;
        write_json_document(&self.paths.recovery_protection_path, &protection)?;
        Ok(RecoveryArtifactReceipt::new(
            path.to_path_buf(),
            identity,
            artifact_write.byte_length,
        ))
    }

    pub fn sign(&self, request: &AdmittedSigningRequest) -> Result<SigningResult, CustodyError> {
        if request.purpose != crate::SigningPurpose::NostrEvent {
            return Err(CustodyError::SigningFailed);
        }
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

    pub fn sign_owner_attestation(
        &self,
        request: &OwnerAttestationRequest,
    ) -> Result<OwnerAttestationResult, CustodyError> {
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
        request.validate(&identity)?;
        let secret = resolved
            .secret
            .ok_or(CustodyError::CustodyDenied(CustodyState::Incomplete))?;
        let keys = secret
            .keys()
            .map_err(|_| CustodyError::CustodyDenied(CustodyState::Incomplete))?;
        let digest = Sha256::digest(
            format!(
                "nostr:agent-auth:{}:{}",
                request.agent_public_key_hex.as_str(),
                request.conditions
            )
            .as_bytes(),
        );
        let message = Message::from_digest(digest.into());
        let signature = keys.sign_schnorr(&message).to_string();
        let auth_tag = vec![
            "auth".to_string(),
            identity.public_key_hex().as_str().to_string(),
            request.conditions.clone(),
            signature,
        ];
        Ok(OwnerAttestationResult {
            request_ref: request.request_ref.clone(),
            identity,
            agent_public_key_hex: request.agent_public_key_hex.clone(),
            auth_tag,
        })
    }

    pub fn sign_nip44_encrypted_to_self(
        &self,
        request: &AdmittedSigningRequest,
    ) -> Result<SigningResult, CustodyError> {
        if request.purpose != crate::SigningPurpose::Nip44EncryptedSelfEvent {
            return Err(CustodyError::SigningFailed);
        }
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
        let keys = secret
            .keys()
            .map_err(|_| CustodyError::CustodyDenied(CustodyState::Incomplete))?;
        let ciphertext = nip44::encrypt(
            keys.secret_key(),
            &keys.public_key(),
            request.event.content.as_bytes(),
            Nip44Version::V2,
        )
        .map_err(|_| CustodyError::SigningFailed)?;
        let mut encrypted_request = request.clone();
        encrypted_request.purpose = crate::SigningPurpose::NostrEvent;
        encrypted_request.event.content = ciphertext;
        let unsigned_event = encrypted_request.unsigned_event(&identity)?;
        let event = unsigned_event
            .sign_with_keys(&keys)
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

    pub fn decrypt_nip44_from(
        &self,
        sender_public_key_hex: &NostrPublicKeyHex,
        ciphertext: &str,
    ) -> Result<String, CustodyError> {
        if ciphertext.len() > 1_048_576 {
            return Err(CustodyError::SigningFailed);
        }
        let _mutation_guard = IdentityMutationGuard::acquire(&self.locator)?;
        self.require_no_reset_locked()?;
        let resolved = self.resolve_locked();
        if resolved.result.state != CustodyState::Ready {
            return Err(CustodyError::CustodyDenied(resolved.result.state));
        }
        let secret = resolved
            .secret
            .ok_or(CustodyError::CustodyDenied(CustodyState::Incomplete))?;
        let keys = secret
            .keys()
            .map_err(|_| CustodyError::CustodyDenied(CustodyState::Incomplete))?;
        let sender_public_key = sender_public_key_hex.public_key()?;
        nip44::decrypt(keys.secret_key(), &sender_public_key, ciphertext.as_bytes())
            .map_err(|_| CustodyError::SigningFailed)
    }

    pub fn decrypt_nip44_from_self(&self, ciphertext: &str) -> Result<String, CustodyError> {
        let identity = self
            .inspect()?
            .identity
            .ok_or(CustodyError::CustodyDenied(CustodyState::Incomplete))?;
        let public_key = NostrPublicKeyHex::new(identity.public_key_hex().as_str())?;
        self.decrypt_nip44_from(&public_key, ciphertext)
    }

    pub fn gift_wrap_private_message(
        &self,
        request: &PrivateMessageRequest,
    ) -> Result<Vec<GiftWrappedPrivateMessage>, CustodyError> {
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
        let keys = secret
            .keys()
            .map_err(|_| CustodyError::CustodyDenied(CustodyState::Incomplete))?;
        let rumor = request.unsigned_rumor(&identity)?;
        rumor.verify_id().map_err(|_| CustodyError::SigningFailed)?;
        let rumor_event_id = rumor.id.ok_or(CustodyError::SigningFailed)?.to_hex();
        let mut wrapped = Vec::with_capacity(request.recipients.len());
        for recipient in &request.recipients {
            let recipient_public_key = recipient.public_key()?;
            let gift_wrap = smol::block_on(EventBuilder::gift_wrap(
                &keys,
                &recipient_public_key,
                rumor.clone(),
                [],
            ))
            .map_err(|_| CustodyError::SigningFailed)?;
            wrapped.push(GiftWrappedPrivateMessage {
                receiver_public_key_hex: recipient.as_str().to_string(),
                rumor_event_id: rumor_event_id.clone(),
                gift_wrap_event_json: gift_wrap
                    .try_as_json()
                    .map_err(|_| CustodyError::SigningFailed)?,
            });
        }
        Ok(wrapped)
    }

    pub fn unwrap_private_message(
        &self,
        gift_wrap_event_json: &str,
    ) -> Result<UnwrappedPrivateMessage, CustodyError> {
        if gift_wrap_event_json.len() > 1_048_576 {
            return Err(CustodyError::SigningFailed);
        }
        let _mutation_guard = IdentityMutationGuard::acquire(&self.locator)?;
        self.require_no_reset_locked()?;
        let resolved = self.resolve_locked();
        if resolved.result.state != CustodyState::Ready {
            return Err(CustodyError::CustodyDenied(resolved.result.state));
        }
        let secret = resolved
            .secret
            .ok_or(CustodyError::CustodyDenied(CustodyState::Incomplete))?;
        let keys = secret
            .keys()
            .map_err(|_| CustodyError::CustodyDenied(CustodyState::Incomplete))?;
        let gift_wrap =
            Event::from_json(gift_wrap_event_json).map_err(|_| CustodyError::SigningFailed)?;
        if gift_wrap.kind != Kind::GiftWrap || gift_wrap.verify().is_err() {
            return Err(CustodyError::SigningFailed);
        }
        let gift = smol::block_on(nip59::extract_rumor(&keys, &gift_wrap))
            .map_err(|_| CustodyError::SigningFailed)?;
        if gift.rumor.kind != Kind::PrivateDirectMessage {
            return Err(CustodyError::SigningFailed);
        }
        gift.rumor
            .verify_id()
            .map_err(|_| CustodyError::SigningFailed)?;
        let rumor_event_id = gift.rumor.id.ok_or(CustodyError::SigningFailed)?.to_hex();
        Ok(UnwrappedPrivateMessage {
            rumor_event_id,
            sender_public_key_hex: gift.sender.to_hex(),
            created_at: gift.rumor.created_at.as_secs(),
            tags: gift
                .rumor
                .tags
                .iter()
                .map(|tag| tag.as_slice().to_vec())
                .collect(),
            content: gift.rumor.content,
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
        self.trigger_proof_crash(ProofCrashBoundary::AfterResetMarker);
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
        self.acknowledge_relaunch_locked(marker)
    }

    fn acknowledge_relaunch_locked(
        &self,
        marker: ResetMarker,
    ) -> Result<CustodyResult, CustodyError> {
        if marker.status != ResetStatus::Complete
            || self
                .store
                .read(&self.locator)
                .map_err(custody_error_from_store)?
                .is_some()
            || self.read_manifest_locked()?.is_some()
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
            || self
                .paths
                .recovery_protection_path
                .try_exists()
                .map_err(|_| CustodyError::ResetFailed)?
        {
            return Err(CustodyError::ResetFailed);
        }
        remove_public_document(&self.paths.reset_path)?;
        self.trigger_proof_crash(ProofCrashBoundary::AfterRelaunchAcknowledge);
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
            proof_crash_boundary: None,
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
                    self.trigger_proof_crash(ProofCrashBoundary::AfterSecretWrite);
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
            self.trigger_proof_crash(ProofCrashBoundary::AfterSecretReadBack);

            transaction.expected_identity = Some(expected_identity.clone());
            write_json_document(&self.paths.transaction_path, &transaction)?;
            let manifest = IdentityManifest::new(
                expected_identity.clone(),
                self.locator.clone(),
                vec![transaction.receipt_ref.clone()],
            );
            self.write_manifest_locked(&manifest)?;
            self.trigger_proof_crash(ProofCrashBoundary::AfterManifestCommit);
            let completion = if self.is_disposable_proof() {
                CompletionRecord::new_for_locator(
                    &manifest,
                    transaction.receipt_ref.clone(),
                    &self.locator,
                )?
            } else {
                CompletionRecord::new(&manifest, transaction.receipt_ref.clone(), self.channel)?
            };
            self.write_completion_locked(&completion, &manifest)?;
            self.remove_mismatched_recovery_protection_locked(&expected_identity)?;

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

    fn read_recovery_protection_locked(
        &self,
    ) -> Result<Option<RecoveryProtectionRecord>, CustodyError> {
        let record: Option<RecoveryProtectionRecord> =
            match read_json_document(&self.paths.recovery_protection_path) {
                Ok(record) => record,
                Err(PublicStoreError::Serialization(_)) => {
                    remove_public_document(&self.paths.recovery_protection_path)?;
                    return Ok(None);
                }
                Err(error) => return Err(error.into()),
            };
        if record
            .as_ref()
            .is_some_and(|record| record.validate().is_err())
        {
            remove_public_document(&self.paths.recovery_protection_path)?;
            return Ok(None);
        }
        Ok(record)
    }

    fn recovery_protection_status_locked(
        &self,
        custody: &CustodyResult,
    ) -> Result<RecoveryProtectionStatus, CustodyError> {
        let Some(identity) = custody.identity.as_ref() else {
            return Ok(RecoveryProtectionStatus::not_applicable());
        };
        match self.read_recovery_protection_locked()? {
            Some(record) if record.identity() == identity => {
                Ok(RecoveryProtectionStatus::protected(record))
            }
            Some(_) => {
                remove_public_document(&self.paths.recovery_protection_path)?;
                Ok(RecoveryProtectionStatus::needed())
            }
            None => Ok(RecoveryProtectionStatus::needed()),
        }
    }

    fn remove_mismatched_recovery_protection_locked(
        &self,
        identity: &PublicIdentity,
    ) -> Result<(), CustodyError> {
        if self
            .read_recovery_protection_locked()?
            .is_some_and(|record| record.identity() != identity)
        {
            remove_public_document(&self.paths.recovery_protection_path)?;
        }
        Ok(())
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
        remove_public_document(&self.paths.recovery_protection_path)
            .map_err(|_| CustodyState::ResetFailed)?;
        marker.status = ResetStatus::Complete;
        write_json_document(&self.paths.reset_path, &marker)
            .map_err(|_| CustodyState::ResetFailed)?;
        self.trigger_proof_crash(ProofCrashBoundary::AfterResetCommit);
        Ok(())
    }

    fn is_disposable_proof(&self) -> bool {
        self.locator.service() == IDENTITY_PROOF_KEYRING_SERVICE
            && self.locator.account() == IDENTITY_PROOF_KEYRING_ACCOUNT
    }

    fn read_manifest_locked(&self) -> Result<Option<IdentityManifest>, PublicStoreError> {
        if self.is_disposable_proof() {
            read_identity_manifest_for_locator(&self.paths.manifest_path, &self.locator)
        } else {
            read_identity_manifest(&self.paths.manifest_path, self.channel)
        }
    }

    fn read_completion_locked(
        &self,
        manifest: &IdentityManifest,
    ) -> Result<Option<CompletionRecord>, PublicStoreError> {
        if self.is_disposable_proof() {
            read_completion_record_for_locator(&self.paths.completion_path, manifest, &self.locator)
        } else {
            read_completion_record(&self.paths.completion_path, manifest, self.channel)
        }
    }

    fn write_manifest_locked(&self, manifest: &IdentityManifest) -> Result<(), PublicStoreError> {
        if self.is_disposable_proof() {
            write_identity_manifest_for_locator(&self.paths.manifest_path, manifest, &self.locator)
        } else {
            write_identity_manifest(&self.paths.manifest_path, manifest, self.channel)
        }
    }

    fn write_completion_locked(
        &self,
        completion: &CompletionRecord,
        manifest: &IdentityManifest,
    ) -> Result<(), PublicStoreError> {
        if self.is_disposable_proof() {
            write_completion_record_for_locator(
                &self.paths.completion_path,
                completion,
                manifest,
                &self.locator,
            )
        } else {
            write_completion_record(
                &self.paths.completion_path,
                completion,
                manifest,
                self.channel,
            )
        }
    }

    fn trigger_proof_crash(&self, boundary: ProofCrashBoundary) {
        if !self.is_disposable_proof() || self.proof_crash_boundary != Some(boundary) {
            return;
        }
        #[cfg(unix)]
        unsafe {
            libc::kill(libc::getpid(), libc::SIGKILL);
        }
        std::process::abort();
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
                return ResolvedCustody::conflict(
                    CustodyConflictReason::AmbiguousSecureStore,
                    self.best_effort_manifest_identity(),
                    None,
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
        let manifest = match self.read_manifest_locked() {
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
                if let Some(expected_identity) = transaction
                    .as_ref()
                    .and_then(|transaction| transaction.expected_identity.as_ref())
                    .filter(|identity| *identity != manifest.identity())
                {
                    ResolvedCustody::conflict(
                        CustodyConflictReason::PendingTransactionMismatch,
                        [manifest.identity().clone(), expected_identity.clone()],
                        None,
                    )
                } else {
                    ResolvedCustody::without_secret(
                        CustodyState::Lost,
                        Some(manifest.identity().clone()),
                    )
                }
            }
            (None, Some(secret)) => match secret.public_identity() {
                Ok(identity) => {
                    if let Some(expected_identity) = transaction
                        .as_ref()
                        .and_then(|transaction| transaction.expected_identity.as_ref())
                        .filter(|expected| *expected != &identity)
                    {
                        ResolvedCustody::conflict(
                            CustodyConflictReason::PendingTransactionMismatch,
                            [identity, expected_identity.clone()],
                            Some(secret),
                        )
                    } else {
                        ResolvedCustody {
                            result: CustodyResult {
                                state: CustodyState::Incomplete,
                                identity: Some(identity),
                                receipt_ref: None,
                            },
                            secret: Some(secret),
                            conflict: None,
                        }
                    }
                }
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
                    return ResolvedCustody::conflict(
                        CustodyConflictReason::PublicManifestCustodyMismatch,
                        [manifest.identity().clone(), identity],
                        Some(secret),
                    );
                }
                if let Some(expected_identity) = transaction
                    .as_ref()
                    .and_then(|transaction| transaction.expected_identity.as_ref())
                    .filter(|expected| *expected != &identity)
                {
                    return ResolvedCustody::conflict(
                        CustodyConflictReason::PendingTransactionMismatch,
                        [identity, expected_identity.clone()],
                        Some(secret),
                    );
                }

                match self.read_completion_locked(&manifest) {
                    Ok(Some(completion))
                        if transaction.as_ref().is_none_or(|transaction| {
                            completion.receipt_ref() == &transaction.receipt_ref
                        }) =>
                    {
                        ResolvedCustody {
                            result: CustodyResult {
                                state: CustodyState::Ready,
                                identity: Some(identity),
                                receipt_ref: Some(completion.receipt_ref().clone()),
                            },
                            secret: Some(secret),
                            conflict: None,
                        }
                    }
                    Ok(Some(_)) | Ok(None) | Err(_) => ResolvedCustody {
                        result: CustodyResult {
                            state: CustodyState::Incomplete,
                            identity: Some(identity),
                            receipt_ref: None,
                        },
                        secret: Some(secret),
                        conflict: None,
                    },
                }
            }
        }
    }

    fn best_effort_manifest_identity(&self) -> Option<PublicIdentity> {
        self.read_manifest_locked()
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
    ResolveConflict,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct IdentityTransaction {
    schema: String,
    operation: TransactionOperation,
    receipt_ref: ReceiptRef,
    candidate_ref: Option<CandidateRef>,
    expected_identity: Option<PublicIdentity>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    conflict_identities: Vec<PublicIdentity>,
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
            conflict_identities: Vec::new(),
        }
    }

    fn is_valid(&self) -> bool {
        self.schema == IDENTITY_TRANSACTION_SCHEMA
            && matches!(
                (
                    self.operation,
                    self.candidate_ref.is_some(),
                    self.conflict_identities.is_empty(),
                    self.expected_identity.is_some(),
                ),
                (TransactionOperation::Create, false, true, _)
                    | (TransactionOperation::Import, true, true, _)
                    | (TransactionOperation::ResolveConflict, true, false, true)
            )
            && self
                .expected_identity
                .as_ref()
                .is_none_or(|identity| identity.validate().is_ok())
            && self
                .conflict_identities
                .iter()
                .all(|identity| identity.validate().is_ok())
            && (self.operation != TransactionOperation::ResolveConflict
                || self.expected_identity.as_ref().is_some_and(|expected| {
                    self.conflict_identities
                        .iter()
                        .any(|identity| identity == expected)
                }))
    }

    fn public_facts(self) -> PendingIdentityTransaction {
        PendingIdentityTransaction {
            operation: match self.operation {
                TransactionOperation::Create => PendingIdentityOperation::Create,
                TransactionOperation::Import => PendingIdentityOperation::Import,
                TransactionOperation::ResolveConflict => PendingIdentityOperation::ResolveConflict,
            },
            receipt_ref: self.receipt_ref,
            expected_identity: self.expected_identity,
        }
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
    conflict: Option<CustodyConflict>,
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
            conflict: None,
        }
    }

    fn conflict(
        reason: CustodyConflictReason,
        identities: impl IntoIterator<Item = PublicIdentity>,
        secret: Option<SecretKeyMaterial>,
    ) -> Self {
        let mut identities = identities.into_iter().collect::<Vec<_>>();
        identities.dedup();
        Self {
            result: CustodyResult {
                state: CustodyState::Conflict,
                identity: identities.first().cloned(),
                receipt_ref: None,
            },
            secret,
            conflict: Some(CustodyConflict { reason, identities }),
        }
    }
}

#[derive(Clone)]
struct CustodyPaths {
    manifest_path: PathBuf,
    completion_path: PathBuf,
    transaction_path: PathBuf,
    reset_path: PathBuf,
    recovery_protection_path: PathBuf,
}

impl CustodyPaths {
    fn for_data_root(root: PathBuf) -> Self {
        Self {
            manifest_path: root.join("identity.json"),
            completion_path: root.join("identity.complete.json"),
            transaction_path: root.join("identity.transaction.json"),
            reset_path: root.join("identity.reset.json"),
            recovery_protection_path: root.join("identity.recovery-protection.json"),
        }
    }
}

fn channel_data_root(channel: AppChannel) -> PathBuf {
    #[cfg(target_os = "macos")]
    {
        paths::home_dir()
            .join("Library/Application Support")
            .join(channel.display_name())
    }
    #[cfg(any(target_os = "linux", target_os = "freebsd"))]
    {
        let base = std::env::var_os("XDG_DATA_HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|| paths::home_dir().join(".local/share"));
        base.join(channel.storage_slug())
    }
    #[cfg(target_os = "windows")]
    {
        std::env::var_os("LOCALAPPDATA")
            .map(PathBuf::from)
            .unwrap_or_else(|| paths::home_dir().join("AppData").join("Local"))
            .join(channel.display_name())
    }
    #[cfg(not(any(
        target_os = "macos",
        target_os = "linux",
        target_os = "freebsd",
        target_os = "windows"
    )))]
    {
        paths::home_dir()
            .join(".config")
            .join(channel.storage_slug())
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
    #[error("the recovery protection record is invalid")]
    InvalidRecoveryProtection,
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
    use sha2::Digest as _;
    use zeroize::Zeroizing;

    use super::*;
    use crate::{RecoveryProtectionState, SigningPurpose, UnsignedEventTemplate};

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
        Unavailable,
        Conflict,
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

        fn replace_secret(&self, bytes: [u8; 32]) {
            let mut state = self.state.lock().expect("lock fake store");
            state.secret = Some(
                SecretKeyMaterial::from_bytes(Zeroizing::new(bytes))
                    .expect("valid replacement secret"),
            );
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
                FakeReadMode::Unavailable => Err(StoreError::Unavailable),
                FakeReadMode::Conflict => Err(StoreError::Conflict),
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
    fn committing_a_different_identity_invalidates_stale_recovery_protection() {
        let temporary_directory = tempfile::tempdir().expect("create temporary directory");
        let store = FakeStore::empty();
        let service = service(store, temporary_directory.path().to_path_buf());
        let stale_identity = SecretKeyMaterial::from_bytes(Zeroizing::new([2; 32]))
            .expect("valid stale secret")
            .public_identity()
            .expect("derive stale identity");
        let stale_record = RecoveryProtectionRecord::new(stale_identity, "a".repeat(64), 100)
            .expect("valid stale recovery protection");
        write_json_document(&service.paths.recovery_protection_path, &stale_record)
            .expect("write stale recovery protection");

        let created = service
            .create(receipt())
            .expect("create different identity");
        assert_ne!(
            created.identity.as_ref(),
            Some(stale_record.identity()),
            "fixture identities must differ"
        );
        assert!(!service.paths.recovery_protection_path.exists());
        assert_eq!(
            service
                .inspect_details()
                .expect("inspect unprotected new identity")
                .recovery_protection
                .state,
            crate::RecoveryProtectionState::Needed
        );
    }

    #[test]
    fn malformed_recovery_protection_never_blocks_ready_custody() {
        let temporary_directory = tempfile::tempdir().expect("create temporary directory");
        let store = FakeStore::empty();
        let service = service(store, temporary_directory.path().to_path_buf());
        let created = service.create(receipt()).expect("create identity");
        std::fs::write(
            &service.paths.recovery_protection_path,
            b"not a recovery protection record",
        )
        .expect("write malformed recovery protection");

        let inspection = service
            .inspect_details()
            .expect("inspect through malformed recovery protection");
        assert_eq!(inspection.custody, created);
        assert_eq!(
            inspection.recovery_protection.state,
            crate::RecoveryProtectionState::Needed
        );
        assert!(inspection.recovery_protection.record.is_none());
        assert!(!service.paths.recovery_protection_path.exists());
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
    fn concurrent_distinct_create_receipts_commit_exactly_one_identity() {
        let temporary_directory = tempfile::tempdir().expect("create temporary directory");
        let data_root = temporary_directory.path().to_path_buf();
        let store = FakeStore::empty();
        let generator_calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let first_service =
            counting_service(store.clone(), data_root.clone(), generator_calls.clone());
        let second_service =
            counting_service(store.clone(), data_root.clone(), generator_calls.clone());
        let start = Arc::new(std::sync::Barrier::new(3));

        let first = std::thread::spawn({
            let start = start.clone();
            move || {
                start.wait();
                first_service.create(
                    ReceiptRef::new("concurrent-owner-action-1")
                        .expect("valid first concurrent receipt"),
                )
            }
        });
        let second = std::thread::spawn({
            let start = start.clone();
            move || {
                start.wait();
                second_service.create(
                    ReceiptRef::new("concurrent-owner-action-2")
                        .expect("valid second concurrent receipt"),
                )
            }
        });
        start.wait();

        let first = first.join().expect("join first create");
        let second = second.join().expect("join second create");
        let successes = [&first, &second]
            .into_iter()
            .filter(|result| {
                result
                    .as_ref()
                    .is_ok_and(|result| result.state == CustodyState::Ready)
            })
            .count();
        let denials = [&first, &second]
            .into_iter()
            .filter(|result| {
                matches!(
                    result,
                    Err(CustodyError::CustodyDenied(CustodyState::Ready))
                )
            })
            .count();

        assert_eq!(successes, 1);
        assert_eq!(denials, 1);
        assert_eq!(generator_calls.load(std::sync::atomic::Ordering::SeqCst), 1);
        assert_eq!(store.state.lock().expect("lock fake store").writes, 1);
        assert_eq!(
            service(store, data_root)
                .inspect()
                .expect("inspect winning identity")
                .state,
            CustodyState::Ready
        );
    }

    #[test]
    fn unavailable_secure_store_blocks_create_before_journaling_or_generation() {
        let temporary_directory = tempfile::tempdir().expect("create temporary directory");
        let store = FakeStore::empty();
        let generator_calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let service = counting_service(
            store.clone(),
            temporary_directory.path().to_path_buf(),
            generator_calls.clone(),
        );
        store.set_read_mode(FakeReadMode::Unavailable);

        assert!(matches!(
            service.create(receipt()),
            Err(CustodyError::CustodyDenied(CustodyState::Locked))
        ));
        assert_eq!(generator_calls.load(std::sync::atomic::Ordering::SeqCst), 0);
        assert_eq!(store.state.lock().expect("lock fake store").writes, 0);
        assert!(!service.paths.transaction_path.exists());
        assert!(!service.paths.manifest_path.exists());
        assert!(!service.paths.completion_path.exists());
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
    fn pending_create_facts_survive_restart_and_resume_the_original_receipt() {
        let temporary_directory = tempfile::tempdir().expect("create temporary directory");
        let store = FakeStore::empty();
        let generator_calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let first_service = counting_service(
            store.clone(),
            temporary_directory.path().to_path_buf(),
            generator_calls.clone(),
        );
        let pending_receipt =
            ReceiptRef::new("restart-create-receipt").expect("valid pending receipt");
        write_json_document(
            &first_service.paths.transaction_path,
            &IdentityTransaction::new(TransactionOperation::Create, pending_receipt.clone(), None),
        )
        .expect("write pending create");

        let restarted_service = counting_service(
            store,
            temporary_directory.path().to_path_buf(),
            generator_calls.clone(),
        );
        let inspection = restarted_service
            .inspect_details()
            .expect("inspect pending create after restart");
        assert_eq!(inspection.custody.state, CustodyState::Incomplete);
        assert_eq!(
            inspection.pending_transaction,
            Some(PendingIdentityTransaction {
                operation: PendingIdentityOperation::Create,
                receipt_ref: pending_receipt.clone(),
                expected_identity: None,
            })
        );

        let resumed = restarted_service
            .resume_incomplete_create()
            .expect("resume pending create");
        assert_eq!(resumed.state, CustodyState::Ready);
        assert_eq!(resumed.receipt_ref, Some(pending_receipt));
        assert_eq!(generator_calls.load(std::sync::atomic::Ordering::SeqCst), 1);
        assert!(
            restarted_service
                .inspect_details()
                .expect("inspect completed create")
                .pending_transaction
                .is_none()
        );
    }

    #[test]
    fn restart_after_manifest_commit_resumes_same_identity_and_receipt() {
        let temporary_directory = tempfile::tempdir().expect("create temporary directory");
        let data_root = temporary_directory.path().to_path_buf();
        let store = FakeStore::empty();
        let generator_calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let initial_service =
            counting_service(store.clone(), data_root.clone(), generator_calls.clone());
        let pending_receipt =
            ReceiptRef::new("manifest-commit-crash").expect("valid pending receipt");
        let secret = SecretKeyMaterial::from_bytes(Zeroizing::new([4; 32]))
            .expect("valid pending identity secret");
        let identity = secret
            .public_identity()
            .expect("derive pending public identity");
        store
            .write(&initial_service.locator, &secret)
            .expect("write pending identity secret");
        let mut transaction =
            IdentityTransaction::new(TransactionOperation::Create, pending_receipt.clone(), None);
        transaction.expected_identity = Some(identity.clone());
        write_json_document(&initial_service.paths.transaction_path, &transaction)
            .expect("write pending transaction");
        let manifest = IdentityManifest::new(
            identity.clone(),
            initial_service.locator.clone(),
            vec![pending_receipt.clone()],
        );
        write_identity_manifest(
            &initial_service.paths.manifest_path,
            &manifest,
            initial_service.channel,
        )
        .expect("write committed manifest");
        assert!(!initial_service.paths.completion_path.exists());

        let restarted_service = counting_service(store, data_root, generator_calls.clone());
        let inspection = restarted_service
            .inspect_details()
            .expect("inspect manifest-only crash state");
        assert_eq!(inspection.custody.state, CustodyState::Incomplete);
        assert_eq!(inspection.custody.identity, Some(identity.clone()));
        assert_eq!(
            inspection
                .pending_transaction
                .expect("pending create facts")
                .receipt_ref,
            pending_receipt
        );

        let resumed = restarted_service
            .resume_incomplete_create()
            .expect("resume after manifest commit");
        assert_eq!(resumed.state, CustodyState::Ready);
        assert_eq!(resumed.identity, Some(identity));
        assert_eq!(resumed.receipt_ref, Some(pending_receipt));
        assert_eq!(generator_calls.load(std::sync::atomic::Ordering::SeqCst), 0);
        assert!(!restarted_service.paths.transaction_path.exists());
    }

    #[test]
    fn restart_after_completion_commit_cleans_matching_create_journal() {
        let temporary_directory = tempfile::tempdir().expect("create temporary directory");
        let data_root = temporary_directory.path().to_path_buf();
        let store = FakeStore::empty();
        let generator_calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let initial_service =
            counting_service(store.clone(), data_root.clone(), generator_calls.clone());
        let completed_receipt =
            ReceiptRef::new("completion-commit-crash").expect("valid completed receipt");
        let completed = initial_service
            .create(completed_receipt.clone())
            .expect("create completed identity");
        let identity = completed.identity.clone().expect("completed identity");
        let mut interrupted_transaction = IdentityTransaction::new(
            TransactionOperation::Create,
            completed_receipt.clone(),
            None,
        );
        interrupted_transaction.expected_identity = Some(identity.clone());
        write_json_document(
            &initial_service.paths.transaction_path,
            &interrupted_transaction,
        )
        .expect("restore journal left at crash boundary");
        let calls_before_restart = generator_calls.load(std::sync::atomic::Ordering::SeqCst);
        let writes_before_restart = store.state.lock().expect("lock fake store").writes;

        let restarted_service = counting_service(store.clone(), data_root, generator_calls.clone());
        let inspection = restarted_service
            .inspect_details()
            .expect("inspect completed crash state");
        assert_eq!(inspection.custody, completed);
        assert_eq!(
            inspection
                .pending_transaction
                .expect("matching create journal")
                .receipt_ref,
            completed_receipt
        );

        let resumed = restarted_service
            .create(completed_receipt)
            .expect("idempotently finalize matching create");
        assert_eq!(resumed.state, CustodyState::Ready);
        assert_eq!(resumed.identity, Some(identity));
        assert!(!restarted_service.paths.transaction_path.exists());
        assert_eq!(
            generator_calls.load(std::sync::atomic::Ordering::SeqCst),
            calls_before_restart
        );
        assert_eq!(
            store.state.lock().expect("lock fake store").writes,
            writes_before_restart
        );
    }

    #[test]
    fn pending_create_with_known_missing_identity_never_rotates() {
        let temporary_directory = tempfile::tempdir().expect("create temporary directory");
        let store = FakeStore::empty();
        let generator_calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let service = counting_service(
            store,
            temporary_directory.path().to_path_buf(),
            generator_calls.clone(),
        );
        let expected_identity = SecretKeyMaterial::from_bytes(Zeroizing::new([4; 32]))
            .expect("valid pending identity secret")
            .public_identity()
            .expect("derive pending identity");
        let mut transaction =
            IdentityTransaction::new(TransactionOperation::Create, receipt(), None);
        transaction.expected_identity = Some(expected_identity);
        write_json_document(&service.paths.transaction_path, &transaction)
            .expect("write pending transaction");

        assert!(matches!(
            service.resume_incomplete_create(),
            Err(CustodyError::TransactionIncomplete)
        ));
        assert_eq!(generator_calls.load(std::sync::atomic::Ordering::SeqCst), 0);
        assert!(service.paths.transaction_path.exists());
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
    fn inspection_distinguishes_keychain_ambiguity_from_identity_mismatch() {
        let temporary_directory = tempfile::tempdir().expect("create temporary directory");
        let store = FakeStore::empty();
        let service = service(store.clone(), temporary_directory.path().to_path_buf());
        let created = service.create(receipt()).expect("create identity");
        let manifest_identity = created.identity.expect("created public identity");

        store.set_read_mode(FakeReadMode::Conflict);
        let ambiguous = service
            .inspect_details()
            .expect("inspect ambiguous secure store");
        assert_eq!(ambiguous.custody.state, CustodyState::Conflict);
        assert_eq!(
            ambiguous.conflict,
            Some(CustodyConflict {
                reason: CustodyConflictReason::AmbiguousSecureStore,
                identities: vec![manifest_identity.clone()],
            })
        );

        store.set_read_mode(FakeReadMode::Substitute([2; 32]));
        let mismatch = service
            .inspect_details()
            .expect("inspect manifest and custody mismatch");
        let mismatch = mismatch.conflict.expect("mismatch details");
        assert_eq!(
            mismatch.reason,
            CustodyConflictReason::PublicManifestCustodyMismatch
        );
        assert_eq!(mismatch.identities.len(), 2);
        assert_eq!(mismatch.identities[0], manifest_identity);
        assert_ne!(mismatch.identities[0], mismatch.identities[1]);
    }

    #[test]
    fn owner_selected_recovery_repairs_a_public_manifest_custody_conflict() {
        let temporary_directory = tempfile::tempdir().expect("create temporary directory");
        let store = FakeStore::empty();
        let service = service(store.clone(), temporary_directory.path().to_path_buf());
        let original = service.create(receipt()).expect("create original identity");
        let original_identity = original.identity.expect("original public identity");
        store.replace_secret([2; 32]);

        let conflict = service
            .inspect_details()
            .expect("inspect identity conflict")
            .conflict
            .expect("public conflict details");
        assert_eq!(
            conflict.reason,
            CustodyConflictReason::PublicManifestCustodyMismatch
        );
        let custody_identity = conflict
            .identities
            .iter()
            .find(|identity| *identity != &original_identity)
            .expect("custody identity")
            .clone();
        let resolution_receipt =
            ReceiptRef::new("conflict-resolution-1").expect("valid resolution receipt");
        let selected = service
            .prepare_import(
                ImportedSecret::new(hex::encode([2; 32])).expect("valid custody import"),
            )
            .map(|prepared| select_one(&service, prepared))
            .expect("prepare selected custody identity");

        let resolved = service
            .resolve_conflict(selected, resolution_receipt.clone())
            .expect("resolve identity conflict");
        assert_eq!(resolved.state, CustodyState::Ready);
        assert_eq!(resolved.identity, Some(custody_identity.clone()));
        assert_eq!(resolved.receipt_ref, Some(resolution_receipt.clone()));
        assert!(
            service
                .inspect_details()
                .expect("inspect repaired identity")
                .conflict
                .is_none()
        );

        let repeated = service
            .prepare_import(
                ImportedSecret::new(hex::encode([2; 32])).expect("valid repeated import"),
            )
            .map(|prepared| select_one(&service, prepared))
            .and_then(|selected| service.resolve_conflict(selected, resolution_receipt))
            .expect("repeat conflict resolution idempotently");
        assert_eq!(repeated.identity, Some(custody_identity));
    }

    #[test]
    fn conflict_resolution_rejects_an_identity_outside_the_inspected_conflict() {
        let temporary_directory = tempfile::tempdir().expect("create temporary directory");
        let store = FakeStore::empty();
        let service = service(store.clone(), temporary_directory.path().to_path_buf());
        service.create(receipt()).expect("create original identity");
        store.replace_secret([2; 32]);
        let unrelated = service
            .prepare_import(
                ImportedSecret::new(hex::encode([3; 32])).expect("valid unrelated import"),
            )
            .map(|prepared| select_one(&service, prepared))
            .expect("prepare unrelated identity");

        assert!(matches!(
            service.resolve_conflict(
                unrelated,
                ReceiptRef::new("conflict-resolution-2").expect("valid resolution receipt")
            ),
            Err(CustodyError::CustodyDenied(CustodyState::Conflict))
        ));
        assert_eq!(
            service
                .inspect_details()
                .expect("conflict remains")
                .custody
                .state,
            CustodyState::Conflict
        );
    }

    #[test]
    fn conflict_resolution_journal_retries_after_verified_delete_failure() {
        let temporary_directory = tempfile::tempdir().expect("create temporary directory");
        let store = FakeStore::empty();
        let service = service(store.clone(), temporary_directory.path().to_path_buf());
        let original = service.create(receipt()).expect("create original identity");
        let original_identity = original.identity.expect("original public identity");
        store.replace_secret([2; 32]);
        store.set_fail_delete(true);
        let resolution_receipt =
            ReceiptRef::new("conflict-resolution-3").expect("valid resolution receipt");
        let selected = service
            .prepare_import(
                ImportedSecret::new(hex::encode([1; 32])).expect("valid original import"),
            )
            .map(|prepared| select_one(&service, prepared))
            .expect("prepare original identity");

        assert!(matches!(
            service.resolve_conflict(selected, resolution_receipt.clone()),
            Err(CustodyError::TransactionIncomplete)
        ));
        let pending = service
            .inspect_details()
            .expect("inspect pending conflict resolution")
            .pending_transaction
            .expect("pending conflict transaction");
        assert_eq!(pending.operation, PendingIdentityOperation::ResolveConflict);
        assert_eq!(pending.receipt_ref, resolution_receipt);

        store.set_fail_delete(false);
        let selected = service
            .prepare_import(ImportedSecret::new(hex::encode([1; 32])).expect("valid retry import"))
            .map(|prepared| select_one(&service, prepared))
            .expect("prepare retry identity");
        let resolved = service
            .resolve_conflict(selected, pending.receipt_ref)
            .expect("retry conflict resolution");
        assert_eq!(resolved.state, CustodyState::Ready);
        assert_eq!(resolved.identity, Some(original_identity));
        assert!(!service.paths.transaction_path.exists());
    }

    #[test]
    fn conflict_resolution_is_not_ready_until_its_receipt_is_completed() {
        let temporary_directory = tempfile::tempdir().expect("create temporary directory");
        let store = FakeStore::empty();
        let service = service(store, temporary_directory.path().to_path_buf());
        let created = service.create(receipt()).expect("create identity");
        let identity = created.identity.expect("created identity");
        let other_identity = SecretKeyMaterial::from_bytes(Zeroizing::new([2; 32]))
            .expect("valid other secret")
            .public_identity()
            .expect("derive other identity");
        let pending_receipt =
            ReceiptRef::new("conflict-resolution-crash").expect("valid pending receipt");
        let mut transaction = IdentityTransaction::new(
            TransactionOperation::ResolveConflict,
            pending_receipt.clone(),
            Some(
                CandidateRef::new("advanced-nostr-import".to_string())
                    .expect("valid candidate reference"),
            ),
        );
        transaction.expected_identity = Some(identity);
        transaction.conflict_identities = vec![
            transaction
                .expected_identity
                .clone()
                .expect("expected identity"),
            other_identity,
        ];
        write_json_document(&service.paths.transaction_path, &transaction)
            .expect("write interrupted conflict transaction");

        let inspection = service
            .inspect_details()
            .expect("inspect interrupted conflict resolution");
        assert_eq!(inspection.custody.state, CustodyState::Incomplete);
        assert_eq!(
            inspection
                .pending_transaction
                .expect("pending conflict resolution")
                .receipt_ref,
            pending_receipt
        );
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
    fn owner_attestation_matches_the_nip_oa_domain_without_exporting_custody() {
        let temporary_directory = tempfile::tempdir().expect("create temporary directory");
        let service = service(FakeStore::empty(), temporary_directory.path().to_path_buf());
        let identity = service
            .create(receipt())
            .expect("create identity")
            .identity
            .expect("created public identity");
        let agent_public_key_hex = NostrPublicKeyHex::new("2".repeat(64)).expect("agent key");
        let result = service
            .sign_owner_attestation(&OwnerAttestationRequest {
                request_ref: ReceiptRef::new("attest-sarah").expect("request ref"),
                identity_ref: identity.identity_ref().clone(),
                agent_public_key_hex: agent_public_key_hex.clone(),
                conditions: String::new(),
            })
            .expect("sign owner attestation");

        assert_eq!(result.agent_public_key_hex, agent_public_key_hex);
        assert_eq!(result.auth_tag[0], "auth");
        assert_eq!(result.auth_tag[1], identity.public_key_hex().as_str());
        assert_eq!(result.auth_tag[2], "");
        let digest = Sha256::digest(
            format!("nostr:agent-auth:{}:", result.agent_public_key_hex.as_str()).as_bytes(),
        );
        let message = Message::from_digest(digest.into());
        let signature = result.auth_tag[3]
            .parse::<nostr::secp256k1::schnorr::Signature>()
            .expect("signature");
        nostr::SECP256K1
            .verify_schnorr(
                &signature,
                &message,
                &identity
                    .public_key_hex()
                    .public_key()
                    .expect("owner public key")
                    .xonly()
                    .expect("x-only owner key"),
            )
            .expect("verify NIP-OA signature");
        assert!(
            !serde_json::to_string(&result)
                .expect("serialize result")
                .contains("private")
        );
    }

    #[test]
    fn nip44_self_encryption_never_signs_the_plaintext_template() {
        let temporary_directory = tempfile::tempdir().expect("create temporary directory");
        let service = service(FakeStore::empty(), temporary_directory.path().to_path_buf());
        let identity = service
            .create(receipt())
            .expect("create identity")
            .identity
            .expect("created public identity");
        let request = AdmittedSigningRequest {
            request_ref: ReceiptRef::new("nip44-self-record").expect("request receipt"),
            identity_ref: identity.identity_ref().clone(),
            purpose: crate::SigningPurpose::Nip44EncryptedSelfEvent,
            event: UnsignedEventTemplate {
                created_at: 1_700_000_002,
                kind: 30_078,
                tags: vec![vec!["d".into(), "read-state:mobile".into()]],
                content: "{\"v\":1,\"client_id\":\"mobile\",\"contexts\":{}}".into(),
            },
        };

        assert!(service.sign(&request).is_err());
        let signed = service
            .sign_nip44_encrypted_to_self(&request)
            .expect("encrypt and sign");
        let event = Event::from_json(&signed.signed_event_json).expect("signed event");
        event.verify().expect("verify signed event");
        assert_ne!(event.content, request.event.content);
        assert!(!event.content.contains("client_id"));
        assert_eq!(
            service
                .decrypt_nip44_from_self(&event.content)
                .expect("decrypt self record"),
            request.event.content
        );
    }

    #[test]
    fn nip17_wrap_and_unwrap_keep_secret_material_inside_custody() {
        let temporary_directory = tempfile::tempdir().expect("create temporary directory");
        let sender = service(
            FakeStore::empty(),
            temporary_directory.path().join("sender"),
        );
        let recipient = IdentityService::new(
            AppChannel::Dev,
            CustodyPaths::for_data_root(temporary_directory.path().join("recipient")),
            FakeStore::empty(),
            Arc::new(FixedGenerator([2; 32])),
        );
        let sender_identity = sender
            .create(ReceiptRef::new("sender-create").expect("sender receipt"))
            .expect("create sender")
            .identity
            .expect("sender identity");
        let recipient_identity = recipient
            .create(ReceiptRef::new("recipient-create").expect("recipient receipt"))
            .expect("create recipient")
            .identity
            .expect("recipient identity");
        let recipient_public_key = recipient_identity.public_key_hex().clone();
        let request = PrivateMessageRequest {
            request_ref: ReceiptRef::new("private-message-1").expect("private receipt"),
            identity_ref: sender_identity.identity_ref().clone(),
            recipients: vec![recipient_public_key.clone()],
            rumor: UnsignedEventTemplate {
                created_at: 1_700_000_001,
                kind: Kind::PrivateDirectMessage.as_u16(),
                tags: vec![
                    vec!["p".to_string(), recipient_public_key.as_str().to_string()],
                    vec!["conversation".to_string(), "sarah.fixture".to_string()],
                ],
                content: "owner-private".to_string(),
            },
        };

        let wrapped = sender
            .gift_wrap_private_message(&request)
            .expect("gift wrap");
        assert_eq!(wrapped.len(), 1);
        let outer = Event::from_json(&wrapped[0].gift_wrap_event_json).expect("outer event");
        assert_eq!(outer.kind, Kind::GiftWrap);
        assert!(!outer.content.contains("owner-private"));

        let unwrapped = recipient
            .unwrap_private_message(&wrapped[0].gift_wrap_event_json)
            .expect("recipient unwrap");
        assert_eq!(unwrapped.rumor_event_id, wrapped[0].rumor_event_id);
        assert_eq!(
            unwrapped.sender_public_key_hex,
            sender_identity.public_key_hex().as_str()
        );
        assert_eq!(unwrapped.content, "owner-private");
        assert!(
            sender
                .unwrap_private_message(&wrapped[0].gift_wrap_event_json)
                .is_err()
        );
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
        let inspection = source_service
            .inspect_details()
            .expect("inspect protected identity");
        assert_eq!(
            inspection.recovery_protection.state,
            crate::RecoveryProtectionState::Protected
        );
        let protection = inspection
            .recovery_protection
            .record
            .expect("recovery protection record");
        assert_eq!(protection.identity(), &identity);
        assert_eq!(protection.byte_length(), artifact.byte_length());
        assert_eq!(
            protection.artifact_digest(),
            hex::encode(sha2::Sha256::digest(&artifact_bytes))
        );
        let protection_document =
            std::fs::read_to_string(&source_service.paths.recovery_protection_path)
                .expect("read recovery protection document");
        assert!(!protection_document.contains(artifact_path.to_string_lossy().as_ref()));
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
        let artifact_directory = tempfile::tempdir().expect("create artifact directory");
        service
            .export_recovery_artifact(
                identity.identity_ref(),
                &artifact_directory.path().join("recovery.ncryptsec"),
                RecoveryPassword::new("reset protection password".to_string())
                    .expect("valid recovery password"),
            )
            .expect("protect identity recovery");
        assert!(service.paths.recovery_protection_path.exists());
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
        assert!(!service.paths.recovery_protection_path.exists());
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
    fn process_start_acknowledges_reset_complete_at_entry() {
        let temporary_directory = tempfile::tempdir().expect("create temporary directory");
        let data_root = temporary_directory.path().to_path_buf();
        let store = FakeStore::empty();
        let initial_service = service(store.clone(), data_root.clone());
        let created = initial_service.create(receipt()).expect("create identity");
        let identity = created.identity.expect("created public identity");
        initial_service
            .reset(
                identity.identity_ref(),
                ReceiptRef::new("process-start-complete").expect("valid reset receipt"),
            )
            .expect("record reset intent");
        assert_eq!(
            initial_service
                .inspect()
                .expect("complete pending reset")
                .state,
            CustodyState::RelaunchRequired
        );

        let restarted_service = service(store, data_root);
        let inspection = restarted_service
            .inspect_for_process_start()
            .expect("inspect completed reset on process start");

        assert_eq!(
            inspection.custody,
            CustodyResult::for_state(CustodyState::Absent)
        );
        assert_eq!(
            inspection.recovery_protection.state,
            RecoveryProtectionState::NotApplicable
        );
        assert!(inspection.pending_transaction.is_none());
        assert!(inspection.conflict.is_none());
        assert!(!restarted_service.paths.reset_path.exists());
    }

    #[test]
    fn process_start_preserves_reset_completed_during_same_call() {
        let temporary_directory = tempfile::tempdir().expect("create temporary directory");
        let data_root = temporary_directory.path().to_path_buf();
        let store = FakeStore::empty();
        let initial_service = service(store.clone(), data_root.clone());
        let created = initial_service.create(receipt()).expect("create identity");
        let identity = created.identity.expect("created public identity");
        initial_service
            .reset(
                identity.identity_ref(),
                ReceiptRef::new("process-start-pending").expect("valid reset receipt"),
            )
            .expect("record reset intent");

        let inspection = initial_service
            .inspect_for_process_start()
            .expect("resume pending reset on process start");

        assert_eq!(inspection.custody.state, CustodyState::RelaunchRequired);
        assert_eq!(
            initial_service
                .read_reset_marker_locked()
                .expect("read reset marker")
                .expect("completed reset marker")
                .status,
            ResetStatus::Complete
        );
        assert_eq!(
            initial_service
                .inspect_details()
                .expect("inspect again in same process")
                .custody
                .state,
            CustodyState::RelaunchRequired
        );

        let restarted_service = service(store, data_root);
        assert_eq!(
            restarted_service
                .inspect_for_process_start()
                .expect("acknowledge reset on next process start")
                .custody
                .state,
            CustodyState::Absent
        );
        assert!(!restarted_service.paths.reset_path.exists());
    }

    #[test]
    fn process_start_preserves_failed_reset_resumed_during_same_call() {
        let temporary_directory = tempfile::tempdir().expect("create temporary directory");
        let data_root = temporary_directory.path().to_path_buf();
        let store = FakeStore::empty();
        let initial_service = service(store.clone(), data_root.clone());
        let created = initial_service.create(receipt()).expect("create identity");
        let identity = created.identity.expect("created public identity");
        initial_service
            .reset(
                identity.identity_ref(),
                ReceiptRef::new("process-start-failed").expect("valid reset receipt"),
            )
            .expect("record reset intent");
        store.set_fail_delete(true);
        assert_eq!(
            initial_service.inspect().expect("fail pending reset").state,
            CustodyState::ResetFailed
        );
        assert_eq!(
            initial_service
                .read_reset_marker_locked()
                .expect("read reset marker")
                .expect("failed reset marker")
                .status,
            ResetStatus::Failed
        );

        store.set_fail_delete(false);
        let inspection = initial_service
            .inspect_for_process_start()
            .expect("resume failed reset on process start");

        assert_eq!(inspection.custody.state, CustodyState::RelaunchRequired);
        assert_eq!(
            initial_service
                .read_reset_marker_locked()
                .expect("read reset marker")
                .expect("completed reset marker")
                .status,
            ResetStatus::Complete
        );

        let restarted_service = service(store, data_root);
        assert_eq!(
            restarted_service
                .inspect_for_process_start()
                .expect("acknowledge reset on next process start")
                .custody
                .state,
            CustodyState::Absent
        );
        assert!(!restarted_service.paths.reset_path.exists());
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
