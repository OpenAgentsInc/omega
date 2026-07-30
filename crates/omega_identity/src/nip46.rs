use std::{
    collections::{BTreeSet, HashSet},
    fmt, fs,
    io::{self, Write as _},
    path::{Path, PathBuf},
};

use app_identity::AppChannel;
use atomic_write_file::AtomicWriteFile;
use nostr::{
    Event, EventBuilder, JsonUtil as _, Keys, Kind, Tag, Timestamp, UnsignedEvent,
    nips::{
        nip44,
        nip46::{NostrConnectMessage, NostrConnectMethod},
    },
};
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};
use thiserror::Error;
use url::Url;
use zeroize::Zeroizing;

use crate::{AccountRef, AccountSelectionToken, NostrPublicKeyHex, PublicIdentity};

const NIP46_PAIRING_SCHEMA: &str = "openagents.omega.nip46-pairing.v1";
const NIP46_CAPABILITY_SCHEMA: &str = "openagents.omega.nip46-capability.v1";
const NIP46_MAX_URI_BYTES: usize = 4_096;
const NIP46_MAX_RELAYS: usize = 4;
const NIP46_MAX_METHODS: usize = 5;
const NIP46_MAX_EVENT_KINDS: usize = 16;
const NIP46_MAX_LIFETIME_SECONDS: u64 = 60 * 60 * 24 * 30;
const NIP46_MAX_RESPONSE_BYTES: usize = 1_048_576;
const NIP46_LOGIN_CHALLENGE_KIND: u16 = 24246;

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum Nip46Error {
    #[error("the NIP-46 URI is invalid")]
    InvalidUri,
    #[error("the NIP-46 relay set is invalid")]
    InvalidRelays,
    #[error("the NIP-46 permission request is invalid")]
    InvalidPermissions,
    #[error("the NIP-46 capability lifetime is invalid")]
    InvalidLifetime,
    #[error("the NIP-46 pairing state is invalid")]
    InvalidPairingState,
    #[error("the NIP-46 client capability store is unavailable")]
    Storage,
    #[error("the NIP-46 response was explicitly rejected")]
    Rejected,
    #[error("the NIP-46 request timed out after relay activity")]
    Timeout,
    #[error("the NIP-46 signer remained silent")]
    Silence,
    #[error("the NIP-46 response author is wrong")]
    WrongAuthor,
    #[error("the NIP-46 response request id is wrong")]
    WrongRequestId,
    #[error("the NIP-46 response arrived from an undeclared relay")]
    WrongRelay,
    #[error("the NIP-46 event is invalid")]
    InvalidEvent,
    #[error("the NIP-46 response ciphertext is malformed")]
    MalformedCiphertext,
    #[error("the NIP-46 response payload is malformed")]
    MalformedResponse,
    #[error("the NIP-46 response duplicated a completed request")]
    DuplicateResponse,
    #[error("the NIP-46 client capability was revoked")]
    Revoked,
    #[error("the selected account generation changed")]
    StaleGeneration,
    #[error("the requested NIP-46 operation is outside the capability")]
    UndeclaredCapability,
    #[error("the NIP-46 signed login challenge is invalid")]
    InvalidChallenge,
}

#[derive(Clone, PartialEq, Eq)]
struct Nip46PairingSecret(Zeroizing<String>);

impl Nip46PairingSecret {
    fn new(value: String) -> Result<Self, Nip46Error> {
        if value.is_empty() || value.len() > 256 {
            return Err(Nip46Error::InvalidUri);
        }
        Ok(Self(Zeroizing::new(value)))
    }

    fn expose(&self) -> &str {
        self.0.as_str()
    }
}

impl fmt::Debug for Nip46PairingSecret {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("Nip46PairingSecret([REDACTED])")
    }
}

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum Nip46UriKind {
    Bunker,
    NostrConnect,
}

#[derive(Clone, PartialEq, Eq)]
pub struct Nip46ConnectionInput {
    kind: Nip46UriKind,
    public_key: NostrPublicKeyHex,
    relays: Vec<String>,
    secret: Option<Nip46PairingSecret>,
    declared_permissions: Vec<String>,
    application_name: Option<String>,
}

impl fmt::Debug for Nip46ConnectionInput {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Nip46ConnectionInput")
            .field("kind", &self.kind)
            .field("public_key", &self.public_key)
            .field("relays", &self.relays)
            .field("secret", &self.secret.as_ref().map(|_| "[REDACTED]"))
            .field("declared_permissions", &self.declared_permissions)
            .field("application_name", &self.application_name)
            .finish()
    }
}

impl Nip46ConnectionInput {
    pub fn parse(value: &str) -> Result<Self, Nip46Error> {
        if value.len() > NIP46_MAX_URI_BYTES {
            return Err(Nip46Error::InvalidUri);
        }
        let uri = Url::parse(value).map_err(|_| Nip46Error::InvalidUri)?;
        if uri.fragment().is_some() || !uri.username().is_empty() || uri.password().is_some() {
            return Err(Nip46Error::InvalidUri);
        }
        if !uri.path().is_empty() && uri.path() != "/" {
            return Err(Nip46Error::InvalidUri);
        }
        let public_key = NostrPublicKeyHex::new(uri.host_str().ok_or(Nip46Error::InvalidUri)?)
            .map_err(|_| Nip46Error::InvalidUri)?;
        let kind = match uri.scheme() {
            "bunker" => Nip46UriKind::Bunker,
            "nostrconnect" => Nip46UriKind::NostrConnect,
            _ => return Err(Nip46Error::InvalidUri),
        };
        let mut relays = Vec::new();
        let mut secret = None;
        let mut permissions = Vec::new();
        let mut application_name = None;
        let mut permissions_seen = false;
        let mut name_seen = false;
        let mut url_seen = false;
        let mut image_seen = false;
        for (key, value) in uri.query_pairs() {
            match key.as_ref() {
                "relay" => relays.push(normalize_relay(&value)?),
                "secret" if secret.is_none() => {
                    secret = Some(Nip46PairingSecret::new(value.into_owned())?)
                }
                "perms" if kind == Nip46UriKind::NostrConnect && !permissions_seen => {
                    permissions_seen = true;
                    permissions = parse_permission_strings(&value)?;
                }
                "name" if kind == Nip46UriKind::NostrConnect && !name_seen => {
                    name_seen = true;
                    let value = value.into_owned();
                    if value.is_empty() || value.len() > 128 {
                        return Err(Nip46Error::InvalidUri);
                    }
                    application_name = Some(value);
                }
                "url" if kind == Nip46UriKind::NostrConnect && !url_seen => {
                    url_seen = true;
                    let parsed = Url::parse(&value).map_err(|_| Nip46Error::InvalidUri)?;
                    if value.len() > 2_048 || parsed.scheme() != "https" {
                        return Err(Nip46Error::InvalidUri);
                    }
                }
                "image" if kind == Nip46UriKind::NostrConnect && !image_seen => {
                    image_seen = true;
                    let parsed = Url::parse(&value).map_err(|_| Nip46Error::InvalidUri)?;
                    if value.len() > 2_048 || parsed.scheme() != "https" {
                        return Err(Nip46Error::InvalidUri);
                    }
                }
                _ => return Err(Nip46Error::InvalidUri),
            }
        }
        validate_relays(&relays)?;
        if kind == Nip46UriKind::NostrConnect && secret.is_none() {
            return Err(Nip46Error::InvalidUri);
        }
        Ok(Self {
            kind,
            public_key,
            relays,
            secret,
            declared_permissions: permissions,
            application_name,
        })
    }

    pub fn kind(&self) -> Nip46UriKind {
        self.kind
    }

    pub fn public_key(&self) -> &NostrPublicKeyHex {
        &self.public_key
    }

    pub fn relays(&self) -> &[String] {
        &self.relays
    }

    pub fn declared_permissions(&self) -> &[String] {
        &self.declared_permissions
    }
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Nip46CapabilityMethod {
    LoginProof,
    SignEvent,
    Nip44Encrypt,
    Nip44Decrypt,
    BulkDecrypt,
}

impl Nip46CapabilityMethod {
    fn protocol_method(self) -> NostrConnectMethod {
        match self {
            Self::LoginProof | Self::SignEvent => NostrConnectMethod::SignEvent,
            Self::Nip44Encrypt => NostrConnectMethod::Nip44Encrypt,
            Self::Nip44Decrypt | Self::BulkDecrypt => NostrConnectMethod::Nip44Decrypt,
        }
    }
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Nip46RecoveryDependency {
    RemoteSigner,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Nip46PermissionPreview {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expected_signer: Option<NostrPublicKeyHex>,
    pub methods: BTreeSet<Nip46CapabilityMethod>,
    pub event_kinds: BTreeSet<u16>,
    pub relays: Vec<String>,
    pub issued_at: u64,
    pub expires_at: u64,
    pub recovery_dependency: Nip46RecoveryDependency,
}

impl Nip46PermissionPreview {
    pub fn omega_first_profile(
        expected_signer: Option<NostrPublicKeyHex>,
        relays: Vec<String>,
        issued_at: u64,
        expires_at: u64,
    ) -> Result<Self, Nip46Error> {
        Self::new(
            expected_signer,
            [
                Nip46CapabilityMethod::LoginProof,
                Nip46CapabilityMethod::SignEvent,
            ],
            [9, 1111, 1984, 22242, 27235],
            relays,
            issued_at,
            expires_at,
        )
    }

    pub fn encryption_profile(
        expected_signer: NostrPublicKeyHex,
        relays: Vec<String>,
        issued_at: u64,
        expires_at: u64,
    ) -> Result<Self, Nip46Error> {
        Self::new(
            Some(expected_signer),
            [
                Nip46CapabilityMethod::Nip44Encrypt,
                Nip46CapabilityMethod::Nip44Decrypt,
            ],
            [],
            relays,
            issued_at,
            expires_at,
        )
    }

    pub fn bulk_decrypt_profile(
        expected_signer: NostrPublicKeyHex,
        relays: Vec<String>,
        issued_at: u64,
        expires_at: u64,
    ) -> Result<Self, Nip46Error> {
        Self::new(
            Some(expected_signer),
            [Nip46CapabilityMethod::BulkDecrypt],
            [],
            relays,
            issued_at,
            expires_at,
        )
    }

    pub fn new(
        expected_signer: Option<NostrPublicKeyHex>,
        methods: impl IntoIterator<Item = Nip46CapabilityMethod>,
        event_kinds: impl IntoIterator<Item = u16>,
        relays: Vec<String>,
        issued_at: u64,
        expires_at: u64,
    ) -> Result<Self, Nip46Error> {
        let preview = Self {
            expected_signer,
            methods: methods.into_iter().collect(),
            event_kinds: event_kinds.into_iter().collect(),
            relays,
            issued_at,
            expires_at,
            recovery_dependency: Nip46RecoveryDependency::RemoteSigner,
        };
        preview.validate()?;
        Ok(preview)
    }

    fn validate(&self) -> Result<(), Nip46Error> {
        validate_relays(&self.relays)?;
        if self.methods.is_empty()
            || self.methods.len() > NIP46_MAX_METHODS
            || self.event_kinds.len() > NIP46_MAX_EVENT_KINDS
            || (self.methods.contains(&Nip46CapabilityMethod::SignEvent)
                && self.event_kinds.is_empty())
        {
            return Err(Nip46Error::InvalidPermissions);
        }
        if self.issued_at == 0
            || self.expires_at <= self.issued_at
            || self.expires_at - self.issued_at > NIP46_MAX_LIFETIME_SECONDS
        {
            return Err(Nip46Error::InvalidLifetime);
        }
        Ok(())
    }
}

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum Nip46Operation {
    LoginProof,
    SignEvent { kind: u16 },
    Nip44Encrypt,
    Nip44Decrypt,
    BulkDecrypt,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Nip46OperationRequest {
    SignEvent {
        unsigned_event_json: String,
    },
    Nip44Encrypt {
        public_key: NostrPublicKeyHex,
        plaintext: String,
    },
    Nip44Decrypt {
        public_key: NostrPublicKeyHex,
        ciphertext: String,
    },
    BulkDecrypt {
        public_key: NostrPublicKeyHex,
        ciphertext: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Nip46OperationResult {
    SignedEvent(Event),
    Ciphertext(String),
    Plaintext(String),
}

impl Nip46Operation {
    fn method(self) -> Nip46CapabilityMethod {
        match self {
            Self::LoginProof => Nip46CapabilityMethod::LoginProof,
            Self::SignEvent { .. } => Nip46CapabilityMethod::SignEvent,
            Self::Nip44Encrypt => Nip46CapabilityMethod::Nip44Encrypt,
            Self::Nip44Decrypt => Nip46CapabilityMethod::Nip44Decrypt,
            Self::BulkDecrypt => Nip46CapabilityMethod::BulkDecrypt,
        }
    }
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Nip46CapabilityState {
    AwaitingRegistration,
    Active,
    Offline,
    Rejected,
    Revoked,
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Nip46PairingFence {
    pub registry_generation: u64,
}

impl Nip46PairingFence {
    pub fn new(registry_generation: u64) -> Result<Self, Nip46Error> {
        if registry_generation == 0 {
            return Err(Nip46Error::StaleGeneration);
        }
        Ok(Self {
            registry_generation,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Nip46Capability {
    schema: String,
    pub capability_ref: String,
    pub account_ref: AccountRef,
    pub account_generation: u64,
    pub user_identity: PublicIdentity,
    pub remote_signer_public_key: NostrPublicKeyHex,
    pub client_public_key: NostrPublicKeyHex,
    pub methods: BTreeSet<Nip46CapabilityMethod>,
    pub event_kinds: BTreeSet<u16>,
    pub relays: Vec<String>,
    pub issued_at: u64,
    pub expires_at: u64,
    pub state: Nip46CapabilityState,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_successful_use: Option<u64>,
}

impl Nip46Capability {
    pub fn authorize(
        &self,
        selection: &AccountSelectionToken,
        operation: Nip46Operation,
        now: u64,
    ) -> Result<(), Nip46Error> {
        if self.state == Nip46CapabilityState::Revoked {
            return Err(Nip46Error::Revoked);
        }
        if self.state != Nip46CapabilityState::Active {
            return Err(Nip46Error::UndeclaredCapability);
        }
        if selection.account_ref != self.account_ref
            || selection.generation != self.account_generation
            || selection.identity != self.user_identity
        {
            return Err(Nip46Error::StaleGeneration);
        }
        if now >= self.expires_at {
            return Err(Nip46Error::Revoked);
        }
        if !self.methods.contains(&operation.method()) {
            return Err(Nip46Error::UndeclaredCapability);
        }
        if let Nip46Operation::SignEvent { kind } = operation
            && !self.event_kinds.contains(&kind)
        {
            return Err(Nip46Error::UndeclaredCapability);
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Nip46PairingState {
    AwaitingApproval,
    AwaitingAcknowledgement,
    AwaitingUserPublicKey,
    AwaitingFinalApproval,
    AwaitingSignedChallenge,
    AwaitingRegistration,
    Active,
    Rejected,
    Revoked,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Nip46PairingRecord {
    schema: String,
    capability_ref: String,
    fence: Nip46PairingFence,
    preview: Nip46PermissionPreview,
    client_public_key: NostrPublicKeyHex,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    remote_signer_public_key: Option<NostrPublicKeyHex>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    user_identity: Option<PublicIdentity>,
    state: Nip46PairingState,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pending: Option<Nip46PendingRequest>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    challenge: Option<Nip46Challenge>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Nip46Challenge {
    content: String,
    created_at: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Nip46RuntimeExpectation {
    SignEvent {
        expected_event_id: String,
        kind: u16,
    },
    Nip44Encrypt,
    Nip44Decrypt,
    BulkDecrypt,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Nip46RuntimeRecord {
    capability_ref: String,
    pending: Nip46PendingRequest,
    expectation: Nip46RuntimeExpectation,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Nip46PendingRequest {
    pub request_id: String,
    pub method: Nip46CapabilityMethod,
    pub registry_generation: u64,
    pub allowed_relays: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expected_author: Option<NostrPublicKeyHex>,
    pub expires_at: u64,
    pub issued_at: u64,
    seen_relay_activity: bool,
    completed: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Nip46RequestEnvelope {
    pub request_id: String,
    pub method: Nip46CapabilityMethod,
    pub relay_urls: Vec<String>,
    pub event_json: String,
}

pub struct Nip46PairingUri(Zeroizing<String>);

impl Nip46PairingUri {
    pub fn expose(&self) -> &str {
        self.0.as_str()
    }
}

impl fmt::Debug for Nip46PairingUri {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("Nip46PairingUri([REDACTED])")
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Nip46ReportedSigner {
    pub user_identity: PublicIdentity,
    pub remote_signer_public_key: NostrPublicKeyHex,
    pub preview: Nip46PermissionPreview,
}

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub struct Nip46InboundEvent<'a> {
    pub relay_url: &'a str,
    pub event_json: &'a str,
    pub received_at: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ValidatedNip46Response {
    author: NostrPublicKeyHex,
    result: String,
}

pub struct Nip46Service {
    root: PathBuf,
}

impl Nip46Service {
    pub fn system(_channel: AppChannel) -> Self {
        Self::for_data_root(paths::data_dir().to_path_buf())
    }

    pub fn for_data_root(data_root: PathBuf) -> Self {
        Self {
            root: data_root.join("identity").join("nip46"),
        }
    }

    pub fn begin_bunker_pairing(
        &self,
        input: Nip46ConnectionInput,
        preview: Nip46PermissionPreview,
        fence: Nip46PairingFence,
    ) -> Result<Nip46PairingSession, Nip46Error> {
        if input.kind != Nip46UriKind::Bunker
            || preview.relays != input.relays
            || preview.expected_signer.as_ref() != Some(&input.public_key)
        {
            return Err(Nip46Error::InvalidPairingState);
        }
        self.begin_pairing(
            preview,
            fence,
            Some(input.public_key),
            input.secret,
            Nip46PairingState::AwaitingApproval,
        )
    }

    pub fn create_nostrconnect_pairing(
        &self,
        preview: Nip46PermissionPreview,
        fence: Nip46PairingFence,
        application_name: &str,
    ) -> Result<(Nip46PairingSession, Nip46PairingUri), Nip46Error> {
        if application_name.is_empty() || application_name.len() > 128 {
            return Err(Nip46Error::InvalidUri);
        }
        let secret = pairing_secret(fence, preview.issued_at);
        let session = self.begin_pairing(
            preview,
            fence,
            None,
            Some(secret.clone()),
            Nip46PairingState::AwaitingAcknowledgement,
        )?;
        let permission_string = capability_permission_string(&session.record.preview);
        let mut uri = Url::parse(&format!(
            "nostrconnect://{}",
            session.record.client_public_key.as_str()
        ))
        .map_err(|_| Nip46Error::InvalidUri)?;
        {
            let mut query = uri.query_pairs_mut();
            for relay in &session.record.preview.relays {
                query.append_pair("relay", relay);
            }
            query.append_pair("secret", secret.expose());
            query.append_pair("perms", &permission_string);
            query.append_pair("name", application_name);
        }
        Ok((session, Nip46PairingUri(Zeroizing::new(uri.to_string()))))
    }

    pub fn resume(&self, capability_ref: &str) -> Result<Nip46PairingSession, Nip46Error> {
        let paths = Nip46Paths::new(&self.root, capability_ref)?;
        let record: Nip46PairingRecord =
            read_json(&paths.pairing_path)?.ok_or(Nip46Error::InvalidPairingState)?;
        record.validate()?;
        Ok(Nip46PairingSession { paths, record })
    }

    pub fn load_capability(&self, capability_ref: &str) -> Result<Nip46Capability, Nip46Error> {
        let paths = Nip46Paths::new(&self.root, capability_ref)?;
        let capability: Nip46Capability =
            read_json(&paths.capability_path)?.ok_or(Nip46Error::InvalidPairingState)?;
        validate_capability(&capability)?;
        Ok(capability)
    }

    pub fn begin_operation(
        &self,
        capability_ref: &str,
        selection: &AccountSelectionToken,
        request: Nip46OperationRequest,
        now: u64,
        timeout_seconds: u64,
    ) -> Result<Nip46RequestEnvelope, Nip46Error> {
        if timeout_seconds == 0 || timeout_seconds > 120 {
            return Err(Nip46Error::InvalidLifetime);
        }
        let capability = self.load_capability(capability_ref)?;
        let paths = Nip46Paths::new(&self.root, capability_ref)?;
        if let Some(existing) = read_json::<Nip46RuntimeRecord>(&paths.runtime_request_path)?
            && !existing.pending.completed
        {
            return Err(Nip46Error::InvalidPairingState);
        }
        let (operation, method, params, expectation) = match request {
            Nip46OperationRequest::SignEvent {
                unsigned_event_json,
            } => {
                if unsigned_event_json.len() > NIP46_MAX_RESPONSE_BYTES {
                    return Err(Nip46Error::InvalidPermissions);
                }
                let mut event = UnsignedEvent::from_json(&unsigned_event_json)
                    .map_err(|_| Nip46Error::MalformedResponse)?;
                event
                    .verify_id()
                    .map_err(|_| Nip46Error::MalformedResponse)?;
                let kind = event.kind.as_u16();
                (
                    Nip46Operation::SignEvent { kind },
                    Nip46CapabilityMethod::SignEvent,
                    vec![unsigned_event_json],
                    Nip46RuntimeExpectation::SignEvent {
                        expected_event_id: event.id().to_hex(),
                        kind,
                    },
                )
            }
            Nip46OperationRequest::Nip44Encrypt {
                public_key,
                plaintext,
            } => (
                Nip46Operation::Nip44Encrypt,
                Nip46CapabilityMethod::Nip44Encrypt,
                vec![public_key.as_str().to_string(), plaintext],
                Nip46RuntimeExpectation::Nip44Encrypt,
            ),
            Nip46OperationRequest::Nip44Decrypt {
                public_key,
                ciphertext,
            } => (
                Nip46Operation::Nip44Decrypt,
                Nip46CapabilityMethod::Nip44Decrypt,
                vec![public_key.as_str().to_string(), ciphertext],
                Nip46RuntimeExpectation::Nip44Decrypt,
            ),
            Nip46OperationRequest::BulkDecrypt {
                public_key,
                ciphertext,
            } => (
                Nip46Operation::BulkDecrypt,
                Nip46CapabilityMethod::BulkDecrypt,
                vec![public_key.as_str().to_string(), ciphertext],
                Nip46RuntimeExpectation::BulkDecrypt,
            ),
        };
        capability.authorize(selection, operation, now)?;
        if params
            .iter()
            .any(|parameter| parameter.len() > NIP46_MAX_RESPONSE_BYTES)
        {
            return Err(Nip46Error::InvalidPermissions);
        }
        let request_id = random_request_id(capability_ref, now);
        let message = NostrConnectMessage::Request {
            id: request_id.clone(),
            method: method.protocol_method(),
            params,
        };
        let event_json = seal_message(
            &paths.client_secret_path,
            &capability.remote_signer_public_key,
            message,
            now,
        )?;
        let record = Nip46RuntimeRecord {
            capability_ref: capability_ref.to_string(),
            pending: Nip46PendingRequest {
                request_id: request_id.clone(),
                method,
                registry_generation: selection.generation,
                allowed_relays: capability.relays.clone(),
                expected_author: Some(capability.remote_signer_public_key),
                expires_at: now.saturating_add(timeout_seconds),
                issued_at: now,
                seen_relay_activity: false,
                completed: false,
            },
            expectation,
        };
        write_json(&paths.runtime_request_path, &record)?;
        Ok(Nip46RequestEnvelope {
            request_id,
            method,
            relay_urls: capability.relays,
            event_json,
        })
    }

    pub fn receive_operation_response(
        &self,
        capability_ref: &str,
        selection: &AccountSelectionToken,
        inbound: Nip46InboundEvent<'_>,
    ) -> Result<Nip46OperationResult, Nip46Error> {
        let paths = Nip46Paths::new(&self.root, capability_ref)?;
        let mut capability = self.load_capability(capability_ref)?;
        let mut record: Nip46RuntimeRecord =
            read_json(&paths.runtime_request_path)?.ok_or(Nip46Error::InvalidPairingState)?;
        if record.capability_ref != capability_ref {
            return Err(Nip46Error::InvalidPairingState);
        }
        if capability.state == Nip46CapabilityState::Revoked {
            record.pending.completed = true;
            write_json(&paths.runtime_request_path, &record)?;
            return Err(Nip46Error::Revoked);
        }
        let response = validate_response(
            &paths.client_secret_path,
            &capability.client_public_key,
            selection.generation,
            &mut record.pending,
            inbound,
        );
        write_json(&paths.runtime_request_path, &record)?;
        let response = response?;
        let result = match record.expectation {
            Nip46RuntimeExpectation::SignEvent {
                expected_event_id,
                kind,
            } => {
                let event =
                    Event::from_json(response.result).map_err(|_| Nip46Error::MalformedResponse)?;
                if event.verify().is_err()
                    || event.id.to_hex() != expected_event_id
                    || event.kind.as_u16() != kind
                    || event.pubkey.to_hex() != capability.user_identity.public_key_hex().as_str()
                {
                    return Err(Nip46Error::InvalidEvent);
                }
                Nip46OperationResult::SignedEvent(event)
            }
            Nip46RuntimeExpectation::Nip44Encrypt => {
                if response.result.is_empty() || response.result.len() > NIP46_MAX_RESPONSE_BYTES {
                    return Err(Nip46Error::MalformedResponse);
                }
                Nip46OperationResult::Ciphertext(response.result)
            }
            Nip46RuntimeExpectation::Nip44Decrypt | Nip46RuntimeExpectation::BulkDecrypt => {
                if response.result.len() > NIP46_MAX_RESPONSE_BYTES {
                    return Err(Nip46Error::MalformedResponse);
                }
                Nip46OperationResult::Plaintext(response.result)
            }
        };
        capability.last_successful_use = Some(inbound.received_at);
        write_json(&paths.capability_path, &capability)?;
        Ok(result)
    }

    pub fn operation_deadline_outcome(
        &self,
        capability_ref: &str,
        now: u64,
    ) -> Result<(), Nip46Error> {
        let paths = Nip46Paths::new(&self.root, capability_ref)?;
        let mut record: Nip46RuntimeRecord =
            read_json(&paths.runtime_request_path)?.ok_or(Nip46Error::InvalidPairingState)?;
        if record.pending.completed || now < record.pending.expires_at {
            return Ok(());
        }
        record.pending.completed = true;
        let error = if record.pending.seen_relay_activity {
            Nip46Error::Timeout
        } else {
            Nip46Error::Silence
        };
        write_json(&paths.runtime_request_path, &record)?;
        Err(error)
    }

    pub fn revoke(&self, capability_ref: &str) -> Result<Nip46Capability, Nip46Error> {
        let paths = Nip46Paths::new(&self.root, capability_ref)?;
        let mut capability = self.load_capability(capability_ref)?;
        delete_client_key_verified(&paths.client_secret_path)?;
        delete_client_key_verified(&paths.pairing_secret_path)?;
        capability.state = Nip46CapabilityState::Revoked;
        write_json(&paths.capability_path, &capability)?;
        if let Some(mut pairing) = read_json::<Nip46PairingRecord>(&paths.pairing_path)? {
            pairing.state = Nip46PairingState::Revoked;
            pairing.pending = None;
            write_json(&paths.pairing_path, &pairing)?;
        }
        Ok(capability)
    }

    pub(crate) fn bind_registered_account(
        &self,
        capability_ref: &str,
        account_ref: &AccountRef,
        account_generation: u64,
    ) -> Result<Nip46Capability, Nip46Error> {
        if account_generation == 0 {
            return Err(Nip46Error::StaleGeneration);
        }
        let paths = Nip46Paths::new(&self.root, capability_ref)?;
        let mut capability = self.load_capability(capability_ref)?;
        if capability.state != Nip46CapabilityState::AwaitingRegistration
            && capability.account_ref != *account_ref
        {
            return Err(Nip46Error::InvalidPairingState);
        }
        capability.account_ref = account_ref.clone();
        capability.account_generation = account_generation;
        capability.state = Nip46CapabilityState::Active;
        validate_capability(&capability)?;
        write_json(&paths.capability_path, &capability)?;
        Ok(capability)
    }

    pub(crate) fn rebind_generation(
        &self,
        capability_ref: &str,
        account_ref: &AccountRef,
        account_generation: u64,
    ) -> Result<Nip46Capability, Nip46Error> {
        let paths = Nip46Paths::new(&self.root, capability_ref)?;
        let mut capability = self.load_capability(capability_ref)?;
        if capability.account_ref != *account_ref
            || capability.state != Nip46CapabilityState::Active
            || account_generation == 0
        {
            return Err(Nip46Error::InvalidPairingState);
        }
        capability.account_generation = account_generation;
        write_json(&paths.capability_path, &capability)?;
        Ok(capability)
    }

    pub(crate) fn capability_directory(&self, capability_ref: &str) -> Result<PathBuf, Nip46Error> {
        let paths = Nip46Paths::new(&self.root, capability_ref)?;
        Ok(paths
            .capability_path
            .parent()
            .ok_or(Nip46Error::Storage)?
            .to_path_buf())
    }

    fn begin_pairing(
        &self,
        preview: Nip46PermissionPreview,
        fence: Nip46PairingFence,
        remote_signer_public_key: Option<NostrPublicKeyHex>,
        pairing_secret: Option<Nip46PairingSecret>,
        initial_state: Nip46PairingState,
    ) -> Result<Nip46PairingSession, Nip46Error> {
        preview.validate()?;
        let keys = Keys::generate();
        let client_public_key = NostrPublicKeyHex::new(keys.public_key().to_hex())
            .map_err(|_| Nip46Error::InvalidPairingState)?;
        let capability_ref = format!(
            "nip46-{}",
            hex::encode(Sha256::digest(
                format!(
                    "{}:{}:{}",
                    fence.registry_generation,
                    preview.issued_at,
                    client_public_key.as_str()
                )
                .as_bytes()
            ))
        );
        let paths = Nip46Paths::new(&self.root, &capability_ref)?;
        write_client_key(
            &paths.client_secret_path,
            keys.secret_key().to_secret_bytes(),
        )?;
        if let Some(secret) = pairing_secret {
            write_secret_text(&paths.pairing_secret_path, secret.expose())?;
        }
        let record = Nip46PairingRecord {
            schema: NIP46_PAIRING_SCHEMA.to_string(),
            capability_ref,
            fence,
            preview,
            client_public_key,
            remote_signer_public_key,
            user_identity: None,
            state: initial_state,
            pending: None,
            challenge: None,
        };
        record.validate()?;
        write_json(&paths.pairing_path, &record)?;
        Ok(Nip46PairingSession { paths, record })
    }
}

pub struct Nip46PairingSession {
    paths: Nip46Paths,
    record: Nip46PairingRecord,
}

impl Nip46PairingSession {
    pub fn capability_ref(&self) -> &str {
        &self.record.capability_ref
    }

    pub fn preview(&self) -> &Nip46PermissionPreview {
        &self.record.preview
    }

    pub fn client_public_key(&self) -> &NostrPublicKeyHex {
        &self.record.client_public_key
    }

    pub fn remote_signer_public_key(&self) -> Option<&NostrPublicKeyHex> {
        self.record.remote_signer_public_key.as_ref()
    }

    pub fn state(&self) -> Nip46PairingState {
        self.record.state.clone()
    }

    pub fn reject(&mut self) -> Result<(), Nip46Error> {
        if !matches!(
            self.record.state,
            Nip46PairingState::AwaitingApproval | Nip46PairingState::AwaitingFinalApproval
        ) {
            return Err(Nip46Error::InvalidPairingState);
        }
        delete_client_key_verified(&self.paths.client_secret_path)?;
        delete_client_key_verified(&self.paths.pairing_secret_path)?;
        self.record.state = Nip46PairingState::Rejected;
        self.record.pending = None;
        self.persist()
    }

    pub fn approve(
        &mut self,
        now: u64,
        timeout_seconds: u64,
    ) -> Result<Nip46RequestEnvelope, Nip46Error> {
        if self.record.state != Nip46PairingState::AwaitingApproval {
            return Err(Nip46Error::InvalidPairingState);
        }
        let remote = self
            .record
            .remote_signer_public_key
            .clone()
            .ok_or(Nip46Error::InvalidPairingState)?;
        let pairing_secret = read_secret_text(&self.paths.pairing_secret_path)?;
        let request = NostrConnectMessage::Request {
            id: request_id(&self.record, "connect"),
            method: NostrConnectMethod::Connect,
            params: vec![
                remote.as_str().to_string(),
                pairing_secret
                    .as_ref()
                    .map(|secret| secret.as_str().to_string())
                    .unwrap_or_default(),
                capability_permission_string(&self.record.preview),
                r#"{"name":"Omega"}"#.to_string(),
            ],
        };
        let envelope = self.begin_request(
            Nip46CapabilityMethod::LoginProof,
            Some(remote),
            request,
            now,
            timeout_seconds,
        )?;
        self.record.state = Nip46PairingState::AwaitingAcknowledgement;
        self.persist()?;
        Ok(envelope)
    }

    pub fn receive_nostrconnect_acknowledgement(
        &mut self,
        current_registry_generation: u64,
        inbound: Nip46InboundEvent<'_>,
        timeout_seconds: u64,
    ) -> Result<Nip46RequestEnvelope, Nip46Error> {
        if self.record.state != Nip46PairingState::AwaitingAcknowledgement
            || self.record.remote_signer_public_key.is_some()
        {
            return Err(Nip46Error::InvalidPairingState);
        }
        let expected_secret = read_secret_text(&self.paths.pairing_secret_path)?
            .ok_or(Nip46Error::InvalidPairingState)?;
        let response = validate_unbound_connect_response(
            &self.paths.client_secret_path,
            &self.record.client_public_key,
            current_registry_generation,
            self.record.fence,
            &self.record.preview.relays,
            inbound,
        );
        let response = match response {
            Ok(response) => response,
            Err(error) => {
                self.reject_terminal_pairing_response()?;
                return Err(error);
            }
        };
        if response.result != expected_secret.as_str() {
            self.reject_terminal_pairing_response()?;
            return Err(Nip46Error::Rejected);
        }
        delete_client_key_verified(&self.paths.pairing_secret_path)?;
        self.record.remote_signer_public_key = Some(response.author.clone());
        let request = NostrConnectMessage::Request {
            id: request_id(&self.record, "get-public-key"),
            method: NostrConnectMethod::GetPublicKey,
            params: Vec::new(),
        };
        let envelope = self.begin_request(
            Nip46CapabilityMethod::LoginProof,
            Some(response.author),
            request,
            inbound.received_at,
            timeout_seconds,
        )?;
        self.record.state = Nip46PairingState::AwaitingUserPublicKey;
        self.persist()?;
        Ok(envelope)
    }

    pub fn receive_acknowledgement(
        &mut self,
        current_registry_generation: u64,
        inbound: Nip46InboundEvent<'_>,
        timeout_seconds: u64,
    ) -> Result<Nip46RequestEnvelope, Nip46Error> {
        if self.record.state != Nip46PairingState::AwaitingAcknowledgement {
            return Err(Nip46Error::InvalidPairingState);
        }
        let expected_secret = read_secret_text(&self.paths.pairing_secret_path)?;
        let response = self.receive_response(current_registry_generation, inbound)?;
        if response.result != "ack"
            && expected_secret
                .as_ref()
                .is_none_or(|secret| response.result != secret.as_str())
        {
            return Err(Nip46Error::MalformedResponse);
        }
        delete_client_key_verified(&self.paths.pairing_secret_path)?;
        self.record.remote_signer_public_key = Some(response.author.clone());
        let request = NostrConnectMessage::Request {
            id: request_id(&self.record, "get-public-key"),
            method: NostrConnectMethod::GetPublicKey,
            params: Vec::new(),
        };
        let envelope = self.begin_request(
            Nip46CapabilityMethod::LoginProof,
            Some(response.author),
            request,
            inbound.received_at,
            timeout_seconds,
        )?;
        self.record.state = Nip46PairingState::AwaitingUserPublicKey;
        self.persist()?;
        Ok(envelope)
    }

    pub fn receive_user_public_key(
        &mut self,
        current_registry_generation: u64,
        inbound: Nip46InboundEvent<'_>,
        _timeout_seconds: u64,
    ) -> Result<Nip46ReportedSigner, Nip46Error> {
        if self.record.state != Nip46PairingState::AwaitingUserPublicKey {
            return Err(Nip46Error::InvalidPairingState);
        }
        let response = self.receive_response(current_registry_generation, inbound)?;
        let user_public_key =
            NostrPublicKeyHex::new(&response.result).map_err(|_| Nip46Error::MalformedResponse)?;
        let identity = PublicIdentity::from_public_key_hex(
            crate::IdentityRef::new(format!("omega-nostr-{}", user_public_key.as_str()))
                .map_err(|_| Nip46Error::MalformedResponse)?,
            user_public_key.as_str(),
        )
        .map_err(|_| Nip46Error::MalformedResponse)?;
        self.record.user_identity = Some(identity.clone());
        self.record.pending = None;
        self.record.state = Nip46PairingState::AwaitingFinalApproval;
        self.persist()?;
        Ok(Nip46ReportedSigner {
            user_identity: identity,
            remote_signer_public_key: self
                .record
                .remote_signer_public_key
                .clone()
                .ok_or(Nip46Error::InvalidPairingState)?,
            preview: self.record.preview.clone(),
        })
    }

    pub fn approve_reported_signer(
        &mut self,
        now: u64,
        timeout_seconds: u64,
    ) -> Result<Nip46RequestEnvelope, Nip46Error> {
        if self.record.state != Nip46PairingState::AwaitingFinalApproval {
            return Err(Nip46Error::InvalidPairingState);
        }
        let identity = self
            .record
            .user_identity
            .clone()
            .ok_or(Nip46Error::InvalidPairingState)?;
        let challenge = Nip46Challenge {
            content: challenge_content(&self.record, &identity),
            created_at: now,
        };
        let unsigned = UnsignedEvent::new(
            identity
                .public_key_hex()
                .public_key()
                .map_err(|_| Nip46Error::MalformedResponse)?,
            Timestamp::from_secs(challenge.created_at),
            Kind::Custom(NIP46_LOGIN_CHALLENGE_KIND),
            Vec::<Tag>::new(),
            challenge.content.clone(),
        );
        let request = NostrConnectMessage::Request {
            id: request_id(&self.record, "signed-challenge"),
            method: NostrConnectMethod::SignEvent,
            params: vec![
                unsigned
                    .try_as_json()
                    .map_err(|_| Nip46Error::MalformedResponse)?,
            ],
        };
        self.record.challenge = Some(challenge);
        let envelope = self.begin_request(
            Nip46CapabilityMethod::LoginProof,
            self.record.remote_signer_public_key.clone(),
            request,
            now,
            timeout_seconds,
        )?;
        self.record.state = Nip46PairingState::AwaitingSignedChallenge;
        self.persist()?;
        Ok(envelope)
    }

    pub fn receive_signed_challenge(
        &mut self,
        current_registry_generation: u64,
        inbound: Nip46InboundEvent<'_>,
    ) -> Result<Nip46Capability, Nip46Error> {
        if self.record.state != Nip46PairingState::AwaitingSignedChallenge {
            return Err(Nip46Error::InvalidPairingState);
        }
        let response = self.receive_response(current_registry_generation, inbound)?;
        let event = Event::from_json(&response.result).map_err(|_| Nip46Error::InvalidChallenge)?;
        let identity = self
            .record
            .user_identity
            .clone()
            .ok_or(Nip46Error::InvalidPairingState)?;
        let challenge = self
            .record
            .challenge
            .clone()
            .ok_or(Nip46Error::InvalidPairingState)?;
        if event.verify().is_err()
            || event.pubkey.to_hex() != identity.public_key_hex().as_str()
            || event.kind != Kind::Custom(NIP46_LOGIN_CHALLENGE_KIND)
            || event.created_at.as_secs() != challenge.created_at
            || !event.tags.is_empty()
            || event.content != challenge.content
        {
            return Err(Nip46Error::InvalidChallenge);
        }
        let capability = Nip46Capability {
            schema: NIP46_CAPABILITY_SCHEMA.to_string(),
            capability_ref: self.record.capability_ref.clone(),
            account_ref: account_ref_for_remote_identity(&identity)?,
            account_generation: 0,
            user_identity: identity,
            remote_signer_public_key: self
                .record
                .remote_signer_public_key
                .clone()
                .ok_or(Nip46Error::InvalidPairingState)?,
            client_public_key: self.record.client_public_key.clone(),
            methods: self.record.preview.methods.clone(),
            event_kinds: self.record.preview.event_kinds.clone(),
            relays: self.record.preview.relays.clone(),
            issued_at: self.record.preview.issued_at,
            expires_at: self.record.preview.expires_at,
            state: Nip46CapabilityState::AwaitingRegistration,
            last_successful_use: None,
        };
        validate_capability(&capability)?;
        write_json(&self.paths.capability_path, &capability)?;
        self.record.state = Nip46PairingState::AwaitingRegistration;
        self.record.pending = None;
        self.persist()?;
        Ok(capability)
    }

    pub fn deadline_outcome(&mut self, now: u64) -> Result<(), Nip46Error> {
        let pending = self
            .record
            .pending
            .as_mut()
            .ok_or(Nip46Error::InvalidPairingState)?;
        if now < pending.expires_at {
            return Ok(());
        }
        pending.completed = true;
        let error = if pending.seen_relay_activity {
            Nip46Error::Timeout
        } else {
            Nip46Error::Silence
        };
        self.persist()?;
        Err(error)
    }

    fn begin_request(
        &mut self,
        method: Nip46CapabilityMethod,
        expected_author: Option<NostrPublicKeyHex>,
        message: NostrConnectMessage,
        now: u64,
        timeout_seconds: u64,
    ) -> Result<Nip46RequestEnvelope, Nip46Error> {
        if timeout_seconds == 0 || timeout_seconds > 120 {
            return Err(Nip46Error::InvalidLifetime);
        }
        let request_id = message.id().to_string();
        let event_json = seal_message(
            &self.paths.client_secret_path,
            expected_author
                .as_ref()
                .ok_or(Nip46Error::InvalidPairingState)?,
            message,
            now,
        )?;
        self.record.pending = Some(Nip46PendingRequest {
            request_id: request_id.clone(),
            method,
            registry_generation: self.record.fence.registry_generation,
            allowed_relays: self.record.preview.relays.clone(),
            expected_author,
            expires_at: now.saturating_add(timeout_seconds),
            issued_at: now,
            seen_relay_activity: false,
            completed: false,
        });
        Ok(Nip46RequestEnvelope {
            request_id,
            method,
            relay_urls: self.record.preview.relays.clone(),
            event_json,
        })
    }

    fn receive_response(
        &mut self,
        current_registry_generation: u64,
        inbound: Nip46InboundEvent<'_>,
    ) -> Result<ValidatedNip46Response, Nip46Error> {
        let pending = self
            .record
            .pending
            .as_mut()
            .ok_or(Nip46Error::InvalidPairingState)?;
        let result = validate_response(
            &self.paths.client_secret_path,
            &self.record.client_public_key,
            current_registry_generation,
            pending,
            inbound,
        );
        if result.is_err() && !matches!(&result, Err(Nip46Error::DuplicateResponse)) {
            self.reject_terminal_pairing_response()?;
        }
        self.persist()?;
        result
    }

    fn reject_terminal_pairing_response(&mut self) -> Result<(), Nip46Error> {
        delete_client_key_verified(&self.paths.client_secret_path)?;
        delete_client_key_verified(&self.paths.pairing_secret_path)?;
        self.record.state = Nip46PairingState::Rejected;
        self.persist()
    }

    fn persist(&self) -> Result<(), Nip46Error> {
        self.record.validate()?;
        write_json(&self.paths.pairing_path, &self.record)
    }
}

impl Nip46PairingRecord {
    fn validate(&self) -> Result<(), Nip46Error> {
        if self.schema != NIP46_PAIRING_SCHEMA
            || self.capability_ref.is_empty()
            || self.fence.registry_generation == 0
            || self.client_public_key.public_key().is_err()
        {
            return Err(Nip46Error::InvalidPairingState);
        }
        self.preview.validate()
    }
}

#[derive(Clone)]
struct Nip46Paths {
    pairing_path: PathBuf,
    capability_path: PathBuf,
    client_secret_path: PathBuf,
    pairing_secret_path: PathBuf,
    runtime_request_path: PathBuf,
}

impl Nip46Paths {
    fn new(root: &Path, capability_ref: &str) -> Result<Self, Nip46Error> {
        if capability_ref.is_empty()
            || capability_ref.len() > 128
            || !capability_ref
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
        {
            return Err(Nip46Error::InvalidPairingState);
        }
        let directory = root.join(capability_ref);
        Ok(Self {
            pairing_path: directory.join("pairing.json"),
            capability_path: directory.join("capability.json"),
            client_secret_path: directory.join("client.secret"),
            pairing_secret_path: directory.join("pairing.secret"),
            runtime_request_path: directory.join("runtime-request.json"),
        })
    }
}

fn validate_response(
    client_secret_path: &Path,
    client_public_key: &NostrPublicKeyHex,
    current_registry_generation: u64,
    pending: &mut Nip46PendingRequest,
    inbound: Nip46InboundEvent<'_>,
) -> Result<ValidatedNip46Response, Nip46Error> {
    if current_registry_generation != pending.registry_generation {
        pending.completed = true;
        return Err(Nip46Error::StaleGeneration);
    }
    if pending.completed {
        return Err(Nip46Error::DuplicateResponse);
    }
    let relay = match normalize_relay(inbound.relay_url) {
        Ok(relay) => relay,
        Err(_) => {
            pending.completed = true;
            return Err(Nip46Error::WrongRelay);
        }
    };
    if !pending.allowed_relays.contains(&relay) {
        pending.completed = true;
        return Err(Nip46Error::WrongRelay);
    }
    pending.seen_relay_activity = true;
    if inbound.received_at >= pending.expires_at {
        pending.completed = true;
        return Err(Nip46Error::Timeout);
    }
    if inbound.event_json.len() > NIP46_MAX_RESPONSE_BYTES {
        pending.completed = true;
        return Err(Nip46Error::MalformedResponse);
    }
    let event = Event::from_json(inbound.event_json).map_err(|_| {
        pending.completed = true;
        Nip46Error::InvalidEvent
    })?;
    if event.verify().is_err()
        || event.kind != Kind::NostrConnect
        || event.created_at.as_secs() < pending.issued_at.saturating_sub(30)
        || event.created_at.as_secs() > inbound.received_at.saturating_add(30)
    {
        pending.completed = true;
        return Err(Nip46Error::InvalidEvent);
    }
    let author =
        NostrPublicKeyHex::new(event.pubkey.to_hex()).map_err(|_| Nip46Error::InvalidEvent)?;
    if pending
        .expected_author
        .as_ref()
        .is_some_and(|expected| expected != &author)
    {
        pending.completed = true;
        return Err(Nip46Error::WrongAuthor);
    }
    let recipient_public_keys: Vec<_> = event.tags.public_keys().collect();
    if recipient_public_keys.len() != 1
        || recipient_public_keys[0].to_hex() != client_public_key.as_str()
        || event.tags.len() != 1
    {
        pending.completed = true;
        return Err(Nip46Error::InvalidEvent);
    }
    let keys = read_client_keys(client_secret_path)?;
    let plaintext = nip44::decrypt(keys.secret_key(), &event.pubkey, event.content.as_bytes())
        .map_err(|_| {
            pending.completed = true;
            Nip46Error::MalformedCiphertext
        })?;
    let message = NostrConnectMessage::from_json(plaintext).map_err(|_| {
        pending.completed = true;
        Nip46Error::MalformedResponse
    })?;
    let NostrConnectMessage::Response { id, result, error } = message else {
        pending.completed = true;
        return Err(Nip46Error::MalformedResponse);
    };
    if id != pending.request_id {
        pending.completed = true;
        return Err(Nip46Error::WrongRequestId);
    }
    if error.is_some() {
        pending.completed = true;
        return Err(Nip46Error::Rejected);
    }
    let result = result.ok_or_else(|| {
        pending.completed = true;
        Nip46Error::MalformedResponse
    })?;
    pending.completed = true;
    Ok(ValidatedNip46Response { author, result })
}

fn validate_unbound_connect_response(
    client_secret_path: &Path,
    client_public_key: &NostrPublicKeyHex,
    current_registry_generation: u64,
    fence: Nip46PairingFence,
    allowed_relays: &[String],
    inbound: Nip46InboundEvent<'_>,
) -> Result<ValidatedNip46Response, Nip46Error> {
    if current_registry_generation != fence.registry_generation {
        return Err(Nip46Error::StaleGeneration);
    }
    let relay = normalize_relay(inbound.relay_url)?;
    if !allowed_relays.contains(&relay) {
        return Err(Nip46Error::WrongRelay);
    }
    if inbound.event_json.len() > NIP46_MAX_RESPONSE_BYTES {
        return Err(Nip46Error::MalformedResponse);
    }
    let event = Event::from_json(inbound.event_json).map_err(|_| Nip46Error::InvalidEvent)?;
    if event.verify().is_err()
        || event.kind != Kind::NostrConnect
        || event.created_at.as_secs() > inbound.received_at.saturating_add(30)
    {
        return Err(Nip46Error::InvalidEvent);
    }
    let recipient_public_keys: Vec<_> = event.tags.public_keys().collect();
    if event.tags.len() != 1
        || recipient_public_keys.len() != 1
        || recipient_public_keys[0].to_hex() != client_public_key.as_str()
    {
        return Err(Nip46Error::InvalidEvent);
    }
    let keys = read_client_keys(client_secret_path)?;
    let plaintext = nip44::decrypt(keys.secret_key(), &event.pubkey, event.content.as_bytes())
        .map_err(|_| Nip46Error::MalformedCiphertext)?;
    let message =
        NostrConnectMessage::from_json(plaintext).map_err(|_| Nip46Error::MalformedResponse)?;
    let NostrConnectMessage::Response { result, error, .. } = message else {
        return Err(Nip46Error::MalformedResponse);
    };
    if error.is_some() {
        return Err(Nip46Error::Rejected);
    }
    Ok(ValidatedNip46Response {
        author: NostrPublicKeyHex::new(event.pubkey.to_hex())
            .map_err(|_| Nip46Error::InvalidEvent)?,
        result: result.ok_or(Nip46Error::MalformedResponse)?,
    })
}

fn seal_message(
    client_secret_path: &Path,
    remote_signer_public_key: &NostrPublicKeyHex,
    message: NostrConnectMessage,
    created_at: u64,
) -> Result<String, Nip46Error> {
    let keys = read_client_keys(client_secret_path)?;
    let remote = remote_signer_public_key
        .public_key()
        .map_err(|_| Nip46Error::InvalidPairingState)?;
    EventBuilder::nostr_connect(&keys, remote, message)
        .map_err(|_| Nip46Error::MalformedCiphertext)?
        .custom_created_at(Timestamp::from_secs(created_at))
        .sign_with_keys(&keys)
        .map_err(|_| Nip46Error::InvalidEvent)?
        .try_as_json()
        .map_err(|_| Nip46Error::InvalidEvent)
}

fn request_id(record: &Nip46PairingRecord, phase: &str) -> String {
    let random = Keys::generate();
    hex::encode(Sha256::digest(
        [
            record.capability_ref.as_bytes(),
            &record.fence.registry_generation.to_be_bytes(),
            phase.as_bytes(),
            random.secret_key().to_secret_bytes().as_slice(),
        ]
        .concat(),
    ))
}

fn random_request_id(capability_ref: &str, now: u64) -> String {
    let random = Keys::generate();
    hex::encode(Sha256::digest(
        [
            capability_ref.as_bytes(),
            &now.to_be_bytes(),
            random.secret_key().to_secret_bytes().as_slice(),
        ]
        .concat(),
    ))
}

fn challenge_content(record: &Nip46PairingRecord, identity: &PublicIdentity) -> String {
    format!(
        "omega:nip46-login:{}:{}:{}:{}",
        record.capability_ref,
        record.fence.registry_generation,
        record.client_public_key.as_str(),
        identity.public_key_hex().as_str()
    )
}

fn pairing_secret(fence: Nip46PairingFence, issued_at: u64) -> Nip46PairingSecret {
    let random = Keys::generate();
    Nip46PairingSecret(Zeroizing::new(hex::encode(Sha256::digest(
        [
            random.secret_key().to_secret_bytes().as_slice(),
            &fence.registry_generation.to_be_bytes(),
            &issued_at.to_be_bytes(),
        ]
        .concat(),
    ))))
}

fn capability_permission_string(preview: &Nip46PermissionPreview) -> String {
    let mut permissions = Vec::new();
    for method in &preview.methods {
        if *method == Nip46CapabilityMethod::LoginProof {
            permissions.push(format!("sign_event:{NIP46_LOGIN_CHALLENGE_KIND}"));
        } else if *method == Nip46CapabilityMethod::SignEvent {
            for kind in &preview.event_kinds {
                permissions.push(format!("sign_event:{kind}"));
            }
        } else if *method != Nip46CapabilityMethod::BulkDecrypt {
            permissions.push(method.protocol_method().to_string());
        }
    }
    permissions.join(",")
}

fn parse_permission_strings(value: &str) -> Result<Vec<String>, Nip46Error> {
    if value.len() > 1_024 {
        return Err(Nip46Error::InvalidPermissions);
    }
    let mut permissions = Vec::new();
    for permission in value.split(',').filter(|permission| !permission.is_empty()) {
        if permission.len() > 64
            || !permission.bytes().all(|byte| {
                byte.is_ascii_lowercase() || byte.is_ascii_digit() || matches!(byte, b'_' | b':')
            })
        {
            return Err(Nip46Error::InvalidPermissions);
        }
        permissions.push(permission.to_string());
    }
    if permissions.len() > NIP46_MAX_METHODS + NIP46_MAX_EVENT_KINDS {
        return Err(Nip46Error::InvalidPermissions);
    }
    Ok(permissions)
}

fn normalize_relay(value: &str) -> Result<String, Nip46Error> {
    let relay = Url::parse(value).map_err(|_| Nip46Error::InvalidRelays)?;
    if relay.scheme() != "wss"
        || relay.host_str().is_none()
        || relay.fragment().is_some()
        || !relay.username().is_empty()
        || relay.password().is_some()
    {
        return Err(Nip46Error::InvalidRelays);
    }
    Ok(relay.to_string())
}

fn validate_relays(relays: &[String]) -> Result<(), Nip46Error> {
    if relays.is_empty() || relays.len() > NIP46_MAX_RELAYS {
        return Err(Nip46Error::InvalidRelays);
    }
    let mut unique = HashSet::new();
    for relay in relays {
        if normalize_relay(relay)? != *relay || !unique.insert(relay) {
            return Err(Nip46Error::InvalidRelays);
        }
    }
    Ok(())
}

fn validate_capability(capability: &Nip46Capability) -> Result<(), Nip46Error> {
    if capability.schema != NIP46_CAPABILITY_SCHEMA
        || capability.user_identity.validate().is_err()
        || (capability.state == Nip46CapabilityState::AwaitingRegistration
            && capability.account_generation != 0)
        || (capability.state != Nip46CapabilityState::AwaitingRegistration
            && capability.account_generation == 0)
        || capability.client_public_key == capability.remote_signer_public_key
    {
        return Err(Nip46Error::InvalidPairingState);
    }
    Nip46PermissionPreview {
        expected_signer: Some(capability.remote_signer_public_key.clone()),
        methods: capability.methods.clone(),
        event_kinds: capability.event_kinds.clone(),
        relays: capability.relays.clone(),
        issued_at: capability.issued_at,
        expires_at: capability.expires_at,
        recovery_dependency: Nip46RecoveryDependency::RemoteSigner,
    }
    .validate()
}

fn write_client_key(path: &Path, secret: [u8; 32]) -> Result<(), Nip46Error> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|_| Nip46Error::Storage)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            fs::set_permissions(parent, fs::Permissions::from_mode(0o700))
                .map_err(|_| Nip46Error::Storage)?;
        }
    }
    let mut file = AtomicWriteFile::open(path).map_err(|_| Nip46Error::Storage)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        file.as_file()
            .set_permissions(fs::Permissions::from_mode(0o600))
            .map_err(|_| Nip46Error::Storage)?;
    }
    file.write_all(&secret).map_err(|_| Nip46Error::Storage)?;
    file.commit().map_err(|_| Nip46Error::Storage)
}

fn write_secret_text(path: &Path, secret: &str) -> Result<(), Nip46Error> {
    if secret.is_empty() || secret.len() > 256 {
        return Err(Nip46Error::Storage);
    }
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|_| Nip46Error::Storage)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            fs::set_permissions(parent, fs::Permissions::from_mode(0o700))
                .map_err(|_| Nip46Error::Storage)?;
        }
    }
    let mut file = AtomicWriteFile::open(path).map_err(|_| Nip46Error::Storage)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        file.as_file()
            .set_permissions(fs::Permissions::from_mode(0o600))
            .map_err(|_| Nip46Error::Storage)?;
    }
    file.write_all(secret.as_bytes())
        .map_err(|_| Nip46Error::Storage)?;
    file.commit().map_err(|_| Nip46Error::Storage)
}

fn read_secret_text(path: &Path) -> Result<Option<Zeroizing<String>>, Nip46Error> {
    match fs::read(path) {
        Ok(bytes) if !bytes.is_empty() && bytes.len() <= 256 => String::from_utf8(bytes)
            .map(Zeroizing::new)
            .map(Some)
            .map_err(|_| Nip46Error::Storage),
        Ok(_) => Err(Nip46Error::Storage),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(_) => Err(Nip46Error::Storage),
    }
}

fn read_client_keys(path: &Path) -> Result<Keys, Nip46Error> {
    let bytes = Zeroizing::new(fs::read(path).map_err(|_| Nip46Error::Storage)?);
    let secret: [u8; 32] = bytes
        .as_slice()
        .try_into()
        .map_err(|_| Nip46Error::Storage)?;
    Ok(Keys::new(
        nostr::SecretKey::from_slice(&secret).map_err(|_| Nip46Error::Storage)?,
    ))
}

fn delete_client_key_verified(path: &Path) -> Result<(), Nip46Error> {
    match fs::remove_file(path) {
        Ok(()) => {}
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(_) => return Err(Nip46Error::Storage),
    }
    if path.try_exists().map_err(|_| Nip46Error::Storage)? {
        return Err(Nip46Error::Storage);
    }
    Ok(())
}

fn read_json<T: serde::de::DeserializeOwned>(path: &Path) -> Result<Option<T>, Nip46Error> {
    match fs::read(path) {
        Ok(bytes) => serde_json::from_slice(&bytes)
            .map(Some)
            .map_err(|_| Nip46Error::Storage),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(_) => Err(Nip46Error::Storage),
    }
}

fn write_json(path: &Path, value: &impl Serialize) -> Result<(), Nip46Error> {
    let serialized = serde_json::to_vec_pretty(value).map_err(|_| Nip46Error::Storage)?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|_| Nip46Error::Storage)?;
    }
    let mut file = AtomicWriteFile::open(path).map_err(|_| Nip46Error::Storage)?;
    file.write_all(&serialized)
        .map_err(|_| Nip46Error::Storage)?;
    file.write_all(b"\n").map_err(|_| Nip46Error::Storage)?;
    file.commit().map_err(|_| Nip46Error::Storage)
}

fn account_ref_for_remote_identity(identity: &PublicIdentity) -> Result<AccountRef, Nip46Error> {
    AccountRef::new(format!(
        "omega-account-{}",
        identity.public_key_hex().as_str()
    ))
    .map_err(|_| Nip46Error::InvalidPairingState)
}

#[cfg(test)]
mod tests {
    use std::error::Error;

    use tempfile::TempDir;

    use super::*;

    const RELAY: &str = "wss://relay.example/";
    const NOW: u64 = 2_000_000_000;

    fn key_hex(keys: &Keys) -> Result<NostrPublicKeyHex, Nip46Error> {
        NostrPublicKeyHex::new(keys.public_key().to_hex()).map_err(|_| Nip46Error::InvalidEvent)
    }

    fn preview(keys: &Keys) -> Result<Nip46PermissionPreview, Nip46Error> {
        Nip46PermissionPreview::omega_first_profile(
            Some(key_hex(keys)?),
            vec![RELAY.to_string()],
            NOW,
            NOW + 3_600,
        )
    }

    fn bunker_session(
        directory: &TempDir,
        signer: &Keys,
    ) -> Result<(Nip46Service, Nip46PairingSession), Nip46Error> {
        let input = Nip46ConnectionInput::parse(&format!(
            "bunker://{}?relay={RELAY}&secret=pairing-secret",
            signer.public_key().to_hex()
        ))?;
        let service = Nip46Service::for_data_root(directory.path().to_path_buf());
        let session =
            service.begin_bunker_pairing(input, preview(signer)?, Nip46PairingFence::new(7)?)?;
        Ok((service, session))
    }

    fn response(
        signer: &Keys,
        recipient: nostr::PublicKey,
        request_id: &str,
        result: Option<String>,
        error: Option<String>,
        created_at: u64,
    ) -> Result<String, Nip46Error> {
        EventBuilder::nostr_connect(
            signer,
            recipient,
            NostrConnectMessage::Response {
                id: request_id.to_string(),
                result,
                error,
            },
        )
        .map_err(|_| Nip46Error::MalformedCiphertext)?
        .custom_created_at(Timestamp::from_secs(created_at))
        .sign_with_keys(signer)
        .map_err(|_| Nip46Error::InvalidEvent)?
        .try_as_json()
        .map_err(|_| Nip46Error::InvalidEvent)
    }

    fn approve_session(
        session: &mut Nip46PairingSession,
    ) -> Result<Nip46RequestEnvelope, Nip46Error> {
        session.approve(NOW, 30)
    }

    #[test]
    fn parses_current_uris_and_rejects_unbounded_or_legacy_shapes() -> Result<(), Box<dyn Error>> {
        let keys = Keys::generate();
        let public_key = keys.public_key().to_hex();
        let bunker = Nip46ConnectionInput::parse(&format!(
            "bunker://{public_key}?relay={RELAY}&secret=keep-private"
        ))?;
        assert_eq!(bunker.kind(), Nip46UriKind::Bunker);
        assert!(!format!("{bunker:?}").contains("keep-private"));

        let current = Nip46ConnectionInput::parse(&format!(
            "nostrconnect://{public_key}?relay={RELAY}&secret=current-secret&perms=sign_event%3A9"
        ))?;
        assert_eq!(current.kind(), Nip46UriKind::NostrConnect);
        assert_eq!(current.declared_permissions(), ["sign_event:9"]);
        assert!(!format!("{current:?}").contains("current-secret"));

        assert_eq!(
            Nip46ConnectionInput::parse(&format!(
                "nostrconnect://{public_key}?relay={RELAY}&metadata=%7B%7D"
            )),
            Err(Nip46Error::InvalidUri)
        );
        assert_eq!(
            Nip46ConnectionInput::parse(&format!("nostrconnect://{public_key}?relay={RELAY}")),
            Err(Nip46Error::InvalidUri)
        );
        let excessive_relays = (0..=NIP46_MAX_RELAYS)
            .map(|index| format!("relay=wss%3A%2F%2Fr{index}.example"))
            .collect::<Vec<_>>()
            .join("&");
        assert_eq!(
            Nip46ConnectionInput::parse(&format!("bunker://{public_key}?{excessive_relays}")),
            Err(Nip46Error::InvalidRelays)
        );
        Ok(())
    }

    #[test]
    fn first_profile_declares_only_exact_login_and_wave_one_signing_permissions()
    -> Result<(), Box<dyn Error>> {
        let signer = Keys::generate();
        let preview = preview(&signer)?;
        assert_eq!(
            preview.methods,
            BTreeSet::from([
                Nip46CapabilityMethod::LoginProof,
                Nip46CapabilityMethod::SignEvent,
            ])
        );
        assert_eq!(
            preview.event_kinds,
            BTreeSet::from([9, 1111, 1984, 22242, 27235])
        );
        let permission_string = capability_permission_string(&preview);
        assert!(permission_string.contains("sign_event:24246"));
        assert!(
            !permission_string
                .split(',')
                .any(|permission| permission == "sign_event")
        );
        assert!(!permission_string.contains("nip44"));
        assert!(
            Nip46PermissionPreview::bulk_decrypt_profile(
                key_hex(&signer)?,
                vec![RELAY.to_string()],
                NOW,
                NOW + 60,
            )?
            .methods
            .contains(&Nip46CapabilityMethod::BulkDecrypt)
        );
        Ok(())
    }

    #[test]
    fn nostrconnect_waits_for_the_secret_bound_inbound_signer() -> Result<(), Box<dyn Error>> {
        let directory = TempDir::new()?;
        let signer = Keys::generate();
        let service = Nip46Service::for_data_root(directory.path().to_path_buf());
        let preview = Nip46PermissionPreview::omega_first_profile(
            None,
            vec![RELAY.to_string()],
            NOW,
            NOW + 3_600,
        )?;
        let (mut session, pairing_uri) =
            service.create_nostrconnect_pairing(preview, Nip46PairingFence::new(7)?, "Omega")?;
        let parsed = Nip46ConnectionInput::parse(pairing_uri.expose())?;
        assert_eq!(parsed.kind(), Nip46UriKind::NostrConnect);
        assert_eq!(parsed.public_key(), session.client_public_key());

        let pairing_secret = read_secret_text(&session.paths.pairing_secret_path)?
            .ok_or(Nip46Error::InvalidPairingState)?;
        let acknowledgement = response(
            &signer,
            session.client_public_key().public_key()?,
            pairing_secret.as_str(),
            Some(pairing_secret.as_str().to_string()),
            None,
            NOW + 1,
        )?;
        let get_public_key = session.receive_nostrconnect_acknowledgement(
            7,
            Nip46InboundEvent {
                relay_url: RELAY,
                event_json: &acknowledgement,
                received_at: NOW + 1,
            },
            30,
        )?;

        assert_eq!(get_public_key.method, Nip46CapabilityMethod::LoginProof);
        assert_eq!(session.state(), Nip46PairingState::AwaitingUserPublicKey);
        assert!(!session.paths.pairing_secret_path.exists());
        Ok(())
    }

    #[cfg(unix)]
    #[test]
    fn pairing_material_is_owner_only_and_rejection_deletes_it() -> Result<(), Box<dyn Error>> {
        use std::os::unix::fs::PermissionsExt as _;

        let directory = TempDir::new()?;
        let signer = Keys::generate();
        let (_service, mut session) = bunker_session(&directory, &signer)?;
        assert_eq!(
            fs::metadata(&session.paths.client_secret_path)?
                .permissions()
                .mode()
                & 0o777,
            0o600
        );
        assert_eq!(
            fs::metadata(
                session
                    .paths
                    .client_secret_path
                    .parent()
                    .ok_or(Nip46Error::Storage)?
            )?
            .permissions()
            .mode()
                & 0o777,
            0o700
        );
        session.reject()?;
        assert!(!session.paths.client_secret_path.exists());
        assert!(!session.paths.pairing_secret_path.exists());
        Ok(())
    }

    #[test]
    fn acknowledgement_and_signed_challenge_activate_same_key_signer_identity()
    -> Result<(), Box<dyn Error>> {
        let directory = TempDir::new()?;
        let signer_and_user = Keys::generate();
        let (service, mut session) = bunker_session(&directory, &signer_and_user)?;
        let connect = approve_session(&mut session)?;
        let acknowledgement = response(
            &signer_and_user,
            session.record.client_public_key.public_key()?,
            &connect.request_id,
            Some("ack".to_string()),
            None,
            NOW + 1,
        )?;
        let get_public_key = session.receive_acknowledgement(
            7,
            Nip46InboundEvent {
                relay_url: RELAY,
                event_json: &acknowledgement,
                received_at: NOW + 1,
            },
            30,
        )?;
        let public_key_response = response(
            &signer_and_user,
            session.record.client_public_key.public_key()?,
            &get_public_key.request_id,
            Some(signer_and_user.public_key().to_hex()),
            None,
            NOW + 2,
        )?;
        let reported = session.receive_user_public_key(
            7,
            Nip46InboundEvent {
                relay_url: RELAY,
                event_json: &public_key_response,
                received_at: NOW + 2,
            },
            30,
        )?;
        assert_eq!(
            reported.remote_signer_public_key,
            *reported.user_identity.public_key_hex()
        );
        let challenge_request = session.approve_reported_signer(NOW + 3, 30)?;
        let challenge = session
            .record
            .challenge
            .as_ref()
            .ok_or(Nip46Error::InvalidPairingState)?;
        let signed_challenge = EventBuilder::new(
            Kind::Custom(NIP46_LOGIN_CHALLENGE_KIND),
            challenge.content.clone(),
        )
        .custom_created_at(Timestamp::from_secs(challenge.created_at))
        .sign_with_keys(&signer_and_user)?;
        let challenge_response = response(
            &signer_and_user,
            session.record.client_public_key.public_key()?,
            &challenge_request.request_id,
            Some(signed_challenge.try_as_json()?),
            None,
            NOW + 4,
        )?;
        let capability = session.receive_signed_challenge(
            7,
            Nip46InboundEvent {
                relay_url: RELAY,
                event_json: &challenge_response,
                received_at: NOW + 4,
            },
        )?;
        assert_eq!(capability.state, Nip46CapabilityState::AwaitingRegistration);
        assert_eq!(session.state(), Nip46PairingState::AwaitingRegistration);
        assert!(service.load_capability(&capability.capability_ref).is_ok());

        let capability = service.bind_registered_account(
            &capability.capability_ref,
            &capability.account_ref,
            8,
        )?;
        let selection = AccountSelectionToken {
            account_ref: capability.account_ref.clone(),
            identity: capability.user_identity.clone(),
            generation: 8,
        };
        let signed_event = EventBuilder::new(Kind::Custom(9), "wave-one message")
            .custom_created_at(Timestamp::from_secs(NOW + 5))
            .sign_with_keys(&signer_and_user)?;
        let unsigned_event = UnsignedEvent::new(
            signer_and_user.public_key(),
            Timestamp::from_secs(NOW + 5),
            Kind::Custom(9),
            Vec::<Tag>::new(),
            "wave-one message",
        );
        let runtime_request = service.begin_operation(
            &capability.capability_ref,
            &selection,
            Nip46OperationRequest::SignEvent {
                unsigned_event_json: unsigned_event.try_as_json()?,
            },
            NOW + 5,
            30,
        )?;
        let runtime_response = response(
            &signer_and_user,
            session.record.client_public_key.public_key()?,
            &runtime_request.request_id,
            Some(signed_event.try_as_json()?),
            None,
            NOW + 6,
        )?;
        assert!(matches!(
            service.receive_operation_response(
                &capability.capability_ref,
                &selection,
                Nip46InboundEvent {
                    relay_url: RELAY,
                    event_json: &runtime_response,
                    received_at: NOW + 6,
                },
            )?,
            Nip46OperationResult::SignedEvent(_)
        ));
        let undeclared = UnsignedEvent::new(
            signer_and_user.public_key(),
            Timestamp::from_secs(NOW + 7),
            Kind::Custom(1),
            Vec::<Tag>::new(),
            "not declared",
        );
        assert_eq!(
            service.begin_operation(
                &capability.capability_ref,
                &selection,
                Nip46OperationRequest::SignEvent {
                    unsigned_event_json: undeclared.try_as_json()?,
                },
                NOW + 7,
                30,
            ),
            Err(Nip46Error::UndeclaredCapability)
        );
        service.revoke(&capability.capability_ref)?;
        assert!(!session.paths.client_secret_path.exists());
        assert_eq!(
            service
                .load_capability(&capability.capability_ref)?
                .authorize(&selection, Nip46Operation::SignEvent { kind: 9 }, NOW + 8),
            Err(Nip46Error::Revoked)
        );
        Ok(())
    }

    #[test]
    fn response_failures_are_distinct_and_terminal() -> Result<(), Box<dyn Error>> {
        let directory = TempDir::new()?;
        let signer = Keys::generate();
        let (_service, mut session) = bunker_session(&directory, &signer)?;
        let request = approve_session(&mut session)?;
        let wrong_author = Keys::generate();
        let event = response(
            &wrong_author,
            session.record.client_public_key.public_key()?,
            &request.request_id,
            Some("ack".to_string()),
            None,
            NOW + 1,
        )?;
        let inbound = Nip46InboundEvent {
            relay_url: RELAY,
            event_json: &event,
            received_at: NOW + 1,
        };
        assert_eq!(
            session.receive_response(7, inbound),
            Err(Nip46Error::WrongAuthor)
        );
        assert_eq!(
            session.receive_response(7, inbound),
            Err(Nip46Error::DuplicateResponse)
        );

        let directory = TempDir::new()?;
        let (_service, mut session) = bunker_session(&directory, &signer)?;
        approve_session(&mut session)?;
        let wrong_id = response(
            &signer,
            session.record.client_public_key.public_key()?,
            "wrong-id",
            Some("ack".to_string()),
            None,
            NOW + 1,
        )?;
        assert_eq!(
            session.receive_response(
                7,
                Nip46InboundEvent {
                    relay_url: RELAY,
                    event_json: &wrong_id,
                    received_at: NOW + 1,
                },
            ),
            Err(Nip46Error::WrongRequestId)
        );

        let directory = TempDir::new()?;
        let (_service, mut session) = bunker_session(&directory, &signer)?;
        let request = approve_session(&mut session)?;
        let rejected = response(
            &signer,
            session.record.client_public_key.public_key()?,
            &request.request_id,
            None,
            Some("denied".to_string()),
            NOW + 1,
        )?;
        assert_eq!(
            session.receive_response(
                7,
                Nip46InboundEvent {
                    relay_url: RELAY,
                    event_json: &rejected,
                    received_at: NOW + 1,
                },
            ),
            Err(Nip46Error::Rejected)
        );
        assert!(!session.paths.client_secret_path.exists());
        assert!(!session.paths.pairing_secret_path.exists());
        Ok(())
    }

    #[test]
    fn relay_generation_ciphertext_and_deadline_failures_are_distinct() -> Result<(), Box<dyn Error>>
    {
        let signer = Keys::generate();

        let directory = TempDir::new()?;
        let (_service, mut session) = bunker_session(&directory, &signer)?;
        let request = approve_session(&mut session)?;
        let valid = response(
            &signer,
            session.record.client_public_key.public_key()?,
            &request.request_id,
            Some("ack".to_string()),
            None,
            NOW + 1,
        )?;
        assert_eq!(
            session.receive_response(
                7,
                Nip46InboundEvent {
                    relay_url: "wss://other.example/",
                    event_json: &valid,
                    received_at: NOW + 1,
                },
            ),
            Err(Nip46Error::WrongRelay)
        );

        let directory = TempDir::new()?;
        let (_service, mut session) = bunker_session(&directory, &signer)?;
        let request = approve_session(&mut session)?;
        let valid = response(
            &signer,
            session.record.client_public_key.public_key()?,
            &request.request_id,
            Some("ack".to_string()),
            None,
            NOW + 1,
        )?;
        assert_eq!(
            session.receive_response(
                8,
                Nip46InboundEvent {
                    relay_url: RELAY,
                    event_json: &valid,
                    received_at: NOW + 1,
                },
            ),
            Err(Nip46Error::StaleGeneration)
        );

        let directory = TempDir::new()?;
        let (_service, mut session) = bunker_session(&directory, &signer)?;
        approve_session(&mut session)?;
        let malformed = EventBuilder::new(Kind::NostrConnect, "not-nip44")
            .tag(Tag::public_key(
                session.record.client_public_key.public_key()?,
            ))
            .custom_created_at(Timestamp::from_secs(NOW + 1))
            .sign_with_keys(&signer)?
            .try_as_json()?;
        assert_eq!(
            session.receive_response(
                7,
                Nip46InboundEvent {
                    relay_url: RELAY,
                    event_json: &malformed,
                    received_at: NOW + 1,
                },
            ),
            Err(Nip46Error::MalformedCiphertext)
        );

        let directory = TempDir::new()?;
        let (_service, mut silent) = bunker_session(&directory, &signer)?;
        approve_session(&mut silent)?;
        assert_eq!(silent.deadline_outcome(NOW + 31), Err(Nip46Error::Silence));

        let directory = TempDir::new()?;
        let (_service, mut active) = bunker_session(&directory, &signer)?;
        approve_session(&mut active)?;
        if let Some(pending) = active.record.pending.as_mut() {
            pending.seen_relay_activity = true;
        }
        assert_eq!(active.deadline_outcome(NOW + 31), Err(Nip46Error::Timeout));
        Ok(())
    }
}
