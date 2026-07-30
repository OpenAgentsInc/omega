use omega_invites::{
    AuthorityLabel, InvitePreview, InviteProfile, JoinPlan, JoinStepKind, JoinStepStatus,
    OpaqueInviteEvidence, ParsedInvite, PlannedJoinStep, ResolvedInvite, SigningOperation,
    SupportLevel, TermsRequirement, Visibility,
};
use sha2::{Digest as _, Sha256};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InviteProtocol {
    Nip29,
    Buzz,
    ArmadaConcordV1,
    ArmadaConcordV2,
    OpenAgents,
    Unsupported,
}

impl InviteProtocol {
    pub fn label(self) -> &'static str {
        match self {
            Self::Nip29 => "NIP-29",
            Self::Buzz => "Buzz",
            Self::ArmadaConcordV1 => "Armada Concord v1",
            Self::ArmadaConcordV2 => "Armada Concord v2",
            Self::OpenAgents => "OpenAgents",
            Self::Unsupported => "Unsupported",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InviteAuthorityScope {
    Nip29Relay,
    BuzzService,
    ConcordCommunity,
    OpenAgentsService,
    Unknown,
}

impl InviteAuthorityScope {
    pub fn label(self) -> &'static str {
        match self {
            Self::Nip29Relay => "NIP-29 relay",
            Self::BuzzService => "Buzz service",
            Self::ConcordCommunity => "Concord community",
            Self::OpenAgentsService => "OpenAgents service",
            Self::Unknown => "Unknown authority",
        }
    }

    pub fn can_grant_openagents_authority(self) -> bool {
        self == Self::OpenAgentsService
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InviteVisibility {
    Public,
    Private,
    Sealed,
    Unknown,
}

impl InviteVisibility {
    pub fn label(self) -> &'static str {
        match self {
            Self::Public => "Public",
            Self::Private => "Private",
            Self::Sealed => "Sealed",
            Self::Unknown => "Unknown",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InviteRecovery {
    NostrIdentity,
    ServiceAccount,
    ConcordInviteAndEpochKeys,
    Unsupported,
}

impl InviteRecovery {
    pub fn label(self) -> &'static str {
        match self {
            Self::NostrIdentity => "Nostr identity",
            Self::ServiceAccount => "Service account",
            Self::ConcordInviteAndEpochKeys => "Invite and epoch keys",
            Self::Unsupported => "Unsupported",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InvitePortability {
    IndependentNip29,
    Buzz,
    Armada,
    Web,
    Mobile,
}

impl InvitePortability {
    pub fn label(self) -> &'static str {
        match self {
            Self::IndependentNip29 => "Independent NIP-29",
            Self::Buzz => "Buzz",
            Self::Armada => "Armada",
            Self::Web => "Web",
            Self::Mobile => "Mobile",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InviteOperation {
    AddRelay,
    AuthenticateNip42,
    ClaimInvite,
    JoinNip29,
    ImportConcordMembership,
    RequestOpenAgentsGrant,
}

impl InviteOperation {
    pub fn label(self) -> &'static str {
        match self {
            Self::AddRelay => "Add relay",
            Self::AuthenticateNip42 => "Authenticate relay",
            Self::ClaimInvite => "Claim invite",
            Self::JoinNip29 => "Join NIP-29 group",
            Self::ImportConcordMembership => "Import Concord membership",
            Self::RequestOpenAgentsGrant => "Request OpenAgents grant",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InviteTerms {
    NotRequired,
    ResolveFromAuthority,
    Required,
    Accepted,
    Unknown,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InvitePreviewProjection {
    pub protocol: InviteProtocol,
    pub authority_scope: InviteAuthorityScope,
    pub authority_label: String,
    pub room_label: String,
    pub visibility: InviteVisibility,
    pub terms: InviteTerms,
    pub operations: Vec<InviteOperation>,
    pub recovery: InviteRecovery,
    pub portability: Vec<InvitePortability>,
    pub join_supported: bool,
    pub opaque_evidence_ref: Option<String>,
}

impl InvitePreviewProjection {
    pub fn can_commit(&self) -> bool {
        self.join_supported
            && matches!(self.terms, InviteTerms::NotRequired | InviteTerms::Accepted)
    }

    pub fn openagents_authority_is_explicit(&self) -> bool {
        self.operations
            .contains(&InviteOperation::RequestOpenAgentsGrant)
            && self.authority_scope.can_grant_openagents_authority()
    }
}

impl InvitePreviewProjection {
    pub fn from_core(preview: &InvitePreview) -> Self {
        let (authority_scope, authority_label) = match &preview.authority {
            AuthorityLabel::Nip29Relay(reference) => {
                (InviteAuthorityScope::Nip29Relay, reference.clone())
            }
            AuthorityLabel::BuzzService(reference) => {
                (InviteAuthorityScope::BuzzService, reference.clone())
            }
            AuthorityLabel::ArmadaControlPlane(reference) => {
                (InviteAuthorityScope::ConcordCommunity, reference.clone())
            }
            AuthorityLabel::OpenAgentsForge(reference) => {
                (InviteAuthorityScope::OpenAgentsService, reference.clone())
            }
            AuthorityLabel::Unknown => (InviteAuthorityScope::Unknown, "Unknown".to_string()),
        };
        let protocol = match preview.profile {
            InviteProfile::Nip29 => InviteProtocol::Nip29,
            InviteProfile::Buzz => InviteProtocol::Buzz,
            InviteProfile::ArmadaConcordV1 => InviteProtocol::ArmadaConcordV1,
            InviteProfile::ArmadaConcordV2 => InviteProtocol::ArmadaConcordV2,
            InviteProfile::OpenAgentsV1 => InviteProtocol::OpenAgents,
            InviteProfile::Unsupported => InviteProtocol::Unsupported,
        };
        let terms = match preview.terms {
            TermsRequirement::None => InviteTerms::NotRequired,
            TermsRequirement::ResolveFromAuthority => InviteTerms::ResolveFromAuthority,
            TermsRequirement::Declared { .. } => InviteTerms::Required,
            TermsRequirement::Unknown => InviteTerms::Unknown,
        };
        let recovery = match preview.recovery {
            omega_invites::RecoveryImplication::RelayRecoverable => InviteRecovery::NostrIdentity,
            omega_invites::RecoveryImplication::ServiceAccountRequired => {
                InviteRecovery::ServiceAccount
            }
            omega_invites::RecoveryImplication::EncryptedCommunityMaterialRequired => {
                InviteRecovery::ConcordInviteAndEpochKeys
            }
            omega_invites::RecoveryImplication::LocalClaimOnly => InviteRecovery::NostrIdentity,
            omega_invites::RecoveryImplication::Unknown => InviteRecovery::Unsupported,
        };
        let operations = preview
            .signing_operations
            .iter()
            .copied()
            .map(|operation| match operation {
                SigningOperation::Nip42Authenticate => InviteOperation::AuthenticateNip42,
                SigningOperation::Nip29JoinRequest | SigningOperation::UpdateNip29GroupList => {
                    InviteOperation::JoinNip29
                }
                SigningOperation::Nip98InviteClaim => InviteOperation::ClaimInvite,
                SigningOperation::FetchArmadaControlPlane => {
                    InviteOperation::ImportConcordMembership
                }
                SigningOperation::OpenAgentsGrantProof => InviteOperation::RequestOpenAgentsGrant,
            })
            .collect();
        let mut portability = Vec::new();
        if preview.profile == InviteProfile::Nip29 {
            portability.push(InvitePortability::IndependentNip29);
        }
        if preview.portability.buzz {
            portability.push(InvitePortability::Buzz);
        }
        if preview.portability.armada {
            portability.push(InvitePortability::Armada);
        }
        if preview.portability.web {
            portability.push(InvitePortability::Web);
        }
        if preview.portability.mobile {
            portability.push(InvitePortability::Mobile);
        }
        Self {
            protocol,
            authority_scope,
            authority_label,
            room_label: preview
                .room_reference
                .clone()
                .unwrap_or_else(|| "Authority assigned".to_string()),
            visibility: match preview.visibility {
                Visibility::Public => InviteVisibility::Public,
                Visibility::Private
                    if matches!(
                        preview.profile,
                        InviteProfile::ArmadaConcordV1 | InviteProfile::ArmadaConcordV2
                    ) =>
                {
                    InviteVisibility::Sealed
                }
                Visibility::Private => InviteVisibility::Private,
                Visibility::AuthorityDetermined | Visibility::Unknown => InviteVisibility::Unknown,
            },
            terms,
            operations,
            recovery,
            portability,
            join_supported: preview.support == SupportLevel::Executable,
            opaque_evidence_ref: preview
                .opaque_evidence
                .as_ref()
                .map(|evidence| format!("sha256:{}:{}", evidence.sha256, evidence.byte_length)),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InviteRefusal {
    Malformed,
    Stale,
    Banned,
    TermsRequired,
    UnsupportedProfile,
}

impl InviteRefusal {
    pub fn label(self) -> &'static str {
        match self {
            Self::Malformed => "Malformed invite",
            Self::Stale => "Stale invite",
            Self::Banned => "Account banned",
            Self::TermsRequired => "Terms required",
            Self::UnsupportedProfile => "Unsupported profile",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum JoinStepState {
    Pending,
    Complete,
    Failed,
    Unsupported,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct JoinStepProjection {
    pub operation: InviteOperation,
    pub state: JoinStepState,
    pub receipt_ref: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct JoinTransactionProjection {
    pub transaction_ref: String,
    pub protocol: InviteProtocol,
    pub authority_scope: InviteAuthorityScope,
    pub authority_label: String,
    pub room_label: String,
    pub steps: Vec<JoinStepProjection>,
}

impl JoinTransactionProjection {
    pub fn from_core(
        transaction: &omega_invites::JoinTransactionProjection,
        preview: &InvitePreviewProjection,
    ) -> Self {
        Self {
            transaction_ref: transaction.transaction_ref.clone(),
            protocol: preview.protocol,
            authority_scope: preview.authority_scope,
            authority_label: preview.authority_label.clone(),
            room_label: preview.room_label.clone(),
            steps: transaction
                .steps
                .iter()
                .map(|step| JoinStepProjection {
                    operation: match step.kind {
                        JoinStepKind::AddRelay => InviteOperation::AddRelay,
                        JoinStepKind::Nip42Authenticate => InviteOperation::AuthenticateNip42,
                        JoinStepKind::ClaimBuzzInvite => InviteOperation::ClaimInvite,
                        JoinStepKind::RequestNip29Join
                        | JoinStepKind::AwaitNip29Admission
                        | JoinStepKind::UpdateNip29GroupList => InviteOperation::JoinNip29,
                        JoinStepKind::VerifyOpenAgentsGrant | JoinStepKind::PersistLocalClaim => {
                            InviteOperation::RequestOpenAgentsGrant
                        }
                    },
                    state: match step.status {
                        JoinStepStatus::Pending | JoinStepStatus::Prepared => {
                            JoinStepState::Pending
                        }
                        JoinStepStatus::Succeeded | JoinStepStatus::Skipped => {
                            JoinStepState::Complete
                        }
                        JoinStepStatus::FailedRetryable | JoinStepStatus::FailedTerminal => {
                            JoinStepState::Failed
                        }
                    },
                    receipt_ref: step.receipt_ref.clone(),
                })
                .collect(),
        }
    }

    pub fn is_complete(&self) -> bool {
        !self.steps.is_empty()
            && self
                .steps
                .iter()
                .all(|step| step.state == JoinStepState::Complete)
    }

    pub fn can_resume(&self) -> bool {
        !self.is_complete()
            && self
                .steps
                .iter()
                .any(|step| matches!(step.state, JoinStepState::Pending | JoinStepState::Failed))
    }

    pub fn openagents_grant_is_complete(&self) -> bool {
        self.authority_scope.can_grant_openagents_authority()
            && self.steps.iter().any(|step| {
                step.operation == InviteOperation::RequestOpenAgentsGrant
                    && step.state == JoinStepState::Complete
            })
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum InviteControlState {
    Empty,
    Preview(InvitePreviewProjection),
    Refused {
        refusal: InviteRefusal,
        opaque_evidence_ref: Option<String>,
    },
    Transaction(JoinTransactionProjection),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InviteControl {
    state: InviteControlState,
}

impl Default for InviteControl {
    fn default() -> Self {
        Self {
            state: InviteControlState::Empty,
        }
    }
}

impl InviteControl {
    pub fn state(&self) -> &InviteControlState {
        &self.state
    }

    pub fn present_preview(&mut self, preview: InvitePreviewProjection) {
        self.state = InviteControlState::Preview(preview);
    }

    pub fn present_refusal(&mut self, refusal: InviteRefusal, opaque_evidence_ref: Option<String>) {
        self.state = InviteControlState::Refused {
            refusal,
            opaque_evidence_ref,
        };
    }

    pub fn present_transaction(&mut self, transaction: JoinTransactionProjection) {
        self.state = InviteControlState::Transaction(transaction);
    }

    pub fn clear(&mut self) {
        self.state = InviteControlState::Empty;
    }
}

pub fn redacted_invite_input_label(input: &str) -> &'static str {
    let normalized = input.trim().to_ascii_lowercase();
    if normalized.starts_with("buzz:") || normalized.contains("buzz") {
        "Buzz invite"
    } else if normalized.starts_with("concord:")
        || normalized.starts_with("armada:")
        || normalized.contains("armada")
    {
        "Armada invite"
    } else if normalized.starts_with("openagents:")
        || normalized.starts_with("omega:")
        || normalized.contains("openagents")
    {
        "OpenAgents invite"
    } else if normalized.starts_with("naddr")
        || normalized.starts_with("wss://")
        || normalized.contains("nip29")
    {
        "NIP-29 destination"
    } else {
        "Invite"
    }
}

pub fn join_plan_for_resolved(
    resolved: &ResolvedInvite,
    exact_input: &[u8],
    created_at: u64,
) -> Result<JoinPlan, omega_invites::JoinStoreError> {
    let evidence =
        resolved
            .preview
            .opaque_evidence
            .clone()
            .unwrap_or_else(|| OpaqueInviteEvidence {
                profile_hint: resolved.preview.profile,
                sha256: format!("{:x}", Sha256::digest(exact_input)),
                byte_length: exact_input.len(),
            });
    let mut steps = Vec::new();
    match &resolved.parsed {
        ParsedInvite::Nip29Group(group) => {
            return crate::omega_nostr_group_join::nip29_join_plan(group, created_at);
        }
        ParsedInvite::Nip29Relay(relay) => {
            steps.push(PlannedJoinStep::new(
                JoinStepKind::AddRelay,
                true,
                relay.relay_url.as_bytes().to_vec(),
            )?);
            steps.push(PlannedJoinStep::new(
                JoinStepKind::UpdateNip29GroupList,
                true,
                exact_input.to_vec(),
            )?);
        }
        ParsedInvite::Buzz(_) => {
            steps.push(PlannedJoinStep::new(
                JoinStepKind::ClaimBuzzInvite,
                true,
                exact_input.to_vec(),
            )?);
            steps.push(PlannedJoinStep::new(
                JoinStepKind::UpdateNip29GroupList,
                true,
                exact_input.to_vec(),
            )?);
        }
        ParsedInvite::OpenAgents(_) => {
            steps.push(PlannedJoinStep::new(
                JoinStepKind::VerifyOpenAgentsGrant,
                true,
                exact_input.to_vec(),
            )?);
            steps.push(PlannedJoinStep::new(
                JoinStepKind::PersistLocalClaim,
                true,
                exact_input.to_vec(),
            )?);
        }
        ParsedInvite::Armada(_) | ParsedInvite::Unsupported(_) => {
            return Err(omega_invites::JoinStoreError::InvalidPlan);
        }
    }
    JoinPlan::new(evidence, steps)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn preview(
        protocol: InviteProtocol,
        authority_scope: InviteAuthorityScope,
    ) -> InvitePreviewProjection {
        InvitePreviewProjection {
            protocol,
            authority_scope,
            authority_label: authority_scope.label().to_string(),
            room_label: "Community".to_string(),
            visibility: InviteVisibility::Private,
            terms: InviteTerms::NotRequired,
            operations: vec![InviteOperation::ClaimInvite],
            recovery: InviteRecovery::NostrIdentity,
            portability: vec![InvitePortability::Web, InvitePortability::Mobile],
            join_supported: true,
            opaque_evidence_ref: None,
        }
    }

    #[test]
    fn every_admitted_protocol_keeps_its_authority_label() {
        for (protocol, authority) in [
            (InviteProtocol::Nip29, InviteAuthorityScope::Nip29Relay),
            (InviteProtocol::Buzz, InviteAuthorityScope::BuzzService),
            (
                InviteProtocol::ArmadaConcordV1,
                InviteAuthorityScope::ConcordCommunity,
            ),
            (
                InviteProtocol::ArmadaConcordV2,
                InviteAuthorityScope::ConcordCommunity,
            ),
            (
                InviteProtocol::OpenAgents,
                InviteAuthorityScope::OpenAgentsService,
            ),
        ] {
            let preview = preview(protocol, authority);
            assert_eq!(preview.protocol.label(), protocol.label());
            assert_eq!(preview.authority_label, authority.label());
        }
    }

    #[test]
    fn refusal_outcomes_remain_distinct() {
        let refusals = [
            InviteRefusal::Malformed,
            InviteRefusal::Stale,
            InviteRefusal::Banned,
            InviteRefusal::TermsRequired,
            InviteRefusal::UnsupportedProfile,
        ];
        for refusal in refusals {
            assert_ne!(refusal.label(), "");
        }
        assert_eq!(
            InviteRefusal::UnsupportedProfile.label(),
            "Unsupported profile"
        );
    }

    #[test]
    fn unsupported_and_unaccepted_terms_cannot_commit() {
        let mut unsupported = preview(
            InviteProtocol::ArmadaConcordV2,
            InviteAuthorityScope::ConcordCommunity,
        );
        unsupported.join_supported = false;
        unsupported.recovery = InviteRecovery::Unsupported;
        unsupported.opaque_evidence_ref = Some("opaque:sha256:1234".to_string());
        assert!(!unsupported.can_commit());

        let mut terms = preview(InviteProtocol::Buzz, InviteAuthorityScope::BuzzService);
        terms.terms = InviteTerms::Required;
        assert!(!terms.can_commit());
        terms.terms = InviteTerms::Accepted;
        assert!(terms.can_commit());
    }

    #[test]
    fn partial_transactions_are_visible_and_resumable() {
        let transaction = JoinTransactionProjection {
            transaction_ref: "join:public-reference".to_string(),
            protocol: InviteProtocol::Nip29,
            authority_scope: InviteAuthorityScope::Nip29Relay,
            authority_label: "relay.example.com".to_string(),
            room_label: "Community".to_string(),
            steps: vec![
                JoinStepProjection {
                    operation: InviteOperation::AddRelay,
                    state: JoinStepState::Complete,
                    receipt_ref: Some("relay:receipt".to_string()),
                },
                JoinStepProjection {
                    operation: InviteOperation::AuthenticateNip42,
                    state: JoinStepState::Failed,
                    receipt_ref: Some("auth:receipt".to_string()),
                },
                JoinStepProjection {
                    operation: InviteOperation::JoinNip29,
                    state: JoinStepState::Pending,
                    receipt_ref: None,
                },
            ],
        };
        assert!(!transaction.is_complete());
        assert!(transaction.can_resume());
    }

    #[test]
    fn non_openagents_authorities_cannot_imply_an_openagents_grant() {
        for authority in [
            InviteAuthorityScope::Nip29Relay,
            InviteAuthorityScope::BuzzService,
            InviteAuthorityScope::ConcordCommunity,
        ] {
            let mut preview = preview(InviteProtocol::Nip29, authority);
            preview
                .operations
                .push(InviteOperation::RequestOpenAgentsGrant);
            assert!(!preview.openagents_authority_is_explicit());

            let transaction = JoinTransactionProjection {
                transaction_ref: "join:public-reference".to_string(),
                protocol: preview.protocol,
                authority_scope: authority,
                authority_label: authority.label().to_string(),
                room_label: "Community".to_string(),
                steps: vec![JoinStepProjection {
                    operation: InviteOperation::RequestOpenAgentsGrant,
                    state: JoinStepState::Complete,
                    receipt_ref: Some("service:receipt".to_string()),
                }],
            };
            assert!(!transaction.openagents_grant_is_complete());
        }
    }

    #[test]
    fn secret_codes_and_fragments_are_never_returned_by_redaction() {
        let inputs = [
            "buzz://join/community?code=super-secret",
            "armada://concord#epoch-key-material",
            "openagents://invite?token=bearer-secret",
            "wss://relay.example.com/group#invite-secret",
        ];
        for input in inputs {
            let label = redacted_invite_input_label(input);
            assert!(!label.contains("secret"));
            assert!(!label.contains('?'));
            assert!(!label.contains('#'));
        }
    }
}
