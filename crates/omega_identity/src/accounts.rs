use std::{
    fs, io,
    path::{Path, PathBuf},
};

use app_identity::AppChannel;
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};
use thiserror::Error;

use crate::{
    AccountLifecycleState, AccountRef, CustodyError, DurableIdentityActionDescriptor,
    HeldIdentityAction, IdentityAccountRecord, IdentityActionAuthorization, IdentityService,
    KeyringLocator, Nip46Capability, Nip46CapabilityState, Nip46Service, PublicIdentity,
    ReceiptRef, RecoveryProtectionState, SignerAvailability, SignerKind,
    mutation_lock::IdentityMutationGuard,
    public_store::{read_json_document, remove_public_document, write_json_document},
    secret::{FileSecretStore, SecretStore, StoreError},
};

const ACCOUNT_REGISTRY_SCHEMA: &str = "openagents.omega.account-registry.v1";
const ACCOUNT_SWITCH_SCHEMA: &str = "openagents.omega.account-switch.v1";
const ACCOUNT_ADD_SCHEMA: &str = "openagents.omega.account-add.v1";
const ACCOUNT_PURGE_SCHEMA: &str = "openagents.omega.account-purge.v1";
const ACCOUNT_REGISTRY_VERSION: u8 = 1;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AccountSelectionToken {
    pub account_ref: AccountRef,
    pub identity: PublicIdentity,
    pub generation: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ActiveAccountSelection {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub account_ref: Option<AccountRef>,
    pub generation: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AccountProfileSummary {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub avatar_ref: Option<String>,
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AccountRetirementState {
    NotRetired,
    Pending,
    Published,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AccountSignerSummary {
    pub kind: SignerKind,
    pub availability: SignerAvailability,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_successful_use: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AccountDashboardEntry {
    pub account_ref: AccountRef,
    pub identity: PublicIdentity,
    pub fingerprint: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub profile: Option<AccountProfileSummary>,
    pub lifecycle: AccountLifecycleState,
    pub signer: AccountSignerSummary,
    pub recovery: RecoveryProtectionState,
    pub retirement: AccountRetirementState,
    pub is_active: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AccountDashboardProjection {
    pub active: ActiveAccountSelection,
    #[serde(default)]
    pub accounts: Vec<AccountDashboardEntry>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pending_switch: Option<PendingAccountSwitch>,
    #[serde(default)]
    pub pending_purges: Vec<AccountPurgeReport>,
}

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum RemoteSignerRuntimeOutcome {
    Ready { used_at: u64 },
    Offline,
    Rejected,
    Revoked,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum AccountStorageLocator {
    LegacyRoot,
    Partitioned { directory_name: String },
    RemoteNip46 { capability_ref: String },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct DurableAccountEntry {
    account_ref: AccountRef,
    identity: PublicIdentity,
    storage: AccountStorageLocator,
    lifecycle: AccountLifecycleState,
    signer: AccountSignerSummary,
    recovery: RecoveryProtectionState,
    retirement: AccountRetirementState,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    profile: Option<AccountProfileSummary>,
    updated_at: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct AccountRegistry {
    schema: String,
    schema_version: u8,
    active: ActiveAccountSelection,
    #[serde(default)]
    accounts: Vec<DurableAccountEntry>,
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum SwitchPhase {
    Prepared,
    RegistryCommitted,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct AccountSwitchJournal {
    schema: String,
    from_account_ref: Option<AccountRef>,
    to_account_ref: AccountRef,
    expected_generation: u64,
    next_generation: u64,
    phase: SwitchPhase,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct AccountAddJournal {
    schema: String,
    receipt_ref: ReceiptRef,
    account: IdentityAccountRecord,
    staging_directory_name: String,
    final_directory_name: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PendingAccountSwitch {
    pub from_account_ref: Option<AccountRef>,
    pub to_account_ref: AccountRef,
    pub expected_generation: u64,
    pub next_generation: u64,
}

impl From<&AccountSwitchJournal> for PendingAccountSwitch {
    fn from(journal: &AccountSwitchJournal) -> Self {
        Self {
            from_account_ref: journal.from_account_ref.clone(),
            to_account_ref: journal.to_account_ref.clone(),
            expected_generation: journal.expected_generation,
            next_generation: journal.next_generation,
        }
    }
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AccountPurgeTarget {
    Secret,
    CustodyDocuments,
    HeldIntents,
    RecoveryMetadata,
    Drafts,
    DecryptedCache,
    WalletState,
    RelayState,
    RoomState,
    SignerSessions,
    DeviceGrants,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AccountPurgeTargetState {
    Pending,
    Deleted,
    Failed { reason: String },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AccountPurgeVerification {
    VerifiedDeleted,
    Failed { reason: String },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AccountPurgeTargetReport {
    pub target: AccountPurgeTarget,
    pub state: AccountPurgeTargetState,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AccountPurgeReport {
    pub operation_ref: String,
    pub account_ref: AccountRef,
    pub complete: bool,
    pub targets: Vec<AccountPurgeTargetReport>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct AccountPurgeJournal {
    schema: String,
    operation_ref: String,
    account_ref: AccountRef,
    storage: AccountStorageLocator,
    targets: Vec<AccountPurgeTargetReport>,
}

impl AccountPurgeJournal {
    fn report(&self) -> AccountPurgeReport {
        AccountPurgeReport {
            operation_ref: self.operation_ref.clone(),
            account_ref: self.account_ref.clone(),
            complete: self
                .targets
                .iter()
                .all(|target| target.state == AccountPurgeTargetState::Deleted),
            targets: self.targets.clone(),
        }
    }
}

#[derive(Debug, Error)]
pub enum AccountRegistryError {
    #[error("account registry storage is unavailable")]
    Storage,
    #[error("account registry state is invalid")]
    InvalidState,
    #[error("the requested account does not exist")]
    AccountNotFound,
    #[error("the account is not available for switching")]
    AccountUnavailable,
    #[error("the account selection changed")]
    StaleSelection,
    #[error("another account switch is already pending")]
    SwitchInProgress,
    #[error("the account secret does not match its public identity")]
    IdentityMismatch,
    #[error("the account partition has not completed custody setup")]
    PartitionIncomplete,
    #[error("the purge operation reference is invalid")]
    InvalidOperationRef,
    #[error("the account mutation lock is unavailable")]
    MutationLock,
    #[error("account custody setup failed")]
    Custody(#[from] CustodyError),
    #[error("remote signer capability is unavailable")]
    RemoteSigner,
}

pub struct AccountRegistryService {
    channel: AppChannel,
    data_root: PathBuf,
    identity_root: PathBuf,
    locator: KeyringLocator,
}

impl AccountRegistryService {
    pub fn system(channel: AppChannel) -> Self {
        Self::for_channel(channel)
    }

    pub fn for_channel(channel: AppChannel) -> Self {
        Self::for_channel_data_root(channel, channel_data_root(channel))
    }

    pub fn for_channel_data_root(channel: AppChannel, data_root: PathBuf) -> Self {
        Self {
            channel,
            identity_root: data_root.join("identity"),
            data_root,
            locator: KeyringLocator::for_channel(channel),
        }
    }

    pub fn inspect(&self) -> Result<AccountDashboardProjection, AccountRegistryError> {
        let _guard = IdentityMutationGuard::acquire(&self.locator)
            .map_err(|_| AccountRegistryError::MutationLock)?;
        let mut registry = self.load_or_migrate_registry_locked()?;
        self.resume_switch_locked(&mut registry)?;
        self.dashboard_locked(&registry)
    }

    pub fn partition_identity_service(&self, account_ref: &AccountRef) -> IdentityService {
        // A migrated pre-multi-account profile keeps its custody documents at
        // the legacy identity root, not in an `accounts/<digest>` partition.
        // Resolving the partition path unconditionally made startup hydration
        // inspect an empty directory and report custody `Absent` on exactly
        // the profiles that already had a working identity (owner outage
        // 2026-07-30). Resolve the registered storage locator first; only a
        // genuinely partitioned (or unknown) account uses the partition path.
        let legacy_root = self.inspect_storage_root(account_ref).ok().flatten();
        match legacy_root {
            Some(root) => IdentityService::for_identity_root(self.channel, root),
            None => IdentityService::for_account_data_root(
                self.channel,
                self.data_root.clone(),
                account_ref,
            ),
        }
    }

    /// The custody root for a registered legacy-root account, `Ok(None)` for
    /// partitioned/remote/unregistered accounts.
    fn inspect_storage_root(
        &self,
        account_ref: &AccountRef,
    ) -> Result<Option<PathBuf>, AccountRegistryError> {
        let _guard = IdentityMutationGuard::acquire(&self.locator)
            .map_err(|_| AccountRegistryError::MutationLock)?;
        let registry = self.load_or_migrate_registry_locked()?;
        let Some(entry) = registry
            .accounts
            .iter()
            .find(|entry| &entry.account_ref == account_ref)
        else {
            return Ok(None);
        };
        match entry.storage {
            AccountStorageLocator::LegacyRoot => Ok(Some(self.identity_root.clone())),
            AccountStorageLocator::Partitioned { .. }
            | AccountStorageLocator::RemoteNip46 { .. } => Ok(None),
        }
    }

    pub fn add_local_account(
        &self,
        receipt_ref: ReceiptRef,
    ) -> Result<AccountDashboardProjection, AccountRegistryError> {
        {
            let _guard = IdentityMutationGuard::acquire(&self.locator)
                .map_err(|_| AccountRegistryError::MutationLock)?;
            let mut registry = self.load_or_migrate_registry_locked()?;
            self.resume_switch_locked(&mut registry)?;
            if let Some(journal) = self.read_add_locked(&receipt_ref)? {
                return self.resume_add_locked(&mut registry, journal);
            }
        }

        let staging_directory_name = format!(".staging-{}", operation_digest(receipt_ref.as_str()));
        let staging_root = self.accounts_root().join(&staging_directory_name);
        let staging_service = IdentityService::for_identity_root(self.channel, staging_root);
        staging_service.create(receipt_ref.clone())?;
        let account = staging_service.inspect_account()?;

        let _guard = IdentityMutationGuard::acquire(&self.locator)
            .map_err(|_| AccountRegistryError::MutationLock)?;
        let mut registry = self.load_or_migrate_registry_locked()?;
        self.resume_switch_locked(&mut registry)?;
        if let Some(journal) = self.read_add_locked(&receipt_ref)? {
            return self.resume_add_locked(&mut registry, journal);
        }
        let journal = AccountAddJournal {
            schema: ACCOUNT_ADD_SCHEMA.to_string(),
            receipt_ref,
            final_directory_name: partition_directory_name(&account.account_ref),
            staging_directory_name,
            account,
        };
        self.write_add_locked(&journal)?;
        self.resume_add_locked(&mut registry, journal)
    }

    pub fn register_partitioned_account(
        &self,
        account: &IdentityAccountRecord,
        recovery: RecoveryProtectionState,
    ) -> Result<AccountDashboardProjection, AccountRegistryError> {
        if !account.validate() {
            return Err(AccountRegistryError::InvalidState);
        }
        let _guard = IdentityMutationGuard::acquire(&self.locator)
            .map_err(|_| AccountRegistryError::MutationLock)?;
        let mut registry = self.load_or_migrate_registry_locked()?;
        self.resume_switch_locked(&mut registry)?;
        if let Some(existing) = registry
            .accounts
            .iter()
            .find(|entry| entry.account_ref == account.account_ref)
        {
            if existing.identity != account.identity {
                return Err(AccountRegistryError::IdentityMismatch);
            }
            let storage = existing.storage.clone();
            self.verify_account_storage_locked(&storage, &account.identity)?;
            self.refresh_account_locked(&mut registry, account, recovery)?;
            self.write_registry_locked(&registry)?;
            return self.dashboard_locked(&registry);
        }

        let storage = AccountStorageLocator::Partitioned {
            directory_name: partition_directory_name(&account.account_ref),
        };
        self.verify_account_storage_locked(&storage, &account.identity)?;
        self.register_account_locked(&mut registry, account, storage, recovery)?;
        self.write_registry_locked(&registry)?;
        self.dashboard_locked(&registry)
    }

    pub fn register_remote_account(
        &self,
        capability_ref: &str,
        expected_generation: u64,
    ) -> Result<AccountDashboardProjection, AccountRegistryError> {
        let _guard = IdentityMutationGuard::acquire(&self.locator)
            .map_err(|_| AccountRegistryError::MutationLock)?;
        let mut registry = self.load_or_migrate_registry_locked()?;
        self.resume_switch_locked(&mut registry)?;
        if registry.active.generation != expected_generation {
            return Err(AccountRegistryError::StaleSelection);
        }
        let service = Nip46Service::for_data_root(self.data_root.clone());
        let capability = service
            .load_capability(capability_ref)
            .map_err(|_| AccountRegistryError::RemoteSigner)?;
        if capability.state != Nip46CapabilityState::AwaitingRegistration {
            return Err(AccountRegistryError::AccountUnavailable);
        }
        if registry
            .accounts
            .iter()
            .any(|entry| entry.account_ref == capability.account_ref)
        {
            return Err(AccountRegistryError::InvalidState);
        }
        let next_generation = next_generation(registry.active.generation)?;
        let capability = service
            .bind_registered_account(capability_ref, &capability.account_ref, next_generation)
            .map_err(|_| AccountRegistryError::RemoteSigner)?;
        let now = unix_time_now();
        if let Some(previous) = registry.active.account_ref.take()
            && let Some(entry) = find_account_mut(&mut registry, &previous)
        {
            entry.lifecycle = AccountLifecycleState::SignedOut;
            entry.signer.availability = SignerAvailability::UserApprovalRequired;
            entry.updated_at = now;
        }
        registry.accounts.push(DurableAccountEntry {
            account_ref: capability.account_ref.clone(),
            identity: capability.user_identity.clone(),
            storage: AccountStorageLocator::RemoteNip46 {
                capability_ref: capability.capability_ref.clone(),
            },
            lifecycle: AccountLifecycleState::Active,
            signer: AccountSignerSummary {
                kind: SignerKind::RemoteNip46,
                availability: SignerAvailability::Ready,
                last_successful_use: capability.last_successful_use,
            },
            recovery: RecoveryProtectionState::NotApplicable,
            retirement: AccountRetirementState::NotRetired,
            profile: None,
            updated_at: now,
        });
        registry.active = ActiveAccountSelection {
            account_ref: Some(capability.account_ref),
            generation: next_generation,
        };
        self.write_registry_locked(&registry)?;
        self.dashboard_locked(&registry)
    }

    pub fn refresh_account(
        &self,
        account: &IdentityAccountRecord,
        recovery: RecoveryProtectionState,
    ) -> Result<AccountDashboardProjection, AccountRegistryError> {
        if !account.validate() {
            return Err(AccountRegistryError::InvalidState);
        }
        let _guard = IdentityMutationGuard::acquire(&self.locator)
            .map_err(|_| AccountRegistryError::MutationLock)?;
        let mut registry = self.load_or_migrate_registry_locked()?;
        self.resume_switch_locked(&mut registry)?;
        let storage = registry
            .accounts
            .iter()
            .find(|entry| entry.account_ref == account.account_ref)
            .map(|entry| entry.storage.clone())
            .ok_or(AccountRegistryError::AccountNotFound)?;
        self.verify_account_storage_locked(&storage, &account.identity)?;
        self.refresh_account_locked(&mut registry, account, recovery)?;
        self.write_registry_locked(&registry)?;
        self.dashboard_locked(&registry)
    }

    pub fn switch_account(
        &self,
        target: &AccountRef,
        expected_generation: u64,
    ) -> Result<AccountDashboardProjection, AccountRegistryError> {
        let _guard = IdentityMutationGuard::acquire(&self.locator)
            .map_err(|_| AccountRegistryError::MutationLock)?;
        let mut registry = self.load_or_migrate_registry_locked()?;
        self.resume_switch_locked(&mut registry)?;
        if registry.active.generation != expected_generation {
            return Err(AccountRegistryError::StaleSelection);
        }
        let target_entry = registry
            .accounts
            .iter()
            .find(|entry| &entry.account_ref == target)
            .ok_or(AccountRegistryError::AccountNotFound)?;
        let recovery_available = match target_entry.storage {
            AccountStorageLocator::RemoteNip46 { .. } => {
                target_entry.recovery == RecoveryProtectionState::NotApplicable
            }
            _ => target_entry.recovery == RecoveryProtectionState::Protected,
        };
        if !matches!(
            target_entry.lifecycle,
            AccountLifecycleState::Active
                | AccountLifecycleState::Locked
                | AccountLifecycleState::SignedOut
        ) || !recovery_available
        {
            return Err(AccountRegistryError::AccountUnavailable);
        }
        self.verify_account_storage_locked(&target_entry.storage, &target_entry.identity)?;
        let journal = AccountSwitchJournal {
            schema: ACCOUNT_SWITCH_SCHEMA.to_string(),
            from_account_ref: registry.active.account_ref.clone(),
            to_account_ref: target.clone(),
            expected_generation,
            next_generation: next_generation(expected_generation)?,
            phase: SwitchPhase::Prepared,
        };
        write_json_document(&self.switch_path(), &journal)
            .map_err(|_| AccountRegistryError::Storage)?;
        self.apply_switch_locked(&mut registry, journal)
    }

    pub fn lock_active(
        &self,
        expected_generation: u64,
    ) -> Result<AccountDashboardProjection, AccountRegistryError> {
        self.mutate_active(expected_generation, |entry| {
            entry.lifecycle = AccountLifecycleState::Locked;
            entry.signer.availability = SignerAvailability::UserApprovalRequired;
        })
    }

    pub fn unlock_active(
        &self,
        expected_generation: u64,
    ) -> Result<AccountDashboardProjection, AccountRegistryError> {
        let _guard = IdentityMutationGuard::acquire(&self.locator)
            .map_err(|_| AccountRegistryError::MutationLock)?;
        let mut registry = self.load_or_migrate_registry_locked()?;
        self.resume_switch_locked(&mut registry)?;
        if registry.active.generation != expected_generation {
            return Err(AccountRegistryError::StaleSelection);
        }
        let active = registry
            .active
            .account_ref
            .clone()
            .ok_or(AccountRegistryError::AccountUnavailable)?;
        let entry = registry
            .accounts
            .iter()
            .find(|entry| entry.account_ref == active)
            .ok_or(AccountRegistryError::AccountNotFound)?;
        let recovery_available = match entry.storage {
            AccountStorageLocator::RemoteNip46 { .. } => {
                entry.recovery == RecoveryProtectionState::NotApplicable
            }
            _ => entry.recovery == RecoveryProtectionState::Protected,
        };
        if entry.lifecycle != AccountLifecycleState::Locked || !recovery_available {
            return Err(AccountRegistryError::AccountUnavailable);
        }
        self.verify_account_storage_locked(&entry.storage, &entry.identity)?;
        let entry = find_account_mut(&mut registry, &active)
            .ok_or(AccountRegistryError::AccountNotFound)?;
        entry.lifecycle = AccountLifecycleState::Active;
        entry.signer.availability = SignerAvailability::Ready;
        entry.updated_at = unix_time_now();
        registry.active.generation = next_generation(registry.active.generation)?;
        self.write_registry_locked(&registry)?;
        self.dashboard_locked(&registry)
    }

    pub fn sign_out(
        &self,
        expected_generation: u64,
    ) -> Result<AccountDashboardProjection, AccountRegistryError> {
        let _guard = IdentityMutationGuard::acquire(&self.locator)
            .map_err(|_| AccountRegistryError::MutationLock)?;
        let mut registry = self.load_or_migrate_registry_locked()?;
        self.resume_switch_locked(&mut registry)?;
        if registry.active.generation != expected_generation {
            return Err(AccountRegistryError::StaleSelection);
        }
        let active = registry
            .active
            .account_ref
            .take()
            .ok_or(AccountRegistryError::AccountUnavailable)?;
        let now = unix_time_now();
        let entry = find_account_mut(&mut registry, &active)
            .ok_or(AccountRegistryError::AccountNotFound)?;
        entry.signer.availability = SignerAvailability::UserApprovalRequired;
        entry.lifecycle = AccountLifecycleState::SignedOut;
        entry.updated_at = now;
        registry.active.generation = next_generation(registry.active.generation)?;
        self.write_registry_locked(&registry)?;
        self.dashboard_locked(&registry)
    }

    pub fn disconnect_remote_signer(
        &self,
        account_ref: &AccountRef,
        expected_generation: u64,
    ) -> Result<AccountDashboardProjection, AccountRegistryError> {
        let _guard = IdentityMutationGuard::acquire(&self.locator)
            .map_err(|_| AccountRegistryError::MutationLock)?;
        let mut registry = self.load_or_migrate_registry_locked()?;
        self.resume_switch_locked(&mut registry)?;
        if registry.active.generation != expected_generation {
            return Err(AccountRegistryError::StaleSelection);
        }
        let entry = registry
            .accounts
            .iter()
            .find(|entry| &entry.account_ref == account_ref)
            .ok_or(AccountRegistryError::AccountNotFound)?;
        let AccountStorageLocator::RemoteNip46 { capability_ref } = &entry.storage else {
            return Err(AccountRegistryError::RemoteSigner);
        };
        Nip46Service::for_data_root(self.data_root.clone())
            .revoke(capability_ref)
            .map_err(|_| AccountRegistryError::RemoteSigner)?;
        let entry = find_account_mut(&mut registry, account_ref)
            .ok_or(AccountRegistryError::AccountNotFound)?;
        entry.lifecycle = AccountLifecycleState::SignedOut;
        entry.signer.availability = SignerAvailability::Revoked;
        entry.updated_at = unix_time_now();
        if registry.active.account_ref.as_ref() == Some(account_ref) {
            registry.active.account_ref = None;
        }
        registry.active.generation = next_generation(registry.active.generation)?;
        self.write_registry_locked(&registry)?;
        self.dashboard_locked(&registry)
    }

    pub fn record_signer_use(
        &self,
        token: &AccountSelectionToken,
        used_at: u64,
    ) -> Result<AccountDashboardProjection, AccountRegistryError> {
        if used_at == 0 {
            return Err(AccountRegistryError::InvalidState);
        }
        let _guard = IdentityMutationGuard::acquire(&self.locator)
            .map_err(|_| AccountRegistryError::MutationLock)?;
        let mut registry = self.load_or_migrate_registry_locked()?;
        self.require_selection_locked(&registry, token)?;
        let entry = find_account_mut(&mut registry, &token.account_ref)
            .ok_or(AccountRegistryError::AccountNotFound)?;
        // Mirrors the `validate_signing_selection` gate: candidate and
        // activating accounts sign (and therefore record use); recovery
        // protection is not a precondition for recording a successful use.
        // Requiring `Active` + `Protected` here failed the whole signing call
        // AFTER a valid signature existed on migrated pre-multi-account
        // profiles (owner outage 2026-07-30).
        let lifecycle_signable = matches!(
            entry.lifecycle,
            AccountLifecycleState::Active
                | AccountLifecycleState::CandidateLocal
                | AccountLifecycleState::CandidateExisting
                | AccountLifecycleState::Activating
        );
        if !lifecycle_signable || entry.signer.availability != SignerAvailability::Ready {
            return Err(AccountRegistryError::AccountUnavailable);
        }
        entry.signer.last_successful_use = Some(used_at);
        entry.updated_at = used_at;
        self.write_registry_locked(&registry)?;
        self.dashboard_locked(&registry)
    }

    pub fn update_profile(
        &self,
        account_ref: &AccountRef,
        profile: Option<AccountProfileSummary>,
    ) -> Result<AccountDashboardProjection, AccountRegistryError> {
        let _guard = IdentityMutationGuard::acquire(&self.locator)
            .map_err(|_| AccountRegistryError::MutationLock)?;
        let mut registry = self.load_or_migrate_registry_locked()?;
        let entry = find_account_mut(&mut registry, account_ref)
            .ok_or(AccountRegistryError::AccountNotFound)?;
        entry.profile = profile;
        entry.updated_at = unix_time_now();
        self.write_registry_locked(&registry)?;
        self.dashboard_locked(&registry)
    }

    pub fn record_hydrated_profile(
        &self,
        token: &AccountSelectionToken,
        profile: Option<AccountProfileSummary>,
    ) -> Result<AccountDashboardProjection, AccountRegistryError> {
        let _guard = IdentityMutationGuard::acquire(&self.locator)
            .map_err(|_| AccountRegistryError::MutationLock)?;
        let mut registry = self.load_or_migrate_registry_locked()?;
        self.require_selection_locked(&registry, token)?;
        let entry = find_account_mut(&mut registry, &token.account_ref)
            .ok_or(AccountRegistryError::AccountNotFound)?;
        entry.profile = profile;
        entry.updated_at = unix_time_now();
        self.write_registry_locked(&registry)?;
        self.dashboard_locked(&registry)
    }

    pub fn retire_account(
        &self,
        account_ref: &AccountRef,
        retirement: AccountRetirementState,
    ) -> Result<AccountDashboardProjection, AccountRegistryError> {
        let _guard = IdentityMutationGuard::acquire(&self.locator)
            .map_err(|_| AccountRegistryError::MutationLock)?;
        let mut registry = self.load_or_migrate_registry_locked()?;
        let entry = find_account_mut(&mut registry, account_ref)
            .ok_or(AccountRegistryError::AccountNotFound)?;
        if entry.lifecycle == AccountLifecycleState::Forgotten {
            return Err(AccountRegistryError::AccountUnavailable);
        }
        entry.retirement = retirement;
        entry.updated_at = unix_time_now();
        self.write_registry_locked(&registry)?;
        self.dashboard_locked(&registry)
    }

    pub fn begin_forget(
        &self,
        account_ref: &AccountRef,
        operation_ref: String,
        expected_generation: u64,
    ) -> Result<AccountPurgeReport, AccountRegistryError> {
        validate_operation_ref(&operation_ref)?;
        let _guard = IdentityMutationGuard::acquire(&self.locator)
            .map_err(|_| AccountRegistryError::MutationLock)?;
        let mut registry = self.load_or_migrate_registry_locked()?;
        self.resume_switch_locked(&mut registry)?;
        if registry.active.generation != expected_generation {
            return Err(AccountRegistryError::StaleSelection);
        }
        if let Some(existing) = self.read_purge_locked(account_ref)? {
            if existing.operation_ref == operation_ref {
                return Ok(existing.report());
            }
            return Err(AccountRegistryError::InvalidState);
        }
        let now = unix_time_now();
        let entry = find_account_mut(&mut registry, account_ref)
            .ok_or(AccountRegistryError::AccountNotFound)?;
        if entry.lifecycle == AccountLifecycleState::Forgotten {
            return Err(AccountRegistryError::AccountUnavailable);
        }
        entry.lifecycle = AccountLifecycleState::ForgetPending;
        entry.signer.availability = SignerAvailability::Lost;
        entry.updated_at = now;
        let storage = entry.storage.clone();
        if registry.active.account_ref.as_ref() == Some(account_ref) {
            registry.active.account_ref = None;
            registry.active.generation = next_generation(registry.active.generation)?;
        }
        self.write_registry_locked(&registry)?;
        let journal = AccountPurgeJournal {
            schema: ACCOUNT_PURGE_SCHEMA.to_string(),
            operation_ref,
            account_ref: account_ref.clone(),
            storage,
            targets: all_purge_targets()
                .into_iter()
                .map(|target| AccountPurgeTargetReport {
                    target,
                    state: AccountPurgeTargetState::Pending,
                })
                .collect(),
        };
        self.write_purge_locked(&journal)?;
        Ok(journal.report())
    }

    pub fn retry_purge(
        &self,
        account_ref: &AccountRef,
        operation_ref: &str,
    ) -> Result<AccountPurgeReport, AccountRegistryError> {
        let _guard = IdentityMutationGuard::acquire(&self.locator)
            .map_err(|_| AccountRegistryError::MutationLock)?;
        let mut registry = self.load_or_migrate_registry_locked()?;
        let mut journal = self
            .read_purge_locked(account_ref)?
            .ok_or(AccountRegistryError::InvalidState)?;
        if journal.operation_ref != operation_ref {
            return Err(AccountRegistryError::InvalidOperationRef);
        }
        let root = self.root_for_storage(&journal.storage)?;
        for index in 0..journal.targets.len() {
            if journal.targets[index].state == AccountPurgeTargetState::Deleted {
                continue;
            }
            let target = journal.targets[index].target;
            if !is_registry_owned_purge_target(target) {
                continue;
            }
            journal.targets[index].state = match delete_and_verify_target(&root, target) {
                Ok(()) => AccountPurgeTargetState::Deleted,
                Err(error) => AccountPurgeTargetState::Failed {
                    reason: format!("filesystem verification failed: {:?}", error.kind()),
                },
            };
            self.write_purge_locked(&journal)?;
        }
        self.complete_purge_if_verified_locked(&mut registry, &journal)?;
        Ok(journal.report())
    }

    pub fn record_purge_target_result(
        &self,
        account_ref: &AccountRef,
        operation_ref: &str,
        target: AccountPurgeTarget,
        verification: AccountPurgeVerification,
    ) -> Result<AccountPurgeReport, AccountRegistryError> {
        if is_registry_owned_purge_target(target) {
            return Err(AccountRegistryError::InvalidState);
        }
        let _guard = IdentityMutationGuard::acquire(&self.locator)
            .map_err(|_| AccountRegistryError::MutationLock)?;
        let mut registry = self.load_or_migrate_registry_locked()?;
        let mut journal = self
            .read_purge_locked(account_ref)?
            .ok_or(AccountRegistryError::InvalidState)?;
        if journal.operation_ref != operation_ref {
            return Err(AccountRegistryError::InvalidOperationRef);
        }
        let target_report = journal
            .targets
            .iter_mut()
            .find(|report| report.target == target)
            .ok_or(AccountRegistryError::InvalidState)?;
        target_report.state = match verification {
            AccountPurgeVerification::VerifiedDeleted => AccountPurgeTargetState::Deleted,
            AccountPurgeVerification::Failed { reason } if !reason.is_empty() => {
                AccountPurgeTargetState::Failed {
                    reason: "owning subsystem did not verify deletion".to_string(),
                }
            }
            AccountPurgeVerification::Failed { .. } => {
                return Err(AccountRegistryError::InvalidState);
            }
        };
        self.write_purge_locked(&journal)?;
        self.complete_purge_if_verified_locked(&mut registry, &journal)?;
        Ok(journal.report())
    }

    pub fn selection_token(&self) -> Result<AccountSelectionToken, AccountRegistryError> {
        let dashboard = self.inspect()?;
        let account_ref = dashboard
            .active
            .account_ref
            .ok_or(AccountRegistryError::AccountUnavailable)?;
        Ok(AccountSelectionToken {
            account_ref,
            identity: dashboard
                .accounts
                .iter()
                .find(|entry| entry.is_active)
                .map(|entry| entry.identity.clone())
                .ok_or(AccountRegistryError::InvalidState)?,
            generation: dashboard.active.generation,
        })
    }

    pub fn validate_selection(
        &self,
        token: &AccountSelectionToken,
    ) -> Result<(), AccountRegistryError> {
        let _guard = IdentityMutationGuard::acquire(&self.locator)
            .map_err(|_| AccountRegistryError::MutationLock)?;
        let registry = self.load_or_migrate_registry_locked()?;
        self.require_selection_locked(&registry, token)
    }

    pub fn validate_signing_selection(
        &self,
        token: &AccountSelectionToken,
    ) -> Result<(), AccountRegistryError> {
        let _guard = IdentityMutationGuard::acquire(&self.locator)
            .map_err(|_| AccountRegistryError::MutationLock)?;
        let registry = self.load_or_migrate_registry_locked()?;
        self.require_selection_locked(&registry, token)?;
        let entry = registry
            .accounts
            .iter()
            .find(|entry| entry.account_ref == token.account_ref)
            .ok_or(AccountRegistryError::AccountNotFound)?;
        // Signing custody is "the selected account's signer is present and
        // ready", not "onboarding is finished". A pre-multi-account identity
        // migrates in as `CandidateExisting` with `recovery: Needed` until the
        // owner completes the activation ceremony; requiring `Active` +
        // `Protected` here made every hosted sign-in fail on exactly the
        // profiles that were already signing before multi-account landed
        // (owner outage 2026-07-30). Candidate and activating lifecycles stay
        // signable; recovery protection gates activation and account
        // switching, not proof signing.
        let lifecycle_signable = matches!(
            entry.lifecycle,
            AccountLifecycleState::Active
                | AccountLifecycleState::CandidateLocal
                | AccountLifecycleState::CandidateExisting
                | AccountLifecycleState::Activating
        );
        let signer_available = entry.signer.availability == SignerAvailability::Ready
            || (entry.signer.kind == SignerKind::RemoteNip46
                && entry.signer.availability == SignerAvailability::Offline);
        if !lifecycle_signable || !signer_available {
            return Err(AccountRegistryError::AccountUnavailable);
        }
        Ok(())
    }

    pub fn remote_signer_capability(
        &self,
        token: &AccountSelectionToken,
    ) -> Result<Nip46Capability, AccountRegistryError> {
        let _guard = IdentityMutationGuard::acquire(&self.locator)
            .map_err(|_| AccountRegistryError::MutationLock)?;
        let registry = self.load_or_migrate_registry_locked()?;
        self.require_selection_locked(&registry, token)?;
        let entry = registry
            .accounts
            .iter()
            .find(|entry| entry.account_ref == token.account_ref)
            .ok_or(AccountRegistryError::AccountNotFound)?;
        if entry.lifecycle != AccountLifecycleState::Active
            || entry.signer.kind != SignerKind::RemoteNip46
            || !matches!(
                entry.signer.availability,
                SignerAvailability::Ready | SignerAvailability::Offline
            )
        {
            return Err(AccountRegistryError::AccountUnavailable);
        }
        let AccountStorageLocator::RemoteNip46 { capability_ref } = &entry.storage else {
            return Err(AccountRegistryError::RemoteSigner);
        };
        let capability = Nip46Service::for_data_root(self.data_root.clone())
            .load_capability(capability_ref)
            .map_err(|_| AccountRegistryError::RemoteSigner)?;
        capability
            .authorize(token, crate::Nip46Operation::LoginProof, unix_time_now())
            .map_err(|_| AccountRegistryError::RemoteSigner)?;
        Ok(capability)
    }

    pub fn authorize_remote_identity_action(
        &self,
        token: &AccountSelectionToken,
        descriptor: DurableIdentityActionDescriptor,
        now: u64,
    ) -> Result<IdentityActionAuthorization, AccountRegistryError> {
        descriptor
            .validate(now)
            .map_err(|_| AccountRegistryError::InvalidState)?;
        let _guard = IdentityMutationGuard::acquire(&self.locator)
            .map_err(|_| AccountRegistryError::MutationLock)?;
        let mut registry = self.load_or_migrate_registry_locked()?;
        self.resume_switch_locked(&mut registry)?;
        self.require_selection_locked(&registry, token)?;
        let entry = registry
            .accounts
            .iter()
            .find(|entry| entry.account_ref == token.account_ref)
            .ok_or(AccountRegistryError::AccountNotFound)?;
        let AccountStorageLocator::RemoteNip46 { capability_ref } = &entry.storage else {
            return Err(AccountRegistryError::RemoteSigner);
        };
        if entry.lifecycle != AccountLifecycleState::Active
            || entry.signer.kind != SignerKind::RemoteNip46
            || !matches!(
                entry.signer.availability,
                SignerAvailability::Ready | SignerAvailability::Offline
            )
            || entry.recovery != RecoveryProtectionState::NotApplicable
        {
            return Err(AccountRegistryError::AccountUnavailable);
        }
        let capability = Nip46Service::for_data_root(self.data_root.clone())
            .load_capability(capability_ref)
            .map_err(|_| AccountRegistryError::RemoteSigner)?;
        if capability.state != Nip46CapabilityState::Active
            || capability.account_ref != token.account_ref
            || capability.account_generation != token.generation
            || capability.user_identity != token.identity
        {
            return Err(AccountRegistryError::StaleSelection);
        }
        let intent = HeldIdentityAction {
            intent_ref: descriptor.intent_ref,
            account_ref: token.account_ref.clone(),
            account_generation: token.generation,
            identity_ref: token.identity.identity_ref().clone(),
            kind: descriptor.kind,
            destination_ref: descriptor.destination_ref,
            authorization_ref: descriptor.authorization_ref,
            payload_digest: descriptor.payload_digest,
            issued_at: now,
            expires_at: descriptor.expires_at,
        };
        if !intent.validate(now) {
            return Err(AccountRegistryError::InvalidState);
        }
        Ok(IdentityActionAuthorization::new(intent))
    }

    pub fn validate_remote_identity_action_authorization(
        &self,
        authorization: &IdentityActionAuthorization,
        now: u64,
    ) -> Result<(), AccountRegistryError> {
        let intent = authorization.intent();
        if !intent.validate(now) {
            return Err(AccountRegistryError::InvalidState);
        }
        let _guard = IdentityMutationGuard::acquire(&self.locator)
            .map_err(|_| AccountRegistryError::MutationLock)?;
        let mut registry = self.load_or_migrate_registry_locked()?;
        self.resume_switch_locked(&mut registry)?;
        if registry.active.account_ref.as_ref() != Some(&intent.account_ref)
            || registry.active.generation != intent.account_generation
        {
            return Err(AccountRegistryError::StaleSelection);
        }
        let entry = registry
            .accounts
            .iter()
            .find(|entry| entry.account_ref == intent.account_ref)
            .ok_or(AccountRegistryError::AccountNotFound)?;
        let AccountStorageLocator::RemoteNip46 { capability_ref } = &entry.storage else {
            return Err(AccountRegistryError::RemoteSigner);
        };
        if entry.lifecycle != AccountLifecycleState::Active
            || entry.signer.kind != SignerKind::RemoteNip46
            || entry.signer.availability != SignerAvailability::Ready
            || entry.recovery != RecoveryProtectionState::NotApplicable
            || entry.identity.identity_ref() != &intent.identity_ref
        {
            return Err(AccountRegistryError::AccountUnavailable);
        }
        let capability = Nip46Service::for_data_root(self.data_root.clone())
            .load_capability(capability_ref)
            .map_err(|_| AccountRegistryError::RemoteSigner)?;
        if capability.state != Nip46CapabilityState::Active
            || capability.account_ref != intent.account_ref
            || capability.account_generation != intent.account_generation
            || capability.user_identity != entry.identity
        {
            return Err(AccountRegistryError::StaleSelection);
        }
        Ok(())
    }

    pub fn record_remote_signer_outcome(
        &self,
        token: &AccountSelectionToken,
        outcome: RemoteSignerRuntimeOutcome,
        observed_at: u64,
    ) -> Result<AccountDashboardProjection, AccountRegistryError> {
        if observed_at == 0 || matches!(outcome, RemoteSignerRuntimeOutcome::Ready { used_at: 0 }) {
            return Err(AccountRegistryError::InvalidState);
        }
        let _guard = IdentityMutationGuard::acquire(&self.locator)
            .map_err(|_| AccountRegistryError::MutationLock)?;
        let mut registry = self.load_or_migrate_registry_locked()?;
        self.resume_switch_locked(&mut registry)?;
        self.require_selection_locked(&registry, token)?;
        let entry = registry
            .accounts
            .iter()
            .find(|entry| entry.account_ref == token.account_ref)
            .ok_or(AccountRegistryError::AccountNotFound)?;
        if entry.lifecycle != AccountLifecycleState::Active
            || entry.signer.kind != SignerKind::RemoteNip46
            || entry.identity != token.identity
            || entry.recovery != RecoveryProtectionState::NotApplicable
            || !matches!(
                entry.signer.availability,
                SignerAvailability::Ready | SignerAvailability::Offline
            )
        {
            return Err(AccountRegistryError::AccountUnavailable);
        }
        let AccountStorageLocator::RemoteNip46 { capability_ref } = &entry.storage else {
            return Err(AccountRegistryError::RemoteSigner);
        };
        let capability_ref = capability_ref.clone();
        if outcome == RemoteSignerRuntimeOutcome::Revoked {
            Nip46Service::for_data_root(self.data_root.clone())
                .revoke(&capability_ref)
                .map_err(|_| AccountRegistryError::RemoteSigner)?;
        }
        let entry = find_account_mut(&mut registry, &token.account_ref)
            .ok_or(AccountRegistryError::AccountNotFound)?;
        entry.signer.availability = match outcome {
            RemoteSignerRuntimeOutcome::Ready { used_at } => {
                entry.signer.last_successful_use = Some(used_at);
                SignerAvailability::Ready
            }
            RemoteSignerRuntimeOutcome::Offline => SignerAvailability::Offline,
            RemoteSignerRuntimeOutcome::Rejected => SignerAvailability::Rejected,
            RemoteSignerRuntimeOutcome::Revoked => SignerAvailability::Revoked,
        };
        entry.updated_at = observed_at;
        self.write_registry_locked(&registry)?;
        self.dashboard_locked(&registry)
    }

    pub(crate) fn active_storage(
        &self,
    ) -> Result<(PathBuf, AccountSelectionToken), AccountRegistryError> {
        let _guard = IdentityMutationGuard::acquire(&self.locator)
            .map_err(|_| AccountRegistryError::MutationLock)?;
        let mut registry = self.load_or_migrate_registry_locked()?;
        self.resume_switch_locked(&mut registry)?;
        let account_ref = registry
            .active
            .account_ref
            .clone()
            .ok_or(AccountRegistryError::AccountUnavailable)?;
        let entry = registry
            .accounts
            .iter()
            .find(|entry| entry.account_ref == account_ref)
            .ok_or(AccountRegistryError::AccountNotFound)?;
        let token = AccountSelectionToken {
            account_ref,
            identity: entry.identity.clone(),
            generation: registry.active.generation,
        };
        if matches!(
            entry.lifecycle,
            AccountLifecycleState::ForgetPending | AccountLifecycleState::Forgotten
        ) {
            return Err(AccountRegistryError::AccountUnavailable);
        }
        if matches!(entry.storage, AccountStorageLocator::RemoteNip46 { .. }) {
            return Err(AccountRegistryError::RemoteSigner);
        }
        Ok((self.root_for_storage(&entry.storage)?, token))
    }

    fn mutate_active(
        &self,
        expected_generation: u64,
        mutation: impl FnOnce(&mut DurableAccountEntry),
    ) -> Result<AccountDashboardProjection, AccountRegistryError> {
        let _guard = IdentityMutationGuard::acquire(&self.locator)
            .map_err(|_| AccountRegistryError::MutationLock)?;
        let mut registry = self.load_or_migrate_registry_locked()?;
        self.resume_switch_locked(&mut registry)?;
        if registry.active.generation != expected_generation {
            return Err(AccountRegistryError::StaleSelection);
        }
        let active = registry
            .active
            .account_ref
            .clone()
            .ok_or(AccountRegistryError::AccountUnavailable)?;
        let entry = find_account_mut(&mut registry, &active)
            .ok_or(AccountRegistryError::AccountNotFound)?;
        mutation(entry);
        entry.updated_at = unix_time_now();
        registry.active.generation = next_generation(registry.active.generation)?;
        self.write_registry_locked(&registry)?;
        self.dashboard_locked(&registry)
    }

    fn require_selection_locked(
        &self,
        registry: &AccountRegistry,
        token: &AccountSelectionToken,
    ) -> Result<(), AccountRegistryError> {
        if registry.active.account_ref.as_ref() != Some(&token.account_ref)
            || registry.active.generation != token.generation
        {
            return Err(AccountRegistryError::StaleSelection);
        }
        let entry = registry
            .accounts
            .iter()
            .find(|entry| entry.account_ref == token.account_ref)
            .ok_or(AccountRegistryError::AccountNotFound)?;
        if entry.identity != token.identity
            || matches!(
                entry.lifecycle,
                AccountLifecycleState::ForgetPending | AccountLifecycleState::Forgotten
            )
        {
            return Err(AccountRegistryError::AccountUnavailable);
        }
        Ok(())
    }

    pub(crate) fn has_durable_registry(&self) -> bool {
        self.registry_path().is_file()
    }

    fn load_or_migrate_registry_locked(&self) -> Result<AccountRegistry, AccountRegistryError> {
        if let Some(registry) = read_json_document::<AccountRegistry>(&self.registry_path())
            .map_err(|_| AccountRegistryError::Storage)?
        {
            validate_registry(&registry)?;
            return Ok(registry);
        }
        let legacy_account = read_json_document::<IdentityAccountRecord>(
            &self.identity_root.join("identity.account.json"),
        )
        .map_err(|_| AccountRegistryError::Storage)?;
        let registry = if let Some(account) = legacy_account {
            if !account.validate() {
                return Err(AccountRegistryError::InvalidState);
            }
            let recovery = read_recovery_state(&self.identity_root, &account.identity)?;
            let lifecycle = activation_lifecycle(&account);
            AccountRegistry {
                schema: ACCOUNT_REGISTRY_SCHEMA.to_string(),
                schema_version: ACCOUNT_REGISTRY_VERSION,
                active: ActiveAccountSelection {
                    account_ref: Some(account.account_ref.clone()),
                    generation: account.account_generation.max(1),
                },
                accounts: vec![DurableAccountEntry {
                    account_ref: account.account_ref,
                    identity: account.identity,
                    storage: AccountStorageLocator::LegacyRoot,
                    lifecycle,
                    signer: AccountSignerSummary {
                        kind: SignerKind::LocalNative,
                        availability: SignerAvailability::Ready,
                        last_successful_use: None,
                    },
                    recovery,
                    retirement: AccountRetirementState::NotRetired,
                    profile: None,
                    updated_at: account.updated_at,
                }],
            }
        } else {
            AccountRegistry {
                schema: ACCOUNT_REGISTRY_SCHEMA.to_string(),
                schema_version: ACCOUNT_REGISTRY_VERSION,
                active: ActiveAccountSelection {
                    account_ref: None,
                    generation: 1,
                },
                accounts: Vec::new(),
            }
        };
        if !registry.accounts.is_empty() {
            self.write_registry_locked(&registry)?;
        }
        Ok(registry)
    }

    fn register_account_locked(
        &self,
        registry: &mut AccountRegistry,
        account: &IdentityAccountRecord,
        storage: AccountStorageLocator,
        recovery: RecoveryProtectionState,
    ) -> Result<(), AccountRegistryError> {
        let now = unix_time_now();
        registry.accounts.push(DurableAccountEntry {
            account_ref: account.account_ref.clone(),
            identity: account.identity.clone(),
            storage,
            lifecycle: activation_lifecycle(account),
            signer: AccountSignerSummary {
                kind: SignerKind::LocalNative,
                availability: SignerAvailability::Ready,
                last_successful_use: None,
            },
            recovery,
            retirement: AccountRetirementState::NotRetired,
            profile: None,
            updated_at: now,
        });
        registry.active.generation = next_generation(registry.active.generation)?;
        if let Some(previous) = registry.active.account_ref.take()
            && let Some(entry) = find_account_mut(registry, &previous)
        {
            entry.lifecycle = AccountLifecycleState::SignedOut;
            entry.signer.availability = SignerAvailability::UserApprovalRequired;
            entry.updated_at = now;
        }
        registry.active.account_ref = Some(account.account_ref.clone());
        Ok(())
    }

    fn refresh_account_locked(
        &self,
        registry: &mut AccountRegistry,
        account: &IdentityAccountRecord,
        recovery: RecoveryProtectionState,
    ) -> Result<(), AccountRegistryError> {
        let is_selected = registry.active.account_ref.as_ref() == Some(&account.account_ref);
        let entry = find_account_mut(registry, &account.account_ref)
            .ok_or(AccountRegistryError::AccountNotFound)?;
        if entry.identity != account.identity
            || matches!(
                entry.lifecycle,
                AccountLifecycleState::ForgetPending | AccountLifecycleState::Forgotten
            )
        {
            return Err(AccountRegistryError::IdentityMismatch);
        }
        let next_lifecycle = activation_lifecycle(account);
        let changed = entry.lifecycle != next_lifecycle || entry.recovery != recovery;
        entry.lifecycle = next_lifecycle;
        entry.recovery = recovery;
        entry.signer.availability = if next_lifecycle == AccountLifecycleState::Active {
            SignerAvailability::Ready
        } else {
            SignerAvailability::UserApprovalRequired
        };
        entry.updated_at = unix_time_now();
        if changed && is_selected {
            registry.active.generation = next_generation(registry.active.generation)?;
        }
        Ok(())
    }

    fn resume_add_locked(
        &self,
        registry: &mut AccountRegistry,
        journal: AccountAddJournal,
    ) -> Result<AccountDashboardProjection, AccountRegistryError> {
        validate_add_journal(&journal)?;
        let staging_root = self.accounts_root().join(&journal.staging_directory_name);
        let final_root = self.accounts_root().join(&journal.final_directory_name);
        let storage = AccountStorageLocator::Partitioned {
            directory_name: journal.final_directory_name.clone(),
        };
        if final_root
            .try_exists()
            .map_err(|_| AccountRegistryError::Storage)?
        {
            self.verify_account_storage_locked(&storage, &journal.account.identity)?;
        } else {
            verify_account_root(&staging_root, &self.locator, &journal.account.identity)?;
            fs::create_dir_all(self.accounts_root()).map_err(|_| AccountRegistryError::Storage)?;
            fs::rename(&staging_root, &final_root).map_err(|_| AccountRegistryError::Storage)?;
            self.verify_account_storage_locked(&storage, &journal.account.identity)?;
        }
        if let Some(existing) = registry
            .accounts
            .iter()
            .find(|entry| entry.account_ref == journal.account.account_ref)
        {
            if existing.identity != journal.account.identity || existing.storage != storage {
                return Err(AccountRegistryError::IdentityMismatch);
            }
        } else {
            self.register_account_locked(
                registry,
                &journal.account,
                storage,
                RecoveryProtectionState::Needed,
            )?;
            self.write_registry_locked(registry)?;
        }
        remove_public_document(&self.add_path(&journal.receipt_ref))
            .map_err(|_| AccountRegistryError::Storage)?;
        self.dashboard_locked(registry)
    }

    fn resume_switch_locked(
        &self,
        registry: &mut AccountRegistry,
    ) -> Result<(), AccountRegistryError> {
        let Some(journal) = read_json_document::<AccountSwitchJournal>(&self.switch_path())
            .map_err(|_| AccountRegistryError::Storage)?
        else {
            return Ok(());
        };
        if journal.schema != ACCOUNT_SWITCH_SCHEMA
            || journal.next_generation != journal.expected_generation.saturating_add(1)
        {
            return Err(AccountRegistryError::InvalidState);
        }
        if registry.active.generation == journal.next_generation
            && registry.active.account_ref.as_ref() == Some(&journal.to_account_ref)
        {
            remove_public_document(&self.switch_path())
                .map_err(|_| AccountRegistryError::Storage)?;
            return Ok(());
        }
        if registry.active.generation != journal.expected_generation
            || registry.active.account_ref != journal.from_account_ref
        {
            return Err(AccountRegistryError::SwitchInProgress);
        }
        self.apply_switch_locked(registry, journal).map(|_| ())
    }

    fn apply_switch_locked(
        &self,
        registry: &mut AccountRegistry,
        mut journal: AccountSwitchJournal,
    ) -> Result<AccountDashboardProjection, AccountRegistryError> {
        let now = unix_time_now();
        let target_storage = registry
            .accounts
            .iter()
            .find(|entry| entry.account_ref == journal.to_account_ref)
            .map(|entry| entry.storage.clone())
            .ok_or(AccountRegistryError::AccountNotFound)?;
        if let AccountStorageLocator::RemoteNip46 { capability_ref } = &target_storage {
            Nip46Service::for_data_root(self.data_root.clone())
                .rebind_generation(
                    capability_ref,
                    &journal.to_account_ref,
                    journal.next_generation,
                )
                .map_err(|_| AccountRegistryError::RemoteSigner)?;
        }
        if let Some(previous) = journal.from_account_ref.as_ref()
            && let Some(entry) = find_account_mut(registry, previous)
        {
            entry.lifecycle = AccountLifecycleState::SignedOut;
            entry.signer.availability = SignerAvailability::UserApprovalRequired;
            entry.updated_at = now;
        }
        let target = find_account_mut(registry, &journal.to_account_ref)
            .ok_or(AccountRegistryError::AccountNotFound)?;
        target.lifecycle = AccountLifecycleState::Active;
        target.signer.availability = SignerAvailability::Ready;
        target.updated_at = now;
        registry.active = ActiveAccountSelection {
            account_ref: Some(journal.to_account_ref.clone()),
            generation: journal.next_generation,
        };
        self.write_registry_locked(registry)?;
        journal.phase = SwitchPhase::RegistryCommitted;
        write_json_document(&self.switch_path(), &journal)
            .map_err(|_| AccountRegistryError::Storage)?;
        remove_public_document(&self.switch_path()).map_err(|_| AccountRegistryError::Storage)?;
        self.dashboard_locked(registry)
    }

    fn dashboard_locked(
        &self,
        registry: &AccountRegistry,
    ) -> Result<AccountDashboardProjection, AccountRegistryError> {
        let pending_switch = read_json_document::<AccountSwitchJournal>(&self.switch_path())
            .map_err(|_| AccountRegistryError::Storage)?
            .as_ref()
            .map(PendingAccountSwitch::from);
        let mut pending_purges = Vec::new();
        for entry in &registry.accounts {
            if let Some(purge) = self.read_purge_locked(&entry.account_ref)? {
                pending_purges.push(purge.report());
            }
        }
        Ok(AccountDashboardProjection {
            active: registry.active.clone(),
            accounts: registry
                .accounts
                .iter()
                .map(|entry| AccountDashboardEntry {
                    account_ref: entry.account_ref.clone(),
                    identity: entry.identity.clone(),
                    fingerprint: entry.identity.fingerprint().display(),
                    profile: entry.profile.clone(),
                    lifecycle: entry.lifecycle,
                    signer: entry.signer.clone(),
                    recovery: entry.recovery,
                    retirement: entry.retirement,
                    is_active: registry.active.account_ref.as_ref() == Some(&entry.account_ref),
                })
                .collect(),
            pending_switch,
            pending_purges,
        })
    }

    fn verify_account_storage_locked(
        &self,
        storage: &AccountStorageLocator,
        identity: &PublicIdentity,
    ) -> Result<(), AccountRegistryError> {
        match storage {
            AccountStorageLocator::LegacyRoot | AccountStorageLocator::Partitioned { .. } => {
                verify_account_root(&self.root_for_storage(storage)?, &self.locator, identity)
            }
            AccountStorageLocator::RemoteNip46 { capability_ref } => {
                let capability = Nip46Service::for_data_root(self.data_root.clone())
                    .load_capability(capability_ref)
                    .map_err(|_| AccountRegistryError::RemoteSigner)?;
                if capability.user_identity != *identity
                    || capability.state != Nip46CapabilityState::Active
                {
                    return Err(AccountRegistryError::IdentityMismatch);
                }
                Ok(())
            }
        }
    }

    fn root_for_storage(
        &self,
        storage: &AccountStorageLocator,
    ) -> Result<PathBuf, AccountRegistryError> {
        match storage {
            AccountStorageLocator::LegacyRoot => Ok(self.identity_root.clone()),
            AccountStorageLocator::Partitioned { directory_name }
                if is_lower_hex_64(directory_name) =>
            {
                Ok(self.accounts_root().join(directory_name))
            }
            AccountStorageLocator::Partitioned { .. } => Err(AccountRegistryError::InvalidState),
            AccountStorageLocator::RemoteNip46 { capability_ref } => {
                Nip46Service::for_data_root(self.data_root.clone())
                    .capability_directory(capability_ref)
                    .map_err(|_| AccountRegistryError::RemoteSigner)
            }
        }
    }

    fn write_registry_locked(
        &self,
        registry: &AccountRegistry,
    ) -> Result<(), AccountRegistryError> {
        validate_registry(registry)?;
        write_json_document(&self.registry_path(), registry)
            .map_err(|_| AccountRegistryError::Storage)
    }

    fn read_purge_locked(
        &self,
        account_ref: &AccountRef,
    ) -> Result<Option<AccountPurgeJournal>, AccountRegistryError> {
        let path = self.purge_path(account_ref);
        let journal = read_json_document::<AccountPurgeJournal>(&path)
            .map_err(|_| AccountRegistryError::Storage)?;
        if let Some(journal) = journal.as_ref()
            && (journal.schema != ACCOUNT_PURGE_SCHEMA
                || journal.account_ref != *account_ref
                || journal.targets.len() != all_purge_targets().len()
                || journal
                    .targets
                    .iter()
                    .map(|report| report.target)
                    .collect::<std::collections::HashSet<_>>()
                    .len()
                    != all_purge_targets().len()
                || matches!(
                    &journal.storage,
                    AccountStorageLocator::Partitioned { directory_name }
                        if directory_name != &partition_directory_name(account_ref)
                ))
        {
            return Err(AccountRegistryError::InvalidState);
        }
        Ok(journal)
    }

    fn write_purge_locked(
        &self,
        journal: &AccountPurgeJournal,
    ) -> Result<(), AccountRegistryError> {
        write_json_document(&self.purge_path(&journal.account_ref), journal)
            .map_err(|_| AccountRegistryError::Storage)
    }

    fn read_add_locked(
        &self,
        receipt_ref: &ReceiptRef,
    ) -> Result<Option<AccountAddJournal>, AccountRegistryError> {
        let journal = read_json_document::<AccountAddJournal>(&self.add_path(receipt_ref))
            .map_err(|_| AccountRegistryError::Storage)?;
        if let Some(journal) = journal.as_ref() {
            validate_add_journal(journal)?;
            if journal.receipt_ref != *receipt_ref {
                return Err(AccountRegistryError::InvalidState);
            }
        }
        Ok(journal)
    }

    fn write_add_locked(&self, journal: &AccountAddJournal) -> Result<(), AccountRegistryError> {
        validate_add_journal(journal)?;
        write_json_document(&self.add_path(&journal.receipt_ref), journal)
            .map_err(|_| AccountRegistryError::Storage)
    }

    fn complete_purge_if_verified_locked(
        &self,
        registry: &mut AccountRegistry,
        journal: &AccountPurgeJournal,
    ) -> Result<(), AccountRegistryError> {
        if journal
            .targets
            .iter()
            .all(|target| target.state == AccountPurgeTargetState::Deleted)
        {
            let entry = find_account_mut(registry, &journal.account_ref)
                .ok_or(AccountRegistryError::AccountNotFound)?;
            entry.lifecycle = AccountLifecycleState::Forgotten;
            entry.updated_at = unix_time_now();
            self.write_registry_locked(registry)?;
        }
        Ok(())
    }

    fn registry_path(&self) -> PathBuf {
        self.accounts_root().join("index.json")
    }

    fn switch_path(&self) -> PathBuf {
        self.accounts_root().join("switch.transaction.json")
    }

    fn add_path(&self, receipt_ref: &ReceiptRef) -> PathBuf {
        self.accounts_root().join(format!(
            "add-{}.transaction.json",
            operation_digest(receipt_ref.as_str())
        ))
    }

    fn purge_path(&self, account_ref: &AccountRef) -> PathBuf {
        self.accounts_root().join(format!(
            "purge-{}.json",
            partition_directory_name(account_ref)
        ))
    }

    fn accounts_root(&self) -> PathBuf {
        self.identity_root.join("accounts")
    }
}

fn activation_lifecycle(account: &IdentityAccountRecord) -> AccountLifecycleState {
    match account.state {
        crate::IdentityActivationState::CandidateLocal => AccountLifecycleState::CandidateLocal,
        crate::IdentityActivationState::CandidateExisting => {
            AccountLifecycleState::CandidateExisting
        }
        crate::IdentityActivationState::Activating => AccountLifecycleState::Activating,
        crate::IdentityActivationState::Active => AccountLifecycleState::Active,
    }
}

fn validate_registry(registry: &AccountRegistry) -> Result<(), AccountRegistryError> {
    if registry.schema != ACCOUNT_REGISTRY_SCHEMA
        || registry.schema_version != ACCOUNT_REGISTRY_VERSION
        || registry.active.generation == 0
    {
        return Err(AccountRegistryError::InvalidState);
    }
    let mut references = std::collections::HashSet::new();
    let mut legacy_roots = 0;
    for entry in &registry.accounts {
        let storage_valid = match &entry.storage {
            AccountStorageLocator::LegacyRoot => {
                legacy_roots += 1;
                legacy_roots == 1
            }
            AccountStorageLocator::Partitioned { directory_name } => {
                is_lower_hex_64(directory_name)
                    && directory_name == &partition_directory_name(&entry.account_ref)
            }
            AccountStorageLocator::RemoteNip46 { capability_ref } => {
                valid_capability_ref(capability_ref)
            }
        };
        if !references.insert(entry.account_ref.as_str())
            || entry.identity.validate().is_err()
            || !storage_valid
            || entry.account_ref.as_str()
                != format!("omega-account-{}", entry.identity.public_key_hex().as_str())
        {
            return Err(AccountRegistryError::InvalidState);
        }
    }
    if let Some(active) = registry.active.account_ref.as_ref() {
        let Some(entry) = registry
            .accounts
            .iter()
            .find(|entry| &entry.account_ref == active)
        else {
            return Err(AccountRegistryError::InvalidState);
        };
        if matches!(
            entry.lifecycle,
            AccountLifecycleState::ForgetPending | AccountLifecycleState::Forgotten
        ) {
            return Err(AccountRegistryError::InvalidState);
        }
    }
    Ok(())
}

fn find_account_mut<'a>(
    registry: &'a mut AccountRegistry,
    account_ref: &AccountRef,
) -> Option<&'a mut DurableAccountEntry> {
    registry
        .accounts
        .iter_mut()
        .find(|entry| &entry.account_ref == account_ref)
}

fn next_generation(generation: u64) -> Result<u64, AccountRegistryError> {
    generation
        .checked_add(1)
        .ok_or(AccountRegistryError::InvalidState)
}

fn partition_directory_name(account_ref: &AccountRef) -> String {
    hex::encode(Sha256::digest(account_ref.as_str().as_bytes()))
}

fn valid_capability_ref(capability_ref: &str) -> bool {
    !capability_ref.is_empty()
        && capability_ref.len() <= 128
        && capability_ref
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
}

fn validate_add_journal(journal: &AccountAddJournal) -> Result<(), AccountRegistryError> {
    if journal.schema != ACCOUNT_ADD_SCHEMA
        || !journal.account.validate()
        || journal.final_directory_name != partition_directory_name(&journal.account.account_ref)
        || journal.staging_directory_name
            != format!(
                ".staging-{}",
                operation_digest(journal.receipt_ref.as_str())
            )
    {
        return Err(AccountRegistryError::InvalidState);
    }
    Ok(())
}

fn verify_account_root(
    root: &Path,
    locator: &KeyringLocator,
    identity: &PublicIdentity,
) -> Result<(), AccountRegistryError> {
    let store = FileSecretStore::new(root.join("identity.secret"));
    let secret = store
        .read(locator)
        .map_err(map_store_error)?
        .ok_or(AccountRegistryError::PartitionIncomplete)?;
    let stored_identity = secret
        .public_identity()
        .map_err(|_| AccountRegistryError::IdentityMismatch)?;
    if stored_identity.public_key_hex() != identity.public_key_hex() {
        return Err(AccountRegistryError::IdentityMismatch);
    }
    let account = read_json_document::<IdentityAccountRecord>(&root.join("identity.account.json"))
        .map_err(|_| AccountRegistryError::Storage)?
        .ok_or(AccountRegistryError::PartitionIncomplete)?;
    if !account.validate() || account.identity != *identity {
        return Err(AccountRegistryError::IdentityMismatch);
    }
    Ok(())
}

fn operation_digest(operation_ref: &str) -> String {
    hex::encode(Sha256::digest(operation_ref.as_bytes()))
}

fn read_recovery_state(
    root: &Path,
    identity: &PublicIdentity,
) -> Result<RecoveryProtectionState, AccountRegistryError> {
    let record = read_json_document::<crate::RecoveryProtectionRecord>(
        &root.join("identity.recovery-protection.json"),
    )
    .map_err(|_| AccountRegistryError::Storage)?;
    match record {
        Some(record)
            if record.validate().is_ok()
                && record.identity().public_key_hex() == identity.public_key_hex() =>
        {
            Ok(RecoveryProtectionState::Protected)
        }
        Some(_) => Err(AccountRegistryError::InvalidState),
        None => Ok(RecoveryProtectionState::Needed),
    }
}

fn map_store_error(_error: StoreError) -> AccountRegistryError {
    AccountRegistryError::Storage
}

fn validate_operation_ref(operation_ref: &str) -> Result<(), AccountRegistryError> {
    if operation_ref.is_empty()
        || operation_ref.len() > 128
        || !operation_ref
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'-' | b'_'))
    {
        return Err(AccountRegistryError::InvalidOperationRef);
    }
    Ok(())
}

fn all_purge_targets() -> [AccountPurgeTarget; 11] {
    [
        AccountPurgeTarget::Secret,
        AccountPurgeTarget::CustodyDocuments,
        AccountPurgeTarget::HeldIntents,
        AccountPurgeTarget::RecoveryMetadata,
        AccountPurgeTarget::Drafts,
        AccountPurgeTarget::DecryptedCache,
        AccountPurgeTarget::WalletState,
        AccountPurgeTarget::RelayState,
        AccountPurgeTarget::RoomState,
        AccountPurgeTarget::SignerSessions,
        AccountPurgeTarget::DeviceGrants,
    ]
}

fn is_registry_owned_purge_target(target: AccountPurgeTarget) -> bool {
    matches!(
        target,
        AccountPurgeTarget::Secret
            | AccountPurgeTarget::CustodyDocuments
            | AccountPurgeTarget::HeldIntents
            | AccountPurgeTarget::RecoveryMetadata
    )
}

fn delete_and_verify_target(root: &Path, target: AccountPurgeTarget) -> io::Result<()> {
    let relative_paths: &[&str] = match target {
        AccountPurgeTarget::Secret => &["identity.secret", "client.secret", "pairing.secret"],
        AccountPurgeTarget::CustodyDocuments => &[
            "identity.json",
            "identity.complete.json",
            "identity.transaction.json",
            "identity.reset.json",
            "identity.account.json",
            "identity.backup-value.json",
            "identity.backup-nudge-dismissed.json",
            "capability.json",
            "pairing.json",
            "runtime-request.json",
        ],
        AccountPurgeTarget::HeldIntents => &["identity.action-intent.json"],
        AccountPurgeTarget::RecoveryMetadata => &["identity.recovery-protection.json"],
        AccountPurgeTarget::Drafts
        | AccountPurgeTarget::DecryptedCache
        | AccountPurgeTarget::WalletState
        | AccountPurgeTarget::RelayState
        | AccountPurgeTarget::RoomState
        | AccountPurgeTarget::SignerSessions
        | AccountPurgeTarget::DeviceGrants => {
            return Err(io::Error::other(
                "purge target must be verified by its owning subsystem",
            ));
        }
    };
    for relative_path in relative_paths {
        let path = root.join(relative_path);
        match fs::remove_file(&path) {
            Ok(()) => {}
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(error),
        }
        if path.try_exists()? {
            return Err(io::Error::other("account purge target still exists"));
        }
    }
    Ok(())
}

fn is_lower_hex_64(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || matches!(byte, b'a'..=b'f'))
}

fn unix_time_now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |duration| duration.as_secs())
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

#[cfg(test)]
mod tests {
    use zeroize::Zeroizing;

    use super::*;
    use crate::{
        AdmittedSigningRequest, IDENTITY_ACCOUNT_SCHEMA, IDENTITY_ACCOUNT_SCHEMA_VERSION,
        IdentityActivationState, IdentityCandidateOrigin, RecoveryProtectionRecord, SigningPurpose,
        UnsignedEventTemplate, secret::SecretKeyMaterial,
    };

    fn active_account(root: &Path, secret_byte: u8) -> IdentityAccountRecord {
        let secret = SecretKeyMaterial::from_bytes(Zeroizing::new([secret_byte; 32]))
            .expect("valid fixture secret");
        let identity = secret.public_identity().expect("fixture public identity");
        let account_ref = AccountRef::new(format!(
            "omega-account-{}",
            identity.public_key_hex().as_str()
        ))
        .expect("fixture account reference");
        FileSecretStore::new(root.join("identity.secret"))
            .write(&KeyringLocator::for_channel(AppChannel::Dev), &secret)
            .expect("write fixture secret");
        let account = IdentityAccountRecord {
            schema: IDENTITY_ACCOUNT_SCHEMA.to_string(),
            schema_version: IDENTITY_ACCOUNT_SCHEMA_VERSION,
            account_ref,
            account_generation: 1,
            identity: identity.clone(),
            state: IdentityActivationState::Active,
            candidate_origin: IdentityCandidateOrigin::Local,
            activation_ref: Some(ReceiptRef::new("fixture-activation").expect("receipt")),
            updated_at: 1,
        };
        write_json_document(&root.join("identity.account.json"), &account)
            .expect("write fixture account");
        let recovery =
            RecoveryProtectionRecord::new(identity, "a".repeat(64), 32).expect("fixture recovery");
        write_json_document(&root.join("identity.recovery-protection.json"), &recovery)
            .expect("write fixture recovery");
        account
    }

    fn register_second_account(
        service: &AccountRegistryService,
        data_root: &Path,
        secret_byte: u8,
    ) -> IdentityAccountRecord {
        let secret = SecretKeyMaterial::from_bytes(Zeroizing::new([secret_byte; 32]))
            .expect("valid fixture secret");
        let identity = secret.public_identity().expect("fixture public identity");
        let account_ref = AccountRef::new(format!(
            "omega-account-{}",
            identity.public_key_hex().as_str()
        ))
        .expect("fixture account reference");
        let root = data_root
            .join("identity")
            .join("accounts")
            .join(partition_directory_name(&account_ref));
        let account = active_account(&root, secret_byte);
        service
            .register_partitioned_account(&account, RecoveryProtectionState::Protected)
            .expect("register fixture account");
        account
    }

    /// The exact 2026-07-30 owner profile shape: a pre-multi-account identity
    /// migrated to the registry as `legacy_root` + `candidate_existing` with
    /// `recovery: needed`. Signing custody exists (the secret is present and
    /// the signer is ready), so hosted proof signing must work — the
    /// activation ceremony and recovery protection gate onboarding, not
    /// signing.
    fn candidate_existing_account(root: &Path, fixture: &str) -> IdentityAccountRecord {
        let service = IdentityService::for_identity_root(AppChannel::Dev, root.to_path_buf());
        service
            .create(ReceiptRef::new(fixture).expect("receipt"))
            .expect("create fixture identity");
        let mut account = service.inspect_account().expect("inspect fixture account");
        account.state = IdentityActivationState::CandidateExisting;
        account.candidate_origin = IdentityCandidateOrigin::Existing;
        account.activation_ref = None;
        write_json_document(&root.join("identity.account.json"), &account)
            .expect("write fixture account");
        account
    }

    #[test]
    fn migrated_candidate_existing_account_without_recovery_signs() {
        let temporary_directory = tempfile::tempdir().expect("temporary directory");
        let identity_root = temporary_directory.path().join("identity");
        let account = candidate_existing_account(&identity_root, "owner-shape-41");
        let service = AccountRegistryService::for_channel_data_root(
            AppChannel::Dev,
            temporary_directory.path().to_path_buf(),
        );
        let dashboard = service.inspect().expect("migrate registry");
        assert_eq!(
            dashboard.accounts[0].lifecycle,
            AccountLifecycleState::CandidateExisting
        );
        assert_eq!(
            dashboard.accounts[0].recovery,
            RecoveryProtectionState::Needed
        );

        let token = service.selection_token().expect("selection token");
        service
            .validate_signing_selection(&token)
            .expect("a migrated candidate account with a ready local signer signs");

        // End to end through custody: the post-signature use recording must
        // not fail the signature either (it did before 2026-07-30).
        let identity_service = IdentityService::for_channel_data_root(
            AppChannel::Dev,
            temporary_directory.path().to_path_buf(),
        );
        let request = AdmittedSigningRequest {
            request_ref: ReceiptRef::new("nip98.fixture").expect("request ref"),
            identity_ref: account.identity.identity_ref().clone(),
            purpose: SigningPurpose::NostrEvent,
            event: UnsignedEventTemplate {
                created_at: 1_700_000_000,
                kind: 27_235,
                tags: vec![
                    vec!["u".to_string(), "https://openagents.com/api".to_string()],
                    vec!["method".to_string(), "POST".to_string()],
                ],
                content: String::new(),
            },
        };
        let result = identity_service
            .sign(&request)
            .expect("hosted proof signing works on the migrated owner profile shape");
        assert_eq!(result.identity, account.identity);
    }

    #[test]
    fn signing_selection_still_refuses_signed_out_and_unready_accounts() {
        let temporary_directory = tempfile::tempdir().expect("temporary directory");
        let identity_root = temporary_directory.path().join("identity");
        candidate_existing_account(&identity_root, "owner-shape-42");
        let service = AccountRegistryService::for_channel_data_root(
            AppChannel::Dev,
            temporary_directory.path().to_path_buf(),
        );
        service.inspect().expect("migrate registry");
        let token = service.selection_token().expect("selection token");
        service
            .sign_out(token.generation)
            .expect("sign out the active account");
        assert!(matches!(
            service.validate_signing_selection(&token),
            Err(AccountRegistryError::StaleSelection)
        ));
    }

    #[test]
    fn partition_identity_service_resolves_legacy_root_storage() {
        let temporary_directory = tempfile::tempdir().expect("temporary directory");
        let identity_root = temporary_directory.path().join("identity");
        let account = candidate_existing_account(&identity_root, "owner-shape-43");
        let service = AccountRegistryService::for_channel_data_root(
            AppChannel::Dev,
            temporary_directory.path().to_path_buf(),
        );
        service.inspect().expect("migrate registry");

        // Before 2026-07-30 this resolved the (empty) partition directory and
        // reported custody `Absent`, which killed startup identity hydration
        // on migrated profiles.
        let inspected = service
            .partition_identity_service(&account.account_ref)
            .inspect_account()
            .expect("legacy-root custody resolves through the account service");
        assert_eq!(inspected.account_ref, account.account_ref);

        // A genuinely partitioned account still resolves its partition.
        let dashboard = service
            .add_local_account(ReceiptRef::new("partition-fixture").expect("receipt"))
            .expect("add partitioned local account");
        let added_ref = dashboard
            .accounts
            .iter()
            .find(|entry| entry.account_ref != account.account_ref)
            .expect("added account")
            .account_ref
            .clone();
        let partitioned = service
            .partition_identity_service(&added_ref)
            .inspect_account()
            .expect("partitioned custody still resolves");
        assert_eq!(partitioned.account_ref, added_ref);
    }

    #[test]
    fn legacy_migration_keeps_secret_in_place_and_projects_public_fields() {
        let temporary_directory = tempfile::tempdir().expect("temporary directory");
        let identity_root = temporary_directory.path().join("identity");
        let account = active_account(&identity_root, 1);
        let original_secret = fs::read(identity_root.join("identity.secret")).expect("read secret");
        let service = AccountRegistryService::for_channel_data_root(
            AppChannel::Dev,
            temporary_directory.path().to_path_buf(),
        );

        let dashboard = service.inspect().expect("migrate registry");

        assert_eq!(
            dashboard.active.account_ref.as_ref(),
            Some(&account.account_ref)
        );
        assert_eq!(dashboard.accounts.len(), 1);
        assert_eq!(
            dashboard.accounts[0].fingerprint,
            account.fingerprint_display()
        );
        assert_eq!(
            fs::read(identity_root.join("identity.secret")).expect("legacy secret remains"),
            original_secret
        );
        assert!(
            !temporary_directory
                .path()
                .join("identity")
                .join("accounts")
                .join(partition_directory_name(&account.account_ref))
                .join("identity.secret")
                .exists()
        );
    }

    #[test]
    fn add_and_switch_preserve_accounts_and_invalidate_a_to_b_to_a_tokens() {
        let temporary_directory = tempfile::tempdir().expect("temporary directory");
        let identity_root = temporary_directory.path().join("identity");
        let first = active_account(&identity_root, 1);
        let service = AccountRegistryService::for_channel_data_root(
            AppChannel::Dev,
            temporary_directory.path().to_path_buf(),
        );
        service.inspect().expect("migrate registry");
        let first_service = IdentityService::for_channel_data_root(
            AppChannel::Dev,
            temporary_directory.path().to_path_buf(),
        );
        let first_token = service.selection_token().expect("first token");
        let second = register_second_account(&service, temporary_directory.path(), 2);

        assert!(matches!(
            first_service.validate_account_selection(),
            Err(CustodyError::StaleAccountSelection)
        ));
        let second_service = IdentityService::for_channel_data_root(
            AppChannel::Dev,
            temporary_directory.path().to_path_buf(),
        );
        assert_eq!(
            second_service
                .inspect()
                .expect("inspect second service")
                .identity,
            Some(second.identity.clone())
        );
        let second_generation = service.inspect().expect("dashboard").active.generation;
        let dashboard = service
            .switch_account(&first.account_ref, second_generation)
            .expect("switch to first");
        let renewed_first_token = service.selection_token().expect("renewed token");
        assert!(renewed_first_token.generation > first_token.generation);
        let dashboard = service
            .switch_account(&second.account_ref, dashboard.active.generation)
            .expect("switch to second again");
        service
            .switch_account(&first.account_ref, dashboard.active.generation)
            .expect("switch back to first again");
        assert!(matches!(
            service.validate_selection(&renewed_first_token),
            Err(AccountRegistryError::StaleSelection)
        ));
        assert!(identity_root.join("identity.secret").exists());
        assert!(
            temporary_directory
                .path()
                .join("identity")
                .join("accounts")
                .join(partition_directory_name(&second.account_ref))
                .join("identity.secret")
                .exists()
        );
    }

    #[test]
    fn hydrated_profile_write_is_generation_fenced_across_account_switches() {
        let temporary_directory = tempfile::tempdir().expect("temporary directory");
        let identity_root = temporary_directory.path().join("identity");
        let first = active_account(&identity_root, 31);
        let service = AccountRegistryService::for_channel_data_root(
            AppChannel::Dev,
            temporary_directory.path().to_path_buf(),
        );
        service.inspect().expect("migrate registry");
        let stale_token = service.selection_token().expect("first token");
        let second = register_second_account(&service, temporary_directory.path(), 32);

        assert!(matches!(
            service.record_hydrated_profile(
                &stale_token,
                Some(AccountProfileSummary {
                    display_name: Some("Stale profile".to_string()),
                    avatar_ref: None,
                }),
            ),
            Err(AccountRegistryError::StaleSelection)
        ));

        let second_token = service.selection_token().expect("second token");
        let dashboard = service
            .record_hydrated_profile(
                &second_token,
                Some(AccountProfileSummary {
                    display_name: Some("Current profile".to_string()),
                    avatar_ref: None,
                }),
            )
            .expect("record current profile");
        let second_entry = dashboard
            .accounts
            .iter()
            .find(|account| account.account_ref == second.account_ref)
            .expect("second account");
        assert_eq!(
            second_entry
                .profile
                .as_ref()
                .and_then(|profile| profile.display_name.as_deref()),
            Some("Current profile")
        );
        let first_entry = dashboard
            .accounts
            .iter()
            .find(|account| account.account_ref == first.account_ref)
            .expect("first account");
        assert!(first_entry.profile.is_none());
    }

    #[test]
    fn lock_unlock_sign_out_and_retire_have_distinct_effects() {
        let temporary_directory = tempfile::tempdir().expect("temporary directory");
        let account = active_account(&temporary_directory.path().join("identity"), 3);
        let service = AccountRegistryService::for_channel_data_root(
            AppChannel::Dev,
            temporary_directory.path().to_path_buf(),
        );
        let initial = service.inspect().expect("initial dashboard");
        let locked = service
            .lock_active(initial.active.generation)
            .expect("lock account");
        assert_eq!(locked.accounts[0].lifecycle, AccountLifecycleState::Locked);
        let locked_service = IdentityService::for_channel_data_root(
            AppChannel::Dev,
            temporary_directory.path().to_path_buf(),
        );
        assert_eq!(
            locked_service
                .inspect()
                .expect("locked inspection")
                .identity,
            Some(account.identity.clone())
        );
        assert!(locked_service.validate_current_account_selection().is_ok());
        assert!(matches!(
            locked_service.validate_account_selection(),
            Err(CustodyError::StaleAccountSelection)
        ));
        let unlocked = service
            .unlock_active(locked.active.generation)
            .expect("unlock account");
        assert_eq!(
            unlocked.accounts[0].lifecycle,
            AccountLifecycleState::Active
        );
        let retired = service
            .retire_account(&account.account_ref, AccountRetirementState::Pending)
            .expect("retire account");
        assert_eq!(retired.accounts[0].lifecycle, AccountLifecycleState::Active);
        assert_eq!(
            retired.accounts[0].retirement,
            AccountRetirementState::Pending
        );
        let signed_out = service
            .sign_out(retired.active.generation)
            .expect("sign out");
        assert!(signed_out.active.account_ref.is_none());
        assert_eq!(
            signed_out.accounts[0].lifecycle,
            AccountLifecycleState::SignedOut
        );
        assert!(
            temporary_directory
                .path()
                .join("identity/identity.secret")
                .exists()
        );
        let signed_out_service = IdentityService::for_channel_data_root(
            AppChannel::Dev,
            temporary_directory.path().to_path_buf(),
        );
        assert!(
            signed_out_service
                .inspect()
                .expect("signed-out inspection")
                .identity
                .is_none()
        );
        assert!(matches!(
            signed_out_service.validate_account_selection(),
            Err(CustodyError::StaleAccountSelection)
        ));
        assert!(matches!(
            signed_out_service.create(ReceiptRef::new("signed-out-create").expect("receipt")),
            Err(CustodyError::StaleAccountSelection)
        ));
        assert!(
            !temporary_directory
                .path()
                .join("identity/accounts/unavailable/identity.secret")
                .exists()
        );
    }

    #[test]
    fn purge_never_claims_external_state_was_deleted_without_owner_verification() {
        let temporary_directory = tempfile::tempdir().expect("temporary directory");
        let account = active_account(&temporary_directory.path().join("identity"), 4);
        let service = AccountRegistryService::for_channel_data_root(
            AppChannel::Dev,
            temporary_directory.path().to_path_buf(),
        );
        let dashboard = service.inspect().expect("initial dashboard");
        service
            .begin_forget(
                &account.account_ref,
                "forget-fixture".to_string(),
                dashboard.active.generation,
            )
            .expect("begin forget");
        let partial = service
            .retry_purge(&account.account_ref, "forget-fixture")
            .expect("purge registry files");
        assert!(!partial.complete);
        assert!(partial.targets.iter().any(|report| {
            report.target == AccountPurgeTarget::Drafts
                && report.state == AccountPurgeTargetState::Pending
        }));
        let pending = service.inspect().expect("pending dashboard");
        assert_eq!(
            pending.accounts[0].lifecycle,
            AccountLifecycleState::ForgetPending
        );

        for target in all_purge_targets()
            .into_iter()
            .filter(|target| !is_registry_owned_purge_target(*target))
        {
            service
                .record_purge_target_result(
                    &account.account_ref,
                    "forget-fixture",
                    target,
                    AccountPurgeVerification::VerifiedDeleted,
                )
                .expect("record owner verification");
        }
        let complete = service.inspect().expect("complete dashboard");
        assert_eq!(
            complete.accounts[0].lifecycle,
            AccountLifecycleState::Forgotten
        );
        assert!(
            complete
                .pending_purges
                .iter()
                .find(|report| report.account_ref == account.account_ref)
                .is_some_and(|report| report.complete)
        );
    }

    #[test]
    fn add_local_account_generates_once_in_partition_and_preserves_legacy_candidate() {
        let temporary_directory = tempfile::tempdir().expect("temporary directory");
        let legacy = active_account(&temporary_directory.path().join("identity"), 5);
        let service = AccountRegistryService::for_channel_data_root(
            AppChannel::Dev,
            temporary_directory.path().to_path_buf(),
        );
        service.inspect().expect("migrate legacy");
        let dashboard = service
            .add_local_account(ReceiptRef::new("add-local-fixture").expect("receipt"))
            .expect("add local account");
        assert_eq!(dashboard.accounts.len(), 2);
        let added = dashboard
            .accounts
            .iter()
            .find(|entry| entry.account_ref != legacy.account_ref)
            .expect("new account");
        assert_eq!(added.lifecycle, AccountLifecycleState::CandidateLocal);
        assert_eq!(added.recovery, RecoveryProtectionState::Needed);
        assert!(
            temporary_directory
                .path()
                .join("identity/identity.secret")
                .exists()
        );
        let candidate_service = IdentityService::for_channel_data_root(
            AppChannel::Dev,
            temporary_directory.path().to_path_buf(),
        );
        assert!(
            candidate_service
                .validate_current_account_selection()
                .is_ok()
        );
        // Owner direction 2026-07-30: a candidate account with a ready local
        // signer signs (hosted login proofs must not wait on the activation
        // ceremony); the ceremony below still gates durable identity actions.
        assert!(candidate_service.validate_account_selection().is_ok());
        let now = unix_time_now();
        assert!(
            candidate_service
                .begin_identity_activation(
                    ReceiptRef::new("candidate-activation").expect("receipt"),
                    now.saturating_add(600),
                )
                .is_ok()
        );
        assert!(
            temporary_directory
                .path()
                .join("identity")
                .join("accounts")
                .join(partition_directory_name(&added.account_ref))
                .join("identity.secret")
                .exists()
        );
    }

    #[test]
    fn fresh_system_create_is_migrated_and_resolved_on_restart() {
        let temporary_directory = tempfile::tempdir().expect("temporary directory");
        let data_root = temporary_directory.path().to_path_buf();
        let service = IdentityService::for_channel_data_root(AppChannel::Dev, data_root.clone());
        let created = service
            .create(ReceiptRef::new("fresh-system-create").expect("receipt"))
            .expect("create legacy identity");
        let identity = created.identity.expect("created identity");
        let registry =
            AccountRegistryService::for_channel_data_root(AppChannel::Dev, data_root.clone());
        let dashboard = registry.inspect().expect("migrate created identity");
        assert_eq!(dashboard.accounts.len(), 1);
        assert_eq!(dashboard.accounts[0].identity, identity);
        assert_eq!(
            dashboard.accounts[0].lifecycle,
            AccountLifecycleState::CandidateLocal
        );

        let restarted = IdentityService::for_channel_data_root(AppChannel::Dev, data_root);
        assert_eq!(
            restarted.inspect().expect("restart inspection").identity,
            Some(identity)
        );
    }

    #[test]
    fn add_journal_resumes_exact_staged_identity_without_regeneration() {
        let temporary_directory = tempfile::tempdir().expect("temporary directory");
        let data_root = temporary_directory.path().to_path_buf();
        let service =
            AccountRegistryService::for_channel_data_root(AppChannel::Dev, data_root.clone());
        let receipt_ref = ReceiptRef::new("journaled-add").expect("receipt");
        let staging_directory_name = format!(".staging-{}", operation_digest(receipt_ref.as_str()));
        let staging_root = data_root
            .join("identity")
            .join("accounts")
            .join(&staging_directory_name);
        let staging_service =
            IdentityService::for_identity_root(AppChannel::Dev, staging_root.clone());
        staging_service
            .create(receipt_ref.clone())
            .expect("stage identity");
        let account = staging_service.inspect_account().expect("staged account");
        let journal = AccountAddJournal {
            schema: ACCOUNT_ADD_SCHEMA.to_string(),
            receipt_ref: receipt_ref.clone(),
            final_directory_name: partition_directory_name(&account.account_ref),
            staging_directory_name,
            account: account.clone(),
        };
        write_json_document(&service.add_path(&receipt_ref), &journal).expect("write add journal");

        let dashboard = service
            .add_local_account(receipt_ref)
            .expect("resume journal");

        assert_eq!(dashboard.accounts.len(), 1);
        assert_eq!(dashboard.accounts[0].identity, account.identity);
        assert!(!staging_root.exists());
        assert!(
            data_root
                .join("identity")
                .join("accounts")
                .join(partition_directory_name(&account.account_ref))
                .join("identity.secret")
                .exists()
        );
    }

    #[test]
    fn add_journal_resumes_after_partition_move() {
        let temporary_directory = tempfile::tempdir().expect("temporary directory");
        let data_root = temporary_directory.path().to_path_buf();
        let service =
            AccountRegistryService::for_channel_data_root(AppChannel::Dev, data_root.clone());
        let receipt_ref = ReceiptRef::new("moved-journaled-add").expect("receipt");
        let staging_directory_name = format!(".staging-{}", operation_digest(receipt_ref.as_str()));
        let staging_root = data_root
            .join("identity")
            .join("accounts")
            .join(&staging_directory_name);
        let staging_service =
            IdentityService::for_identity_root(AppChannel::Dev, staging_root.clone());
        staging_service
            .create(receipt_ref.clone())
            .expect("stage identity");
        let account = staging_service.inspect_account().expect("staged account");
        let final_directory_name = partition_directory_name(&account.account_ref);
        let final_root = data_root
            .join("identity")
            .join("accounts")
            .join(&final_directory_name);
        let journal = AccountAddJournal {
            schema: ACCOUNT_ADD_SCHEMA.to_string(),
            receipt_ref: receipt_ref.clone(),
            account: account.clone(),
            staging_directory_name,
            final_directory_name,
        };
        write_json_document(&service.add_path(&receipt_ref), &journal).expect("write add journal");
        fs::rename(staging_root, &final_root).expect("simulate completed move");

        let dashboard = service
            .add_local_account(receipt_ref.clone())
            .expect("resume moved journal");

        assert_eq!(dashboard.accounts[0].identity, account.identity);
        assert!(!service.add_path(&receipt_ref).exists());
    }

    #[test]
    fn prepared_switch_journal_resumes_idempotently() {
        let temporary_directory = tempfile::tempdir().expect("temporary directory");
        let data_root = temporary_directory.path().to_path_buf();
        let first = active_account(&data_root.join("identity"), 6);
        let service =
            AccountRegistryService::for_channel_data_root(AppChannel::Dev, data_root.clone());
        service.inspect().expect("migrate first account");
        let second = register_second_account(&service, &data_root, 7);
        let before = service.inspect().expect("dashboard before switch");
        let journal = AccountSwitchJournal {
            schema: ACCOUNT_SWITCH_SCHEMA.to_string(),
            from_account_ref: Some(second.account_ref),
            to_account_ref: first.account_ref.clone(),
            expected_generation: before.active.generation,
            next_generation: before.active.generation + 1,
            phase: SwitchPhase::Prepared,
        };
        write_json_document(&service.switch_path(), &journal).expect("write switch journal");

        let resumed = service.inspect().expect("resume switch");

        assert_eq!(resumed.active.account_ref, Some(first.account_ref));
        assert_eq!(resumed.active.generation, journal.next_generation);
        assert!(!service.switch_path().exists());
    }

    #[test]
    fn successful_signing_records_last_use_but_refused_signing_does_not() {
        let temporary_directory = tempfile::tempdir().expect("temporary directory");
        let data_root = temporary_directory.path().to_path_buf();
        let setup_service =
            IdentityService::for_identity_root(AppChannel::Dev, data_root.join("identity"));
        setup_service
            .create(ReceiptRef::new("last-use-create").expect("receipt"))
            .expect("create signing identity");
        let mut account = setup_service.inspect_account().expect("candidate account");
        account.state = IdentityActivationState::Active;
        account.activation_ref = Some(ReceiptRef::new("last-use-activation").expect("receipt"));
        write_json_document(&data_root.join("identity/identity.account.json"), &account)
            .expect("activate fixture account");
        let recovery = RecoveryProtectionRecord::new(account.identity.clone(), "a".repeat(64), 32)
            .expect("fixture recovery");
        write_json_document(
            &data_root.join("identity/identity.recovery-protection.json"),
            &recovery,
        )
        .expect("write recovery");
        let registry =
            AccountRegistryService::for_channel_data_root(AppChannel::Dev, data_root.clone());
        registry.inspect().expect("migrate active account");
        let identity_service = IdentityService::for_channel_data_root(AppChannel::Dev, data_root);
        let mut request = crate::AdmittedSigningRequest {
            request_ref: ReceiptRef::new("last-use-sign").expect("receipt"),
            identity_ref: account.identity.identity_ref().clone(),
            purpose: SigningPurpose::Nip44EncryptedSelfEvent,
            event: UnsignedEventTemplate {
                created_at: 1_700_000_000,
                kind: 1,
                tags: Vec::new(),
                content: "last use fixture".to_string(),
            },
        };
        assert!(identity_service.sign(&request).is_err());
        assert_eq!(
            registry.inspect().expect("dashboard").accounts[0]
                .signer
                .last_successful_use,
            None
        );

        request.purpose = SigningPurpose::NostrEvent;
        identity_service.sign(&request).expect("sign event");

        assert!(
            registry.inspect().expect("dashboard").accounts[0]
                .signer
                .last_successful_use
                .is_some()
        );
    }

    #[test]
    fn remote_runtime_outcomes_are_generation_fenced_and_offline_is_retryable() {
        let temporary_directory = tempfile::tempdir().expect("temporary directory");
        let secret =
            SecretKeyMaterial::from_bytes(Zeroizing::new([19; 32])).expect("valid fixture secret");
        let identity = secret.public_identity().expect("fixture identity");
        let account_ref = AccountRef::new(format!(
            "omega-account-{}",
            identity.public_key_hex().as_str()
        ))
        .expect("account ref");
        let token = AccountSelectionToken {
            account_ref: account_ref.clone(),
            identity: identity.clone(),
            generation: 7,
        };
        let registry = AccountRegistry {
            schema: ACCOUNT_REGISTRY_SCHEMA.to_string(),
            schema_version: ACCOUNT_REGISTRY_VERSION,
            active: ActiveAccountSelection {
                account_ref: Some(account_ref.clone()),
                generation: token.generation,
            },
            accounts: vec![DurableAccountEntry {
                account_ref,
                identity,
                storage: AccountStorageLocator::RemoteNip46 {
                    capability_ref: "runtime-outcome-capability".to_string(),
                },
                lifecycle: AccountLifecycleState::Active,
                signer: AccountSignerSummary {
                    kind: SignerKind::RemoteNip46,
                    availability: SignerAvailability::Ready,
                    last_successful_use: None,
                },
                recovery: RecoveryProtectionState::NotApplicable,
                retirement: AccountRetirementState::NotRetired,
                profile: None,
                updated_at: 1,
            }],
        };
        let service = AccountRegistryService::for_channel_data_root(
            AppChannel::Dev,
            temporary_directory.path().to_path_buf(),
        );
        service
            .write_registry_locked(&registry)
            .expect("write remote registry");

        let offline = service
            .record_remote_signer_outcome(
                &token,
                RemoteSignerRuntimeOutcome::Offline,
                1_700_000_001,
            )
            .expect("record offline");
        assert_eq!(
            offline.accounts[0].signer.availability,
            SignerAvailability::Offline
        );
        service
            .validate_signing_selection(&token)
            .expect("explicit offline retry is allowed");

        let ready = service
            .record_remote_signer_outcome(
                &token,
                RemoteSignerRuntimeOutcome::Ready {
                    used_at: 1_700_000_002,
                },
                1_700_000_002,
            )
            .expect("record success");
        assert_eq!(
            ready.accounts[0].signer.availability,
            SignerAvailability::Ready
        );
        assert_eq!(
            ready.accounts[0].signer.last_successful_use,
            Some(1_700_000_002)
        );

        let mut stale = token.clone();
        stale.generation += 1;
        assert!(matches!(
            service.record_remote_signer_outcome(
                &stale,
                RemoteSignerRuntimeOutcome::Offline,
                1_700_000_003,
            ),
            Err(AccountRegistryError::StaleSelection)
        ));

        let rejected = service
            .record_remote_signer_outcome(
                &token,
                RemoteSignerRuntimeOutcome::Rejected,
                1_700_000_004,
            )
            .expect("record rejection");
        assert_eq!(
            rejected.accounts[0].signer.availability,
            SignerAvailability::Rejected
        );
        assert!(matches!(
            service.validate_signing_selection(&token),
            Err(AccountRegistryError::AccountUnavailable)
        ));
    }
}
