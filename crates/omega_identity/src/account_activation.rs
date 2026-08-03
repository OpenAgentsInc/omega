use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::{
    AccountRef, AuthenticationContractError, IdentityRef, ProofRef, PublicIdentity, ReceiptRef,
    RecoveryProtectionStatus, ResourceRef,
};

pub const IDENTITY_ACCOUNT_SCHEMA: &str = "openagents.omega.identity-account.v1";
pub const IDENTITY_ACCOUNT_SCHEMA_VERSION: u8 = 1;

#[derive(Debug, Copy, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IdentityCandidateOrigin {
    Local,
    Existing,
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IdentityActivationState {
    CandidateLocal,
    CandidateExisting,
    Activating,
    Active,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct IdentityAccountRecord {
    pub schema: String,
    pub schema_version: u8,
    pub account_ref: AccountRef,
    pub account_generation: u64,
    pub identity: PublicIdentity,
    pub state: IdentityActivationState,
    pub candidate_origin: IdentityCandidateOrigin,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub activation_ref: Option<ReceiptRef>,
    pub updated_at: u64,
}

impl IdentityAccountRecord {
    pub fn fingerprint_display(&self) -> String {
        self.identity.fingerprint().display()
    }

    pub fn is_active(&self) -> bool {
        self.state == IdentityActivationState::Active
    }

    pub(crate) fn validate(&self) -> bool {
        self.schema == IDENTITY_ACCOUNT_SCHEMA
            && self.schema_version == IDENTITY_ACCOUNT_SCHEMA_VERSION
            && self.account_generation > 0
            && self.identity.validate().is_ok()
            && self.account_ref.as_str()
                == format!("omega-account-{}", self.identity.public_key_hex().as_str())
            && match self.state {
                IdentityActivationState::CandidateLocal => {
                    self.candidate_origin == IdentityCandidateOrigin::Local
                        && self.activation_ref.is_none()
                }
                IdentityActivationState::CandidateExisting => {
                    self.candidate_origin == IdentityCandidateOrigin::Existing
                        && self.activation_ref.is_none()
                }
                IdentityActivationState::Activating | IdentityActivationState::Active => {
                    self.activation_ref.is_some()
                }
            }
    }
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DurableIdentityActionKind {
    AccountSetup,
    PublicPost,
    CommunityJoin,
    DeviceGrant,
    HostedAccountLink,
    AgentAttestation,
    WorkroomActivity,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct IdentityActivationInspection {
    pub account: IdentityAccountRecord,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub held_intent: Option<HeldIdentityAction>,
    pub recovery_protection: RecoveryProtectionStatus,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DurableIdentityActionDescriptor {
    pub intent_ref: ReceiptRef,
    pub kind: DurableIdentityActionKind,
    pub destination_ref: ResourceRef,
    pub authorization_ref: ProofRef,
    pub payload_digest: String,
    pub expires_at: u64,
}

impl DurableIdentityActionDescriptor {
    pub fn validate(&self, now: u64) -> Result<(), AuthenticationContractError> {
        if !is_lower_hex_64(&self.payload_digest)
            || self.expires_at <= now
            || self.expires_at > now.saturating_add(900)
        {
            return Err(AuthenticationContractError::InvalidSigningContext);
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HeldIdentityAction {
    pub intent_ref: ReceiptRef,
    pub account_ref: AccountRef,
    pub account_generation: u64,
    pub identity_ref: IdentityRef,
    pub kind: DurableIdentityActionKind,
    pub destination_ref: ResourceRef,
    pub authorization_ref: ProofRef,
    pub payload_digest: String,
    pub issued_at: u64,
    pub expires_at: u64,
}

impl HeldIdentityAction {
    pub(crate) fn validate(&self, now: u64) -> bool {
        self.account_generation > 0
            && is_lower_hex_64(&self.payload_digest)
            && self.issued_at > 0
            && self.issued_at < self.expires_at
            && self.expires_at > now
            && self.expires_at <= self.issued_at.saturating_add(900)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DurableIdentityActionDecision {
    Authorized(IdentityActionAuthorization),
    ActivationRequired {
        account: IdentityAccountRecord,
        intent: HeldIdentityAction,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[error("Set up identity {fingerprint} before this durable action")]
pub struct IdentityActivationRequired {
    fingerprint: String,
    account: IdentityAccountRecord,
    intent: HeldIdentityAction,
}

impl IdentityActivationRequired {
    pub fn new(account: IdentityAccountRecord, intent: HeldIdentityAction) -> Self {
        Self {
            fingerprint: account.fingerprint_display(),
            account,
            intent,
        }
    }

    pub fn account(&self) -> &IdentityAccountRecord {
        &self.account
    }

    pub fn intent(&self) -> &HeldIdentityAction {
        &self.intent
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IdentityActionAuthorization {
    intent: HeldIdentityAction,
}

impl IdentityActionAuthorization {
    pub(crate) fn new(intent: HeldIdentityAction) -> Self {
        Self { intent }
    }

    pub fn intent(&self) -> &HeldIdentityAction {
        &self.intent
    }
}

fn is_lower_hex_64(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || matches!(byte, b'a'..=b'f'))
}

#[cfg(test)]
mod tests {
    use super::DurableIdentityActionKind;

    #[test]
    fn workroom_activity_action_has_a_stable_public_name() {
        assert_eq!(
            serde_json::to_string(&DurableIdentityActionKind::WorkroomActivity)
                .expect("serialize Workroom identity action"),
            "\"workroom_activity\""
        );
    }
}
