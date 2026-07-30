use std::collections::HashSet;

use serde::{Deserialize, Deserializer, Serialize, de::Error as _};
use serde_json::Value;
use thiserror::Error;

use crate::{IdentityRef, NostrPublicKeyHex, PublicIdentity, SigningPurpose};

pub const AUTHENTICATION_CONTRACT_SCHEMA: &str = "openagents.omega.authentication-contract.v1";
pub const AUTHENTICATION_CONTRACT_VERSION: u8 = 1;

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum AuthenticationContractError {
    #[error("{field} is not a portable reference")]
    InvalidReference { field: &'static str },
    #[error("the authentication contract schema or version is unsupported")]
    UnsupportedSchema,
    #[error("account generation must be greater than zero")]
    InvalidGeneration,
    #[error("person, device, agent, and hosted-user identities must stay distinct")]
    CollapsedPrincipal,
    #[error("authentication evidence is invalid")]
    InvalidEvidence,
    #[error("authentication state and evidence disagree")]
    StateEvidenceMismatch,
    #[error("the signing authorization context is invalid")]
    InvalidSigningContext,
    #[error("the public projection contains secret-shaped material")]
    SecretShapedMaterial,
    #[error("the public projection could not be encoded")]
    Serialization,
}

fn validate_reference(field: &'static str, value: &str) -> Result<(), AuthenticationContractError> {
    if value.is_empty()
        || value.len() > 256
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'-' | b'_' | b':'))
    {
        return Err(AuthenticationContractError::InvalidReference { field });
    }
    Ok(())
}

macro_rules! portable_reference {
    ($name:ident, $field:literal) => {
        #[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize)]
        #[serde(transparent)]
        pub struct $name(String);

        impl $name {
            pub fn new(value: impl Into<String>) -> Result<Self, AuthenticationContractError> {
                let value = value.into();
                validate_reference($field, &value)?;
                Ok(Self(value))
            }

            pub fn as_str(&self) -> &str {
                &self.0
            }
        }

        impl<'de> Deserialize<'de> for $name {
            fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
            where
                D: Deserializer<'de>,
            {
                Self::new(String::deserialize(deserializer)?).map_err(D::Error::custom)
            }
        }
    };
}

portable_reference!(AccountRef, "account_ref");
portable_reference!(DeviceIdentityRef, "device_identity_ref");
portable_reference!(AgentIdentityRef, "agent_identity_ref");
portable_reference!(HostedUserRef, "hosted_user_ref");
portable_reference!(SignerRef, "signer_ref");
portable_reference!(CapabilityRef, "capability_ref");
portable_reference!(SubsystemRef, "calling_subsystem");
portable_reference!(ResourceRef, "resource_ref");
portable_reference!(ProofRef, "proof_ref");

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PublicPersonIdentity {
    pub account_ref: AccountRef,
    pub identity: PublicIdentity,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PublicDeviceIdentity {
    pub device_identity_ref: DeviceIdentityRef,
    pub owner_account_ref: AccountRef,
    pub public_key_hex: NostrPublicKeyHex,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PublicAgentIdentity {
    pub agent_identity_ref: AgentIdentityRef,
    pub owner_account_ref: AccountRef,
    pub public_key_hex: NostrPublicKeyHex,
    pub owner_attestation_ref: ProofRef,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PublicHostedUserIdentity {
    pub hosted_user_ref: HostedUserRef,
    pub linked_account_ref: AccountRef,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PrincipalSet {
    pub person_identity_ref: IdentityRef,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub device_identity_ref: Option<DeviceIdentityRef>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_identity_ref: Option<AgentIdentityRef>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hosted_user_ref: Option<HostedUserRef>,
}

impl PrincipalSet {
    fn validate(&self) -> Result<(), AuthenticationContractError> {
        let mut references = HashSet::new();
        references.insert(self.person_identity_ref.as_str());
        for reference in [
            self.device_identity_ref
                .as_ref()
                .map(DeviceIdentityRef::as_str),
            self.agent_identity_ref
                .as_ref()
                .map(AgentIdentityRef::as_str),
            self.hosted_user_ref.as_ref().map(HostedUserRef::as_str),
        ]
        .into_iter()
        .flatten()
        {
            if !references.insert(reference) {
                return Err(AuthenticationContractError::CollapsedPrincipal);
            }
        }
        Ok(())
    }
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AccountLifecycleState {
    CandidateLocal,
    CandidateExisting,
    Activating,
    Active,
    Switching,
    Locked,
    SignedOut,
    ForgetPending,
    Forgotten,
    RepairRequired,
    Conflict,
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SignerKind {
    LocalNative,
    RemoteNip46,
    BrowserNip07,
    AndroidNip55,
    DeviceGrant,
    AgentGrant,
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SignerAvailability {
    Ready,
    UserApprovalRequired,
    Offline,
    Rejected,
    Revoked,
    Lost,
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SignerRecoveryState {
    Required,
    Protected,
    External,
    Unavailable,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SignerProjection {
    pub signer_ref: SignerRef,
    pub kind: SignerKind,
    pub availability: SignerAvailability,
    pub recovery: SignerRecoveryState,
    #[serde(default)]
    pub capabilities: Vec<CapabilityRef>,
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LocalSignerState {
    Unavailable,
    UserApprovalRequired,
    Ready,
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RelayAuthenticationState {
    NotConnected,
    ChallengePending,
    Authenticated,
    Refused,
    Stale,
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RelayConnectionAuthenticationState {
    Disconnected,
    ChallengePending,
    Authenticated,
    Refused,
    Stale,
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RelayAuthenticationRefusal {
    MalformedChallenge,
    InvalidEvent,
    WrongAccount,
    StaleEvent,
    ReplayedProof,
    RelayRejected,
    AcknowledgementMissing,
    StaleConnection,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RelayAuthenticationReceipt {
    pub relay_url: String,
    pub account_public_key_hex: NostrPublicKeyHex,
    pub connection_generation: u64,
    pub state: RelayConnectionAuthenticationState,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub challenge_ref: Option<ProofRef>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub auth_event_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub refusal: Option<RelayAuthenticationRefusal>,
    pub observed_at: u64,
}

impl RelayAuthenticationReceipt {
    pub fn validate(&self) -> Result<(), AuthenticationContractError> {
        if !is_normalized_relay_url(&self.relay_url)
            || self.connection_generation == 0
            || self.observed_at == 0
        {
            return Err(AuthenticationContractError::InvalidEvidence);
        }

        let shape_is_valid = match self.state {
            RelayConnectionAuthenticationState::Disconnected => {
                self.challenge_ref.is_none()
                    && self.auth_event_id.is_none()
                    && self.refusal.is_none()
            }
            RelayConnectionAuthenticationState::ChallengePending => {
                self.challenge_ref.is_some()
                    && self.auth_event_id.is_none()
                    && self.refusal.is_none()
            }
            RelayConnectionAuthenticationState::Authenticated => {
                self.challenge_ref.is_some()
                    && self.auth_event_id.as_deref().is_some_and(is_lower_hex_64)
                    && self.refusal.is_none()
            }
            RelayConnectionAuthenticationState::Refused => {
                self.challenge_ref.is_some()
                    && self.auth_event_id.as_deref().is_none_or(is_lower_hex_64)
                    && self.refusal.is_some()
            }
            RelayConnectionAuthenticationState::Stale => {
                self.refusal == Some(RelayAuthenticationRefusal::StaleConnection)
            }
        };
        if !shape_is_valid {
            return Err(AuthenticationContractError::InvalidEvidence);
        }

        let value =
            serde_json::to_value(self).map_err(|_| AuthenticationContractError::Serialization)?;
        reject_secret_shaped_value(&value)
    }

    pub fn public_json(&self) -> Result<String, AuthenticationContractError> {
        self.validate()?;
        serde_json::to_string(self).map_err(|_| AuthenticationContractError::Serialization)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct RelayAuthenticationProjection {
    #[serde(default)]
    pub relays: Vec<RelayAuthenticationReceipt>,
}

impl RelayAuthenticationProjection {
    pub fn validate(&self) -> Result<(), AuthenticationContractError> {
        let mut connections = HashSet::new();
        for receipt in &self.relays {
            receipt.validate()?;
            if !connections.insert((&receipt.relay_url, receipt.connection_generation)) {
                return Err(AuthenticationContractError::InvalidEvidence);
            }
        }
        Ok(())
    }

    pub fn public_json(&self) -> Result<String, AuthenticationContractError> {
        self.validate()?;
        serde_json::to_string(self).map_err(|_| AuthenticationContractError::Serialization)
    }

    pub fn for_account_public_key_hex(&self, public_key_hex: &str) -> Self {
        Self {
            relays: self
                .relays
                .iter()
                .filter(|receipt| receipt.account_public_key_hex.as_str() == public_key_hex)
                .cloned()
                .collect(),
        }
    }
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GroupAdmissionState {
    Unknown,
    NotMember,
    Pending,
    Admitted,
    Refused,
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HostedAccountLinkState {
    Unlinked,
    ProofPending,
    Linked,
    Refused,
    Unavailable,
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ActionAuthorizationState {
    NotEvaluated,
    Authorized,
    Refused,
    Expired,
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, Hash)]
pub enum AuthorityDomain {
    LocalSigner,
    RelayConnection,
    GroupAdmission,
    HostedAccount,
    Action,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "domain", rename_all = "snake_case", deny_unknown_fields)]
pub enum AuthenticationEvidence {
    LocalSigner {
        signer_ref: SignerRef,
    },
    RelayConnection {
        relay_url: String,
        auth_event_id: String,
        authenticated_at: u64,
    },
    GroupAdmission {
        group_ref: ResourceRef,
        authority_ref: ProofRef,
    },
    HostedAccount {
        hosted_user_ref: HostedUserRef,
        proof_ref: ProofRef,
    },
    Action {
        authorization_ref: ProofRef,
        capability_ref: CapabilityRef,
        issued_at: u64,
        expires_at: u64,
    },
}

impl AuthenticationEvidence {
    pub fn domain(&self) -> AuthorityDomain {
        match self {
            Self::LocalSigner { .. } => AuthorityDomain::LocalSigner,
            Self::RelayConnection { .. } => AuthorityDomain::RelayConnection,
            Self::GroupAdmission { .. } => AuthorityDomain::GroupAdmission,
            Self::HostedAccount { .. } => AuthorityDomain::HostedAccount,
            Self::Action { .. } => AuthorityDomain::Action,
        }
    }

    pub fn proves(&self, domain: AuthorityDomain) -> bool {
        self.domain() == domain
    }

    fn validate(&self) -> Result<(), AuthenticationContractError> {
        match self {
            Self::LocalSigner { .. } => Ok(()),
            Self::RelayConnection {
                relay_url,
                auth_event_id,
                authenticated_at,
            } => {
                if (!relay_url.starts_with("wss://") && !relay_url.starts_with("ws://"))
                    || !is_lower_hex_64(auth_event_id)
                    || *authenticated_at == 0
                {
                    return Err(AuthenticationContractError::InvalidEvidence);
                }
                Ok(())
            }
            Self::GroupAdmission { .. } | Self::HostedAccount { .. } => Ok(()),
            Self::Action {
                issued_at,
                expires_at,
                ..
            } => {
                if *issued_at == 0 || expires_at <= issued_at {
                    return Err(AuthenticationContractError::InvalidEvidence);
                }
                Ok(())
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AuthenticationStates {
    pub local_signer: LocalSignerState,
    pub relay_authentication: RelayAuthenticationState,
    pub group_admission: GroupAdmissionState,
    pub hosted_account_link: HostedAccountLinkState,
    pub action_authorization: ActionAuthorizationState,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AccountAuthenticationProjection {
    pub schema: String,
    pub schema_version: u8,
    pub account_ref: AccountRef,
    pub account_generation: u64,
    pub principals: PrincipalSet,
    pub lifecycle: AccountLifecycleState,
    pub signer: SignerProjection,
    pub states: AuthenticationStates,
    #[serde(default)]
    pub evidence: Vec<AuthenticationEvidence>,
}

impl AccountAuthenticationProjection {
    pub fn validate(&self) -> Result<(), AuthenticationContractError> {
        if self.schema != AUTHENTICATION_CONTRACT_SCHEMA
            || self.schema_version != AUTHENTICATION_CONTRACT_VERSION
        {
            return Err(AuthenticationContractError::UnsupportedSchema);
        }
        if self.account_generation == 0 {
            return Err(AuthenticationContractError::InvalidGeneration);
        }
        self.principals.validate()?;

        let mut evidence_domains = HashSet::new();
        for evidence in &self.evidence {
            evidence.validate()?;
            if !evidence_domains.insert(evidence.domain()) {
                return Err(AuthenticationContractError::InvalidEvidence);
            }
        }

        let required_domains = [
            (
                self.states.local_signer == LocalSignerState::Ready,
                AuthorityDomain::LocalSigner,
            ),
            (
                self.states.relay_authentication == RelayAuthenticationState::Authenticated,
                AuthorityDomain::RelayConnection,
            ),
            (
                self.states.group_admission == GroupAdmissionState::Admitted,
                AuthorityDomain::GroupAdmission,
            ),
            (
                self.states.hosted_account_link == HostedAccountLinkState::Linked,
                AuthorityDomain::HostedAccount,
            ),
            (
                self.states.action_authorization == ActionAuthorizationState::Authorized,
                AuthorityDomain::Action,
            ),
        ];
        for (state_requires_evidence, domain) in required_domains {
            if state_requires_evidence != evidence_domains.contains(&domain) {
                return Err(AuthenticationContractError::StateEvidenceMismatch);
            }
        }

        self.public_json()?;
        Ok(())
    }

    pub fn action_is_authorized(&self) -> bool {
        self.states.action_authorization == ActionAuthorizationState::Authorized
            && self
                .evidence
                .iter()
                .any(|evidence| evidence.proves(AuthorityDomain::Action))
    }

    pub fn public_json(&self) -> Result<String, AuthenticationContractError> {
        let value =
            serde_json::to_value(self).map_err(|_| AuthenticationContractError::Serialization)?;
        reject_secret_shaped_value(&value)?;
        serde_json::to_string(self).map_err(|_| AuthenticationContractError::Serialization)
    }
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UserGesture {
    Required,
    Observed,
    NotRequired,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SigningAuthorizationContext {
    pub authorization_ref: ProofRef,
    pub account_ref: AccountRef,
    pub account_generation: u64,
    pub signer_ref: SignerRef,
    pub calling_subsystem: SubsystemRef,
    pub purpose: SigningPurpose,
    pub event_kind: u16,
    pub resource_ref: ResourceRef,
    pub origin: String,
    pub content_digest: String,
    pub capability_ref: CapabilityRef,
    pub user_gesture: UserGesture,
    pub issued_at: u64,
    pub expires_at: u64,
}

impl SigningAuthorizationContext {
    pub fn validate(&self) -> Result<(), AuthenticationContractError> {
        if self.account_generation == 0
            || self.origin.len() > 2_048
            || self.origin.chars().any(char::is_control)
            || !is_lower_hex_64(&self.content_digest)
            || self.issued_at == 0
            || self.expires_at <= self.issued_at
        {
            return Err(AuthenticationContractError::InvalidSigningContext);
        }
        let value =
            serde_json::to_value(self).map_err(|_| AuthenticationContractError::Serialization)?;
        reject_secret_shaped_value(&value)
    }
}

fn is_lower_hex_64(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || matches!(byte, b'a'..=b'f'))
}

fn is_normalized_relay_url(value: &str) -> bool {
    if value.len() > 2_048 || value.trim() != value || value.chars().any(char::is_control) {
        return false;
    }
    let Ok(parsed) = url::Url::parse(value) else {
        return false;
    };
    if !matches!(parsed.scheme(), "ws" | "wss")
        || parsed.host_str().is_none()
        || !parsed.username().is_empty()
        || parsed.password().is_some()
        || parsed.query().is_some()
        || parsed.fragment().is_some()
    {
        return false;
    }
    let mut normalized = parsed.to_string();
    if normalized.ends_with('/') {
        normalized.pop();
    }
    normalized == value
}

fn reject_secret_shaped_value(value: &Value) -> Result<(), AuthenticationContractError> {
    match value {
        Value::Object(entries) => {
            for (key, value) in entries {
                let normalized = key.to_ascii_lowercase();
                if [
                    "secret",
                    "private_key",
                    "privatekey",
                    "nsec",
                    "password",
                    "mnemonic",
                    "seed",
                    "access_token",
                    "accesstoken",
                    "refresh_token",
                    "refreshtoken",
                    "ciphertext",
                    "decrypted",
                    "private_prompt",
                    "privateprompt",
                ]
                .iter()
                .any(|forbidden| normalized.contains(forbidden))
                {
                    return Err(AuthenticationContractError::SecretShapedMaterial);
                }
                reject_secret_shaped_value(value)?;
            }
            Ok(())
        }
        Value::Array(values) => {
            for value in values {
                reject_secret_shaped_value(value)?;
            }
            Ok(())
        }
        Value::String(value)
            if value.starts_with("nsec1")
                || value.starts_with("ncryptsec1")
                || value.starts_with("bunker://")
                || value.to_ascii_lowercase().starts_with("bearer ") =>
        {
            Err(AuthenticationContractError::SecretShapedMaterial)
        }
        _ => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn canonical_projection() -> AccountAuthenticationProjection {
        serde_json::from_str(include_str!(
            "../fixtures/omega_authentication_contract_v1.canonical.json"
        ))
        .expect("canonical authentication fixture")
    }

    #[test]
    fn canonical_fixture_is_valid_and_public_safe() {
        let projection = canonical_projection();
        projection.validate().expect("valid contract");
        let encoded = projection.public_json().expect("public projection");
        for forbidden in [
            "nsec1",
            "ncryptsec1",
            "access_token",
            "refresh_token",
            "private_key",
            "password",
            "mnemonic",
            "ciphertext",
            "decrypted",
        ] {
            assert!(!encoded.to_ascii_lowercase().contains(forbidden));
        }
    }

    #[test]
    fn authentication_domains_never_substitute_for_action_authority() {
        let vectors: Vec<AuthenticationEvidence> = serde_json::from_str(include_str!(
            "../fixtures/omega_authentication_contract_v1.negative-non-action-evidence.json"
        ))
        .expect("negative authorization vectors");
        assert_eq!(vectors.len(), 4);
        assert!(
            vectors
                .iter()
                .all(|evidence| !evidence.proves(AuthorityDomain::Action))
        );

        let mut projection = canonical_projection();
        projection.states.action_authorization = ActionAuthorizationState::Authorized;
        projection
            .evidence
            .retain(|evidence| evidence.domain() != AuthorityDomain::Action);
        assert_eq!(
            projection.validate(),
            Err(AuthenticationContractError::StateEvidenceMismatch)
        );
        assert!(!projection.action_is_authorized());
    }

    #[test]
    fn distinct_principal_types_cannot_be_collapsed() {
        let projection: AccountAuthenticationProjection = serde_json::from_str(include_str!(
            "../fixtures/omega_authentication_contract_v1.negative-collapsed-principals.json"
        ))
        .expect("collapsed-principal fixture has valid syntax");
        assert_eq!(
            projection.validate(),
            Err(AuthenticationContractError::CollapsedPrincipal)
        );
    }

    #[test]
    fn secret_shaped_fields_and_values_are_refused() {
        assert!(
            serde_json::from_str::<AccountAuthenticationProjection>(include_str!(
                "../fixtures/omega_authentication_contract_v1.negative-secret-field.json"
            ))
            .is_err()
        );

        let mut projection = canonical_projection();
        projection.signer.signer_ref =
            SignerRef::new("nsec1secret-shaped-value").expect("portable reference");
        assert_eq!(
            projection.public_json(),
            Err(AuthenticationContractError::SecretShapedMaterial)
        );

        assert_eq!(
            reject_secret_shaped_value(&serde_json::json!({
                "private_prompt": "not for a public projection"
            })),
            Err(AuthenticationContractError::SecretShapedMaterial)
        );
    }

    #[test]
    fn signing_context_binds_every_authorization_dimension() {
        let context: SigningAuthorizationContext = serde_json::from_str(include_str!(
            "../fixtures/omega_signing_authorization_context_v1.canonical.json"
        ))
        .expect("canonical signing context");
        context.validate().expect("valid signing context");

        let mut stale_generation = context.clone();
        stale_generation.account_generation = 0;
        assert_eq!(
            stale_generation.validate(),
            Err(AuthenticationContractError::InvalidSigningContext)
        );

        let mut invalid_digest = context;
        invalid_digest.content_digest = "not-a-digest".to_string();
        assert_eq!(
            invalid_digest.validate(),
            Err(AuthenticationContractError::InvalidSigningContext)
        );
    }

    #[test]
    fn relay_authentication_receipts_are_public_safe_and_connection_bound() {
        let receipt = RelayAuthenticationReceipt {
            relay_url: "wss://relay.example.com".to_string(),
            account_public_key_hex: NostrPublicKeyHex::new(
                "79be667ef9dcbbac55a06295ce870b07029bfcdb2dce28d959f2815b16f81798",
            )
            .expect("valid account key"),
            connection_generation: 7,
            state: RelayConnectionAuthenticationState::Authenticated,
            challenge_ref: Some(
                ProofRef::new("nip42.0123456789abcdef0123456789abcdef")
                    .expect("portable challenge reference"),
            ),
            auth_event_id: Some("a".repeat(64)),
            refusal: None,
            observed_at: 1_700_000_000,
        };
        receipt.validate().expect("valid relay receipt");
        let public_json = receipt.public_json().expect("public relay receipt");
        assert!(!public_json.contains("challenge-value"));

        let duplicate_projection = RelayAuthenticationProjection {
            relays: vec![receipt.clone(), receipt],
        };
        assert_eq!(
            duplicate_projection.validate(),
            Err(AuthenticationContractError::InvalidEvidence)
        );
    }

    #[test]
    fn relay_authentication_receipt_state_requires_matching_evidence() {
        let receipt = RelayAuthenticationReceipt {
            relay_url: "wss://relay.example.com".to_string(),
            account_public_key_hex: NostrPublicKeyHex::new(
                "79be667ef9dcbbac55a06295ce870b07029bfcdb2dce28d959f2815b16f81798",
            )
            .expect("valid account key"),
            connection_generation: 1,
            state: RelayConnectionAuthenticationState::Authenticated,
            challenge_ref: None,
            auth_event_id: Some("a".repeat(64)),
            refusal: None,
            observed_at: 1,
        };
        assert_eq!(
            receipt.validate(),
            Err(AuthenticationContractError::InvalidEvidence)
        );
    }

    #[test]
    fn relay_authentication_projection_filters_exactly_by_account() {
        let first_key = NostrPublicKeyHex::new(
            "79be667ef9dcbbac55a06295ce870b07029bfcdb2dce28d959f2815b16f81798",
        )
        .expect("valid first account key");
        let second_key = NostrPublicKeyHex::new(
            "c6047f9441ed7d6d3045406e95c07cd85c778e4b8cef3ca7abac09b95c709ee5",
        )
        .expect("valid second account key");
        let receipt_for =
            |account_public_key_hex: NostrPublicKeyHex, generation| RelayAuthenticationReceipt {
                relay_url: "wss://relay.example.com".to_string(),
                account_public_key_hex,
                connection_generation: generation,
                state: RelayConnectionAuthenticationState::Disconnected,
                challenge_ref: None,
                auth_event_id: None,
                refusal: None,
                observed_at: generation,
            };
        let projection = RelayAuthenticationProjection {
            relays: vec![
                receipt_for(first_key, 1),
                receipt_for(second_key.clone(), 2),
            ],
        };
        let selected = projection.for_account_public_key_hex(second_key.as_str());
        assert_eq!(selected.relays.len(), 1);
        assert_eq!(selected.relays[0].account_public_key_hex, second_key);
        assert!(
            projection
                .for_account_public_key_hex("not-an-account-key")
                .relays
                .is_empty()
        );
    }
}
