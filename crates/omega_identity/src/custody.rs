use std::{path::PathBuf, sync::Arc};

use app_identity::AppChannel;
use nostr::JsonUtil;
use thiserror::Error;

use crate::{
    AdmittedSigningRequest, CompletionRecord, ContractError, CustodyResult, CustodyState,
    IdentityManifest, IdentityRef, ImportedSecret, KeyringLocator, PublicIdentity,
    PublicStoreError, ReceiptRef, SigningResult,
    mutation_lock::{IdentityMutationGuard, MutationLockError},
    public_store::{
        read_completion_record, read_identity_manifest, remove_public_document,
        write_completion_record, write_identity_manifest,
    },
    secret::{SecretKeyMaterial, SecretStore, StoreError, SystemKeyringStore},
};

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
        Ok(self.resolve_locked().result)
    }

    pub fn open(&self, expected_identity: &IdentityRef) -> Result<CustodyResult, CustodyError> {
        let _mutation_guard = IdentityMutationGuard::acquire(&self.locator)?;
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
        self.require_absent_locked()?;
        self.commit_secret_locked(self.generator.generate(), receipt_ref)
    }

    pub fn import(
        &self,
        imported_secret: ImportedSecret,
        receipt_ref: ReceiptRef,
    ) -> Result<CustodyResult, CustodyError> {
        let secret = SecretKeyMaterial::from_imported(imported_secret)
            .map_err(|_| CustodyError::InvalidImportedSecret)?;
        let _mutation_guard = IdentityMutationGuard::acquire(&self.locator)?;
        self.require_absent_locked()?;
        self.commit_secret_locked(secret, receipt_ref)
    }

    pub fn sign(&self, request: &AdmittedSigningRequest) -> Result<SigningResult, CustodyError> {
        let _mutation_guard = IdentityMutationGuard::acquire(&self.locator)?;
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
        let resolved = self.resolve_locked();
        if let Some(identity) = &resolved.result.identity {
            if identity.identity_ref() != expected_identity {
                return Err(CustodyError::CustodyDenied(CustodyState::Conflict));
            }
        } else {
            return Err(CustodyError::CustodyDenied(resolved.result.state));
        }

        self.store
            .delete(&self.locator)
            .map_err(|_| CustodyError::ResetFailed)?;
        match self.store.read(&self.locator) {
            Ok(None) => {}
            Ok(Some(_)) | Err(_) => return Err(CustodyError::ResetFailed),
        }

        remove_public_document(&self.paths.completion_path)
            .map_err(|_| CustodyError::ResetFailed)?;
        remove_public_document(&self.paths.manifest_path).map_err(|_| CustodyError::ResetFailed)?;

        Ok(CustodyResult {
            state: CustodyState::RelaunchRequired,
            identity: None,
            receipt_ref: Some(authorization_ref),
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

    fn require_absent_locked(&self) -> Result<(), CustodyError> {
        let state = self.resolve_locked().result.state;
        if state != CustodyState::Absent {
            return Err(CustodyError::CustodyDenied(state));
        }
        Ok(())
    }

    fn commit_secret_locked(
        &self,
        secret: SecretKeyMaterial,
        receipt_ref: ReceiptRef,
    ) -> Result<CustodyResult, CustodyError> {
        let expected_identity = secret
            .public_identity()
            .map_err(|_| CustodyError::InvalidImportedSecret)?;
        self.store
            .write(&self.locator, &secret)
            .map_err(custody_error_from_store)?;

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

        let manifest = IdentityManifest::new(
            expected_identity.clone(),
            self.locator.clone(),
            vec![receipt_ref.clone()],
        );
        write_identity_manifest(&self.paths.manifest_path, &manifest, self.channel)?;
        let completion = CompletionRecord::new(&manifest, receipt_ref.clone(), self.channel)?;
        write_completion_record(
            &self.paths.completion_path,
            &completion,
            &manifest,
            self.channel,
        )?;

        Ok(CustodyResult {
            state: CustodyState::Ready,
            identity: Some(expected_identity),
            receipt_ref: Some(receipt_ref),
        })
    }

    fn resolve_locked(&self) -> ResolvedCustody {
        let manifest = match read_identity_manifest(&self.paths.manifest_path, self.channel) {
            Ok(manifest) => manifest,
            Err(_) => {
                return ResolvedCustody::without_secret(CustodyState::Incomplete, None);
            }
        };
        let secret = match self.store.read(&self.locator) {
            Ok(secret) => secret,
            Err(StoreError::Locked | StoreError::Unavailable) => {
                return ResolvedCustody::without_secret(
                    CustodyState::Locked,
                    manifest.map(|manifest| manifest.identity().clone()),
                );
            }
            Err(StoreError::Conflict) => {
                return ResolvedCustody::without_secret(
                    CustodyState::Conflict,
                    manifest.map(|manifest| manifest.identity().clone()),
                );
            }
            Err(StoreError::Corrupt | StoreError::Configuration) => {
                return ResolvedCustody::without_secret(
                    CustodyState::Incomplete,
                    manifest.map(|manifest| manifest.identity().clone()),
                );
            }
        };

        match (manifest, secret) {
            (None, None) => ResolvedCustody::without_secret(CustodyState::Absent, None),
            (Some(manifest), None) => ResolvedCustody::without_secret(
                CustodyState::Lost,
                Some(manifest.identity().clone()),
            ),
            (None, Some(secret)) => match secret.public_identity() {
                Ok(identity) => ResolvedCustody {
                    result: CustodyResult {
                        state: CustodyState::Incomplete,
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
}

impl CustodyPaths {
    fn for_data_root(root: PathBuf) -> Self {
        Self {
            manifest_path: root.join("identity.json"),
            completion_path: root.join("identity.complete.json"),
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
    #[error("identity signing failed")]
    SigningFailed,
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
                }),
            })
        }

        fn set_read_mode(&self, mode: FakeReadMode) {
            self.state.lock().expect("lock fake store").read_mode = mode;
        }

        fn set_mode_after_write(&self, mode: FakeReadMode) {
            self.state.lock().expect("lock fake store").mode_after_write = Some(mode);
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
            self.state.lock().expect("lock fake store").secret = None;
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

    fn service(store: Arc<FakeStore>, data_root: PathBuf) -> IdentityService {
        IdentityService::new(
            AppChannel::Dev,
            CustodyPaths::for_data_root(data_root),
            store,
            Arc::new(FixedGenerator([1; 32])),
        )
    }

    fn receipt() -> ReceiptRef {
        ReceiptRef::new("owner-action-1").expect("valid test receipt")
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
        }
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
            service.inspect().expect("inspect after reset").state,
            CustodyState::Absent
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
