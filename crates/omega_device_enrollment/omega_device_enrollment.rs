use std::{
    collections::BTreeSet,
    fmt, fs,
    io::{self, Read as _, Write as _},
    path::{Path, PathBuf},
    sync::{Mutex, MutexGuard, OnceLock},
};

use atomic_write_file::AtomicWriteFile;
use hkdf::Hkdf;
use hmac::{Hmac, Mac as _};
use nostr::{Keys, PublicKey, SecretKey, nips::nip44::v2::ConversationKey};
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};
use thiserror::Error;
use url::Url;

const STORE_SCHEMA: &str = "openagents.omega.device-enrollment.store.v1";
const INVITE_SCHEMA: &str = "openagents.omega.device-enrollment.invite.v1";
const TRANSCRIPT_SCHEMA: &str = "openagents.omega.device-enrollment.transcript.v1";
const LOCAL_CREDENTIAL_SCHEMA: &str = "openagents.omega.device-enrollment.credential.v1";
const PENDING_DEVICE_SCHEMA: &str = "openagents.omega.device-enrollment.pending-device.v1";
const MAX_FILE_BYTES: u64 = 1024 * 1024;
const MAX_PAIRING_LIFETIME: u64 = 15 * 60;
const MAX_GRANT_LIFETIME: u64 = 90 * 24 * 60 * 60;

#[derive(Debug, Error)]
pub enum EnrollmentError {
    #[error("the account fence is invalid")]
    InvalidFence,
    #[error("the pairing invitation is invalid")]
    InvalidInvite,
    #[error("the pairing invitation expired")]
    ExpiredInvite,
    #[error("the pairing transcript or peer proof does not match")]
    TranscriptMismatch,
    #[error("the short authentication string does not match")]
    WrongSas,
    #[error("the pairing invitation has already been used")]
    AlreadyRedeemed,
    #[error("the pairing invitation is not ready for this operation")]
    InvalidPairingState,
    #[error("the selected account generation changed")]
    WrongGeneration,
    #[error("the device grant is invalid or expired")]
    InvalidGrant,
    #[error("the requested device capability was not granted")]
    CapabilityDenied,
    #[error("the device has been revoked")]
    Revoked,
    #[error("the device enrollment record does not exist")]
    NotFound,
    #[error("the device enrollment store is unavailable")]
    Storage,
    #[error("the device enrollment store is malformed or unsupported")]
    InvalidStoredData,
    #[error("cryptographic key derivation failed")]
    Crypto,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EnrollmentAccountFence {
    pub account_ref: String,
    pub owner_public_key_hex: String,
    pub generation: u64,
}

impl EnrollmentAccountFence {
    pub fn new(
        account_ref: impl Into<String>,
        owner_public_key_hex: impl Into<String>,
        generation: u64,
    ) -> Result<Self, EnrollmentError> {
        let fence = Self {
            account_ref: account_ref.into(),
            owner_public_key_hex: owner_public_key_hex.into(),
            generation,
        };
        fence.validate()?;
        Ok(fence)
    }

    fn validate(&self) -> Result<(), EnrollmentError> {
        if !valid_reference(&self.account_ref)
            || !valid_hex_64(&self.owner_public_key_hex)
            || self.generation == 0
        {
            return Err(EnrollmentError::InvalidFence);
        }
        Ok(())
    }
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DeviceCapability {
    DesktopLocal,
    Nip46,
    Nip07,
    Nip55,
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DevicePlatform {
    Desktop,
    Web,
    Android,
    Ios,
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PairingInvite {
    pub schema: String,
    pub pairing_id: String,
    pub endpoint: String,
    pub account: EnrollmentAccountFence,
    pub approved_platform: DevicePlatform,
    pub approved_capabilities: BTreeSet<DeviceCapability>,
    pub owner_authorization_ref: String,
    pub host_ephemeral_public_key_hex: String,
    pub issued_at: u64,
    pub expires_at: u64,
    pairing_secret_hex: String,
}

impl PairingInvite {
    pub fn parse_wire_json(bytes: &[u8], now: u64) -> Result<Self, EnrollmentError> {
        let invite: Self =
            serde_json::from_slice(bytes).map_err(|_| EnrollmentError::InvalidInvite)?;
        invite.validate(now)?;
        Ok(invite)
    }

    pub fn wire_json(&self) -> Result<Vec<u8>, EnrollmentError> {
        serde_json::to_vec(self).map_err(|_| EnrollmentError::Storage)
    }

    fn validate(&self, now: u64) -> Result<(), EnrollmentError> {
        self.validate_shape()?;
        if now >= self.expires_at {
            return Err(EnrollmentError::ExpiredInvite);
        }
        Ok(())
    }

    fn validate_shape(&self) -> Result<(), EnrollmentError> {
        self.account.validate()?;
        let endpoint = Url::parse(&self.endpoint).map_err(|_| EnrollmentError::InvalidInvite)?;
        if self.schema != INVITE_SCHEMA
            || !valid_hex_64(&self.pairing_id)
            || !valid_hex_64(&self.host_ephemeral_public_key_hex)
            || !valid_hex_64(&self.pairing_secret_hex)
            || !matches!(endpoint.scheme(), "ws" | "wss")
            || endpoint.host_str().is_none()
            || !endpoint.username().is_empty()
            || endpoint.password().is_some()
            || endpoint.fragment().is_some()
            || !valid_reference(&self.owner_authorization_ref)
            || validate_platform_capabilities(self.approved_platform, &self.approved_capabilities)
                .is_err()
            || self.issued_at == 0
            || self.expires_at <= self.issued_at
            || self.expires_at.saturating_sub(self.issued_at) > MAX_PAIRING_LIFETIME
        {
            return Err(EnrollmentError::InvalidInvite);
        }
        Ok(())
    }
}

impl fmt::Debug for PairingInvite {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("PairingInvite")
            .field("schema", &self.schema)
            .field("pairing_id", &self.pairing_id)
            .field("endpoint", &self.endpoint)
            .field("account", &self.account)
            .field("approved_platform", &self.approved_platform)
            .field("approved_capabilities", &self.approved_capabilities)
            .field("owner_authorization_ref", &self.owner_authorization_ref)
            .field(
                "host_ephemeral_public_key_hex",
                &self.host_ephemeral_public_key_hex,
            )
            .field("issued_at", &self.issued_at)
            .field("expires_at", &self.expires_at)
            .field("pairing_secret_hex", &"[REDACTED]")
            .finish()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct PairingTranscript {
    schema: String,
    pairing_id: String,
    endpoint: String,
    account: EnrollmentAccountFence,
    host_ephemeral_public_key_hex: String,
    join_ephemeral_public_key_hex: String,
    device_public_key_hex: String,
    device_label: String,
    platform: DevicePlatform,
    capabilities: BTreeSet<DeviceCapability>,
    owner_authorization_ref: String,
    invite_expires_at: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PairingResponse {
    pub pairing_id: String,
    pub account: EnrollmentAccountFence,
    pub join_ephemeral_public_key_hex: String,
    pub device_public_key_hex: String,
    pub device_label: String,
    pub platform: DevicePlatform,
    pub capabilities: BTreeSet<DeviceCapability>,
    pub transcript_digest: String,
    pub client_hello_proof: String,
}

#[derive(Clone, PartialEq, Eq)]
pub struct PendingDeviceEnrollment {
    schema: String,
    invite: PairingInvite,
    response: PairingResponse,
    join_ephemeral_secret_key_hex: String,
    device_secret_key_hex: String,
    sas: String,
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct DurablePendingDeviceEnrollment {
    schema: String,
    invite: PairingInvite,
    response: PairingResponse,
    join_ephemeral_secret_key_hex: String,
    device_secret_key_hex: String,
    sas: String,
}

impl From<&PendingDeviceEnrollment> for DurablePendingDeviceEnrollment {
    fn from(pending: &PendingDeviceEnrollment) -> Self {
        Self {
            schema: pending.schema.clone(),
            invite: pending.invite.clone(),
            response: pending.response.clone(),
            join_ephemeral_secret_key_hex: pending.join_ephemeral_secret_key_hex.clone(),
            device_secret_key_hex: pending.device_secret_key_hex.clone(),
            sas: pending.sas.clone(),
        }
    }
}

impl From<DurablePendingDeviceEnrollment> for PendingDeviceEnrollment {
    fn from(pending: DurablePendingDeviceEnrollment) -> Self {
        Self {
            schema: pending.schema,
            invite: pending.invite,
            response: pending.response,
            join_ephemeral_secret_key_hex: pending.join_ephemeral_secret_key_hex,
            device_secret_key_hex: pending.device_secret_key_hex,
            sas: pending.sas,
        }
    }
}

impl PendingDeviceEnrollment {
    fn begin(
        invite: PairingInvite,
        device_label: impl Into<String>,
        platform: DevicePlatform,
        capabilities: BTreeSet<DeviceCapability>,
        now: u64,
    ) -> Result<(Self, PairingResponse), EnrollmentError> {
        invite.validate(now)?;
        let device_label = device_label.into();
        validate_device_request(&device_label, platform, &capabilities)?;
        if platform != invite.approved_platform || capabilities != invite.approved_capabilities {
            return Err(EnrollmentError::CapabilityDenied);
        }
        let join_keys = Keys::generate();
        let device_keys = Keys::generate();
        let transcript = transcript(
            &invite,
            join_keys.public_key().to_hex(),
            device_keys.public_key().to_hex(),
            device_label,
            platform,
            capabilities,
        );
        let transcript_bytes =
            serde_json::to_vec(&transcript).map_err(|_| EnrollmentError::Crypto)?;
        let transcript_digest = digest(&transcript_bytes);
        let conversation_key = derive_key(
            join_keys.secret_key(),
            &invite.host_ephemeral_public_key_hex,
        )?;
        let client_hello_proof = proof(
            &conversation_key,
            &invite.pairing_secret_hex,
            &transcript_bytes,
            b"client-hello",
        )?;
        let response = PairingResponse {
            pairing_id: invite.pairing_id.clone(),
            account: invite.account.clone(),
            join_ephemeral_public_key_hex: join_keys.public_key().to_hex(),
            device_public_key_hex: device_keys.public_key().to_hex(),
            device_label: transcript.device_label,
            platform,
            capabilities: transcript.capabilities,
            transcript_digest,
            client_hello_proof,
        };
        let pending = Self {
            schema: PENDING_DEVICE_SCHEMA.into(),
            sas: sas(
                &conversation_key,
                &invite.pairing_secret_hex,
                &transcript_bytes,
            )?,
            invite,
            response: response.clone(),
            join_ephemeral_secret_key_hex: join_keys.secret_key().to_secret_hex(),
            device_secret_key_hex: device_keys.secret_key().to_secret_hex(),
        };
        Ok((pending, response))
    }

    pub fn sas(&self) -> &str {
        &self.sas
    }

    pub fn device_public_key_hex(&self) -> &str {
        &self.response.device_public_key_hex
    }

    pub fn confirm(
        &self,
        challenge: &SasChallenge,
        confirmed_sas: &str,
    ) -> Result<PairingConfirmation, EnrollmentError> {
        if confirmed_sas != self.sas || challenge.sas != self.sas {
            return Err(EnrollmentError::WrongSas);
        }
        if challenge.pairing_id != self.response.pairing_id
            || challenge.transcript_digest != self.response.transcript_digest
        {
            return Err(EnrollmentError::TranscriptMismatch);
        }
        let transcript = transcript_from_response(&self.invite, &self.response)?;
        let transcript_bytes =
            serde_json::to_vec(&transcript).map_err(|_| EnrollmentError::Crypto)?;
        let secret = SecretKey::from_hex(&self.join_ephemeral_secret_key_hex)
            .map_err(|_| EnrollmentError::Crypto)?;
        let conversation_key = derive_key(&secret, &self.invite.host_ephemeral_public_key_hex)?;
        let expected_host_proof = proof(
            &conversation_key,
            &self.invite.pairing_secret_hex,
            &transcript_bytes,
            b"host-confirm",
        )?;
        if challenge.host_confirmation_proof != expected_host_proof {
            return Err(EnrollmentError::TranscriptMismatch);
        }
        Ok(PairingConfirmation {
            pairing_id: self.response.pairing_id.clone(),
            account: self.response.account.clone(),
            device_public_key_hex: self.response.device_public_key_hex.clone(),
            transcript_digest: self.response.transcript_digest.clone(),
            client_confirmation_proof: proof(
                &conversation_key,
                &self.invite.pairing_secret_hex,
                &transcript_bytes,
                b"client-confirm",
            )?,
        })
    }

    fn validate_stored(&self) -> Result<(), EnrollmentError> {
        if self.schema != PENDING_DEVICE_SCHEMA
            || self.invite.account != self.response.account
            || self.invite.pairing_id != self.response.pairing_id
            || self.invite.approved_platform != self.response.platform
            || self.invite.approved_capabilities != self.response.capabilities
        {
            return Err(EnrollmentError::InvalidStoredData);
        }
        self.invite.validate_shape()?;
        let join_secret = SecretKey::from_hex(&self.join_ephemeral_secret_key_hex)
            .map_err(|_| EnrollmentError::InvalidStoredData)?;
        let device_secret = SecretKey::from_hex(&self.device_secret_key_hex)
            .map_err(|_| EnrollmentError::InvalidStoredData)?;
        if Keys::new(join_secret.clone()).public_key().to_hex()
            != self.response.join_ephemeral_public_key_hex
            || Keys::new(device_secret).public_key().to_hex() != self.response.device_public_key_hex
        {
            return Err(EnrollmentError::InvalidStoredData);
        }
        let transcript = transcript_from_response(&self.invite, &self.response)?;
        let transcript_bytes =
            serde_json::to_vec(&transcript).map_err(|_| EnrollmentError::InvalidStoredData)?;
        if digest(&transcript_bytes) != self.response.transcript_digest {
            return Err(EnrollmentError::InvalidStoredData);
        }
        let conversation_key =
            derive_key(&join_secret, &self.invite.host_ephemeral_public_key_hex)?;
        if proof(
            &conversation_key,
            &self.invite.pairing_secret_hex,
            &transcript_bytes,
            b"client-hello",
        )? != self.response.client_hello_proof
            || sas(
                &conversation_key,
                &self.invite.pairing_secret_hex,
                &transcript_bytes,
            )? != self.sas
        {
            return Err(EnrollmentError::InvalidStoredData);
        }
        Ok(())
    }
}

impl fmt::Debug for PendingDeviceEnrollment {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("PendingDeviceEnrollment")
            .field("schema", &self.schema)
            .field("invite", &self.invite)
            .field("response", &self.response)
            .field("join_ephemeral_secret_key_hex", &"[REDACTED]")
            .field("device_secret_key_hex", &"[REDACTED]")
            .field("sas", &"[REDACTED]")
            .finish()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SasChallenge {
    pub pairing_id: String,
    pub transcript_digest: String,
    pub sas: String,
    pub host_confirmation_proof: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PairingConfirmation {
    pub pairing_id: String,
    pub account: EnrollmentAccountFence,
    pub device_public_key_hex: String,
    pub transcript_digest: String,
    pub client_confirmation_proof: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EnrollmentGrant {
    pub grant_id: String,
    pub account: EnrollmentAccountFence,
    pub device_public_key_hex: String,
    pub platform: DevicePlatform,
    pub capabilities: BTreeSet<DeviceCapability>,
    pub owner_authorization_ref: String,
    pub issued_at: u64,
    pub expires_at: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DeviceInventoryEntry {
    pub device_public_key_hex: String,
    pub device_label: String,
    pub platform: DevicePlatform,
    pub capabilities: BTreeSet<DeviceCapability>,
    pub grant_id: String,
    pub enrolled_at: u64,
    pub expires_at: u64,
    pub last_used_at: Option<u64>,
    pub revoked_at: Option<u64>,
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PairingLifecycle {
    Open,
    AwaitingConfirmation,
    Redeemed,
    Expired,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PairingLifecycleProjection {
    pub pairing_id: String,
    pub endpoint: String,
    pub account: EnrollmentAccountFence,
    pub issued_at: u64,
    pub expires_at: u64,
    pub lifecycle: PairingLifecycle,
    pub device_public_key_hex: Option<String>,
    pub device_label: Option<String>,
    pub platform: Option<DevicePlatform>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuthorizedDevice {
    pub account: EnrollmentAccountFence,
    pub device_public_key_hex: String,
    pub capability: DeviceCapability,
    pub grant_id: String,
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
struct SensitiveSecret(String);

impl fmt::Debug for SensitiveSecret {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("SensitiveSecret([REDACTED])")
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum PairingState {
    Open,
    AwaitingConfirmation,
    Redeemed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct DurablePairing {
    invite: PairingInvite,
    host_ephemeral_secret: SensitiveSecret,
    response: Option<PairingResponse>,
    transcript_digest: Option<String>,
    sas_digest: Option<String>,
    confirmation_digest: Option<String>,
    state: PairingState,
    grant: Option<EnrollmentGrant>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct DurableHostStore {
    schema: String,
    account: EnrollmentAccountFence,
    pairings: Vec<DurablePairing>,
    devices: Vec<DeviceInventoryEntry>,
}

impl DurableHostStore {
    fn validate(&self) -> Result<(), EnrollmentError> {
        self.account.validate()?;
        if self.schema != STORE_SCHEMA {
            return Err(EnrollmentError::InvalidStoredData);
        }
        let mut pairing_ids = BTreeSet::new();
        for pairing in &self.pairings {
            if !pairing_ids.insert(&pairing.invite.pairing_id) {
                return Err(EnrollmentError::InvalidStoredData);
            }
            let mut validation_invite = pairing.invite.clone();
            if pairing.state == PairingState::Redeemed {
                validation_invite.pairing_secret_hex = "0".repeat(64);
            }
            validation_invite.validate_shape()?;
            if validation_invite.account != self.account {
                return Err(EnrollmentError::WrongGeneration);
            }
            match pairing.state {
                PairingState::Open
                    if pairing.response.is_none()
                        && pairing.grant.is_none()
                        && valid_hex_64(&pairing.host_ephemeral_secret.0) => {}
                PairingState::AwaitingConfirmation
                    if pairing.response.is_some()
                        && pairing.transcript_digest.is_some()
                        && pairing.sas_digest.is_some()
                        && pairing.grant.is_none()
                        && valid_hex_64(&pairing.host_ephemeral_secret.0) => {}
                PairingState::Redeemed
                    if pairing.response.is_none()
                        && pairing.transcript_digest.is_none()
                        && pairing.sas_digest.is_none()
                        && pairing.confirmation_digest.is_some()
                        && pairing.grant.is_some()
                        && pairing.host_ephemeral_secret.0.is_empty()
                        && pairing.invite.pairing_secret_hex.is_empty() => {}
                _ => return Err(EnrollmentError::InvalidStoredData),
            }
            if let Some(response) = &pairing.response {
                if response.account != self.account
                    || response.pairing_id != pairing.invite.pairing_id
                    || response.platform != pairing.invite.approved_platform
                    || response.capabilities != pairing.invite.approved_capabilities
                    || !valid_hex_64(&response.join_ephemeral_public_key_hex)
                    || !valid_hex_64(&response.device_public_key_hex)
                {
                    return Err(EnrollmentError::InvalidStoredData);
                }
                let transcript = transcript_from_response(&pairing.invite, response)?;
                let transcript_bytes = serde_json::to_vec(&transcript)
                    .map_err(|_| EnrollmentError::InvalidStoredData)?;
                if digest(&transcript_bytes) != response.transcript_digest
                    || pairing.transcript_digest.as_deref() != Some(&response.transcript_digest)
                {
                    return Err(EnrollmentError::InvalidStoredData);
                }
                let host_secret = SecretKey::from_hex(&pairing.host_ephemeral_secret.0)
                    .map_err(|_| EnrollmentError::InvalidStoredData)?;
                let conversation_key =
                    derive_key(&host_secret, &response.join_ephemeral_public_key_hex)?;
                if proof(
                    &conversation_key,
                    &pairing.invite.pairing_secret_hex,
                    &transcript_bytes,
                    b"client-hello",
                )? != response.client_hello_proof
                {
                    return Err(EnrollmentError::InvalidStoredData);
                }
            }
        }
        let mut device_keys = BTreeSet::new();
        let mut grant_ids = BTreeSet::new();
        for device in &self.devices {
            if !valid_hex_64(&device.device_public_key_hex)
                || !valid_hex_64(&device.grant_id)
                || !device_keys.insert(&device.device_public_key_hex)
                || !grant_ids.insert(&device.grant_id)
                || device.device_label.trim().is_empty()
                || device.enrolled_at == 0
                || device.expires_at <= device.enrolled_at
                || validate_platform_capabilities(device.platform, &device.capabilities).is_err()
            {
                return Err(EnrollmentError::InvalidStoredData);
            }
        }
        for grant in self
            .pairings
            .iter()
            .filter_map(|pairing| pairing.grant.as_ref())
        {
            if grant.account != self.account
                || !valid_hex_64(&grant.grant_id)
                || !valid_hex_64(&grant.device_public_key_hex)
                || !valid_reference(&grant.owner_authorization_ref)
                || validate_platform_capabilities(grant.platform, &grant.capabilities).is_err()
                || !self.devices.iter().any(|device| {
                    device.grant_id == grant.grant_id
                        && device.device_public_key_hex == grant.device_public_key_hex
                        && device.platform == grant.platform
                        && device.capabilities == grant.capabilities
                })
            {
                return Err(EnrollmentError::InvalidStoredData);
            }
        }
        Ok(())
    }
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct LocalDeviceCredential {
    schema: String,
    grant: EnrollmentGrant,
    device_secret_key_hex: SensitiveSecret,
    persisted_at: u64,
}

impl fmt::Debug for LocalDeviceCredential {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("LocalDeviceCredential")
            .field("schema", &self.schema)
            .field("grant", &self.grant)
            .field("device_secret_key_hex", &"[REDACTED]")
            .field("persisted_at", &self.persisted_at)
            .finish()
    }
}

impl LocalDeviceCredential {
    fn validate(&self) -> Result<(), EnrollmentError> {
        self.grant.account.validate()?;
        let secret = SecretKey::from_hex(&self.device_secret_key_hex.0)
            .map_err(|_| EnrollmentError::InvalidStoredData)?;
        if self.schema != LOCAL_CREDENTIAL_SCHEMA
            || self.persisted_at == 0
            || self.grant.issued_at == 0
            || self.grant.expires_at <= self.grant.issued_at
            || !valid_hex_64(&self.grant.grant_id)
            || !valid_reference(&self.grant.owner_authorization_ref)
            || validate_platform_capabilities(self.grant.platform, &self.grant.capabilities)
                .is_err()
            || Keys::new(secret).public_key().to_hex() != self.grant.device_public_key_hex
        {
            return Err(EnrollmentError::InvalidStoredData);
        }
        Ok(())
    }
}

pub struct DeviceEnrollmentStore {
    root: PathBuf,
}

impl DeviceEnrollmentStore {
    pub fn system() -> Self {
        Self::for_data_root(paths::data_dir())
    }

    pub fn for_data_root(data_root: impl Into<PathBuf>) -> Self {
        Self {
            root: data_root.into().join("identity").join("device-enrollment"),
        }
    }

    pub fn create_pairing_invite(
        &self,
        account: EnrollmentAccountFence,
        endpoint: impl Into<String>,
        approved_platform: DevicePlatform,
        approved_capabilities: BTreeSet<DeviceCapability>,
        owner_authorization_ref: impl Into<String>,
        now: u64,
        lifetime_seconds: u64,
    ) -> Result<PairingInvite, EnrollmentError> {
        let _guard = store_guard()?;
        account.validate()?;
        if now == 0 || lifetime_seconds == 0 || lifetime_seconds > MAX_PAIRING_LIFETIME {
            return Err(EnrollmentError::InvalidInvite);
        }
        let host_keys = Keys::generate();
        let random = Keys::generate().secret_key().to_secret_hex();
        let pairing_id = digest(
            format!(
                "{}:{}:{}:{}",
                account.owner_public_key_hex,
                account.generation,
                now,
                Keys::generate().secret_key().to_secret_hex()
            )
            .as_bytes(),
        );
        let invite = PairingInvite {
            schema: INVITE_SCHEMA.into(),
            pairing_id,
            endpoint: endpoint.into(),
            account: account.clone(),
            approved_platform,
            approved_capabilities,
            owner_authorization_ref: owner_authorization_ref.into(),
            host_ephemeral_public_key_hex: host_keys.public_key().to_hex(),
            issued_at: now,
            expires_at: now.saturating_add(lifetime_seconds),
            pairing_secret_hex: random,
        };
        invite.validate(now)?;
        let mut store = self.read_or_new_host_store(&account)?;
        store.pairings.retain(|pairing| {
            pairing.state == PairingState::Redeemed || pairing.invite.expires_at > now
        });
        store.pairings.push(DurablePairing {
            invite: invite.clone(),
            host_ephemeral_secret: SensitiveSecret(host_keys.secret_key().to_secret_hex()),
            response: None,
            transcript_digest: None,
            sas_digest: None,
            confirmation_digest: None,
            state: PairingState::Open,
            grant: None,
        });
        self.write_host_store(&store)?;
        Ok(invite)
    }

    pub fn begin_device_enrollment(
        &self,
        invite: PairingInvite,
        device_label: impl Into<String>,
        platform: DevicePlatform,
        capabilities: BTreeSet<DeviceCapability>,
        now: u64,
    ) -> Result<(PendingDeviceEnrollment, PairingResponse), EnrollmentError> {
        let _guard = store_guard()?;
        let (pending, response) =
            PendingDeviceEnrollment::begin(invite, device_label, platform, capabilities, now)?;
        let path = self.pending_device_path(&pending.invite.account, &pending.invite.pairing_id)?;
        write_private_json(
            &path,
            &DurablePendingDeviceEnrollment::from(&pending),
            &self.root,
        )?;
        let verified: DurablePendingDeviceEnrollment = read_private_json(&path, &self.root)?;
        let verified = PendingDeviceEnrollment::from(verified);
        verified.validate_stored()?;
        if verified != pending {
            return Err(EnrollmentError::Storage);
        }
        Ok((pending, response))
    }

    pub fn resume_pending_device(
        &self,
        account: &EnrollmentAccountFence,
        pairing_id: &str,
    ) -> Result<PendingDeviceEnrollment, EnrollmentError> {
        let _guard = store_guard()?;
        let path = self.pending_device_path(account, pairing_id)?;
        let durable: DurablePendingDeviceEnrollment = read_private_json(&path, &self.root)?;
        let pending = PendingDeviceEnrollment::from(durable);
        if pending.schema != PENDING_DEVICE_SCHEMA
            || pending.invite.account != *account
            || pending.invite.pairing_id != pairing_id
            || pending.response.account != *account
            || pending.response.pairing_id != pairing_id
        {
            return Err(EnrollmentError::InvalidStoredData);
        }
        pending.validate_stored()?;
        Ok(pending)
    }

    pub fn accept_pairing_response(
        &self,
        account: &EnrollmentAccountFence,
        response: PairingResponse,
        now: u64,
    ) -> Result<SasChallenge, EnrollmentError> {
        let _guard = store_guard()?;
        let mut store = self.read_host_store(account)?;
        let pairing = find_pairing_mut(&mut store, &response.pairing_id)?;
        if pairing.state == PairingState::Redeemed {
            return Err(EnrollmentError::AlreadyRedeemed);
        }
        pairing.invite.validate(now)?;
        if response.account != *account {
            return Err(EnrollmentError::WrongGeneration);
        }
        if !valid_hex_64(&response.join_ephemeral_public_key_hex)
            || !valid_hex_64(&response.device_public_key_hex)
            || PublicKey::from_hex(&response.join_ephemeral_public_key_hex).is_err()
            || PublicKey::from_hex(&response.device_public_key_hex).is_err()
        {
            return Err(EnrollmentError::TranscriptMismatch);
        }
        if response.platform != pairing.invite.approved_platform
            || response.capabilities != pairing.invite.approved_capabilities
        {
            return Err(EnrollmentError::CapabilityDenied);
        }
        validate_device_request(
            &response.device_label,
            response.platform,
            &response.capabilities,
        )?;
        let transcript = transcript_from_response(&pairing.invite, &response)?;
        let transcript_bytes =
            serde_json::to_vec(&transcript).map_err(|_| EnrollmentError::Crypto)?;
        if digest(&transcript_bytes) != response.transcript_digest {
            return Err(EnrollmentError::TranscriptMismatch);
        }
        let host_secret = SecretKey::from_hex(&pairing.host_ephemeral_secret.0)
            .map_err(|_| EnrollmentError::InvalidStoredData)?;
        let conversation_key = derive_key(&host_secret, &response.join_ephemeral_public_key_hex)?;
        let expected = proof(
            &conversation_key,
            &pairing.invite.pairing_secret_hex,
            &transcript_bytes,
            b"client-hello",
        )?;
        if expected != response.client_hello_proof {
            return Err(EnrollmentError::TranscriptMismatch);
        }
        if let Some(existing) = &pairing.response {
            if existing != &response {
                return Err(EnrollmentError::TranscriptMismatch);
            }
        }
        let short_code = sas(
            &conversation_key,
            &pairing.invite.pairing_secret_hex,
            &transcript_bytes,
        )?;
        pairing.response = Some(response.clone());
        pairing.transcript_digest = Some(response.transcript_digest.clone());
        pairing.sas_digest = Some(digest(short_code.as_bytes()));
        pairing.state = PairingState::AwaitingConfirmation;
        let challenge = SasChallenge {
            pairing_id: response.pairing_id,
            transcript_digest: response.transcript_digest,
            sas: short_code,
            host_confirmation_proof: proof(
                &conversation_key,
                &pairing.invite.pairing_secret_hex,
                &transcript_bytes,
                b"host-confirm",
            )?,
        };
        self.write_host_store(&store)?;
        Ok(challenge)
    }

    pub fn redeem_pairing(
        &self,
        account: &EnrollmentAccountFence,
        confirmation: &PairingConfirmation,
        confirmed_sas: &str,
        grant_lifetime_seconds: u64,
        now: u64,
    ) -> Result<EnrollmentGrant, EnrollmentError> {
        let _guard = store_guard()?;
        if grant_lifetime_seconds == 0 || grant_lifetime_seconds > MAX_GRANT_LIFETIME {
            return Err(EnrollmentError::InvalidGrant);
        }
        let mut store = self.read_host_store(account)?;
        let pairing_index = store
            .pairings
            .iter()
            .position(|pairing| pairing.invite.pairing_id == confirmation.pairing_id)
            .ok_or(EnrollmentError::NotFound)?;
        if confirmation.account != *account {
            return Err(EnrollmentError::WrongGeneration);
        }
        let confirmation_bytes =
            serde_json::to_vec(confirmation).map_err(|_| EnrollmentError::Crypto)?;
        let confirmation_digest = digest(&confirmation_bytes);
        {
            let existing = store
                .pairings
                .get(pairing_index)
                .ok_or(EnrollmentError::InvalidStoredData)?;
            if existing.state == PairingState::Redeemed {
                if existing.confirmation_digest.as_deref() == Some(&confirmation_digest) {
                    return existing
                        .grant
                        .clone()
                        .ok_or(EnrollmentError::InvalidStoredData);
                }
                return Err(EnrollmentError::AlreadyRedeemed);
            }
        }
        if store
            .devices
            .iter()
            .any(|device| device.device_public_key_hex == confirmation.device_public_key_hex)
        {
            return Err(EnrollmentError::InvalidGrant);
        }
        let pairing = store
            .pairings
            .get_mut(pairing_index)
            .ok_or(EnrollmentError::InvalidStoredData)?;
        pairing.invite.validate(now)?;
        if pairing.state != PairingState::AwaitingConfirmation {
            return Err(EnrollmentError::InvalidPairingState);
        }
        let response = pairing
            .response
            .clone()
            .ok_or(EnrollmentError::InvalidStoredData)?;
        if response.device_public_key_hex != confirmation.device_public_key_hex
            || response.transcript_digest != confirmation.transcript_digest
            || pairing.sas_digest.as_deref() != Some(&digest(confirmed_sas.as_bytes()))
        {
            return Err(EnrollmentError::WrongSas);
        }
        let transcript = transcript_from_response(&pairing.invite, &response)?;
        let transcript_bytes =
            serde_json::to_vec(&transcript).map_err(|_| EnrollmentError::Crypto)?;
        let host_secret = SecretKey::from_hex(&pairing.host_ephemeral_secret.0)
            .map_err(|_| EnrollmentError::InvalidStoredData)?;
        let conversation_key = derive_key(&host_secret, &response.join_ephemeral_public_key_hex)?;
        let expected = proof(
            &conversation_key,
            &pairing.invite.pairing_secret_hex,
            &transcript_bytes,
            b"client-confirm",
        )?;
        if expected != confirmation.client_confirmation_proof {
            return Err(EnrollmentError::TranscriptMismatch);
        }
        let grant = EnrollmentGrant {
            grant_id: digest(
                format!(
                    "{}:{}:{}:{}",
                    confirmation.transcript_digest,
                    confirmation.device_public_key_hex,
                    account.generation,
                    now
                )
                .as_bytes(),
            ),
            account: account.clone(),
            device_public_key_hex: response.device_public_key_hex.clone(),
            platform: response.platform,
            capabilities: response.capabilities.clone(),
            owner_authorization_ref: pairing.invite.owner_authorization_ref.clone(),
            issued_at: now,
            expires_at: now.saturating_add(grant_lifetime_seconds),
        };
        let device_label = response.device_label;
        pairing.state = PairingState::Redeemed;
        pairing.confirmation_digest = Some(confirmation_digest);
        pairing.grant = Some(grant.clone());
        pairing.response = None;
        pairing.transcript_digest = None;
        pairing.sas_digest = None;
        pairing.host_ephemeral_secret.0.clear();
        pairing.invite.pairing_secret_hex.clear();
        store.devices.push(DeviceInventoryEntry {
            device_public_key_hex: grant.device_public_key_hex.clone(),
            device_label,
            platform: grant.platform,
            capabilities: grant.capabilities.clone(),
            grant_id: grant.grant_id.clone(),
            enrolled_at: now,
            expires_at: grant.expires_at,
            last_used_at: None,
            revoked_at: None,
        });
        self.write_host_store(&store)?;
        Ok(grant)
    }

    pub fn recover_redeemed_grant(
        &self,
        account: &EnrollmentAccountFence,
        pairing_id: &str,
        device_public_key_hex: &str,
    ) -> Result<EnrollmentGrant, EnrollmentError> {
        let _guard = store_guard()?;
        let store = self.read_host_store(account)?;
        let pairing = store
            .pairings
            .iter()
            .find(|pairing| pairing.invite.pairing_id == pairing_id)
            .ok_or(EnrollmentError::NotFound)?;
        let grant = pairing
            .grant
            .clone()
            .filter(|grant| grant.device_public_key_hex == device_public_key_hex)
            .ok_or(EnrollmentError::InvalidGrant)?;
        Ok(grant)
    }

    pub fn persist_local_device(
        &self,
        pending: &PendingDeviceEnrollment,
        grant: &EnrollmentGrant,
        now: u64,
    ) -> Result<(), EnrollmentError> {
        let _guard = store_guard()?;
        if grant.account != pending.invite.account
            || grant.device_public_key_hex != pending.response.device_public_key_hex
            || grant.platform != pending.response.platform
            || grant.capabilities != pending.response.capabilities
            || now >= grant.expires_at
        {
            return Err(EnrollmentError::InvalidGrant);
        }
        let secret = SecretKey::from_hex(&pending.device_secret_key_hex)
            .map_err(|_| EnrollmentError::Crypto)?;
        if Keys::new(secret).public_key().to_hex() != grant.device_public_key_hex {
            return Err(EnrollmentError::InvalidGrant);
        }
        let credential = LocalDeviceCredential {
            schema: LOCAL_CREDENTIAL_SCHEMA.into(),
            grant: grant.clone(),
            device_secret_key_hex: SensitiveSecret(pending.device_secret_key_hex.clone()),
            persisted_at: now,
        };
        let path = self.local_device_path(&grant.account, &grant.device_public_key_hex)?;
        write_private_json(&path, &credential, &self.root)?;
        let verified: LocalDeviceCredential = read_private_json(&path, &self.root)?;
        verified.validate()?;
        if verified != credential {
            return Err(EnrollmentError::Storage);
        }
        let pending_path = self.pending_device_path(&grant.account, &pending.invite.pairing_id)?;
        remove_private_file(&pending_path, &self.root)?;
        Ok(())
    }

    pub fn finalize_redeemed_device(
        &self,
        account: &EnrollmentAccountFence,
        pairing_id: &str,
        grant: &EnrollmentGrant,
        now: u64,
    ) -> Result<(), EnrollmentError> {
        let pending = self.resume_pending_device(account, pairing_id)?;
        self.persist_local_device(&pending, grant, now)
    }

    pub fn local_device_grant(
        &self,
        account: &EnrollmentAccountFence,
        device_public_key_hex: &str,
    ) -> Result<EnrollmentGrant, EnrollmentError> {
        let _guard = store_guard()?;
        let path = self.local_device_path(account, device_public_key_hex)?;
        let credential: LocalDeviceCredential = read_private_json(&path, &self.root)?;
        credential.validate()?;
        if credential.grant.account != *account
            || credential.grant.device_public_key_hex != device_public_key_hex
        {
            return Err(EnrollmentError::InvalidStoredData);
        }
        Ok(credential.grant)
    }

    pub fn remove_local_device(
        &self,
        account: &EnrollmentAccountFence,
        device_public_key_hex: &str,
    ) -> Result<(), EnrollmentError> {
        let _guard = store_guard()?;
        let path = self.local_device_path(account, device_public_key_hex)?;
        let credential: LocalDeviceCredential = read_private_json(&path, &self.root)?;
        credential.validate()?;
        if credential.grant.account != *account
            || credential.grant.device_public_key_hex != device_public_key_hex
        {
            return Err(EnrollmentError::InvalidStoredData);
        }
        remove_private_file(&path, &self.root)
    }

    pub fn authorize(
        &self,
        account: &EnrollmentAccountFence,
        grant_id: &str,
        device_public_key_hex: &str,
        capability: DeviceCapability,
        now: u64,
    ) -> Result<AuthorizedDevice, EnrollmentError> {
        let _guard = store_guard()?;
        let store = self.read_host_store(account)?;
        let device = store
            .devices
            .iter()
            .find(|device| {
                device.grant_id == grant_id && device.device_public_key_hex == device_public_key_hex
            })
            .ok_or(EnrollmentError::InvalidGrant)?;
        if device.revoked_at.is_some() {
            return Err(EnrollmentError::Revoked);
        }
        if now >= device.expires_at {
            return Err(EnrollmentError::InvalidGrant);
        }
        if !device.capabilities.contains(&capability) {
            return Err(EnrollmentError::CapabilityDenied);
        }
        Ok(AuthorizedDevice {
            account: account.clone(),
            device_public_key_hex: device.device_public_key_hex.clone(),
            capability,
            grant_id: device.grant_id.clone(),
        })
    }

    pub fn record_last_use(
        &self,
        authorization: &AuthorizedDevice,
        now: u64,
    ) -> Result<(), EnrollmentError> {
        let _guard = store_guard()?;
        let mut store = self.read_host_store(&authorization.account)?;
        let device = store
            .devices
            .iter_mut()
            .find(|device| {
                device.grant_id == authorization.grant_id
                    && device.device_public_key_hex == authorization.device_public_key_hex
            })
            .ok_or(EnrollmentError::InvalidGrant)?;
        if device.revoked_at.is_some() {
            return Err(EnrollmentError::Revoked);
        }
        if now >= device.expires_at || !device.capabilities.contains(&authorization.capability) {
            return Err(EnrollmentError::InvalidGrant);
        }
        device.last_used_at = Some(now);
        self.write_host_store(&store)
    }

    pub fn revoke_device(
        &self,
        account: &EnrollmentAccountFence,
        device_public_key_hex: &str,
        now: u64,
    ) -> Result<(), EnrollmentError> {
        let _guard = store_guard()?;
        let mut store = self.read_host_store(account)?;
        let device = store
            .devices
            .iter_mut()
            .find(|device| device.device_public_key_hex == device_public_key_hex)
            .ok_or(EnrollmentError::NotFound)?;
        if device.revoked_at.is_none() {
            device.revoked_at = Some(now);
            self.write_host_store(&store)?;
        }
        Ok(())
    }

    pub fn device_inventory(
        &self,
        account: &EnrollmentAccountFence,
    ) -> Result<Vec<DeviceInventoryEntry>, EnrollmentError> {
        let _guard = store_guard()?;
        Ok(self.read_host_store(account)?.devices)
    }

    pub fn pairing_lifecycle(
        &self,
        account: &EnrollmentAccountFence,
        pairing_id: &str,
        now: u64,
    ) -> Result<PairingLifecycleProjection, EnrollmentError> {
        let _guard = store_guard()?;
        let store = self.read_host_store(account)?;
        let pairing = store
            .pairings
            .iter()
            .find(|pairing| pairing.invite.pairing_id == pairing_id)
            .ok_or(EnrollmentError::NotFound)?;
        let lifecycle =
            if pairing.state != PairingState::Redeemed && now >= pairing.invite.expires_at {
                PairingLifecycle::Expired
            } else {
                match pairing.state {
                    PairingState::Open => PairingLifecycle::Open,
                    PairingState::AwaitingConfirmation => PairingLifecycle::AwaitingConfirmation,
                    PairingState::Redeemed => PairingLifecycle::Redeemed,
                }
            };
        let redeemed_grant = pairing.grant.as_ref();
        let inventory = redeemed_grant.and_then(|grant| {
            store
                .devices
                .iter()
                .find(|device| device.grant_id == grant.grant_id)
        });
        Ok(PairingLifecycleProjection {
            pairing_id: pairing.invite.pairing_id.clone(),
            endpoint: pairing.invite.endpoint.clone(),
            account: pairing.invite.account.clone(),
            issued_at: pairing.invite.issued_at,
            expires_at: pairing.invite.expires_at,
            lifecycle,
            device_public_key_hex: pairing
                .response
                .as_ref()
                .map(|response| response.device_public_key_hex.clone())
                .or_else(|| redeemed_grant.map(|grant| grant.device_public_key_hex.clone())),
            device_label: pairing
                .response
                .as_ref()
                .map(|response| response.device_label.clone())
                .or_else(|| inventory.map(|device| device.device_label.clone())),
            platform: pairing
                .response
                .as_ref()
                .map(|response| response.platform)
                .or_else(|| redeemed_grant.map(|grant| grant.platform)),
        })
    }

    pub fn purge_expired_pairings(
        &self,
        account: &EnrollmentAccountFence,
        now: u64,
    ) -> Result<usize, EnrollmentError> {
        let _guard = store_guard()?;
        let mut store = self.read_host_store(account)?;
        let original_len = store.pairings.len();
        let expired_pairing_ids = store
            .pairings
            .iter()
            .filter(|pairing| {
                pairing.state != PairingState::Redeemed && pairing.invite.expires_at <= now
            })
            .map(|pairing| pairing.invite.pairing_id.clone())
            .collect::<Vec<_>>();
        store.pairings.retain(|pairing| {
            pairing.state == PairingState::Redeemed || pairing.invite.expires_at > now
        });
        let removed = original_len.saturating_sub(store.pairings.len());
        if removed > 0 {
            self.write_host_store(&store)?;
            let verified = self.read_host_store(account)?;
            if verified.pairings.iter().any(|pairing| {
                pairing.state != PairingState::Redeemed && pairing.invite.expires_at <= now
            }) {
                return Err(EnrollmentError::Storage);
            }
            for pairing_id in expired_pairing_ids {
                let path = self.pending_device_path(account, &pairing_id)?;
                if path_exists_regular(&path)?.is_some() {
                    remove_private_file(&path, &self.root)?;
                }
            }
        }
        Ok(removed)
    }

    fn read_or_new_host_store(
        &self,
        account: &EnrollmentAccountFence,
    ) -> Result<DurableHostStore, EnrollmentError> {
        let path = self.host_store_path(account)?;
        if path_exists_regular(&path)?.is_none() {
            return Ok(DurableHostStore {
                schema: STORE_SCHEMA.into(),
                account: account.clone(),
                pairings: Vec::new(),
                devices: Vec::new(),
            });
        }
        self.read_host_store(account)
    }

    fn read_host_store(
        &self,
        account: &EnrollmentAccountFence,
    ) -> Result<DurableHostStore, EnrollmentError> {
        let path = self.host_store_path(account)?;
        let store: DurableHostStore = read_private_json(&path, &self.root)?;
        store.validate()?;
        if store.account != *account {
            return Err(EnrollmentError::WrongGeneration);
        }
        Ok(store)
    }

    fn write_host_store(&self, store: &DurableHostStore) -> Result<(), EnrollmentError> {
        store.validate()?;
        let path = self.host_store_path(&store.account)?;
        write_private_json(&path, store, &self.root)?;
        let verified: DurableHostStore = read_private_json(&path, &self.root)?;
        verified.validate()?;
        if &verified != store {
            return Err(EnrollmentError::Storage);
        }
        Ok(())
    }

    fn host_store_path(
        &self,
        account: &EnrollmentAccountFence,
    ) -> Result<PathBuf, EnrollmentError> {
        account.validate()?;
        Ok(self
            .root
            .join("host")
            .join(&account.owner_public_key_hex)
            .join("enrollment.json"))
    }

    fn local_device_path(
        &self,
        account: &EnrollmentAccountFence,
        device_public_key_hex: &str,
    ) -> Result<PathBuf, EnrollmentError> {
        account.validate()?;
        if !valid_hex_64(device_public_key_hex) {
            return Err(EnrollmentError::InvalidGrant);
        }
        Ok(self
            .root
            .join("local")
            .join(&account.owner_public_key_hex)
            .join("devices")
            .join(format!("{device_public_key_hex}.json")))
    }

    fn pending_device_path(
        &self,
        account: &EnrollmentAccountFence,
        pairing_id: &str,
    ) -> Result<PathBuf, EnrollmentError> {
        account.validate()?;
        if !valid_hex_64(pairing_id) {
            return Err(EnrollmentError::InvalidInvite);
        }
        Ok(self
            .root
            .join("local")
            .join(&account.owner_public_key_hex)
            .join("pending")
            .join(format!("{pairing_id}.json")))
    }
}

fn find_pairing_mut<'a>(
    store: &'a mut DurableHostStore,
    pairing_id: &str,
) -> Result<&'a mut DurablePairing, EnrollmentError> {
    store
        .pairings
        .iter_mut()
        .find(|pairing| pairing.invite.pairing_id == pairing_id)
        .ok_or(EnrollmentError::NotFound)
}

fn transcript(
    invite: &PairingInvite,
    join_ephemeral_public_key_hex: String,
    device_public_key_hex: String,
    device_label: String,
    platform: DevicePlatform,
    capabilities: BTreeSet<DeviceCapability>,
) -> PairingTranscript {
    PairingTranscript {
        schema: TRANSCRIPT_SCHEMA.into(),
        pairing_id: invite.pairing_id.clone(),
        endpoint: invite.endpoint.clone(),
        account: invite.account.clone(),
        host_ephemeral_public_key_hex: invite.host_ephemeral_public_key_hex.clone(),
        join_ephemeral_public_key_hex,
        device_public_key_hex,
        device_label,
        platform,
        capabilities,
        owner_authorization_ref: invite.owner_authorization_ref.clone(),
        invite_expires_at: invite.expires_at,
    }
}

fn transcript_from_response(
    invite: &PairingInvite,
    response: &PairingResponse,
) -> Result<PairingTranscript, EnrollmentError> {
    if response.pairing_id != invite.pairing_id || response.account != invite.account {
        return Err(EnrollmentError::TranscriptMismatch);
    }
    Ok(transcript(
        invite,
        response.join_ephemeral_public_key_hex.clone(),
        response.device_public_key_hex.clone(),
        response.device_label.clone(),
        response.platform,
        response.capabilities.clone(),
    ))
}

fn validate_device_request(
    device_label: &str,
    platform: DevicePlatform,
    capabilities: &BTreeSet<DeviceCapability>,
) -> Result<(), EnrollmentError> {
    if device_label.trim().is_empty()
        || device_label.len() > 128
        || capabilities.is_empty()
        || capabilities.len() > 4
    {
        return Err(EnrollmentError::InvalidInvite);
    }
    validate_platform_capabilities(platform, capabilities)
}

fn validate_platform_capabilities(
    platform: DevicePlatform,
    capabilities: &BTreeSet<DeviceCapability>,
) -> Result<(), EnrollmentError> {
    if capabilities.is_empty() || capabilities.len() > 4 {
        return Err(EnrollmentError::CapabilityDenied);
    }
    let admitted = match platform {
        DevicePlatform::Desktop => capabilities.iter().all(|capability| {
            matches!(
                capability,
                DeviceCapability::DesktopLocal | DeviceCapability::Nip46
            )
        }),
        DevicePlatform::Web => capabilities.iter().all(|capability| {
            matches!(
                capability,
                DeviceCapability::Nip07 | DeviceCapability::Nip46
            )
        }),
        DevicePlatform::Android => capabilities == &BTreeSet::from([DeviceCapability::Nip55]),
        DevicePlatform::Ios => capabilities == &BTreeSet::from([DeviceCapability::Nip46]),
    };
    if !admitted {
        return Err(EnrollmentError::CapabilityDenied);
    }
    Ok(())
}

fn derive_key(
    local_secret: &SecretKey,
    remote_public_key_hex: &str,
) -> Result<ConversationKey, EnrollmentError> {
    let remote = PublicKey::from_hex(remote_public_key_hex).map_err(|_| EnrollmentError::Crypto)?;
    ConversationKey::derive(local_secret, &remote).map_err(|_| EnrollmentError::Crypto)
}

fn proof(
    conversation_key: &ConversationKey,
    pairing_secret_hex: &str,
    transcript: &[u8],
    purpose: &[u8],
) -> Result<String, EnrollmentError> {
    let key = authentication_key(conversation_key, pairing_secret_hex, transcript, b"proof")?;
    let mut authentication =
        Hmac::<Sha256>::new_from_slice(&key).map_err(|_| EnrollmentError::Crypto)?;
    authentication.update(b"openagents.omega.device-enrollment.proof.v1");
    authentication.update(&(purpose.len() as u64).to_be_bytes());
    authentication.update(purpose);
    authentication.update(&(transcript.len() as u64).to_be_bytes());
    authentication.update(transcript);
    Ok(hex::encode(authentication.finalize().into_bytes()))
}

fn sas(
    conversation_key: &ConversationKey,
    pairing_secret_hex: &str,
    transcript: &[u8],
) -> Result<String, EnrollmentError> {
    let key = authentication_key(conversation_key, pairing_secret_hex, transcript, b"sas")?;
    let mut authentication =
        Hmac::<Sha256>::new_from_slice(&key).map_err(|_| EnrollmentError::Crypto)?;
    authentication.update(b"openagents.omega.device-enrollment.sas.v1");
    authentication.update(&(transcript.len() as u64).to_be_bytes());
    authentication.update(transcript);
    let bytes = authentication.finalize().into_bytes();
    let value = bytes.iter().take(4).fold(0_u32, |value, byte| {
        value.wrapping_shl(8) | u32::from(*byte)
    }) % 1_000_000;
    Ok(format!("{value:06}"))
}

fn authentication_key(
    conversation_key: &ConversationKey,
    pairing_secret_hex: &str,
    transcript: &[u8],
    purpose: &[u8],
) -> Result<[u8; 32], EnrollmentError> {
    let pairing_secret = hex::decode(pairing_secret_hex).map_err(|_| EnrollmentError::Crypto)?;
    let transcript_digest = Sha256::digest(transcript);
    let derivation = Hkdf::<Sha256>::new(Some(&pairing_secret), conversation_key.as_bytes());
    let mut information = Vec::with_capacity(64 + purpose.len());
    information.extend_from_slice(b"openagents.omega.device-enrollment.auth-key.v1");
    information.extend_from_slice(&(purpose.len() as u64).to_be_bytes());
    information.extend_from_slice(purpose);
    information.extend_from_slice(&transcript_digest);
    let mut key = [0_u8; 32];
    derivation
        .expand(&information, &mut key)
        .map_err(|_| EnrollmentError::Crypto)?;
    Ok(key)
}

fn digest(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}

fn store_guard() -> Result<MutexGuard<'static, ()>, EnrollmentError> {
    static STORE_MUTEX: OnceLock<Mutex<()>> = OnceLock::new();
    STORE_MUTEX
        .get_or_init(|| Mutex::new(()))
        .lock()
        .map_err(|_| EnrollmentError::Storage)
}

fn write_private_json<T: Serialize>(
    path: &Path,
    value: &T,
    root: &Path,
) -> Result<(), EnrollmentError> {
    let bytes = serde_json::to_vec(value).map_err(|_| EnrollmentError::Storage)?;
    if bytes.len() as u64 > MAX_FILE_BYTES {
        return Err(EnrollmentError::Storage);
    }
    let parent = path.parent().ok_or(EnrollmentError::Storage)?;
    create_private_directory(parent, root)?;
    if let Some(metadata) = path_exists_regular(path)? {
        verify_private_file_mode(&metadata)?;
    }
    let mut file = AtomicWriteFile::open(path).map_err(|_| EnrollmentError::Storage)?;
    file.write_all(&bytes)
        .map_err(|_| EnrollmentError::Storage)?;
    file.commit().map_err(|_| EnrollmentError::Storage)?;
    set_private_file_mode(path)?;
    let persisted: serde_json::Value = read_private_json(path, root)?;
    let expected: serde_json::Value =
        serde_json::from_slice(&bytes).map_err(|_| EnrollmentError::Storage)?;
    if persisted != expected {
        return Err(EnrollmentError::Storage);
    }
    Ok(())
}

fn read_private_json<T: for<'de> Deserialize<'de>>(
    path: &Path,
    root: &Path,
) -> Result<T, EnrollmentError> {
    verify_private_ancestors(path, root)?;
    let metadata = path_exists_regular(path)?.ok_or(EnrollmentError::NotFound)?;
    verify_private_file_mode(&metadata)?;
    if metadata.len() > MAX_FILE_BYTES {
        return Err(EnrollmentError::InvalidStoredData);
    }
    let file = fs::File::open(path).map_err(|_| EnrollmentError::Storage)?;
    let mut bytes = Vec::new();
    file.take(MAX_FILE_BYTES.saturating_add(1))
        .read_to_end(&mut bytes)
        .map_err(|_| EnrollmentError::Storage)?;
    if bytes.len() as u64 > MAX_FILE_BYTES {
        return Err(EnrollmentError::InvalidStoredData);
    }
    serde_json::from_slice(&bytes).map_err(|_| EnrollmentError::InvalidStoredData)
}

fn remove_private_file(path: &Path, root: &Path) -> Result<(), EnrollmentError> {
    verify_private_ancestors(path, root)?;
    let metadata = path_exists_regular(path)?.ok_or(EnrollmentError::NotFound)?;
    verify_private_file_mode(&metadata)?;
    fs::remove_file(path).map_err(|_| EnrollmentError::Storage)?;
    if path.try_exists().map_err(|_| EnrollmentError::Storage)? {
        return Err(EnrollmentError::Storage);
    }
    Ok(())
}

fn create_private_directory(path: &Path, root: &Path) -> Result<(), EnrollmentError> {
    let mut missing = Vec::new();
    let mut cursor = path;
    while !cursor.try_exists().map_err(|_| EnrollmentError::Storage)? {
        missing.push(cursor.to_path_buf());
        cursor = cursor.parent().ok_or(EnrollmentError::Storage)?;
    }
    if cursor.starts_with(root) {
        verify_private_ancestors(&cursor.join("placeholder"), root)?;
    }
    for directory in missing.into_iter().rev() {
        fs::create_dir(&directory).map_err(|_| EnrollmentError::Storage)?;
        set_private_directory_mode(&directory)?;
    }
    verify_private_ancestors(&path.join("placeholder"), root)
}

fn verify_private_ancestors(path: &Path, root: &Path) -> Result<(), EnrollmentError> {
    let mut cursor = path.parent().ok_or(EnrollmentError::Storage)?;
    loop {
        if !cursor.starts_with(root) {
            break;
        }
        let metadata = fs::symlink_metadata(cursor).map_err(|_| EnrollmentError::Storage)?;
        if !metadata.is_dir() || metadata.file_type().is_symlink() {
            return Err(EnrollmentError::Storage);
        }
        verify_private_directory_mode(&metadata)?;
        if cursor == root {
            break;
        }
        cursor = cursor.parent().ok_or(EnrollmentError::Storage)?;
    }
    Ok(())
}

#[cfg(unix)]
fn set_private_directory_mode(path: &Path) -> Result<(), EnrollmentError> {
    use std::os::unix::fs::PermissionsExt as _;
    fs::set_permissions(path, fs::Permissions::from_mode(0o700))
        .map_err(|_| EnrollmentError::Storage)
}

#[cfg(not(unix))]
fn set_private_directory_mode(_path: &Path) -> Result<(), EnrollmentError> {
    Ok(())
}

#[cfg(unix)]
fn set_private_file_mode(path: &Path) -> Result<(), EnrollmentError> {
    use std::os::unix::fs::PermissionsExt as _;
    fs::set_permissions(path, fs::Permissions::from_mode(0o600))
        .map_err(|_| EnrollmentError::Storage)
}

#[cfg(not(unix))]
fn set_private_file_mode(_path: &Path) -> Result<(), EnrollmentError> {
    Ok(())
}

#[cfg(unix)]
fn verify_private_directory_mode(metadata: &fs::Metadata) -> Result<(), EnrollmentError> {
    use std::os::unix::fs::PermissionsExt as _;
    if metadata.permissions().mode() & 0o077 != 0 {
        return Err(EnrollmentError::Storage);
    }
    Ok(())
}

#[cfg(not(unix))]
fn verify_private_directory_mode(_metadata: &fs::Metadata) -> Result<(), EnrollmentError> {
    Ok(())
}

#[cfg(unix)]
fn verify_private_file_mode(metadata: &fs::Metadata) -> Result<(), EnrollmentError> {
    use std::os::unix::fs::PermissionsExt as _;
    if metadata.permissions().mode() & 0o177 != 0 {
        return Err(EnrollmentError::Storage);
    }
    Ok(())
}

#[cfg(not(unix))]
fn verify_private_file_mode(_metadata: &fs::Metadata) -> Result<(), EnrollmentError> {
    Ok(())
}

fn path_exists_regular(path: &Path) -> Result<Option<fs::Metadata>, EnrollmentError> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.is_file() && !metadata.file_type().is_symlink() => {
            Ok(Some(metadata))
        }
        Ok(_) => Err(EnrollmentError::Storage),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(_) => Err(EnrollmentError::Storage),
    }
}

fn valid_reference(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'-' | b'_'))
}

fn valid_hex_64(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn account(generation: u64) -> EnrollmentAccountFence {
        EnrollmentAccountFence::new(
            "primary",
            "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            generation,
        )
        .expect("account fence")
    }

    fn capabilities() -> BTreeSet<DeviceCapability> {
        [DeviceCapability::DesktopLocal, DeviceCapability::Nip46]
            .into_iter()
            .collect()
    }

    fn create_invite(
        store: &DeviceEnrollmentStore,
        now: u64,
        lifetime_seconds: u64,
    ) -> PairingInvite {
        store
            .create_pairing_invite(
                account(1),
                "wss://desktop.example/pair",
                DevicePlatform::Desktop,
                capabilities(),
                "owner-gesture.auth-08",
                now,
                lifetime_seconds,
            )
            .expect("pairing invite")
    }

    struct PairingFixture {
        store: DeviceEnrollmentStore,
        pending: PendingDeviceEnrollment,
        response: PairingResponse,
        challenge: SasChallenge,
        confirmation: PairingConfirmation,
    }

    fn pairing_fixture(directory: &tempfile::TempDir) -> PairingFixture {
        let store = DeviceEnrollmentStore::for_data_root(directory.path());
        let invite = create_invite(&store, 100, 300);
        let (pending, response) = store
            .begin_device_enrollment(
                invite,
                "Phone",
                DevicePlatform::Desktop,
                capabilities(),
                101,
            )
            .expect("begin enrollment");
        let challenge = store
            .accept_pairing_response(&account(1), response.clone(), 102)
            .expect("accept response");
        let confirmation = pending
            .confirm(&challenge, pending.sas())
            .expect("confirm challenge");
        PairingFixture {
            store,
            pending,
            response,
            challenge,
            confirmation,
        }
    }

    #[test]
    fn two_screen_sas_must_match() {
        let directory = tempfile::tempdir().expect("temporary directory");
        let fixture = pairing_fixture(&directory);
        assert_eq!(fixture.pending.sas(), fixture.challenge.sas);
        let wrong_sas = if fixture.pending.sas() == "000000" {
            "000001"
        } else {
            "000000"
        };
        assert!(matches!(
            fixture.pending.confirm(&fixture.challenge, wrong_sas),
            Err(EnrollmentError::WrongSas)
        ));
        assert!(fixture.store.device_inventory(&account(1)).is_ok());
    }

    #[test]
    fn leaked_or_expired_qr_cannot_be_used() {
        let directory = tempfile::tempdir().expect("temporary directory");
        let store = DeviceEnrollmentStore::for_data_root(directory.path());
        let invite = create_invite(&store, 100, 10);
        let wire = invite.wire_json().expect("wire invite");
        assert!(matches!(
            PairingInvite::parse_wire_json(&wire, 110),
            Err(EnrollmentError::ExpiredInvite)
        ));
        assert!(matches!(
            store.begin_device_enrollment(
                invite,
                "Leaked phone",
                DevicePlatform::Desktop,
                capabilities(),
                110,
            ),
            Err(EnrollmentError::ExpiredInvite)
        ));
    }

    #[test]
    fn expiry_purge_verified_deletes_host_and_pending_secrets() {
        let directory = tempfile::tempdir().expect("temporary directory");
        let store = DeviceEnrollmentStore::for_data_root(directory.path());
        let invite = create_invite(&store, 100, 10);
        let pairing_id = invite.pairing_id.clone();
        store
            .begin_device_enrollment(
                invite,
                "Phone",
                DevicePlatform::Desktop,
                capabilities(),
                101,
            )
            .expect("stage pending device");
        assert_eq!(
            store
                .purge_expired_pairings(&account(1), 110)
                .expect("purge expired pairing"),
            1
        );
        assert!(matches!(
            store.pairing_lifecycle(&account(1), &pairing_id, 110),
            Err(EnrollmentError::NotFound)
        ));
        assert!(matches!(
            store.resume_pending_device(&account(1), &pairing_id),
            Err(EnrollmentError::NotFound)
        ));
    }

    #[test]
    fn peer_substitution_changes_the_bound_transcript() {
        let directory = tempfile::tempdir().expect("temporary directory");
        let store = DeviceEnrollmentStore::for_data_root(directory.path());
        let invite = create_invite(&store, 100, 300);
        let (_pending, mut response) = store
            .begin_device_enrollment(
                invite,
                "Phone",
                DevicePlatform::Desktop,
                capabilities(),
                101,
            )
            .expect("begin enrollment");
        response.device_public_key_hex = Keys::generate().public_key().to_hex();
        assert!(matches!(
            store.accept_pairing_response(&account(1), response, 102),
            Err(EnrollmentError::TranscriptMismatch)
        ));
    }

    #[test]
    fn wrong_account_generation_is_rejected() {
        let directory = tempfile::tempdir().expect("temporary directory");
        let fixture = pairing_fixture(&directory);
        assert!(matches!(
            fixture.store.redeem_pairing(
                &account(2),
                &fixture.confirmation,
                fixture.pending.sas(),
                600,
                103,
            ),
            Err(EnrollmentError::WrongGeneration)
        ));
    }

    #[test]
    fn redemption_is_one_time_and_exact_retry_is_crash_safe() {
        let directory = tempfile::tempdir().expect("temporary directory");
        let fixture = pairing_fixture(&directory);
        let restarted_before_redemption = DeviceEnrollmentStore::for_data_root(directory.path());
        let resumed = restarted_before_redemption
            .resume_pending_device(&account(1), &fixture.response.pairing_id)
            .expect("resume pending device");
        assert_eq!(resumed.response, fixture.response);
        assert_eq!(
            restarted_before_redemption
                .accept_pairing_response(&account(1), fixture.response.clone(), 102)
                .expect("repeat exact response"),
            fixture.challenge
        );
        let resumed_confirmation = resumed
            .confirm(&fixture.challenge, resumed.sas())
            .expect("confirm after restart");
        let grant = restarted_before_redemption
            .redeem_pairing(&account(1), &resumed_confirmation, resumed.sas(), 600, 103)
            .expect("redeem");

        let restarted = DeviceEnrollmentStore::for_data_root(directory.path());
        let retried = restarted
            .redeem_pairing(&account(1), &resumed_confirmation, resumed.sas(), 600, 104)
            .expect("recover redemption after return-path crash");
        assert_eq!(retried, grant);
        assert_eq!(
            restarted
                .recover_redeemed_grant(
                    &account(1),
                    &fixture.response.pairing_id,
                    resumed.device_public_key_hex(),
                )
                .expect("recover returned grant"),
            grant
        );
        assert!(matches!(
            restarted.accept_pairing_response(&account(1), fixture.response.clone(), 104),
            Err(EnrollmentError::AlreadyRedeemed)
        ));

        let mut changed = resumed_confirmation;
        changed.client_confirmation_proof = digest(b"different-confirmation");
        assert!(matches!(
            restarted.redeem_pairing(&account(1), &changed, resumed.sas(), 600, 104,),
            Err(EnrollmentError::AlreadyRedeemed)
        ));
        let host_store = restarted
            .read_host_store(&account(1))
            .expect("read host store");
        let redeemed = host_store
            .pairings
            .iter()
            .find(|pairing| pairing.invite.pairing_id == fixture.response.pairing_id)
            .expect("redeemed pairing");
        assert!(redeemed.host_ephemeral_secret.0.is_empty());
        assert!(redeemed.invite.pairing_secret_hex.is_empty());
        assert!(redeemed.response.is_none());
        assert!(redeemed.sas_digest.is_none());
        restarted
            .finalize_redeemed_device(&account(1), &fixture.response.pairing_id, &grant, 105)
            .expect("finalize recovered device");
        assert!(matches!(
            restarted.resume_pending_device(&account(1), &fixture.response.pairing_id),
            Err(EnrollmentError::NotFound)
        ));
        assert_eq!(
            restarted
                .local_device_grant(&account(1), &grant.device_public_key_hex)
                .expect("recovered local grant"),
            grant
        );
    }

    #[test]
    fn local_device_key_is_permanent_and_root_secret_is_never_persisted() {
        let directory = tempfile::tempdir().expect("temporary directory");
        let fixture = pairing_fixture(&directory);
        let grant = fixture
            .store
            .redeem_pairing(
                &account(1),
                &fixture.confirmation,
                fixture.pending.sas(),
                600,
                103,
            )
            .expect("redeem");
        fixture
            .store
            .persist_local_device(&fixture.pending, &grant, 104)
            .expect("persist local device");
        let credential_path = directory
            .path()
            .join("identity/device-enrollment/local")
            .join(&account(1).owner_public_key_hex)
            .join("devices")
            .join(format!("{}.json", grant.device_public_key_hex));
        let bytes = fs::read(credential_path).expect("credential file");
        let text = String::from_utf8(bytes).expect("credential UTF-8");
        assert!(text.contains(&fixture.pending.device_secret_key_hex));
        assert!(!text.contains("nsec1"));
        assert!(!format!("{:?}", fixture.pending).contains(&fixture.pending.device_secret_key_hex));
        let pending_path = directory
            .path()
            .join("identity/device-enrollment/local")
            .join(&account(1).owner_public_key_hex)
            .join("pending")
            .join(format!("{}.json", fixture.response.pairing_id));
        assert!(!pending_path.exists());
        assert_eq!(
            fixture
                .store
                .local_device_grant(&account(1), &grant.device_public_key_hex)
                .expect("read local grant"),
            grant
        );
        fixture
            .store
            .remove_local_device(&account(1), &grant.device_public_key_hex)
            .expect("forget local device");
        assert!(matches!(
            fixture
                .store
                .local_device_grant(&account(1), &grant.device_public_key_hex),
            Err(EnrollmentError::NotFound)
        ));
    }

    #[test]
    fn platform_capabilities_are_admitted_by_core() {
        let directory = tempfile::tempdir().expect("temporary directory");
        let store = DeviceEnrollmentStore::for_data_root(directory.path());
        let invite = create_invite(&store, 100, 300);
        assert!(matches!(
            store.begin_device_enrollment(
                invite,
                "Android",
                DevicePlatform::Android,
                BTreeSet::from([DeviceCapability::Nip46]),
                101,
            ),
            Err(EnrollmentError::CapabilityDenied)
        ));
    }

    #[test]
    fn revoking_one_device_does_not_revoke_another() {
        let directory = tempfile::tempdir().expect("temporary directory");
        let first = pairing_fixture(&directory);
        let first_grant = first
            .store
            .redeem_pairing(
                &account(1),
                &first.confirmation,
                first.pending.sas(),
                600,
                103,
            )
            .expect("first grant");

        let invite = create_invite(&first.store, 104, 300);
        let (second_pending, second_response) = first
            .store
            .begin_device_enrollment(
                invite,
                "Tablet",
                DevicePlatform::Desktop,
                capabilities(),
                105,
            )
            .expect("second begin");
        let second_challenge = first
            .store
            .accept_pairing_response(&account(1), second_response, 106)
            .expect("second response");
        let second_confirmation = second_pending
            .confirm(&second_challenge, second_pending.sas())
            .expect("second confirmation");
        let second_grant = first
            .store
            .redeem_pairing(
                &account(1),
                &second_confirmation,
                second_pending.sas(),
                600,
                107,
            )
            .expect("second grant");

        first
            .store
            .revoke_device(&account(1), &first_grant.device_public_key_hex, 108)
            .expect("revoke first");
        assert!(matches!(
            first.store.authorize(
                &account(1),
                &first_grant.grant_id,
                &first_grant.device_public_key_hex,
                DeviceCapability::Nip46,
                109,
            ),
            Err(EnrollmentError::Revoked)
        ));
        let second_authorization = first
            .store
            .authorize(
                &account(1),
                &second_grant.grant_id,
                &second_grant.device_public_key_hex,
                DeviceCapability::Nip46,
                109,
            )
            .expect("second remains authorized");
        first
            .store
            .record_last_use(&second_authorization, 109)
            .expect("record last use");
        let inventory = first
            .store
            .device_inventory(&account(1))
            .expect("inventory");
        assert_eq!(inventory.len(), 2);
        assert_eq!(
            inventory
                .iter()
                .find(|device| device.device_public_key_hex == second_grant.device_public_key_hex)
                .and_then(|device| device.last_used_at),
            Some(109)
        );
    }

    #[test]
    fn weak_permissions_and_symlinks_are_refused() {
        use std::os::unix::fs::{PermissionsExt as _, symlink};

        let directory = tempfile::tempdir().expect("temporary directory");
        let fixture = pairing_fixture(&directory);
        let host_path = directory
            .path()
            .join("identity/device-enrollment/host")
            .join(&account(1).owner_public_key_hex)
            .join("enrollment.json");
        fs::set_permissions(&host_path, fs::Permissions::from_mode(0o644))
            .expect("weaken permissions");
        assert!(matches!(
            fixture.store.device_inventory(&account(1)),
            Err(EnrollmentError::Storage)
        ));

        fs::remove_file(&host_path).expect("remove weak file");
        let target = directory.path().join("outside.json");
        fs::write(&target, b"{}").expect("outside target");
        symlink(target, host_path).expect("symlink store");
        assert!(matches!(
            fixture.store.device_inventory(&account(1)),
            Err(EnrollmentError::Storage)
        ));
    }
}
