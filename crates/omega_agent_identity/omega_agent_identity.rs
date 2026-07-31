use std::{
    collections::BTreeSet,
    fs,
    io::{self, Read as _, Write as _},
    path::{Path, PathBuf},
    sync::{Mutex, MutexGuard, OnceLock},
};

use atomic_write_file::AtomicWriteFile;
use nostr::{
    EventBuilder, JsonUtil as _, Keys, Kind, PublicKey, Tag, Timestamp,
    secp256k1::{Message, schnorr::Signature},
};
use omega_identity::{
    AccountRef, AccountSelectionToken, AdmittedSigningRequest, AgentIdentityRef, IdentityRef,
    NostrPublicKeyHex, OwnerAttestationRequest, OwnerAttestationResult, ProofRef, PublicIdentity,
    ReceiptRef, ResourceRef, SignerRef, SigningAuthorizationContext, SigningResult,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};
use thiserror::Error;
use zeroize::Zeroizing;

const AGENT_RECORD_SCHEMA: &str = "openagents.omega.agent-identity.v1";
const AGENT_GRANT_SCHEMA: &str = "openagents.omega.agent-grant.v1";
const MAX_FILE_BYTES: u64 = 1024 * 1024;
const MAX_LABEL_BYTES: usize = 128;
const MAX_SCOPE_ITEMS: usize = 128;

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum AgentIdentityError {
    #[error("the agent identity request is invalid")]
    InvalidRequest,
    #[error("the owner attestation is invalid")]
    InvalidOwnerAttestation,
    #[error("the agent identity already exists")]
    AlreadyExists,
    #[error("the agent identity was not found")]
    NotFound,
    #[error("the agent grant does not authorize this operation")]
    NotAuthorized,
    #[error("the agent grant has expired")]
    Expired,
    #[error("the agent grant was revoked")]
    Revoked,
    #[error("the selected account generation is stale")]
    StaleGeneration,
    #[error("agent identity storage failed")]
    Storage,
    #[error("stored agent identity data is invalid")]
    InvalidStoredData,
    #[error("agent signing failed")]
    Signing,
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentMethod {
    SignEvent,
    Nip42RelayAuthentication,
    NipAaRelayAuthentication,
    Nip44Encrypt,
    Nip44Decrypt,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AgentGrantScope {
    pub methods: BTreeSet<AgentMethod>,
    pub event_kinds: BTreeSet<u16>,
    pub rooms_or_tenants: BTreeSet<String>,
}

impl AgentGrantScope {
    fn validate(&self) -> Result<(), AgentIdentityError> {
        if self.methods.is_empty()
            || self.methods.len() > MAX_SCOPE_ITEMS
            || self.event_kinds.len() > MAX_SCOPE_ITEMS
            || self.rooms_or_tenants.is_empty()
            || self.rooms_or_tenants.len() > MAX_SCOPE_ITEMS
            || self
                .rooms_or_tenants
                .iter()
                .any(|value| !valid_reference(value))
        {
            return Err(AgentIdentityError::InvalidRequest);
        }
        if self.methods.contains(&AgentMethod::SignEvent) && self.event_kinds.is_empty() {
            return Err(AgentIdentityError::InvalidRequest);
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AgentGrant {
    pub schema: String,
    pub grant_ref: ReceiptRef,
    pub owner_account_ref: AccountRef,
    pub owner_identity_ref: IdentityRef,
    pub owner_public_key_hex: NostrPublicKeyHex,
    pub agent_identity_ref: AgentIdentityRef,
    pub agent_public_key_hex: NostrPublicKeyHex,
    pub signer_ref: SignerRef,
    pub scope: AgentGrantScope,
    pub account_generation: u64,
    pub issued_at: u64,
    pub expires_at: u64,
    pub owner_attestation_ref: ProofRef,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub revoked_at: Option<u64>,
}

impl AgentGrant {
    fn validate(&self) -> Result<(), AgentIdentityError> {
        self.scope.validate()?;
        if self.schema != AGENT_GRANT_SCHEMA
            || self.account_generation == 0
            || self.issued_at >= self.expires_at
            || self.owner_public_key_hex == self.agent_public_key_hex
            || self.owner_account_ref.as_str() == self.agent_identity_ref.as_str()
            || self.owner_identity_ref.as_str() == self.agent_identity_ref.as_str()
            || self
                .revoked_at
                .is_some_and(|revoked_at| revoked_at < self.issued_at)
        {
            return Err(AgentIdentityError::InvalidRequest);
        }
        Ok(())
    }

    fn conditions(&self) -> Result<String, AgentIdentityError> {
        let bounded = UnsignedAgentGrant {
            schema: &self.schema,
            grant_ref: &self.grant_ref,
            owner_account_ref: &self.owner_account_ref,
            owner_identity_ref: &self.owner_identity_ref,
            owner_public_key_hex: &self.owner_public_key_hex,
            agent_identity_ref: &self.agent_identity_ref,
            agent_public_key_hex: &self.agent_public_key_hex,
            signer_ref: &self.signer_ref,
            scope: &self.scope,
            account_generation: self.account_generation,
            issued_at: self.issued_at,
            expires_at: self.expires_at,
        };
        let encoded =
            serde_json::to_vec(&bounded).map_err(|_| AgentIdentityError::InvalidRequest)?;
        Ok(format!("omega-agent-grant-v1:{}", digest(&encoded)))
    }
}

#[derive(Serialize)]
struct UnsignedAgentGrant<'a> {
    schema: &'a str,
    grant_ref: &'a ReceiptRef,
    owner_account_ref: &'a AccountRef,
    owner_identity_ref: &'a IdentityRef,
    owner_public_key_hex: &'a NostrPublicKeyHex,
    agent_identity_ref: &'a AgentIdentityRef,
    agent_public_key_hex: &'a NostrPublicKeyHex,
    signer_ref: &'a SignerRef,
    scope: &'a AgentGrantScope,
    account_generation: u64,
    issued_at: u64,
    expires_at: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentIdentityPlan {
    pub request_ref: ReceiptRef,
    pub grant_ref: ReceiptRef,
    pub owner_account_ref: AccountRef,
    pub owner_identity: PublicIdentity,
    pub agent_identity_ref: AgentIdentityRef,
    pub label: String,
    pub scope: AgentGrantScope,
    pub account_generation: u64,
    pub issued_at: u64,
    pub expires_at: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreparedAgentIdentity {
    pub request_ref: ReceiptRef,
    pub agent_identity_ref: AgentIdentityRef,
    pub agent_public_key_hex: NostrPublicKeyHex,
    pub owner_attestation_request: OwnerAttestationRequest,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AgentIdentityProjection {
    pub owner_account_ref: AccountRef,
    pub owner_identity_ref: IdentityRef,
    pub owner_public_key_hex: NostrPublicKeyHex,
    pub agent_identity_ref: AgentIdentityRef,
    pub agent_public_key_hex: NostrPublicKeyHex,
    pub label: String,
    pub signer_ref: SignerRef,
    pub owner_auth_tag: Vec<String>,
    pub grant: AgentGrant,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_used_at: Option<u64>,
}

impl AgentIdentityProjection {
    pub fn is_active_at(&self, account_generation: u64, now: u64) -> bool {
        self.grant.revoked_at.is_none()
            && self.grant.account_generation == account_generation
            && now < self.grant.expires_at
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentAuthorizationRequest {
    pub request_ref: ReceiptRef,
    pub owner_account_ref: AccountRef,
    pub account_generation: u64,
    pub agent_identity_ref: AgentIdentityRef,
    pub grant_ref: ReceiptRef,
    pub method: AgentMethod,
    pub event_kind: Option<u16>,
    pub room_or_tenant: String,
    pub destination_resource_ref: ResourceRef,
    pub issued_at: u64,
    pub expires_at: u64,
    pub signing_context: Option<SigningAuthorizationContext>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentAuthorization {
    request: AgentAuthorizationRequest,
    agent_public_key_hex: NostrPublicKeyHex,
    owner_public_key_hex: NostrPublicKeyHex,
    owner_attestation_ref: ProofRef,
    signer_ref: SignerRef,
    owner_auth_tag: Vec<String>,
    grant_conditions: String,
    grant_expires_at: u64,
}

impl AgentAuthorization {
    pub fn request_ref(&self) -> &ReceiptRef {
        &self.request.request_ref
    }

    pub fn agent_public_key_hex(&self) -> &NostrPublicKeyHex {
        &self.agent_public_key_hex
    }

    pub fn owner_public_key_hex(&self) -> &NostrPublicKeyHex {
        &self.owner_public_key_hex
    }

    pub fn owner_attestation_ref(&self) -> &ProofRef {
        &self.owner_attestation_ref
    }

    pub fn signer_ref(&self) -> &SignerRef {
        &self.signer_ref
    }

    pub fn owner_auth_tag(&self) -> &[String] {
        &self.owner_auth_tag
    }

    pub fn grant_conditions(&self) -> &str {
        &self.grant_conditions
    }

    pub fn grant_expires_at(&self) -> u64 {
        self.grant_expires_at
    }

    pub fn owner_account_ref(&self) -> &AccountRef {
        &self.request.owner_account_ref
    }

    pub fn account_generation(&self) -> u64 {
        self.request.account_generation
    }

    pub fn agent_identity_ref(&self) -> &AgentIdentityRef {
        &self.request.agent_identity_ref
    }

    pub fn grant_ref(&self) -> &ReceiptRef {
        &self.request.grant_ref
    }

    pub fn method(&self) -> AgentMethod {
        self.request.method
    }

    pub fn event_kind(&self) -> Option<u16> {
        self.request.event_kind
    }

    pub fn room_or_tenant(&self) -> &str {
        &self.request.room_or_tenant
    }

    pub fn destination_resource_ref(&self) -> &ResourceRef {
        &self.request.destination_resource_ref
    }

    pub fn issued_at(&self) -> u64 {
        self.request.issued_at
    }

    pub fn expires_at(&self) -> u64 {
        self.request.expires_at
    }

    pub fn signing_context(&self) -> Option<&SigningAuthorizationContext> {
        self.request.signing_context.as_ref()
    }
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct StoredAgentIdentity {
    schema: String,
    label: String,
    agent_secret_key_hex: AgentSecret,
    grant: AgentGrant,
    owner_attestation: StoredOwnerAttestation,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    last_used_at: Option<u64>,
}

impl std::fmt::Debug for StoredAgentIdentity {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("StoredAgentIdentity")
            .field("schema", &self.schema)
            .field("label", &self.label)
            .field("grant", &self.grant)
            .field("owner_attestation", &self.owner_attestation)
            .field("last_used_at", &self.last_used_at)
            .finish()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct StoredOwnerAttestation {
    request_ref: ReceiptRef,
    conditions_digest: String,
    auth_tag: Vec<String>,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct PendingAgentIdentity {
    schema: String,
    request_ref: ReceiptRef,
    label: String,
    agent_secret_key_hex: AgentSecret,
    grant: AgentGrant,
}

impl std::fmt::Debug for PendingAgentIdentity {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("PendingAgentIdentity")
            .field("schema", &self.schema)
            .field("request_ref", &self.request_ref)
            .field("label", &self.label)
            .field("grant", &self.grant)
            .finish()
    }
}

struct AgentSecret(Zeroizing<String>);

impl AgentSecret {
    fn new(value: String) -> Self {
        Self(Zeroizing::new(value))
    }

    fn as_str(&self) -> &str {
        self.0.as_str()
    }
}

impl std::fmt::Debug for AgentSecret {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("AgentSecret([REDACTED])")
    }
}

impl Serialize for AgentSecret {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for AgentSecret {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        Ok(Self::new(String::deserialize(deserializer)?))
    }
}

pub struct AgentIdentityStore {
    root: PathBuf,
}

impl AgentIdentityStore {
    pub fn system() -> Self {
        Self::new(paths::data_dir().clone())
    }

    pub fn new(root: PathBuf) -> Self {
        Self { root }
    }

    pub fn prepare_agent_identity(
        &self,
        current_selection: &AccountSelectionToken,
        plan: AgentIdentityPlan,
    ) -> Result<PreparedAgentIdentity, AgentIdentityError> {
        validate_plan(&plan)?;
        validate_selection(
            current_selection,
            &plan.owner_account_ref,
            &plan.owner_identity,
            plan.account_generation,
        )?;
        let _guard = store_guard(&self.root)?;
        let keys = Keys::generate();
        let agent_public_key_hex = NostrPublicKeyHex::new(keys.public_key().to_hex())
            .map_err(|_| AgentIdentityError::InvalidRequest)?;
        if agent_public_key_hex == *plan.owner_identity.public_key_hex() {
            return Err(AgentIdentityError::InvalidRequest);
        }
        let grant = AgentGrant {
            schema: AGENT_GRANT_SCHEMA.to_string(),
            grant_ref: plan.grant_ref,
            owner_account_ref: plan.owner_account_ref,
            owner_identity_ref: plan.owner_identity.identity_ref().clone(),
            owner_public_key_hex: plan.owner_identity.public_key_hex().clone(),
            agent_identity_ref: plan.agent_identity_ref.clone(),
            agent_public_key_hex: agent_public_key_hex.clone(),
            signer_ref: SignerRef::new(format!(
                "agent-signer-{}",
                digest(agent_public_key_hex.as_str().as_bytes())
            ))
            .map_err(|_| AgentIdentityError::InvalidRequest)?,
            scope: plan.scope,
            account_generation: plan.account_generation,
            issued_at: plan.issued_at,
            expires_at: plan.expires_at,
            owner_attestation_ref: ProofRef::new(plan.request_ref.as_str())
                .map_err(|_| AgentIdentityError::InvalidRequest)?,
            revoked_at: None,
        };
        grant.validate()?;
        let conditions = grant.conditions()?;
        if conditions.len() > 1_024 || conditions.chars().any(char::is_control) {
            return Err(AgentIdentityError::InvalidRequest);
        }
        match self.find_record(&grant.owner_account_ref, |record| {
            record.grant.agent_identity_ref == grant.agent_identity_ref
                || record.grant.grant_ref == grant.grant_ref
        }) {
            Ok(_) => return Err(AgentIdentityError::AlreadyExists),
            Err(AgentIdentityError::NotFound) => {}
            Err(error) => return Err(error),
        }
        let path = self.pending_path(&plan.request_ref);
        if path_exists_regular(&path)?.is_some()
            || self
                .record_path(&grant.owner_account_ref, &agent_public_key_hex)
                .try_exists()
                .map_err(|_| AgentIdentityError::Storage)?
        {
            return Err(AgentIdentityError::AlreadyExists);
        }
        let pending = PendingAgentIdentity {
            schema: AGENT_RECORD_SCHEMA.to_string(),
            request_ref: plan.request_ref.clone(),
            label: plan.label,
            agent_secret_key_hex: AgentSecret::new(keys.secret_key().to_secret_hex()),
            grant,
        };
        write_private_json(&path, &pending, &self.root)?;
        Ok(PreparedAgentIdentity {
            request_ref: plan.request_ref.clone(),
            agent_identity_ref: plan.agent_identity_ref,
            agent_public_key_hex: agent_public_key_hex.clone(),
            owner_attestation_request: OwnerAttestationRequest {
                request_ref: plan.request_ref,
                identity_ref: pending.grant.owner_identity_ref,
                agent_public_key_hex,
                conditions,
            },
        })
    }

    pub fn complete_owner_attestation(
        &self,
        current_selection: &AccountSelectionToken,
        request_ref: &ReceiptRef,
        result: &OwnerAttestationResult,
    ) -> Result<AgentIdentityProjection, AgentIdentityError> {
        let _guard = store_guard(&self.root)?;
        match self.find_record(&current_selection.account_ref, |record| {
            record.grant.owner_attestation_ref.as_str() == request_ref.as_str()
        }) {
            Ok((_, existing)) => {
                validate_grant_selection(current_selection, &existing.grant)?;
                validate_completed_result(&existing, result)?;
                let pending_path = self.pending_path(request_ref);
                if path_exists_regular(&pending_path)?.is_some() {
                    let pending: PendingAgentIdentity =
                        read_private_json(&pending_path, &self.root)?;
                    validate_pending(&pending)?;
                    validate_grant_selection(current_selection, &pending.grant)?;
                    let conditions = pending.grant.conditions()?;
                    verify_owner_attestation(&pending, result, &conditions)?;
                    if pending.grant != existing.grant || pending.label != existing.label {
                        return Err(AgentIdentityError::AlreadyExists);
                    }
                    remove_private_file(&pending_path, &self.root)?;
                }
                return Ok(projection(&existing));
            }
            Err(AgentIdentityError::NotFound) => {}
            Err(error) => return Err(error),
        }
        let pending_path = self.pending_path(request_ref);
        let pending: PendingAgentIdentity = read_private_json(&pending_path, &self.root)?;
        validate_pending(&pending)?;
        validate_grant_selection(current_selection, &pending.grant)?;
        let conditions = pending.grant.conditions()?;
        verify_owner_attestation(&pending, result, &conditions)?;
        let record = StoredAgentIdentity {
            schema: AGENT_RECORD_SCHEMA.to_string(),
            label: pending.label,
            agent_secret_key_hex: pending.agent_secret_key_hex,
            grant: pending.grant,
            owner_attestation: StoredOwnerAttestation {
                request_ref: result.request_ref.clone(),
                conditions_digest: digest(conditions.as_bytes()),
                auth_tag: result.auth_tag.clone(),
            },
            last_used_at: None,
        };
        validate_record(&record)?;
        let record_path = self.record_path(
            &record.grant.owner_account_ref,
            &record.grant.agent_public_key_hex,
        );
        if path_exists_regular(&record_path)?.is_some() {
            let existing: StoredAgentIdentity = read_private_json(&record_path, &self.root)?;
            validate_record(&existing)?;
            if projection(&existing) != projection(&record) {
                return Err(AgentIdentityError::AlreadyExists);
            }
            remove_private_file(&pending_path, &self.root)?;
            return Ok(projection(&existing));
        }
        write_private_json(&record_path, &record, &self.root)?;
        remove_private_file(&pending_path, &self.root)?;
        Ok(projection(&record))
    }

    pub fn cancel_pending_agent_identity(
        &self,
        current_selection: &AccountSelectionToken,
        request_ref: &ReceiptRef,
    ) -> Result<(), AgentIdentityError> {
        let _guard = store_guard(&self.root)?;
        let path = self.pending_path(request_ref);
        let pending: PendingAgentIdentity = read_private_json(&path, &self.root)?;
        validate_pending(&pending)?;
        validate_grant_selection(current_selection, &pending.grant)?;
        remove_private_file(&path, &self.root)
    }

    pub fn agent_inventory(
        &self,
        owner_account_ref: &AccountRef,
    ) -> Result<Vec<AgentIdentityProjection>, AgentIdentityError> {
        let _guard = store_guard(&self.root)?;
        let directory = self.account_directory(owner_account_ref);
        match fs::read_dir(&directory) {
            Ok(entries) => {
                let mut projections = Vec::new();
                for entry in entries {
                    let entry = entry.map_err(|_| AgentIdentityError::Storage)?;
                    if !entry
                        .file_type()
                        .map_err(|_| AgentIdentityError::Storage)?
                        .is_file()
                    {
                        return Err(AgentIdentityError::Storage);
                    }
                    let record: StoredAgentIdentity = read_private_json(&entry.path(), &self.root)?;
                    validate_record(&record)?;
                    if &record.grant.owner_account_ref != owner_account_ref {
                        return Err(AgentIdentityError::InvalidStoredData);
                    }
                    projections.push(projection(&record));
                }
                projections.sort_by(|left, right| {
                    left.agent_identity_ref
                        .as_str()
                        .cmp(right.agent_identity_ref.as_str())
                });
                Ok(projections)
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(Vec::new()),
            Err(_) => Err(AgentIdentityError::Storage),
        }
    }

    pub fn revoke_agent_grant(
        &self,
        current_selection: &AccountSelectionToken,
        grant_ref: &ReceiptRef,
        now: u64,
    ) -> Result<AgentIdentityProjection, AgentIdentityError> {
        let _guard = store_guard(&self.root)?;
        let (path, mut record) = self.find_record(&current_selection.account_ref, |record| {
            &record.grant.grant_ref == grant_ref
        })?;
        validate_grant_selection(current_selection, &record.grant)?;
        if now < record.grant.issued_at {
            return Err(AgentIdentityError::InvalidRequest);
        }
        if record.grant.revoked_at.is_none() {
            record.grant.revoked_at = Some(now);
            write_private_json(&path, &record, &self.root)?;
        }
        Ok(projection(&record))
    }

    pub fn authorize(
        &self,
        current_selection: &AccountSelectionToken,
        request: AgentAuthorizationRequest,
        now: u64,
    ) -> Result<AgentAuthorization, AgentIdentityError> {
        validate_authorization_request(&request)?;
        let _guard = store_guard(&self.root)?;
        let (_, record) = self.find_record(&request.owner_account_ref, |record| {
            record.grant.agent_identity_ref == request.agent_identity_ref
                && record.grant.grant_ref == request.grant_ref
        })?;
        validate_grant_selection(current_selection, &record.grant)?;
        authorize_record(&record, &request, now)?;
        Ok(AgentAuthorization {
            request,
            agent_public_key_hex: record.grant.agent_public_key_hex.clone(),
            owner_public_key_hex: record.grant.owner_public_key_hex.clone(),
            owner_attestation_ref: record.grant.owner_attestation_ref.clone(),
            signer_ref: record.grant.signer_ref.clone(),
            owner_auth_tag: record.owner_attestation.auth_tag.clone(),
            grant_conditions: record.grant.conditions()?,
            grant_expires_at: record.grant.expires_at,
        })
    }

    pub fn sign_authorized_event(
        &self,
        current_selection: &AccountSelectionToken,
        authorization: &AgentAuthorization,
        signing_context: &SigningAuthorizationContext,
        request: &AdmittedSigningRequest,
        now: u64,
    ) -> Result<SigningResult, AgentIdentityError> {
        signing_context
            .validate()
            .map_err(|_| AgentIdentityError::NotAuthorized)?;
        request
            .validate()
            .map_err(|_| AgentIdentityError::InvalidRequest)?;
        let expected_capability = match authorization.request.method {
            AgentMethod::SignEvent => "agent.sign-event",
            AgentMethod::Nip42RelayAuthentication => "nip-42.relay-auth",
            AgentMethod::NipAaRelayAuthentication => "nip-aa.relay-auth",
            AgentMethod::Nip44Encrypt | AgentMethod::Nip44Decrypt => {
                return Err(AgentIdentityError::NotAuthorized);
            }
        };
        if authorization.request.event_kind.is_none()
            || now < authorization.request.issued_at
            || now >= authorization.request.expires_at
            || authorization.request.signing_context.as_ref() != Some(signing_context)
            || signing_context.event_kind != request.event.kind
            || signing_context.purpose != request.purpose
            || signing_context.event_kind != authorization.request.event_kind.unwrap_or_default()
            || signing_context.resource_ref != authorization.request.destination_resource_ref
            || signing_context.issued_at < authorization.request.issued_at
            || signing_context.expires_at > authorization.request.expires_at
            || now < signing_context.issued_at
            || now >= signing_context.expires_at
            || signing_context
                .expires_at
                .saturating_sub(signing_context.issued_at)
                > 300
            || signing_context.authorization_ref.as_str() != request.request_ref.as_str()
            || authorization.request.request_ref != request.request_ref
            || signing_context.capability_ref.as_str() != expected_capability
            || signing_context.origin.is_empty()
            || signing_context.content_digest != signing_content_digest(request)?
        {
            return Err(AgentIdentityError::NotAuthorized);
        }
        let _guard = store_guard(&self.root)?;
        let (path, mut record) =
            self.find_record(&authorization.request.owner_account_ref, |record| {
                record.grant.agent_identity_ref == authorization.request.agent_identity_ref
                    && record.grant.grant_ref == authorization.request.grant_ref
            })?;
        validate_grant_selection(current_selection, &record.grant)?;
        authorize_record(&record, &authorization.request, now)?;
        if record.grant.agent_public_key_hex != authorization.agent_public_key_hex {
            return Err(AgentIdentityError::NotAuthorized);
        }
        if record.grant.owner_public_key_hex != authorization.owner_public_key_hex
            || record.grant.owner_attestation_ref != authorization.owner_attestation_ref
            || record.grant.signer_ref != authorization.signer_ref
            || record.owner_attestation.auth_tag != authorization.owner_auth_tag
            || record.grant.conditions()? != authorization.grant_conditions
            || record.grant.expires_at != authorization.grant_expires_at
        {
            return Err(AgentIdentityError::NotAuthorized);
        }
        let keys = Keys::parse(record.agent_secret_key_hex.as_str())
            .map_err(|_| AgentIdentityError::InvalidStoredData)?;
        if keys.public_key().to_hex() != record.grant.agent_public_key_hex.as_str() {
            return Err(AgentIdentityError::InvalidStoredData);
        }
        let identity_ref = IdentityRef::new(record.grant.agent_identity_ref.as_str())
            .map_err(|_| AgentIdentityError::InvalidStoredData)?;
        if request.identity_ref != identity_ref
            || signing_context.signer_ref != record.grant.signer_ref
        {
            return Err(AgentIdentityError::NotAuthorized);
        }
        let parsed_tags = request
            .event
            .tags
            .iter()
            .cloned()
            .map(|tag| Tag::parse(tag).map_err(|_| AgentIdentityError::InvalidRequest))
            .collect::<Result<Vec<_>, _>>()?;
        let event = EventBuilder::new(
            Kind::from(request.event.kind),
            request.event.content.clone(),
        )
        .tags(parsed_tags)
        .custom_created_at(Timestamp::from_secs(request.event.created_at))
        .sign_with_keys(&keys)
        .map_err(|_| AgentIdentityError::Signing)?;
        record.last_used_at = Some(now);
        write_private_json(&path, &record, &self.root)?;
        let identity = PublicIdentity::from_public_key_hex(
            identity_ref,
            record.grant.agent_public_key_hex.as_str(),
        )
        .map_err(|_| AgentIdentityError::InvalidStoredData)?;
        Ok(SigningResult {
            request_ref: request.request_ref.clone(),
            identity,
            event_id: event.id.to_hex(),
            signature: event.sig.to_string(),
            signed_event_json: event.as_json(),
        })
    }

    fn find_record(
        &self,
        account: &AccountRef,
        predicate: impl Fn(&StoredAgentIdentity) -> bool,
    ) -> Result<(PathBuf, StoredAgentIdentity), AgentIdentityError> {
        let directory = self.account_directory(account);
        let entries = fs::read_dir(directory).map_err(|error| {
            if error.kind() == io::ErrorKind::NotFound {
                AgentIdentityError::NotFound
            } else {
                AgentIdentityError::Storage
            }
        })?;
        for entry in entries {
            let path = entry.map_err(|_| AgentIdentityError::Storage)?.path();
            let record: StoredAgentIdentity = read_private_json(&path, &self.root)?;
            validate_record(&record)?;
            if predicate(&record) {
                return Ok((path, record));
            }
        }
        Err(AgentIdentityError::NotFound)
    }

    fn account_directory(&self, account: &AccountRef) -> PathBuf {
        self.root
            .join("identity")
            .join("agents")
            .join("records")
            .join(digest(account.as_str().as_bytes()))
    }

    fn record_path(&self, account: &AccountRef, public_key: &NostrPublicKeyHex) -> PathBuf {
        self.account_directory(account)
            .join(format!("{}.json", public_key.as_str()))
    }

    fn pending_path(&self, request: &ReceiptRef) -> PathBuf {
        self.root
            .join("identity")
            .join("agents")
            .join("pending")
            .join(format!("{}.json", digest(request.as_str().as_bytes())))
    }
}

fn validate_completed_result(
    record: &StoredAgentIdentity,
    result: &OwnerAttestationResult,
) -> Result<(), AgentIdentityError> {
    if result.request_ref.as_str() != record.grant.owner_attestation_ref.as_str()
        || result.identity.identity_ref() != &record.grant.owner_identity_ref
        || result.identity.public_key_hex() != &record.grant.owner_public_key_hex
        || result.agent_public_key_hex != record.grant.agent_public_key_hex
        || result.auth_tag != record.owner_attestation.auth_tag
    {
        return Err(AgentIdentityError::InvalidOwnerAttestation);
    }
    Ok(())
}

fn validate_plan(plan: &AgentIdentityPlan) -> Result<(), AgentIdentityError> {
    plan.scope.validate()?;
    if plan.label.trim().is_empty()
        || plan.label.len() > MAX_LABEL_BYTES
        || plan.label.chars().any(char::is_control)
        || plan.account_generation == 0
        || plan.issued_at >= plan.expires_at
        || plan.owner_account_ref.as_str() == plan.agent_identity_ref.as_str()
        || plan.owner_identity.identity_ref().as_str() == plan.agent_identity_ref.as_str()
    {
        return Err(AgentIdentityError::InvalidRequest);
    }
    Ok(())
}

fn validate_selection(
    selection: &AccountSelectionToken,
    owner_account_ref: &AccountRef,
    owner_identity: &PublicIdentity,
    account_generation: u64,
) -> Result<(), AgentIdentityError> {
    if &selection.account_ref != owner_account_ref
        || &selection.identity != owner_identity
        || selection.generation != account_generation
        || selection.generation == 0
    {
        return Err(AgentIdentityError::StaleGeneration);
    }
    Ok(())
}

fn validate_grant_selection(
    selection: &AccountSelectionToken,
    grant: &AgentGrant,
) -> Result<(), AgentIdentityError> {
    if selection.account_ref != grant.owner_account_ref
        || selection.identity.identity_ref() != &grant.owner_identity_ref
        || selection.identity.public_key_hex() != &grant.owner_public_key_hex
        || selection.generation != grant.account_generation
        || selection.generation == 0
    {
        return Err(AgentIdentityError::StaleGeneration);
    }
    Ok(())
}

fn validate_pending(pending: &PendingAgentIdentity) -> Result<(), AgentIdentityError> {
    if pending.schema != AGENT_RECORD_SCHEMA
        || pending.label.trim().is_empty()
        || pending.label.len() > MAX_LABEL_BYTES
    {
        return Err(AgentIdentityError::InvalidStoredData);
    }
    pending
        .grant
        .validate()
        .map_err(|_| AgentIdentityError::InvalidStoredData)?;
    let keys = Keys::parse(pending.agent_secret_key_hex.as_str())
        .map_err(|_| AgentIdentityError::InvalidStoredData)?;
    if keys.public_key().to_hex() != pending.grant.agent_public_key_hex.as_str() {
        return Err(AgentIdentityError::InvalidStoredData);
    }
    Ok(())
}

fn validate_record(record: &StoredAgentIdentity) -> Result<(), AgentIdentityError> {
    let conditions = record.grant.conditions()?;
    let expected_auth_prefix = [
        "auth",
        record.grant.owner_public_key_hex.as_str(),
        conditions.as_str(),
    ];
    if record.schema != AGENT_RECORD_SCHEMA
        || record.label.trim().is_empty()
        || record.label.len() > MAX_LABEL_BYTES
        || record.owner_attestation.request_ref.as_str()
            != record.grant.owner_attestation_ref.as_str()
        || record.owner_attestation.conditions_digest != digest(conditions.as_bytes())
        || record.owner_attestation.auth_tag.len() != 4
        || !record
            .owner_attestation
            .auth_tag
            .iter()
            .take(3)
            .map(String::as_str)
            .eq(expected_auth_prefix)
    {
        return Err(AgentIdentityError::InvalidStoredData);
    }
    record
        .grant
        .validate()
        .map_err(|_| AgentIdentityError::InvalidStoredData)?;
    let keys = Keys::parse(record.agent_secret_key_hex.as_str())
        .map_err(|_| AgentIdentityError::InvalidStoredData)?;
    if keys.public_key().to_hex() != record.grant.agent_public_key_hex.as_str() {
        return Err(AgentIdentityError::InvalidStoredData);
    }
    verify_attestation_signature(
        &record.grant.owner_public_key_hex,
        &record.grant.agent_public_key_hex,
        &conditions,
        record
            .owner_attestation
            .auth_tag
            .get(3)
            .ok_or(AgentIdentityError::InvalidStoredData)?,
    )
    .map_err(|_| AgentIdentityError::InvalidStoredData)
}

fn verify_owner_attestation(
    pending: &PendingAgentIdentity,
    result: &OwnerAttestationResult,
    conditions: &str,
) -> Result<(), AgentIdentityError> {
    let tag = result.auth_tag.as_slice();
    if result.request_ref != pending.request_ref
        || result.identity.identity_ref() != &pending.grant.owner_identity_ref
        || result.identity.public_key_hex() != &pending.grant.owner_public_key_hex
        || result.agent_public_key_hex != pending.grant.agent_public_key_hex
        || tag.len() != 4
        || tag[0] != "auth"
        || tag[1] != pending.grant.owner_public_key_hex.as_str()
        || tag[2] != conditions
    {
        return Err(AgentIdentityError::InvalidOwnerAttestation);
    }
    verify_attestation_signature(
        &pending.grant.owner_public_key_hex,
        &pending.grant.agent_public_key_hex,
        conditions,
        &tag[3],
    )
}

fn verify_attestation_signature(
    owner_public_key: &NostrPublicKeyHex,
    agent_public_key: &NostrPublicKeyHex,
    conditions: &str,
    signature: &str,
) -> Result<(), AgentIdentityError> {
    let owner = PublicKey::from_hex(owner_public_key.as_str())
        .map_err(|_| AgentIdentityError::InvalidOwnerAttestation)?;
    let signature = signature
        .parse::<Signature>()
        .map_err(|_| AgentIdentityError::InvalidOwnerAttestation)?;
    let digest = Sha256::digest(
        format!(
            "nostr:agent-auth:{}:{}",
            agent_public_key.as_str(),
            conditions
        )
        .as_bytes(),
    );
    nostr::SECP256K1
        .verify_schnorr(
            &signature,
            &Message::from_digest(digest.into()),
            &owner
                .xonly()
                .map_err(|_| AgentIdentityError::InvalidOwnerAttestation)?,
        )
        .map_err(|_| AgentIdentityError::InvalidOwnerAttestation)
}

fn validate_authorization_request(
    request: &AgentAuthorizationRequest,
) -> Result<(), AgentIdentityError> {
    let signing_method = matches!(
        request.method,
        AgentMethod::SignEvent
            | AgentMethod::Nip42RelayAuthentication
            | AgentMethod::NipAaRelayAuthentication
    );
    if request.account_generation == 0
        || !valid_reference(&request.room_or_tenant)
        || request.issued_at >= request.expires_at
        || signing_method != request.event_kind.is_some()
        || signing_method != request.signing_context.is_some()
    {
        return Err(AgentIdentityError::InvalidRequest);
    }
    if let Some(context) = &request.signing_context {
        context
            .validate()
            .map_err(|_| AgentIdentityError::InvalidRequest)?;
        if context.account_ref != request.owner_account_ref
            || context.account_generation != request.account_generation
            || context.event_kind != request.event_kind.unwrap_or_default()
            || context.resource_ref != request.destination_resource_ref
            || context.issued_at != request.issued_at
            || context.expires_at != request.expires_at
            || context.authorization_ref.as_str() != request.request_ref.as_str()
            || context.capability_ref.as_str() != method_capability(request.method)
        {
            return Err(AgentIdentityError::InvalidRequest);
        }
    }
    Ok(())
}

fn method_capability(method: AgentMethod) -> &'static str {
    match method {
        AgentMethod::SignEvent => "agent.sign-event",
        AgentMethod::Nip42RelayAuthentication => "nip-42.relay-auth",
        AgentMethod::NipAaRelayAuthentication => "nip-aa.relay-auth",
        AgentMethod::Nip44Encrypt => "agent.nip44-encrypt",
        AgentMethod::Nip44Decrypt => "agent.nip44-decrypt",
    }
}

fn authorize_record(
    record: &StoredAgentIdentity,
    request: &AgentAuthorizationRequest,
    now: u64,
) -> Result<(), AgentIdentityError> {
    if record.grant.revoked_at.is_some() {
        return Err(AgentIdentityError::Revoked);
    }
    if request.account_generation != record.grant.account_generation {
        return Err(AgentIdentityError::StaleGeneration);
    }
    if now < request.issued_at
        || now >= request.expires_at
        || now < record.grant.issued_at
        || now >= record.grant.expires_at
        || request.issued_at < record.grant.issued_at
        || request.issued_at >= record.grant.expires_at
        || request.expires_at > record.grant.expires_at
    {
        return Err(AgentIdentityError::Expired);
    }
    if !record.grant.scope.methods.contains(&request.method)
        || !record
            .grant
            .scope
            .rooms_or_tenants
            .contains(&request.room_or_tenant)
        || request
            .event_kind
            .is_some_and(|kind| !record.grant.scope.event_kinds.contains(&kind))
    {
        return Err(AgentIdentityError::NotAuthorized);
    }
    Ok(())
}

fn projection(record: &StoredAgentIdentity) -> AgentIdentityProjection {
    AgentIdentityProjection {
        owner_account_ref: record.grant.owner_account_ref.clone(),
        owner_identity_ref: record.grant.owner_identity_ref.clone(),
        owner_public_key_hex: record.grant.owner_public_key_hex.clone(),
        agent_identity_ref: record.grant.agent_identity_ref.clone(),
        agent_public_key_hex: record.grant.agent_public_key_hex.clone(),
        label: record.label.clone(),
        signer_ref: record.grant.signer_ref.clone(),
        owner_auth_tag: record.owner_attestation.auth_tag.clone(),
        grant: record.grant.clone(),
        last_used_at: record.last_used_at,
    }
}

fn signing_content_digest(request: &AdmittedSigningRequest) -> Result<String, AgentIdentityError> {
    let encoded =
        serde_json::to_vec(&request.event).map_err(|_| AgentIdentityError::InvalidRequest)?;
    Ok(digest(&encoded))
}

fn valid_reference(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 512
        && value
            .chars()
            .all(|character| character.is_ascii_graphic() && !character.is_ascii_control())
}

fn digest(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}

struct StoreGuard {
    _process_guard: MutexGuard<'static, ()>,
    _platform_guard: PlatformStoreGuard,
}

fn store_guard(root: &Path) -> Result<StoreGuard, AgentIdentityError> {
    static STORE_MUTEX: OnceLock<Mutex<()>> = OnceLock::new();
    let process_guard = STORE_MUTEX
        .get_or_init(|| Mutex::new(()))
        .lock()
        .map_err(|_| AgentIdentityError::Storage)?;
    let platform_guard = PlatformStoreGuard::acquire(root)?;
    Ok(StoreGuard {
        _process_guard: process_guard,
        _platform_guard: platform_guard,
    })
}

#[cfg(unix)]
struct PlatformStoreGuard {
    _file: fs::File,
}

#[cfg(unix)]
impl PlatformStoreGuard {
    fn acquire(root: &Path) -> Result<Self, AgentIdentityError> {
        use std::os::unix::{
            fs::{MetadataExt as _, OpenOptionsExt as _},
            io::AsRawFd as _,
        };

        let lock_directory = root.join("identity").join("agents");
        create_private_directory(&lock_directory, root)?;
        let lock_path = lock_directory.join(".mutation.lock");
        let file = fs::OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .mode(0o600)
            .custom_flags(libc::O_CLOEXEC | libc::O_NOFOLLOW)
            .open(lock_path)
            .map_err(|_| AgentIdentityError::Storage)?;
        let metadata = file.metadata().map_err(|_| AgentIdentityError::Storage)?;
        let user_id = unsafe { libc::getuid() };
        if !metadata.is_file()
            || metadata.uid() != user_id
            || metadata.nlink() != 1
            || metadata.mode() & 0o177 != 0
        {
            return Err(AgentIdentityError::Storage);
        }
        loop {
            let result = unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX) };
            if result == 0 {
                break;
            }
            if io::Error::last_os_error().kind() != io::ErrorKind::Interrupted {
                return Err(AgentIdentityError::Storage);
            }
        }
        Ok(Self { _file: file })
    }
}

#[cfg(target_os = "windows")]
struct PlatformStoreGuard {
    mutex_handle: windows_sys::Win32::Foundation::HANDLE,
}

#[cfg(target_os = "windows")]
impl PlatformStoreGuard {
    fn acquire(root: &Path) -> Result<Self, AgentIdentityError> {
        use windows_sys::Win32::{
            Foundation::{WAIT_ABANDONED, WAIT_OBJECT_0},
            Security::SECURITY_ATTRIBUTES,
            System::Threading::{CreateMutexW, INFINITE, WaitForSingleObject},
        };

        let mutex_name = format!(
            "Local\\OmegaAgentIdentity-{}",
            digest(root.as_os_str().as_encoded_bytes())
        );
        let wide_name = mutex_name
            .encode_utf16()
            .chain(std::iter::once(0))
            .collect::<Vec<_>>();
        let mutex_handle = unsafe {
            CreateMutexW(
                std::ptr::null::<SECURITY_ATTRIBUTES>(),
                0,
                wide_name.as_ptr(),
            )
        };
        if mutex_handle.is_null() {
            return Err(AgentIdentityError::Storage);
        }
        let wait_result = unsafe { WaitForSingleObject(mutex_handle, INFINITE) };
        if wait_result != WAIT_OBJECT_0 && wait_result != WAIT_ABANDONED {
            unsafe {
                windows_sys::Win32::Foundation::CloseHandle(mutex_handle);
            }
            return Err(AgentIdentityError::Storage);
        }
        Ok(Self { mutex_handle })
    }
}

#[cfg(target_os = "windows")]
impl Drop for PlatformStoreGuard {
    fn drop(&mut self) {
        unsafe {
            windows_sys::Win32::System::Threading::ReleaseMutex(self.mutex_handle);
            windows_sys::Win32::Foundation::CloseHandle(self.mutex_handle);
        }
    }
}

#[cfg(not(any(unix, target_os = "windows")))]
struct PlatformStoreGuard;

#[cfg(not(any(unix, target_os = "windows")))]
impl PlatformStoreGuard {
    fn acquire(_root: &Path) -> Result<Self, AgentIdentityError> {
        Err(AgentIdentityError::Storage)
    }
}

fn write_private_json<T: Serialize>(
    path: &Path,
    value: &T,
    root: &Path,
) -> Result<(), AgentIdentityError> {
    let bytes = serde_json::to_vec(value).map_err(|_| AgentIdentityError::Storage)?;
    if bytes.len() as u64 > MAX_FILE_BYTES {
        return Err(AgentIdentityError::Storage);
    }
    let parent = path.parent().ok_or(AgentIdentityError::Storage)?;
    create_private_directory(parent, root)?;
    if let Some(metadata) = path_exists_regular(path)? {
        verify_private_file_mode(&metadata)?;
    }
    let mut file = AtomicWriteFile::open(path).map_err(|_| AgentIdentityError::Storage)?;
    file.write_all(&bytes)
        .map_err(|_| AgentIdentityError::Storage)?;
    file.commit().map_err(|_| AgentIdentityError::Storage)?;
    set_private_file_mode(path)?;
    let persisted: serde_json::Value = read_private_json(path, root)?;
    let expected: serde_json::Value =
        serde_json::from_slice(&bytes).map_err(|_| AgentIdentityError::Storage)?;
    if persisted != expected {
        return Err(AgentIdentityError::Storage);
    }
    Ok(())
}

fn read_private_json<T: for<'de> Deserialize<'de>>(
    path: &Path,
    root: &Path,
) -> Result<T, AgentIdentityError> {
    verify_private_ancestors(path, root)?;
    let metadata = path_exists_regular(path)?.ok_or(AgentIdentityError::NotFound)?;
    verify_private_file_mode(&metadata)?;
    if metadata.len() > MAX_FILE_BYTES {
        return Err(AgentIdentityError::InvalidStoredData);
    }
    let file = fs::File::open(path).map_err(|_| AgentIdentityError::Storage)?;
    let mut bytes = Vec::new();
    file.take(MAX_FILE_BYTES.saturating_add(1))
        .read_to_end(&mut bytes)
        .map_err(|_| AgentIdentityError::Storage)?;
    if bytes.len() as u64 > MAX_FILE_BYTES {
        return Err(AgentIdentityError::InvalidStoredData);
    }
    serde_json::from_slice(&bytes).map_err(|_| AgentIdentityError::InvalidStoredData)
}

fn remove_private_file(path: &Path, root: &Path) -> Result<(), AgentIdentityError> {
    verify_private_ancestors(path, root)?;
    let metadata = path_exists_regular(path)?.ok_or(AgentIdentityError::NotFound)?;
    verify_private_file_mode(&metadata)?;
    fs::remove_file(path).map_err(|_| AgentIdentityError::Storage)?;
    if path.try_exists().map_err(|_| AgentIdentityError::Storage)? {
        return Err(AgentIdentityError::Storage);
    }
    Ok(())
}

fn create_private_directory(path: &Path, root: &Path) -> Result<(), AgentIdentityError> {
    let mut missing = Vec::new();
    let mut cursor = path;
    while !cursor
        .try_exists()
        .map_err(|_| AgentIdentityError::Storage)?
    {
        missing.push(cursor.to_path_buf());
        cursor = cursor.parent().ok_or(AgentIdentityError::Storage)?;
    }
    if cursor.starts_with(root) {
        verify_private_ancestors(&cursor.join("placeholder"), root)?;
    }
    for directory in missing.into_iter().rev() {
        fs::create_dir(&directory).map_err(|_| AgentIdentityError::Storage)?;
        set_private_directory_mode(&directory)?;
    }
    verify_private_ancestors(&path.join("placeholder"), root)
}

fn verify_private_ancestors(path: &Path, root: &Path) -> Result<(), AgentIdentityError> {
    let mut cursor = path.parent().ok_or(AgentIdentityError::Storage)?;
    loop {
        if !cursor.starts_with(root) {
            break;
        }
        let metadata = fs::symlink_metadata(cursor).map_err(|_| AgentIdentityError::Storage)?;
        if !metadata.is_dir() || metadata.file_type().is_symlink() {
            return Err(AgentIdentityError::Storage);
        }
        verify_private_directory_mode(&metadata)?;
        if cursor == root {
            break;
        }
        cursor = cursor.parent().ok_or(AgentIdentityError::Storage)?;
    }
    Ok(())
}

#[cfg(unix)]
fn set_private_directory_mode(path: &Path) -> Result<(), AgentIdentityError> {
    use std::os::unix::fs::PermissionsExt as _;
    fs::set_permissions(path, fs::Permissions::from_mode(0o700))
        .map_err(|_| AgentIdentityError::Storage)
}

#[cfg(not(unix))]
fn set_private_directory_mode(_path: &Path) -> Result<(), AgentIdentityError> {
    Ok(())
}

#[cfg(unix)]
fn set_private_file_mode(path: &Path) -> Result<(), AgentIdentityError> {
    use std::os::unix::fs::PermissionsExt as _;
    fs::set_permissions(path, fs::Permissions::from_mode(0o600))
        .map_err(|_| AgentIdentityError::Storage)
}

#[cfg(not(unix))]
fn set_private_file_mode(_path: &Path) -> Result<(), AgentIdentityError> {
    Ok(())
}

#[cfg(unix)]
fn verify_private_directory_mode(metadata: &fs::Metadata) -> Result<(), AgentIdentityError> {
    use std::os::unix::fs::PermissionsExt as _;
    if metadata.permissions().mode() & 0o077 != 0 {
        return Err(AgentIdentityError::Storage);
    }
    Ok(())
}

#[cfg(not(unix))]
fn verify_private_directory_mode(_metadata: &fs::Metadata) -> Result<(), AgentIdentityError> {
    Ok(())
}

#[cfg(unix)]
fn verify_private_file_mode(metadata: &fs::Metadata) -> Result<(), AgentIdentityError> {
    use std::os::unix::fs::PermissionsExt as _;
    if metadata.permissions().mode() & 0o177 != 0 {
        return Err(AgentIdentityError::Storage);
    }
    Ok(())
}

#[cfg(not(unix))]
fn verify_private_file_mode(_metadata: &fs::Metadata) -> Result<(), AgentIdentityError> {
    Ok(())
}

fn path_exists_regular(path: &Path) -> Result<Option<fs::Metadata>, AgentIdentityError> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.is_file() && !metadata.file_type().is_symlink() => {
            Ok(Some(metadata))
        }
        Ok(_) => Err(AgentIdentityError::Storage),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(_) => Err(AgentIdentityError::Storage),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use omega_identity::{
        CapabilityRef, SigningPurpose, SubsystemRef, UnsignedEventTemplate, UserGesture,
    };

    const NOW: u64 = 100;
    const GENERATION: u64 = 7;

    struct Fixture {
        _directory: tempfile::TempDir,
        store: AgentIdentityStore,
        owner_keys: Keys,
        selection: AccountSelectionToken,
        projection: AgentIdentityProjection,
    }

    impl Fixture {
        fn new() -> Self {
            let directory = tempfile::tempdir().expect("temporary directory");
            set_private_directory_mode(directory.path()).expect("private test root");
            let store = AgentIdentityStore::new(directory.path().to_path_buf());
            let owner_keys = Keys::generate();
            let selection = selection(&owner_keys);
            let prepared = store
                .prepare_agent_identity(&selection, plan(&owner_keys))
                .expect("prepare agent");
            let attestation = owner_attestation(&owner_keys, &prepared.owner_attestation_request);
            let projection = store
                .complete_owner_attestation(&selection, &prepared.request_ref, &attestation)
                .expect("complete attestation");
            Self {
                _directory: directory,
                store,
                owner_keys,
                selection,
                projection,
            }
        }

        fn authorization_request(&self) -> AgentAuthorizationRequest {
            AgentAuthorizationRequest {
                request_ref: receipt("agent-operation"),
                owner_account_ref: self.projection.owner_account_ref.clone(),
                account_generation: GENERATION,
                agent_identity_ref: self.projection.agent_identity_ref.clone(),
                grant_ref: self.projection.grant.grant_ref.clone(),
                method: AgentMethod::SignEvent,
                event_kind: Some(9),
                room_or_tenant: "room:omega".to_string(),
                destination_resource_ref: ResourceRef::new("room:omega").expect("resource"),
                issued_at: NOW,
                expires_at: NOW + 10,
                signing_context: Some(signing_context_for(
                    &self.projection,
                    receipt("agent-operation"),
                )),
            }
        }
    }

    fn receipt(value: &str) -> ReceiptRef {
        ReceiptRef::new(value).expect("receipt ref")
    }

    fn account(value: &str) -> AccountRef {
        AccountRef::new(value).expect("account ref")
    }

    fn identity(value: &str) -> IdentityRef {
        IdentityRef::new(value).expect("identity ref")
    }

    fn agent(value: &str) -> AgentIdentityRef {
        AgentIdentityRef::new(value).expect("agent ref")
    }

    fn public_identity(owner_keys: &Keys) -> PublicIdentity {
        PublicIdentity::from_public_key_hex(
            identity("person-identity"),
            owner_keys.public_key().to_hex(),
        )
        .expect("public identity")
    }

    fn selection(owner_keys: &Keys) -> AccountSelectionToken {
        AccountSelectionToken {
            account_ref: account("person-account"),
            identity: public_identity(owner_keys),
            generation: GENERATION,
        }
    }

    fn plan(owner_keys: &Keys) -> AgentIdentityPlan {
        AgentIdentityPlan {
            request_ref: receipt("attest-omega-agent"),
            grant_ref: receipt("grant-omega-agent"),
            owner_account_ref: account("person-account"),
            owner_identity: public_identity(owner_keys),
            agent_identity_ref: agent("omega-agent"),
            label: "Omega Agent".to_string(),
            scope: AgentGrantScope {
                methods: BTreeSet::from([
                    AgentMethod::SignEvent,
                    AgentMethod::Nip42RelayAuthentication,
                    AgentMethod::NipAaRelayAuthentication,
                ]),
                event_kinds: BTreeSet::from([9, 2_2242]),
                rooms_or_tenants: BTreeSet::from([
                    "room:omega".to_string(),
                    "tenant:openagents".to_string(),
                ]),
            },
            account_generation: GENERATION,
            issued_at: NOW,
            expires_at: NOW + 100,
        }
    }

    fn signing_request(projection: &AgentIdentityProjection) -> AdmittedSigningRequest {
        AdmittedSigningRequest {
            request_ref: receipt("agent-operation"),
            identity_ref: identity(projection.agent_identity_ref.as_str()),
            purpose: SigningPurpose::NostrEvent,
            event: UnsignedEventTemplate {
                created_at: NOW + 1,
                kind: 9,
                tags: vec![vec!["h".to_string(), "omega".to_string()]],
                content: "agent payload".to_string(),
            },
        }
    }

    fn signing_context_for(
        projection: &AgentIdentityProjection,
        request_ref: ReceiptRef,
    ) -> SigningAuthorizationContext {
        let request = signing_request(projection);
        SigningAuthorizationContext {
            authorization_ref: ProofRef::new(request_ref.as_str()).expect("proof ref"),
            account_ref: projection.owner_account_ref.clone(),
            account_generation: GENERATION,
            signer_ref: projection.signer_ref.clone(),
            calling_subsystem: SubsystemRef::new("omega.agent").expect("subsystem"),
            purpose: SigningPurpose::NostrEvent,
            event_kind: request.event.kind,
            resource_ref: ResourceRef::new("room:omega").expect("resource"),
            origin: "wss://relay.example".to_string(),
            content_digest: signing_content_digest(&request).expect("digest"),
            capability_ref: CapabilityRef::new("agent.sign-event").expect("capability"),
            user_gesture: UserGesture::Observed,
            issued_at: NOW,
            expires_at: NOW + 10,
        }
    }

    fn owner_attestation(
        owner_keys: &Keys,
        request: &OwnerAttestationRequest,
    ) -> OwnerAttestationResult {
        let digest = Sha256::digest(
            format!(
                "nostr:agent-auth:{}:{}",
                request.agent_public_key_hex.as_str(),
                request.conditions
            )
            .as_bytes(),
        );
        let signature = owner_keys
            .sign_schnorr(&Message::from_digest(digest.into()))
            .to_string();
        OwnerAttestationResult {
            request_ref: request.request_ref.clone(),
            identity: public_identity(owner_keys),
            agent_public_key_hex: request.agent_public_key_hex.clone(),
            auth_tag: vec![
                "auth".to_string(),
                owner_keys.public_key().to_hex(),
                request.conditions.clone(),
                signature,
            ],
        }
    }

    #[test]
    fn distinct_agent_key_is_attested_without_exposing_a_person_or_agent_secret() {
        let fixture = Fixture::new();
        assert_ne!(
            fixture.projection.owner_public_key_hex,
            fixture.projection.agent_public_key_hex
        );
        assert_eq!(
            fixture.projection.grant.owner_account_ref,
            account("person-account")
        );
        assert_eq!(
            fixture.projection.grant.agent_identity_ref,
            agent("omega-agent")
        );

        let public_json = serde_json::to_string(&fixture.projection).expect("serialize projection");
        assert!(!public_json.contains("secret"));
        assert!(!public_json.contains("nsec"));

        let inventory = fixture
            .store
            .agent_inventory(&account("person-account"))
            .expect("inventory");
        assert_eq!(inventory, vec![fixture.projection]);
    }

    #[test]
    fn wrong_owner_attestation_is_refused_and_pending_secret_is_redacted() {
        let directory = tempfile::tempdir().expect("temporary directory");
        set_private_directory_mode(directory.path()).expect("private test root");
        let store = AgentIdentityStore::new(directory.path().to_path_buf());
        let owner_keys = Keys::generate();
        let current_selection = selection(&owner_keys);
        let prepared = store
            .prepare_agent_identity(&current_selection, plan(&owner_keys))
            .expect("prepare");
        let wrong = owner_attestation(&Keys::generate(), &prepared.owner_attestation_request);
        assert_eq!(
            store.complete_owner_attestation(&current_selection, &prepared.request_ref, &wrong),
            Err(AgentIdentityError::InvalidOwnerAttestation)
        );
        let pending: PendingAgentIdentity =
            read_private_json(&store.pending_path(&prepared.request_ref), directory.path())
                .expect("pending");
        assert!(!format!("{pending:?}").contains(pending.agent_secret_key_hex.as_str()));
    }

    #[test]
    fn wrong_account_generation_kind_room_and_method_are_refused() {
        let fixture = Fixture::new();
        let exact = fixture.authorization_request();
        assert!(
            fixture
                .store
                .authorize(&fixture.selection, exact.clone(), NOW + 1)
                .is_ok()
        );

        let mut wrong_generation = exact.clone();
        wrong_generation.account_generation += 1;
        wrong_generation
            .signing_context
            .as_mut()
            .expect("signing context")
            .account_generation += 1;
        assert_eq!(
            fixture
                .store
                .authorize(&fixture.selection, wrong_generation, NOW + 1),
            Err(AgentIdentityError::StaleGeneration)
        );

        let mut wrong_kind = exact.clone();
        wrong_kind.event_kind = Some(1);
        wrong_kind
            .signing_context
            .as_mut()
            .expect("signing context")
            .event_kind = 1;
        assert_eq!(
            fixture
                .store
                .authorize(&fixture.selection, wrong_kind, NOW + 1),
            Err(AgentIdentityError::NotAuthorized)
        );

        let mut wrong_room = exact.clone();
        wrong_room.room_or_tenant = "room:other".to_string();
        assert_eq!(
            fixture
                .store
                .authorize(&fixture.selection, wrong_room, NOW + 1),
            Err(AgentIdentityError::NotAuthorized)
        );

        let mut wrong_method = exact;
        wrong_method.method = AgentMethod::Nip44Encrypt;
        wrong_method.event_kind = None;
        wrong_method.signing_context = None;
        assert_eq!(
            fixture
                .store
                .authorize(&fixture.selection, wrong_method, NOW + 1),
            Err(AgentIdentityError::NotAuthorized)
        );
    }

    #[test]
    fn expiry_and_revocation_are_rechecked_at_use_time() {
        let fixture = Fixture::new();
        let request = fixture.authorization_request();
        let authorization = fixture
            .store
            .authorize(&fixture.selection, request.clone(), NOW + 1)
            .expect("authorize");

        assert_eq!(
            fixture
                .store
                .authorize(&fixture.selection, request.clone(), NOW + 10),
            Err(AgentIdentityError::Expired)
        );

        let second_store = AgentIdentityStore::new(fixture._directory.path().to_path_buf());
        second_store
            .revoke_agent_grant(
                &fixture.selection,
                &fixture.projection.grant.grant_ref,
                NOW + 2,
            )
            .expect("revoke");
        assert_eq!(
            fixture.store.sign_authorized_event(
                &fixture.selection,
                &authorization,
                request.signing_context.as_ref().expect("signing context"),
                &signing_request(&fixture.projection),
                NOW + 3
            ),
            Err(AgentIdentityError::Revoked)
        );
        assert_eq!(
            second_store
                .agent_inventory(&fixture.projection.owner_account_ref)
                .expect("second-store inventory")[0]
                .grant
                .revoked_at,
            Some(NOW + 2)
        );
    }

    #[test]
    fn authorized_agent_signs_only_as_its_own_key_and_records_use() {
        let fixture = Fixture::new();
        let request = fixture.authorization_request();
        let authorization = fixture
            .store
            .authorize(&fixture.selection, request.clone(), NOW + 1)
            .expect("authorize");
        let receipt = fixture
            .store
            .sign_authorized_event(
                &fixture.selection,
                &authorization,
                request.signing_context.as_ref().expect("signing context"),
                &signing_request(&fixture.projection),
                NOW + 1,
            )
            .expect("sign");
        let event =
            nostr::Event::from_json(&receipt.signed_event_json).expect("signed Nostr event");
        assert_eq!(
            event.pubkey.to_hex(),
            fixture.projection.agent_public_key_hex.as_str()
        );
        assert_ne!(
            event.pubkey.to_hex(),
            fixture.owner_keys.public_key().to_hex()
        );
        assert_eq!(
            fixture
                .store
                .agent_inventory(&fixture.projection.owner_account_ref)
                .expect("inventory")[0]
                .last_used_at,
            Some(NOW + 1)
        );
    }

    #[cfg(unix)]
    #[test]
    fn agent_secret_uses_an_atomic_owner_only_file_and_weak_modes_are_refused() {
        use std::os::unix::fs::PermissionsExt as _;

        let fixture = Fixture::new();
        let path = fixture.store.record_path(
            &fixture.projection.owner_account_ref,
            &fixture.projection.agent_public_key_hex,
        );
        let metadata = fs::metadata(&path).expect("metadata");
        assert_eq!(metadata.permissions().mode() & 0o777, 0o600);
        assert_eq!(
            fs::metadata(path.parent().expect("parent"))
                .expect("parent metadata")
                .permissions()
                .mode()
                & 0o777,
            0o700
        );

        fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).expect("weaken permissions");
        assert_eq!(
            fixture
                .store
                .agent_inventory(&fixture.projection.owner_account_ref),
            Err(AgentIdentityError::Storage)
        );
    }

    #[test]
    fn tampering_with_the_durable_grant_is_detected() {
        let fixture = Fixture::new();
        let path = fixture.store.record_path(
            &fixture.projection.owner_account_ref,
            &fixture.projection.agent_public_key_hex,
        );
        let mut value: serde_json::Value =
            read_private_json(&path, &fixture.store.root).expect("record json");
        value["grant"]["account_generation"] = serde_json::json!(GENERATION + 1);
        write_private_json(&path, &value, &fixture.store.root).expect("write tampering");
        assert_eq!(
            fixture
                .store
                .agent_inventory(&fixture.projection.owner_account_ref),
            Err(AgentIdentityError::InvalidStoredData)
        );
    }

    #[test]
    fn completion_is_idempotent_and_pending_creation_can_be_cancelled() {
        let directory = tempfile::tempdir().expect("temporary directory");
        set_private_directory_mode(directory.path()).expect("private test root");
        let store = AgentIdentityStore::new(directory.path().to_path_buf());
        let owner_keys = Keys::generate();
        let current_selection = selection(&owner_keys);
        let prepared = store
            .prepare_agent_identity(&current_selection, plan(&owner_keys))
            .expect("prepare");
        let result = owner_attestation(&owner_keys, &prepared.owner_attestation_request);
        let first = store
            .complete_owner_attestation(&current_selection, &prepared.request_ref, &result)
            .expect("complete");
        let record_path = store.record_path(&first.owner_account_ref, &first.agent_public_key_hex);
        let record: StoredAgentIdentity =
            read_private_json(&record_path, directory.path()).expect("record");
        let crash_duplicate = PendingAgentIdentity {
            schema: AGENT_RECORD_SCHEMA.to_string(),
            request_ref: prepared.request_ref.clone(),
            label: record.label.clone(),
            agent_secret_key_hex: AgentSecret::new(
                record.agent_secret_key_hex.as_str().to_string(),
            ),
            grant: record.grant.clone(),
        };
        write_private_json(
            &store.pending_path(&prepared.request_ref),
            &crash_duplicate,
            directory.path(),
        )
        .expect("simulate crash before pending deletion");
        let retried = store
            .complete_owner_attestation(&current_selection, &prepared.request_ref, &result)
            .expect("idempotent retry");
        assert_eq!(retried, first);
        assert!(matches!(
            path_exists_regular(&store.pending_path(&prepared.request_ref)),
            Ok(None)
        ));

        let mut second_plan = plan(&owner_keys);
        second_plan.request_ref = receipt("attest-sarah-agent");
        second_plan.grant_ref = receipt("grant-sarah-agent");
        second_plan.agent_identity_ref = agent("sarah-agent");
        second_plan.label = "Sarah".to_string();
        let second = store
            .prepare_agent_identity(&current_selection, second_plan)
            .expect("prepare second");
        store
            .cancel_pending_agent_identity(&current_selection, &second.request_ref)
            .expect("cancel");
        assert!(matches!(
            read_private_json::<PendingAgentIdentity>(
                &store.pending_path(&second.request_ref),
                directory.path()
            ),
            Err(AgentIdentityError::NotFound)
        ));
    }

    #[test]
    fn a_stale_live_selection_and_substituted_signing_context_are_refused() {
        let fixture = Fixture::new();
        let request = fixture.authorization_request();
        let mut stale_selection = fixture.selection.clone();
        stale_selection.generation += 1;
        assert_eq!(
            fixture
                .store
                .authorize(&stale_selection, request.clone(), NOW + 1),
            Err(AgentIdentityError::StaleGeneration)
        );

        let authorization = fixture
            .store
            .authorize(&fixture.selection, request.clone(), NOW + 1)
            .expect("authorize");
        let mut substituted = request.signing_context.expect("signing context");
        substituted.origin = "wss://attacker.example".to_string();
        assert_eq!(
            fixture.store.sign_authorized_event(
                &fixture.selection,
                &authorization,
                &substituted,
                &signing_request(&fixture.projection),
                NOW + 1,
            ),
            Err(AgentIdentityError::NotAuthorized)
        );
    }

    #[test]
    fn nip_aa_is_an_explicit_method_separate_from_nip_42() {
        let fixture = Fixture::new();
        let mut request = fixture.authorization_request();
        request.method = AgentMethod::NipAaRelayAuthentication;
        request.event_kind = Some(22_242);
        request.room_or_tenant = "tenant:openagents".to_string();
        request.destination_resource_ref =
            ResourceRef::new("relay.0123456789abcdef").expect("resource");
        let context = request.signing_context.as_mut().expect("signing context");
        context.event_kind = 22_242;
        context.resource_ref = request.destination_resource_ref.clone();
        context.capability_ref = CapabilityRef::new("nip-aa.relay-auth").expect("capability");
        let authorization = fixture
            .store
            .authorize(&fixture.selection, request, NOW + 1)
            .expect("admit NIP-AA grant");
        assert_eq!(
            authorization.owner_auth_tag(),
            fixture.projection.owner_auth_tag
        );
        assert_eq!(
            authorization.grant_conditions(),
            fixture.projection.owner_auth_tag[2]
        );
        assert_eq!(
            authorization.owner_attestation_ref(),
            &fixture.projection.grant.owner_attestation_ref
        );
    }
}
