use std::{convert::Infallible, fmt};

use app_identity::AppChannel;
use nostr::{EventId, Kind, PublicKey, Tag, Timestamp, ToBech32, UnsignedEvent};
use serde::{Deserialize, Deserializer, Serialize, de::Error as _};
use sha2::{Digest, Sha256};
use thiserror::Error;

use crate::RecoveryProtectionStatus;

pub const BUZZ_SOURCE_COMMIT: &str = "acfbb1bb6af54cb29cb152496ff43b8285dcb8cf";
pub const IDENTITY_CONTRACT_VERSION: u32 = 1;
pub const FRESH_PROFILE_ID: &str = "openagents.omega.nostr_only.v1";
pub const PUBLIC_MANIFEST_SCHEMA: &str = "openagents.omega.identity-manifest.v1";
pub const COMPLETION_RECORD_SCHEMA: &str = "openagents.omega.identity-completion.v1";
pub const KEYRING_ACCOUNT: &str = "omega-sovereign-identity-v1";

#[derive(Debug, Error)]
pub enum ContractError {
    #[error("{field} must be a non-empty portable identifier")]
    InvalidReference { field: &'static str },
    #[error("invalid Nostr public key")]
    InvalidPublicKey,
    #[error("public identity does not match its public key")]
    PublicIdentityMismatch,
    #[error("manifest uses an unsupported schema, version, or profile")]
    UnsupportedManifest,
    #[error("manifest keyring locator does not match the selected application channel")]
    WrongKeyringLocator,
    #[error("signing request contains an invalid tag")]
    InvalidTag,
    #[error("signing request is not admitted by the Omega identity contract")]
    SigningNotAdmitted,
    #[error("public document serialization failed")]
    Serialization(#[from] serde_json::Error),
}

#[derive(Clone, PartialEq, Eq, Hash, Serialize)]
#[serde(transparent)]
pub struct IdentityRef(String);

impl IdentityRef {
    pub fn new(value: impl Into<String>) -> Result<Self, ContractError> {
        let value = value.into();
        validate_reference("identity_ref", &value)?;
        Ok(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for IdentityRef {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.debug_tuple("IdentityRef").field(&self.0).finish()
    }
}

impl<'de> Deserialize<'de> for IdentityRef {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        Self::new(String::deserialize(deserializer)?).map_err(D::Error::custom)
    }
}

#[derive(Clone, PartialEq, Eq, Hash, Serialize)]
#[serde(transparent)]
pub struct ReceiptRef(String);

impl ReceiptRef {
    pub fn new(value: impl Into<String>) -> Result<Self, ContractError> {
        let value = value.into();
        validate_reference("receipt_ref", &value)?;
        Ok(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for ReceiptRef {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.debug_tuple("ReceiptRef").field(&self.0).finish()
    }
}

impl<'de> Deserialize<'de> for ReceiptRef {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        Self::new(String::deserialize(deserializer)?).map_err(D::Error::custom)
    }
}

fn validate_reference(field: &'static str, value: &str) -> Result<(), ContractError> {
    if value.is_empty()
        || value.len() > 128
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'-' | b'_'))
    {
        return Err(ContractError::InvalidReference { field });
    }
    Ok(())
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(transparent)]
pub struct NostrPublicKeyHex(String);

impl NostrPublicKeyHex {
    pub fn new(value: impl AsRef<str>) -> Result<Self, ContractError> {
        let public_key = parse_public_key(value.as_ref())?;
        Ok(Self(public_key.to_hex()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub(crate) fn public_key(&self) -> Result<PublicKey, ContractError> {
        parse_public_key(&self.0)
    }
}

impl<'de> Deserialize<'de> for NostrPublicKeyHex {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        Self::new(String::deserialize(deserializer)?).map_err(D::Error::custom)
    }
}

fn parse_public_key(value: &str) -> Result<PublicKey, ContractError> {
    let public_key = PublicKey::from_hex(value).map_err(|_| ContractError::InvalidPublicKey)?;
    public_key
        .xonly()
        .map_err(|_| ContractError::InvalidPublicKey)?;
    Ok(public_key)
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(transparent)]
pub struct Npub(String);

impl Npub {
    fn from_public_key(public_key: &PublicKey) -> Self {
        let encoded = public_key
            .to_bech32()
            .unwrap_or_else(|never: Infallible| match never {});
        Self(encoded)
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(transparent)]
pub struct PublicFingerprint(String);

impl PublicFingerprint {
    fn from_public_key(public_key: &PublicKey) -> Self {
        Self(hex::encode(Sha256::digest(public_key.as_bytes())))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn display(&self) -> String {
        self.0
            .chars()
            .take(16)
            .collect::<String>()
            .as_bytes()
            .chunks(4)
            .map(|chunk| String::from_utf8_lossy(chunk).to_uppercase())
            .collect::<Vec<_>>()
            .join(" ")
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PublicIdentity {
    identity_ref: IdentityRef,
    public_key_hex: NostrPublicKeyHex,
    npub: Npub,
    fingerprint: PublicFingerprint,
}

impl<'de> Deserialize<'de> for PublicIdentity {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct PublicIdentityWire {
            identity_ref: IdentityRef,
            public_key_hex: NostrPublicKeyHex,
            npub: String,
            fingerprint: String,
        }

        let wire = PublicIdentityWire::deserialize(deserializer)?;
        let identity = Self {
            identity_ref: wire.identity_ref,
            public_key_hex: wire.public_key_hex,
            npub: Npub(wire.npub),
            fingerprint: PublicFingerprint(wire.fingerprint),
        };
        identity.validate().map_err(D::Error::custom)?;
        Ok(identity)
    }
}

impl PublicIdentity {
    pub fn from_public_key_hex(
        identity_ref: IdentityRef,
        public_key_hex: impl AsRef<str>,
    ) -> Result<Self, ContractError> {
        let public_key = parse_public_key(public_key_hex.as_ref())?;
        Ok(Self {
            identity_ref,
            public_key_hex: NostrPublicKeyHex(public_key.to_hex()),
            npub: Npub::from_public_key(&public_key),
            fingerprint: PublicFingerprint::from_public_key(&public_key),
        })
    }

    pub fn validate(&self) -> Result<(), ContractError> {
        validate_reference("identity_ref", self.identity_ref.as_str())?;
        let public_key = self.public_key_hex.public_key()?;
        if self.npub != Npub::from_public_key(&public_key)
            || self.fingerprint != PublicFingerprint::from_public_key(&public_key)
        {
            return Err(ContractError::PublicIdentityMismatch);
        }
        Ok(())
    }

    pub fn identity_ref(&self) -> &IdentityRef {
        &self.identity_ref
    }

    pub fn public_key_hex(&self) -> &NostrPublicKeyHex {
        &self.public_key_hex
    }

    pub fn npub(&self) -> &Npub {
        &self.npub
    }

    pub fn fingerprint(&self) -> &PublicFingerprint {
        &self.fingerprint
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct KeyringLocator {
    service: String,
    account: String,
}

impl KeyringLocator {
    pub fn for_channel(channel: AppChannel) -> Self {
        Self {
            service: channel.credential_namespace().to_string(),
            account: KEYRING_ACCOUNT.to_string(),
        }
    }

    pub fn service(&self) -> &str {
        &self.service
    }

    pub fn account(&self) -> &str {
        &self.account
    }

    pub(crate) fn proof(service: &'static str, account: &'static str) -> Self {
        Self {
            service: service.to_string(),
            account: account.to_string(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct IdentityManifest {
    schema: String,
    contract_version: u32,
    profile: String,
    identity: PublicIdentity,
    keyring_locator: KeyringLocator,
    receipt_refs: Vec<ReceiptRef>,
}

impl IdentityManifest {
    pub fn new(
        identity: PublicIdentity,
        keyring_locator: KeyringLocator,
        receipt_refs: Vec<ReceiptRef>,
    ) -> Self {
        Self {
            schema: PUBLIC_MANIFEST_SCHEMA.to_string(),
            contract_version: IDENTITY_CONTRACT_VERSION,
            profile: FRESH_PROFILE_ID.to_string(),
            identity,
            keyring_locator,
            receipt_refs,
        }
    }

    pub fn validate(&self, channel: AppChannel) -> Result<(), ContractError> {
        self.validate_for_locator(&KeyringLocator::for_channel(channel))
    }

    pub(crate) fn validate_for_locator(
        &self,
        expected_locator: &KeyringLocator,
    ) -> Result<(), ContractError> {
        if self.schema != PUBLIC_MANIFEST_SCHEMA
            || self.contract_version != IDENTITY_CONTRACT_VERSION
            || self.profile != FRESH_PROFILE_ID
        {
            return Err(ContractError::UnsupportedManifest);
        }
        self.identity.validate()?;
        for receipt_ref in &self.receipt_refs {
            validate_reference("receipt_ref", receipt_ref.as_str())?;
        }
        if &self.keyring_locator != expected_locator {
            return Err(ContractError::WrongKeyringLocator);
        }
        Ok(())
    }

    pub fn identity(&self) -> &PublicIdentity {
        &self.identity
    }

    pub fn keyring_locator(&self) -> &KeyringLocator {
        &self.keyring_locator
    }

    pub fn canonical_json(&self) -> Result<String, ContractError> {
        Ok(serde_json::to_string(self)?)
    }

    pub fn digest(&self) -> Result<String, ContractError> {
        Ok(hex::encode(Sha256::digest(
            self.canonical_json()?.as_bytes(),
        )))
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CompletionRecord {
    schema: String,
    contract_version: u32,
    identity_ref: IdentityRef,
    public_key_hex: NostrPublicKeyHex,
    manifest_digest: String,
    receipt_ref: ReceiptRef,
}

impl CompletionRecord {
    pub fn new(
        manifest: &IdentityManifest,
        receipt_ref: ReceiptRef,
        channel: AppChannel,
    ) -> Result<Self, ContractError> {
        let record = Self {
            schema: COMPLETION_RECORD_SCHEMA.to_string(),
            contract_version: IDENTITY_CONTRACT_VERSION,
            identity_ref: manifest.identity.identity_ref.clone(),
            public_key_hex: manifest.identity.public_key_hex.clone(),
            manifest_digest: manifest.digest()?,
            receipt_ref,
        };
        record.validate_against(manifest, channel)?;
        Ok(record)
    }

    pub(crate) fn new_for_locator(
        manifest: &IdentityManifest,
        receipt_ref: ReceiptRef,
        locator: &KeyringLocator,
    ) -> Result<Self, ContractError> {
        let record = Self {
            schema: COMPLETION_RECORD_SCHEMA.to_string(),
            contract_version: IDENTITY_CONTRACT_VERSION,
            identity_ref: manifest.identity.identity_ref.clone(),
            public_key_hex: manifest.identity.public_key_hex.clone(),
            manifest_digest: manifest.digest()?,
            receipt_ref,
        };
        record.validate_against_locator(manifest, locator)?;
        Ok(record)
    }

    pub fn validate(&self) -> Result<(), ContractError> {
        if self.schema != COMPLETION_RECORD_SCHEMA
            || self.contract_version != IDENTITY_CONTRACT_VERSION
        {
            return Err(ContractError::UnsupportedManifest);
        }
        validate_reference("identity_ref", self.identity_ref.as_str())?;
        self.public_key_hex.public_key()?;
        validate_reference("receipt_ref", self.receipt_ref.as_str())?;
        if self.manifest_digest.len() != 64
            || !self
                .manifest_digest
                .bytes()
                .all(|byte| byte.is_ascii_digit() || matches!(byte, b'a'..=b'f'))
        {
            return Err(ContractError::UnsupportedManifest);
        }
        Ok(())
    }

    pub fn receipt_ref(&self) -> &ReceiptRef {
        &self.receipt_ref
    }

    pub fn validate_against(
        &self,
        manifest: &IdentityManifest,
        channel: AppChannel,
    ) -> Result<(), ContractError> {
        manifest.validate(channel)?;
        self.validate_against_valid_manifest(manifest)
    }

    pub(crate) fn validate_against_locator(
        &self,
        manifest: &IdentityManifest,
        locator: &KeyringLocator,
    ) -> Result<(), ContractError> {
        manifest.validate_for_locator(locator)?;
        self.validate_against_valid_manifest(manifest)
    }

    fn validate_against_valid_manifest(
        &self,
        manifest: &IdentityManifest,
    ) -> Result<(), ContractError> {
        self.validate()?;
        if self.identity_ref != manifest.identity.identity_ref
            || self.public_key_hex != manifest.identity.public_key_hex
            || self.manifest_digest != manifest.digest()?
            || !manifest.receipt_refs.contains(&self.receipt_ref)
        {
            return Err(ContractError::UnsupportedManifest);
        }
        Ok(())
    }
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum CustodyState {
    Absent,
    Ready,
    Locked,
    Incomplete,
    Lost,
    Conflict,
    ResetFailed,
    RelaunchRequired,
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum RecoveryState {
    NotRequired,
    DiscoveryRequired,
    CandidateFound,
    OwnerSelectionRequired,
    Blocked,
    Complete,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "operation", rename_all = "snake_case")]
#[serde(deny_unknown_fields)]
pub enum CustodyRequest {
    Inspect,
    Create {
        request_ref: ReceiptRef,
    },
    Import {
        request_ref: ReceiptRef,
    },
    Open {
        expected_identity: IdentityRef,
    },
    Reset {
        expected_identity: IdentityRef,
        authorization_ref: ReceiptRef,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CustodyResult {
    pub state: CustodyState,
    pub identity: Option<PublicIdentity>,
    pub receipt_ref: Option<ReceiptRef>,
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum PendingIdentityOperation {
    Create,
    Import,
    ResolveConflict,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PendingIdentityTransaction {
    pub operation: PendingIdentityOperation,
    pub receipt_ref: ReceiptRef,
    pub expected_identity: Option<PublicIdentity>,
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum CustodyConflictReason {
    AmbiguousSecureStore,
    PublicManifestCustodyMismatch,
    PendingTransactionMismatch,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CustodyConflict {
    pub reason: CustodyConflictReason,
    pub identities: Vec<PublicIdentity>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct IdentityInspection {
    pub custody: CustodyResult,
    pub pending_transaction: Option<PendingIdentityTransaction>,
    pub conflict: Option<CustodyConflict>,
    pub recovery_protection: RecoveryProtectionStatus,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RecoveryRequest {
    pub identity_ref: IdentityRef,
    pub candidate_refs: Vec<IdentityRef>,
    pub authorization_ref: Option<ReceiptRef>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RecoveryResult {
    pub state: RecoveryState,
    pub identity: Option<PublicIdentity>,
    pub receipt_ref: Option<ReceiptRef>,
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SigningPurpose {
    NostrEvent,
    Nip44EncryptedSelfEvent,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UnsignedEventTemplate {
    pub created_at: u64,
    pub kind: u16,
    pub tags: Vec<Vec<String>>,
    pub content: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AdmittedSigningRequest {
    pub request_ref: ReceiptRef,
    pub identity_ref: IdentityRef,
    pub purpose: SigningPurpose,
    pub event: UnsignedEventTemplate,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OwnerAttestationRequest {
    pub request_ref: ReceiptRef,
    pub identity_ref: IdentityRef,
    pub agent_public_key_hex: NostrPublicKeyHex,
    pub conditions: String,
}

impl OwnerAttestationRequest {
    pub fn validate(&self, identity: &PublicIdentity) -> Result<(), ContractError> {
        validate_reference("request_ref", self.request_ref.as_str())?;
        validate_reference("identity_ref", self.identity_ref.as_str())?;
        self.agent_public_key_hex.public_key()?;
        if self.identity_ref != *identity.identity_ref()
            || self.agent_public_key_hex == *identity.public_key_hex()
            || self.conditions.len() > 1_024
            || self.conditions.chars().any(char::is_control)
        {
            return Err(ContractError::SigningNotAdmitted);
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OwnerAttestationResult {
    pub request_ref: ReceiptRef,
    pub identity: PublicIdentity,
    pub agent_public_key_hex: NostrPublicKeyHex,
    pub auth_tag: Vec<String>,
}

impl AdmittedSigningRequest {
    pub fn validate(&self) -> Result<(), ContractError> {
        validate_reference("request_ref", self.request_ref.as_str())?;
        validate_reference("identity_ref", self.identity_ref.as_str())?;
        if self.event.content.len() > 1_048_576 || self.event.tags.len() > 1_024 {
            return Err(ContractError::SigningNotAdmitted);
        }
        self.parsed_tags()?;
        Ok(())
    }

    pub fn event_id(&self, identity: &PublicIdentity) -> Result<EventId, ContractError> {
        let mut unsigned_event = self.unsigned_event(identity)?;
        Ok(unsigned_event.id())
    }

    pub(crate) fn unsigned_event(
        &self,
        identity: &PublicIdentity,
    ) -> Result<UnsignedEvent, ContractError> {
        self.validate()?;
        identity.validate()?;
        if &self.identity_ref != identity.identity_ref() {
            return Err(ContractError::SigningNotAdmitted);
        }
        let public_key = identity.public_key_hex.public_key()?;
        Ok(UnsignedEvent::new(
            public_key,
            Timestamp::from_secs(self.event.created_at),
            Kind::from_u16(self.event.kind),
            self.parsed_tags()?,
            self.event.content.clone(),
        ))
    }

    fn parsed_tags(&self) -> Result<Vec<Tag>, ContractError> {
        self.event
            .tags
            .iter()
            .cloned()
            .map(|tag| Tag::parse(tag).map_err(|_| ContractError::InvalidTag))
            .collect()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SigningResult {
    pub request_ref: ReceiptRef,
    pub identity: PublicIdentity,
    pub event_id: String,
    pub signature: String,
    pub signed_event_json: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PrivateMessageRequest {
    pub request_ref: ReceiptRef,
    pub identity_ref: IdentityRef,
    pub recipients: Vec<NostrPublicKeyHex>,
    pub rumor: UnsignedEventTemplate,
}

impl PrivateMessageRequest {
    pub fn validate(&self) -> Result<(), ContractError> {
        validate_reference("request_ref", self.request_ref.as_str())?;
        validate_reference("identity_ref", self.identity_ref.as_str())?;
        if self.recipients.is_empty()
            || self.recipients.len() > 8
            || self.rumor.kind != Kind::PrivateDirectMessage.as_u16()
            || self.rumor.content.len() > 64 * 1024
            || self.rumor.tags.len() > 128
        {
            return Err(ContractError::SigningNotAdmitted);
        }
        let mut unique = std::collections::HashSet::new();
        for recipient in &self.recipients {
            recipient.public_key()?;
            if !unique.insert(recipient.as_str()) {
                return Err(ContractError::SigningNotAdmitted);
            }
        }
        self.rumor
            .tags
            .iter()
            .cloned()
            .map(|tag| Tag::parse(tag).map_err(|_| ContractError::InvalidTag))
            .collect::<Result<Vec<_>, _>>()?;
        let recipient_set: std::collections::HashSet<&str> = self
            .recipients
            .iter()
            .map(NostrPublicKeyHex::as_str)
            .collect();
        let mut tagged_recipient_set = std::collections::HashSet::new();
        for tag in &self.rumor.tags {
            if tag.first().map(String::as_str) != Some("p") {
                continue;
            }
            let tagged_recipient = tag.get(1).ok_or(ContractError::SigningNotAdmitted)?;
            NostrPublicKeyHex::new(tagged_recipient)?;
            if !tagged_recipient_set.insert(tagged_recipient.as_str()) {
                return Err(ContractError::SigningNotAdmitted);
            }
        }
        if tagged_recipient_set != recipient_set {
            return Err(ContractError::SigningNotAdmitted);
        }
        Ok(())
    }

    pub(crate) fn unsigned_rumor(
        &self,
        identity: &PublicIdentity,
    ) -> Result<UnsignedEvent, ContractError> {
        self.validate()?;
        identity.validate()?;
        if &self.identity_ref != identity.identity_ref() {
            return Err(ContractError::SigningNotAdmitted);
        }
        let tags = self
            .rumor
            .tags
            .iter()
            .cloned()
            .map(|tag| Tag::parse(tag).map_err(|_| ContractError::InvalidTag))
            .collect::<Result<Vec<_>, _>>()?;
        let mut rumor = UnsignedEvent::new(
            identity.public_key_hex().public_key()?,
            Timestamp::from_secs(self.rumor.created_at),
            Kind::PrivateDirectMessage,
            tags,
            self.rumor.content.clone(),
        );
        rumor.ensure_id();
        Ok(rumor)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GiftWrappedPrivateMessage {
    pub receiver_public_key_hex: String,
    pub rumor_event_id: String,
    pub gift_wrap_event_json: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UnwrappedPrivateMessage {
    pub rumor_event_id: String,
    pub sender_public_key_hex: String,
    pub created_at: u64,
    pub tags: Vec<Vec<String>>,
    pub content: String,
}

#[cfg(test)]
mod tests {
    use std::{collections::HashSet, str::FromStr};

    use nostr::{EventId, JsonUtil, PublicKey};
    use serde_json::Value;

    use super::*;

    const PUBLIC_KEY_HEX: &str = "f86c44a2de95d9149b51c6a29afeabba264c18e2fa7c49de93424a0c56947785";
    const EXPECTED_NPUB: &str = "npub1lpkyfgk7jhv3fx63c63f4l4thgnycx8zlf7ynh5ngf9qc455w7zs7s8hua";
    const EXPECTED_EVENT_ID: &str =
        "2be17aa3031bdcb006f0fce80c146dea9c1c0268b0af2398bb673365c6444d45";

    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    struct ContractVector {
        source_commit: String,
        profile: String,
        public_identity: PublicIdentity,
        fingerprint_display: String,
        manifest: IdentityManifest,
        manifest_digest: String,
        signing_request: AdmittedSigningRequest,
        expected_event_id: String,
        signature: String,
    }

    fn identity() -> PublicIdentity {
        PublicIdentity::from_public_key_hex(
            IdentityRef::new("omega-test-identity").expect("valid fixture reference"),
            PUBLIC_KEY_HEX,
        )
        .expect("valid fixture public key")
    }

    fn request() -> AdmittedSigningRequest {
        AdmittedSigningRequest {
            request_ref: ReceiptRef::new("signing-vector-1").expect("valid fixture reference"),
            identity_ref: identity().identity_ref().clone(),
            purpose: SigningPurpose::NostrEvent,
            event: UnsignedEventTemplate {
                created_at: 1_640_839_235,
                kind: 4,
                tags: vec![vec![
                    "p".to_string(),
                    "13adc511de7e1cfcf1c6b7f6365fb5a03442d7bcacf565ea57fa7770912c023d".to_string(),
                ]],
                content: "uRuvYr585B80L6rSJiHocw==?iv=oh6LVqdsYYol3JfFnXTbPA==".to_string(),
            },
        }
    }

    #[test]
    fn keyring_locators_are_exact_and_unique() {
        let locators = AppChannel::ALL.map(KeyringLocator::for_channel);
        assert_eq!(
            locators.each_ref().map(|locator| locator.service.as_str()),
            [
                "com.openagents.omega.credentials.dev",
                "com.openagents.omega.credentials.nightly",
                "com.openagents.omega.credentials.rc",
                "com.openagents.omega.credentials",
            ]
        );
        assert_eq!(
            locators
                .iter()
                .map(KeyringLocator::service)
                .collect::<HashSet<_>>()
                .len(),
            4
        );
        assert!(
            locators
                .iter()
                .all(|locator| locator.account == KEYRING_ACCOUNT)
        );
    }

    #[test]
    fn public_key_vector_derives_npub_and_fingerprint() {
        let identity = identity();
        assert_eq!(identity.public_key_hex().as_str(), PUBLIC_KEY_HEX);
        assert_eq!(identity.npub().as_str(), EXPECTED_NPUB);
        assert_eq!(
            identity.fingerprint().as_str(),
            "daee948a8bf214f3d48ab71cf2b672bee2396252900e9d74296e1e3d7ad1d139"
        );
        assert_eq!(identity.fingerprint().display(), "DAEE 948A 8BF2 14F3");
    }

    #[test]
    fn admitted_signing_vector_has_deterministic_event_id() {
        let event_id = request()
            .event_id(&identity())
            .expect("fixture signing request is admitted");
        assert_eq!(
            event_id,
            EventId::from_hex(EXPECTED_EVENT_ID).expect("valid fixture event id")
        );
    }

    #[test]
    fn checked_in_contract_vector_is_frozen_and_public_only() {
        let vector_json = include_str!("../fixtures/omega_nostr_identity_v1.json");
        let vector: ContractVector =
            serde_json::from_str(vector_json).expect("parse checked-in contract vector");

        assert_eq!(vector.source_commit, BUZZ_SOURCE_COMMIT);
        assert_eq!(vector.profile, FRESH_PROFILE_ID);
        vector.public_identity.validate().expect("public identity");
        assert_eq!(
            vector.public_identity.fingerprint().display(),
            vector.fingerprint_display
        );
        vector
            .manifest
            .validate(AppChannel::Dev)
            .expect("manifest contract");
        assert_eq!(
            vector.manifest.digest().expect("manifest digest"),
            vector.manifest_digest
        );
        assert_eq!(
            vector
                .signing_request
                .event_id(&vector.public_identity)
                .expect("admitted signing request")
                .to_hex(),
            vector.expected_event_id
        );
        assert_eq!(vector.signature.len(), 128);
        assert_public_only(
            &serde_json::from_str(vector_json).expect("parse raw public fixture value"),
        );
        assert_public_only(
            &serde_json::to_value(vector.manifest).expect("serialize public fixture"),
        );
    }

    #[test]
    fn admitted_signing_vector_signature_verifies() {
        let event_json = format!(
            r#"{{"id":"{EXPECTED_EVENT_ID}","pubkey":"{PUBLIC_KEY_HEX}","created_at":1640839235,"kind":4,"tags":[["p","13adc511de7e1cfcf1c6b7f6365fb5a03442d7bcacf565ea57fa7770912c023d"]],"content":"uRuvYr585B80L6rSJiHocw==?iv=oh6LVqdsYYol3JfFnXTbPA==","sig":"a5d9290ef9659083c490b303eb7ee41356d8778ff19f2f91776c8dc4443388a64ffcf336e61af4c25c05ac3ae952d1ced889ed655b67790891222aaa15b99fdd"}}"#
        );
        let event = nostr::Event::from_json(event_json).expect("valid signed fixture");
        event.verify().expect("fixture signature verifies");
        assert_eq!(
            event.pubkey,
            PublicKey::from_str(PUBLIC_KEY_HEX).expect("valid fixture public key")
        );
    }

    #[test]
    fn malformed_or_wrong_identity_signing_requests_are_rejected() {
        let mut malformed = request();
        malformed.event.tags = vec![Vec::new()];
        assert!(matches!(
            malformed.validate(),
            Err(ContractError::InvalidTag)
        ));

        let mut wrong_identity = request();
        wrong_identity.identity_ref =
            IdentityRef::new("another-identity").expect("valid fixture reference");
        assert!(matches!(
            wrong_identity.event_id(&identity()),
            Err(ContractError::SigningNotAdmitted)
        ));
    }

    #[test]
    fn public_contract_serialization_contains_no_secret_or_wallet_fields() {
        let manifest = IdentityManifest::new(
            identity(),
            KeyringLocator::for_channel(AppChannel::Dev),
            vec![ReceiptRef::new("creation-receipt").expect("valid fixture reference")],
        );
        let value = serde_json::to_value(manifest).expect("serialize public fixture");
        assert_public_only(&value);
    }

    #[test]
    fn deserialization_enforces_reference_and_public_identity_invariants() {
        assert!(serde_json::from_str::<IdentityRef>(r#""""#).is_err());
        assert!(serde_json::from_str::<ReceiptRef>(r#""contains spaces""#).is_err());
        assert!(serde_json::from_str::<NostrPublicKeyHex>(r#""not-a-public-key""#).is_err());

        let mut value = serde_json::to_value(identity()).expect("serialize public identity");
        let Some(object) = value.as_object_mut() else {
            panic!("public identity fixture must serialize as an object");
        };
        object.insert("npub".to_string(), Value::String("npub1wrong".to_string()));
        assert!(serde_json::from_value::<PublicIdentity>(value).is_err());
    }

    #[test]
    fn manifest_rejects_unknown_secret_and_wallet_fields() {
        let manifest = IdentityManifest::new(
            identity(),
            KeyringLocator::for_channel(AppChannel::Dev),
            vec![ReceiptRef::new("creation-receipt").expect("valid fixture reference")],
        );
        for forbidden_field in ["nsec", "secret_key", "wallet", "spark"] {
            let mut value = serde_json::to_value(&manifest).expect("serialize manifest");
            let Some(object) = value.as_object_mut() else {
                panic!("manifest fixture must serialize as an object");
            };
            object.insert(
                forbidden_field.to_string(),
                Value::String("forbidden".to_string()),
            );
            assert!(
                serde_json::from_value::<IdentityManifest>(value).is_err(),
                "accepted forbidden field {forbidden_field}"
            );
        }
    }

    #[test]
    fn completion_record_must_match_its_manifest() {
        let receipt_ref = ReceiptRef::new("creation-receipt").expect("valid fixture reference");
        let manifest = IdentityManifest::new(
            identity(),
            KeyringLocator::for_channel(AppChannel::Dev),
            vec![receipt_ref.clone()],
        );
        let mut completion = CompletionRecord::new(&manifest, receipt_ref, AppChannel::Dev)
            .expect("completion record");
        completion.manifest_digest = "0".repeat(64);
        assert!(
            completion
                .validate_against(&manifest, AppChannel::Dev)
                .is_err()
        );
    }

    fn assert_public_only(value: &Value) {
        const FORBIDDEN: [&str; 7] = [
            "spark",
            "wallet",
            "mnemonic",
            "seed",
            "nsec",
            "private_key",
            "secret_key",
        ];
        match value {
            Value::Object(object) => {
                for (key, value) in object {
                    assert!(
                        FORBIDDEN.iter().all(|forbidden| !key.contains(forbidden)),
                        "forbidden public-contract field: {key}"
                    );
                    assert_public_only(value);
                }
            }
            Value::Array(values) => {
                for value in values {
                    assert_public_only(value);
                }
            }
            Value::String(value) => {
                assert!(FORBIDDEN.iter().all(|forbidden| !value.contains(forbidden)));
            }
            _ => {}
        }
    }
}
