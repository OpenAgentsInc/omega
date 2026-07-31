use std::{collections::BTreeSet, path::PathBuf};

use omega_device_enrollment::{
    AuthorizedDevice, DeviceCapability, DeviceEnrollmentStore, DeviceInventoryEntry,
    DevicePlatform, EnrollmentAccountFence, EnrollmentError, EnrollmentGrant, PairingConfirmation,
    PairingInvite, PairingLifecycleProjection, PairingResponse, PendingDeviceEnrollment,
    SasChallenge,
};
use omega_identity::AccountSelectionToken;

use crate::omega_host_signer_capabilities::{HostPlatform, HostSignerCapability};

pub struct DeviceEnrollmentController {
    store: DeviceEnrollmentStore,
}

impl DeviceEnrollmentController {
    pub fn system() -> Self {
        Self {
            store: DeviceEnrollmentStore::system(),
        }
    }

    pub fn for_data_root(data_root: impl Into<PathBuf>) -> Self {
        Self {
            store: DeviceEnrollmentStore::for_data_root(data_root),
        }
    }

    pub fn account_fence(
        selection: &AccountSelectionToken,
    ) -> Result<EnrollmentAccountFence, EnrollmentError> {
        EnrollmentAccountFence::new(
            selection.account_ref.as_str(),
            selection.identity.public_key_hex().as_str(),
            selection.generation,
        )
    }

    pub fn create_pairing_introduction(
        &self,
        selection: &AccountSelectionToken,
        endpoint: impl Into<String>,
        approved_platform: HostPlatform,
        approved_capabilities: BTreeSet<HostSignerCapability>,
        owner_authorization_ref: impl Into<String>,
        now: u64,
        lifetime_seconds: u64,
    ) -> Result<PairingInvite, EnrollmentError> {
        let approved_capabilities = approved_capabilities
            .into_iter()
            .map(device_capability)
            .collect::<BTreeSet<_>>();
        self.store.create_pairing_invite(
            Self::account_fence(selection)?,
            endpoint,
            device_platform(approved_platform),
            approved_capabilities,
            owner_authorization_ref,
            now,
            lifetime_seconds,
        )
    }

    pub fn begin_target_enrollment(
        &self,
        invite: PairingInvite,
        device_label: impl Into<String>,
        platform: HostPlatform,
        capabilities: BTreeSet<HostSignerCapability>,
        now: u64,
    ) -> Result<(PendingTargetEnrollment, PairingResponse), EnrollmentError> {
        let capabilities = capabilities
            .into_iter()
            .map(device_capability)
            .collect::<BTreeSet<_>>();
        let (pending, response) = self.store.begin_device_enrollment(
            invite,
            device_label,
            device_platform(platform),
            capabilities,
            now,
        )?;
        Ok((PendingTargetEnrollment(pending), response))
    }

    pub fn resume_target_enrollment(
        &self,
        account: &EnrollmentAccountFence,
        pairing_id: &str,
    ) -> Result<PendingTargetEnrollment, EnrollmentError> {
        self.store
            .resume_pending_device(account, pairing_id)
            .map(PendingTargetEnrollment)
    }

    pub fn accept_target_response(
        &self,
        selection: &AccountSelectionToken,
        response: PairingResponse,
        now: u64,
    ) -> Result<HostSasChallenge, EnrollmentError> {
        let fence = Self::account_fence(selection)?;
        if response.account != fence {
            return Err(EnrollmentError::WrongGeneration);
        }
        self.store
            .accept_pairing_response(&fence, response, now)
            .map(HostSasChallenge)
    }

    pub fn parse_target_response(bytes: &[u8]) -> Result<PairingResponse, EnrollmentError> {
        serde_json::from_slice(bytes).map_err(|_| EnrollmentError::InvalidInvite)
    }

    pub fn confirm_target_sas(
        &self,
        pending: &PendingTargetEnrollment,
        challenge: &HostSasChallenge,
        confirmed_sas: &str,
    ) -> Result<TargetPairingConfirmation, EnrollmentError> {
        pending
            .0
            .confirm(&challenge.0, confirmed_sas)
            .map(TargetPairingConfirmation)
    }

    pub fn redeem_confirmed_pairing(
        &self,
        selection: &AccountSelectionToken,
        confirmation: &TargetPairingConfirmation,
        confirmed_sas: &str,
        grant_lifetime_seconds: u64,
        now: u64,
    ) -> Result<EnrollmentGrant, EnrollmentError> {
        let fence = Self::account_fence(selection)?;
        if confirmation.0.account != fence {
            return Err(EnrollmentError::WrongGeneration);
        }
        self.store.redeem_pairing(
            &fence,
            &confirmation.0,
            confirmed_sas,
            grant_lifetime_seconds,
            now,
        )
    }

    pub fn recover_redeemed_grant(
        &self,
        selection: &AccountSelectionToken,
        pairing_id: &str,
        device_public_key_hex: &str,
    ) -> Result<EnrollmentGrant, EnrollmentError> {
        self.store.recover_redeemed_grant(
            &Self::account_fence(selection)?,
            pairing_id,
            device_public_key_hex,
        )
    }

    pub fn finalize_target_credential(
        &self,
        account: &EnrollmentAccountFence,
        pairing_id: &str,
        grant: &EnrollmentGrant,
        now: u64,
    ) -> Result<(), EnrollmentError> {
        self.store
            .finalize_redeemed_device(account, pairing_id, grant, now)
    }

    pub fn authorize_host_capability(
        &self,
        selection: &AccountSelectionToken,
        grant_id: &str,
        device_public_key_hex: &str,
        capability: HostSignerCapability,
        now: u64,
    ) -> Result<AuthorizedDevice, EnrollmentError> {
        self.store.authorize(
            &Self::account_fence(selection)?,
            grant_id,
            device_public_key_hex,
            device_capability(capability),
            now,
        )
    }

    pub fn record_last_use(
        &self,
        authorization: &AuthorizedDevice,
        now: u64,
    ) -> Result<(), EnrollmentError> {
        self.store.record_last_use(authorization, now)
    }

    pub fn revoke_device(
        &self,
        selection: &AccountSelectionToken,
        device_public_key_hex: &str,
        now: u64,
    ) -> Result<(), EnrollmentError> {
        self.store
            .revoke_device(&Self::account_fence(selection)?, device_public_key_hex, now)
    }

    pub fn device_inventory(
        &self,
        selection: &AccountSelectionToken,
    ) -> Result<Vec<DeviceInventoryEntry>, EnrollmentError> {
        self.store
            .device_inventory(&Self::account_fence(selection)?)
    }

    pub fn pairing_lifecycle(
        &self,
        selection: &AccountSelectionToken,
        pairing_id: &str,
        now: u64,
    ) -> Result<PairingLifecycleProjection, EnrollmentError> {
        self.store
            .pairing_lifecycle(&Self::account_fence(selection)?, pairing_id, now)
    }
}

pub struct PendingTargetEnrollment(PendingDeviceEnrollment);

impl PendingTargetEnrollment {
    pub fn sas(&self) -> &str {
        self.0.sas()
    }

    pub fn device_public_key_hex(&self) -> &str {
        self.0.device_public_key_hex()
    }
}

impl std::fmt::Debug for PendingTargetEnrollment {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("PendingTargetEnrollment")
            .field("device_public_key_hex", &self.device_public_key_hex())
            .field("sas", &"[REDACTED]")
            .finish()
    }
}

pub struct HostSasChallenge(SasChallenge);

impl HostSasChallenge {
    pub fn parse_wire_json(bytes: &[u8]) -> Result<Self, EnrollmentError> {
        serde_json::from_slice(bytes)
            .map(Self)
            .map_err(|_| EnrollmentError::InvalidInvite)
    }

    pub fn pairing_id(&self) -> &str {
        &self.0.pairing_id
    }

    pub fn sas(&self) -> &str {
        &self.0.sas
    }

    pub fn wire_json(&self) -> Result<Vec<u8>, EnrollmentError> {
        serde_json::to_vec(&self.0).map_err(|_| EnrollmentError::Storage)
    }
}

impl std::fmt::Debug for HostSasChallenge {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("HostSasChallenge")
            .field("pairing_id", &self.0.pairing_id)
            .field("transcript_digest", &self.0.transcript_digest)
            .field("sas", &"[REDACTED]")
            .field("host_confirmation_proof", &"[REDACTED]")
            .finish()
    }
}

pub struct TargetPairingConfirmation(PairingConfirmation);

impl TargetPairingConfirmation {
    pub fn parse_wire_json(bytes: &[u8]) -> Result<Self, EnrollmentError> {
        serde_json::from_slice(bytes)
            .map(Self)
            .map_err(|_| EnrollmentError::InvalidInvite)
    }

    pub fn wire_json(&self) -> Result<Vec<u8>, EnrollmentError> {
        serde_json::to_vec(&self.0).map_err(|_| EnrollmentError::Storage)
    }
}

impl std::fmt::Debug for TargetPairingConfirmation {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("TargetPairingConfirmation")
            .field("pairing_id", &self.0.pairing_id)
            .field("account", &self.0.account)
            .field("device_public_key_hex", &self.0.device_public_key_hex)
            .field("transcript_digest", &self.0.transcript_digest)
            .field("client_confirmation_proof", &"[REDACTED]")
            .finish()
    }
}

fn device_platform(platform: HostPlatform) -> DevicePlatform {
    match platform {
        HostPlatform::Desktop => DevicePlatform::Desktop,
        HostPlatform::Web => DevicePlatform::Web,
        HostPlatform::Android => DevicePlatform::Android,
        HostPlatform::Ios => DevicePlatform::Ios,
    }
}

fn device_capability(capability: HostSignerCapability) -> DeviceCapability {
    match capability {
        HostSignerCapability::DesktopLocal => DeviceCapability::DesktopLocal,
        HostSignerCapability::Nip46 => DeviceCapability::Nip46,
        HostSignerCapability::Nip07 => DeviceCapability::Nip07,
        HostSignerCapability::Nip55 => DeviceCapability::Nip55,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nostr::{Keys, SecretKey};
    use omega_identity::{AccountRef, IdentityRef, PublicIdentity};

    fn selection(generation: u64) -> AccountSelectionToken {
        let keys = Keys::new(SecretKey::from_hex(&"1".repeat(64)).expect("secret key"));
        AccountSelectionToken {
            account_ref: AccountRef::new("omega-account-owner").expect("account"),
            identity: PublicIdentity::from_public_key_hex(
                IdentityRef::new("omega-owner").expect("identity"),
                keys.public_key().to_hex(),
            )
            .expect("identity"),
            generation,
        }
    }

    #[test]
    fn controller_uses_core_two_peer_sas_and_exact_capability_grant() {
        let directory = tempfile::tempdir().expect("temporary directory");
        let host = DeviceEnrollmentController::for_data_root(directory.path().join("host"));
        let target = DeviceEnrollmentController::for_data_root(directory.path().join("target"));
        let current_selection = selection(3);
        let invite = host
            .create_pairing_introduction(
                &current_selection,
                "wss://desktop.example.com/device-enrollment",
                HostPlatform::Android,
                BTreeSet::from([HostSignerCapability::Nip55]),
                "owner-approved-android",
                100,
                300,
            )
            .expect("pairing introduction");
        let (pending, response) = target
            .begin_target_enrollment(
                invite,
                "Android phone",
                HostPlatform::Android,
                BTreeSet::from([HostSignerCapability::Nip55]),
                101,
            )
            .expect("target response");
        let challenge = host
            .accept_target_response(&current_selection, response.clone(), 102)
            .expect("host challenge");
        assert_eq!(pending.sas(), challenge.sas());
        assert!(!format!("{pending:?}").contains(pending.sas()));
        assert!(!format!("{challenge:?}").contains(challenge.sas()));
        let resumed = target
            .resume_target_enrollment(&response.account, &response.pairing_id)
            .expect("resumed target");
        assert_eq!(
            resumed.device_public_key_hex(),
            pending.device_public_key_hex()
        );
        let confirmation = target
            .confirm_target_sas(&resumed, &challenge, resumed.sas())
            .expect("target confirmation");
        let grant = host
            .redeem_confirmed_pairing(&current_selection, &confirmation, challenge.sas(), 600, 103)
            .expect("grant");
        assert_eq!(
            host.recover_redeemed_grant(
                &current_selection,
                &response.pairing_id,
                &response.device_public_key_hex,
            )
            .expect("recovered grant"),
            grant
        );
        target
            .finalize_target_credential(&response.account, &response.pairing_id, &grant, 104)
            .expect("finalized target credential");
        host.authorize_host_capability(
            &current_selection,
            &grant.grant_id,
            &grant.device_public_key_hex,
            HostSignerCapability::Nip55,
            104,
        )
        .expect("authorized NIP-55");
        assert!(matches!(
            host.authorize_host_capability(
                &current_selection,
                &grant.grant_id,
                &grant.device_public_key_hex,
                HostSignerCapability::Nip46,
                104,
            ),
            Err(EnrollmentError::CapabilityDenied)
        ));
        assert!(matches!(
            host.authorize_host_capability(
                &selection(4),
                &grant.grant_id,
                &grant.device_public_key_hex,
                HostSignerCapability::Nip55,
                104,
            ),
            Err(EnrollmentError::NotFound
                | EnrollmentError::WrongGeneration
                | EnrollmentError::InvalidGrant)
        ));
    }

    #[test]
    fn controller_refuses_nip55_on_ios_before_any_grant_exists() {
        let directory = tempfile::tempdir().expect("temporary directory");
        let host = DeviceEnrollmentController::for_data_root(directory.path().join("host"));
        let target = DeviceEnrollmentController::for_data_root(directory.path().join("target"));
        let invite = host
            .create_pairing_introduction(
                &selection(1),
                "wss://desktop.example.com/device-enrollment",
                HostPlatform::Ios,
                BTreeSet::from([HostSignerCapability::Nip46]),
                "owner-approved-ios",
                100,
                300,
            )
            .expect("pairing introduction");
        assert!(matches!(
            target.begin_target_enrollment(
                invite,
                "iPhone",
                HostPlatform::Ios,
                BTreeSet::from([HostSignerCapability::Nip55]),
                101,
            ),
            Err(EnrollmentError::CapabilityDenied)
        ));
    }
}
