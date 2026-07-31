use std::fmt;

use nostr::{
    Event, JsonUtil as _, PublicKey,
    secp256k1::{Message, schnorr::Signature},
};
use omega_agent_identity::{AgentAuthorization, AgentMethod};
use omega_identity::{
    AccountRef, AccountSelectionToken, AdmittedSigningRequest, AgentIdentityRef,
    AuthenticationEvidence, AuthorityDomain, CapabilityRef, IdentityRef, NostrPublicKeyHex,
    ProofRef, ReceiptRef, ResourceRef, SignerRef, SigningAuthorizationContext, SigningPurpose,
    SigningResult, SubsystemRef, UserGesture,
};
use sha2::{Digest as _, Sha256};
use thiserror::Error;

const MAX_SIGNING_AUTHORIZATION_SECONDS: u64 = 5 * 60;
const NIP_AA_AUTH_EVENT_FRESHNESS_SECONDS: u64 = 120;
pub const NIP_AA_AGENT_RELAY_PROFILE: &str = "openagents.nip-aa.agent-relay.v1";

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AssuredSignerPrincipal {
    Person,
    Agent {
        agent_identity_ref: AgentIdentityRef,
        owner_attestation_ref: ProofRef,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AssuredSigner {
    pub signer_ref: SignerRef,
    pub identity_ref: IdentityRef,
    pub public_key_hex: NostrPublicKeyHex,
    pub capability_ref: CapabilityRef,
    pub principal: AssuredSignerPrincipal,
}

impl AssuredSigner {
    fn validate_for_account(
        &self,
        selection: &AccountSelectionToken,
    ) -> Result<(), SigningAssuranceError> {
        match &self.principal {
            AssuredSignerPrincipal::Person => {
                if self.identity_ref != *selection.identity.identity_ref()
                    || self.public_key_hex != *selection.identity.public_key_hex()
                {
                    return Err(SigningAssuranceError::WrongSigner);
                }
            }
            AssuredSignerPrincipal::Agent {
                agent_identity_ref,
                owner_attestation_ref,
            } => {
                if agent_identity_ref.as_str() == selection.identity.identity_ref().as_str()
                    || self.identity_ref.as_str() == selection.identity.identity_ref().as_str()
                    || self.public_key_hex == *selection.identity.public_key_hex()
                    || owner_attestation_ref.as_str().is_empty()
                {
                    return Err(SigningAssuranceError::CollapsedPrincipal);
                }
            }
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SigningRequestScope {
    pub calling_subsystem: SubsystemRef,
    pub purpose: SigningPurpose,
    pub room_or_tenant: String,
    pub destination: ResourceRef,
    pub origin: String,
    pub capability_ref: CapabilityRef,
    pub user_gesture: UserGesture,
}

#[derive(Clone, PartialEq, Eq)]
pub struct AssuredSigningAdmission {
    selection: AccountSelectionToken,
    signer: AssuredSigner,
    authorization: SigningAuthorizationContext,
    request: AdmittedSigningRequest,
}

impl fmt::Debug for AssuredSigningAdmission {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("AssuredSigningAdmission")
            .field("account_ref", &self.selection.account_ref)
            .field("account_generation", &self.selection.generation)
            .field("signer_ref", &self.signer.signer_ref)
            .field("request_ref", &self.request.request_ref)
            .field("event_kind", &self.request.event.kind)
            .field("content_digest", &self.authorization.content_digest)
            .finish()
    }
}

impl AssuredSigningAdmission {
    pub fn admit(
        selection: AccountSelectionToken,
        signer: AssuredSigner,
        authorization: SigningAuthorizationContext,
        request: AdmittedSigningRequest,
        expected_scope: &SigningRequestScope,
        now: u64,
    ) -> Result<Self, SigningAssuranceError> {
        authorization
            .validate()
            .map_err(|_| SigningAssuranceError::InvalidAuthorization)?;
        request
            .validate()
            .map_err(|_| SigningAssuranceError::InvalidSigningRequest)?;
        signer.validate_for_account(&selection)?;

        if selection.generation == 0
            || authorization.account_ref != selection.account_ref
            || authorization.account_generation != selection.generation
        {
            return Err(SigningAssuranceError::StaleAccount);
        }
        if authorization.signer_ref != signer.signer_ref
            || authorization.capability_ref != signer.capability_ref
            || request.identity_ref != signer.identity_ref
        {
            return Err(SigningAssuranceError::WrongSigner);
        }
        if authorization.calling_subsystem != expected_scope.calling_subsystem
            || authorization.purpose != expected_scope.purpose
            || authorization.event_kind != request.event.kind
            || authorization.resource_ref != expected_scope.destination
            || authorization.origin != expected_scope.origin
            || authorization.capability_ref != expected_scope.capability_ref
            || authorization.user_gesture != expected_scope.user_gesture
            || request.purpose != expected_scope.purpose
        {
            return Err(SigningAssuranceError::ScopeMismatch);
        }
        if authorization.authorization_ref.as_str() != request.request_ref.as_str()
            || authorization.content_digest != signing_content_digest(&request)?
        {
            return Err(SigningAssuranceError::RequestMismatch);
        }
        validate_time(&authorization, now)?;

        Ok(Self {
            selection,
            signer,
            authorization,
            request,
        })
    }

    pub fn request(&self) -> &AdmittedSigningRequest {
        &self.request
    }

    pub fn admit_agent(
        selection: AccountSelectionToken,
        signer: AssuredSigner,
        authorization: SigningAuthorizationContext,
        request: AdmittedSigningRequest,
        expected_scope: &SigningRequestScope,
        grant_authorization: &AgentAuthorization,
        now: u64,
    ) -> Result<Self, SigningAssuranceError> {
        let admission = Self::admit(
            selection,
            signer,
            authorization,
            request,
            expected_scope,
            now,
        )?;
        let AssuredSignerPrincipal::Agent {
            agent_identity_ref, ..
        } = &admission.signer.principal
        else {
            return Err(SigningAssuranceError::WrongSigner);
        };
        if grant_authorization.owner_account_ref() != &admission.selection.account_ref
            || grant_authorization.account_generation() != admission.selection.generation
            || grant_authorization.agent_identity_ref() != agent_identity_ref
            || grant_authorization.agent_public_key_hex() != &admission.signer.public_key_hex
            || grant_authorization.request_ref() != &admission.request.request_ref
            || grant_authorization.signing_context() != Some(&admission.authorization)
            || grant_authorization.event_kind() != Some(admission.request.event.kind)
            || grant_authorization.room_or_tenant() != expected_scope.room_or_tenant
            || grant_authorization.destination_resource_ref() != &expected_scope.destination
            || grant_authorization.issued_at() != admission.authorization.issued_at
            || grant_authorization.expires_at() != admission.authorization.expires_at
            || method_capability(grant_authorization.method())
                != admission.authorization.capability_ref.as_str()
        {
            return Err(SigningAssuranceError::GrantMismatch);
        }
        Ok(admission)
    }

    pub fn authorization(&self) -> &SigningAuthorizationContext {
        &self.authorization
    }

    pub fn verify_response(
        &self,
        current_selection: &AccountSelectionToken,
        result: &SigningResult,
        now: u64,
    ) -> Result<Event, SigningAssuranceError> {
        if current_selection != &self.selection {
            return Err(SigningAssuranceError::StaleAccount);
        }
        validate_time(&self.authorization, now)?;
        if result.request_ref != self.request.request_ref
            || result.identity.identity_ref() != &self.signer.identity_ref
            || result.identity.public_key_hex() != &self.signer.public_key_hex
        {
            return Err(SigningAssuranceError::MismatchedResponse);
        }

        let event = Event::from_json(&result.signed_event_json)
            .map_err(|_| SigningAssuranceError::MismatchedResponse)?;
        event
            .verify()
            .map_err(|_| SigningAssuranceError::MismatchedResponse)?;
        let expected_id = self
            .request
            .event_id(&result.identity)
            .map_err(|_| SigningAssuranceError::MismatchedResponse)?;
        let tags = event
            .tags
            .iter()
            .map(|tag| tag.as_slice().to_vec())
            .collect::<Vec<_>>();
        if event.id != expected_id
            || event.id.to_hex() != result.event_id
            || event.sig.to_string() != result.signature
            || event.pubkey.to_hex() != self.signer.public_key_hex.as_str()
            || event.kind.as_u16() != self.request.event.kind
            || event.created_at.as_secs() != self.request.event.created_at
            || event.tags.len() != self.request.event.tags.len()
            || tags != self.request.event.tags
            || event.content.as_bytes() != self.request.event.content.as_bytes()
            || signing_content_digest(&self.request)? != self.authorization.content_digest
        {
            return Err(SigningAssuranceError::MismatchedResponse);
        }
        Ok(event)
    }
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

pub fn signing_content_digest(
    request: &AdmittedSigningRequest,
) -> Result<String, SigningAssuranceError> {
    let encoded = serde_json::to_vec(&request.event)
        .map_err(|_| SigningAssuranceError::InvalidSigningRequest)?;
    Ok(hex::encode(Sha256::digest(encoded)))
}

fn validate_time(
    authorization: &SigningAuthorizationContext,
    now: u64,
) -> Result<(), SigningAssuranceError> {
    if now < authorization.issued_at
        || now >= authorization.expires_at
        || authorization
            .expires_at
            .saturating_sub(authorization.issued_at)
            > MAX_SIGNING_AUTHORIZATION_SECONDS
    {
        return Err(SigningAssuranceError::Expired);
    }
    Ok(())
}

#[derive(Clone, PartialEq, Eq)]
pub struct NipAaProfileAdmission {
    profile_ref: String,
    owner_account_ref: AccountRef,
    owner_attestation_ref: ProofRef,
    agent_identity_ref: AgentIdentityRef,
    agent_public_key_hex: NostrPublicKeyHex,
    owner_auth_tag: Vec<String>,
    grant_ref: ReceiptRef,
    account_generation: u64,
    expires_at: u64,
}

impl fmt::Debug for NipAaProfileAdmission {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("NipAaProfileAdmission")
            .field("profile_ref", &self.profile_ref)
            .field("owner_account_ref", &self.owner_account_ref)
            .field("owner_attestation_ref", &self.owner_attestation_ref)
            .field("agent_identity_ref", &self.agent_identity_ref)
            .field("grant_ref", &self.grant_ref)
            .field("account_generation", &self.account_generation)
            .field("expires_at", &self.expires_at)
            .finish()
    }
}

pub fn admit_nip_aa_profile(
    selection: &AccountSelectionToken,
    profile_ref: &str,
    grant_authorization: &AgentAuthorization,
    now: u64,
) -> Result<NipAaProfileAdmission, SigningAssuranceError> {
    if profile_ref != NIP_AA_AGENT_RELAY_PROFILE {
        return Err(SigningAssuranceError::UnadmittedNipAaProfile);
    }
    if grant_authorization.owner_account_ref() != &selection.account_ref
        || grant_authorization.owner_public_key_hex() != selection.identity.public_key_hex()
        || grant_authorization.account_generation() != selection.generation
        || now >= grant_authorization.expires_at()
        || now >= grant_authorization.grant_expires_at()
        || grant_authorization.method() != AgentMethod::NipAaRelayAuthentication
    {
        return Err(SigningAssuranceError::StaleAccount);
    }
    if grant_authorization.agent_identity_ref().as_str()
        == selection.identity.identity_ref().as_str()
        || grant_authorization.agent_public_key_hex() == selection.identity.public_key_hex()
        || grant_authorization
            .owner_auth_tag()
            .get(2)
            .is_none_or(|conditions| conditions != grant_authorization.grant_conditions())
        || !valid_owner_auth_tag(
            grant_authorization.owner_auth_tag(),
            selection.identity.public_key_hex().as_str(),
            grant_authorization.agent_public_key_hex().as_str(),
        )
    {
        return Err(SigningAssuranceError::CollapsedPrincipal);
    }
    Ok(NipAaProfileAdmission {
        profile_ref: profile_ref.to_string(),
        owner_account_ref: grant_authorization.owner_account_ref().clone(),
        owner_attestation_ref: grant_authorization.owner_attestation_ref().clone(),
        agent_identity_ref: grant_authorization.agent_identity_ref().clone(),
        agent_public_key_hex: grant_authorization.agent_public_key_hex().clone(),
        owner_auth_tag: grant_authorization.owner_auth_tag().to_vec(),
        grant_ref: grant_authorization.grant_ref().clone(),
        account_generation: grant_authorization.account_generation(),
        expires_at: grant_authorization.grant_expires_at(),
    })
}

#[derive(Clone, PartialEq, Eq)]
pub struct NipAaRelayAuthenticationAdmission {
    profile: NipAaProfileAdmission,
    relay_url: String,
    connection_generation: u64,
    challenge_ref: ProofRef,
    signing: AssuredSigningAdmission,
}

impl fmt::Debug for NipAaRelayAuthenticationAdmission {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("NipAaRelayAuthenticationAdmission")
            .field("profile", &self.profile)
            .field("relay_url", &self.relay_url)
            .field("connection_generation", &self.connection_generation)
            .field("challenge_ref", &self.challenge_ref)
            .field("signing_request_ref", &self.signing.request.request_ref)
            .finish()
    }
}

impl NipAaRelayAuthenticationAdmission {
    pub fn admit(
        profile: NipAaProfileAdmission,
        relay_url: impl Into<String>,
        connection_generation: u64,
        challenge: &str,
        challenge_ref: ProofRef,
        signing: AssuredSigningAdmission,
        now: u64,
    ) -> Result<Self, SigningAssuranceError> {
        let relay_url = relay_url.into();
        let expected_resource = format!(
            "relay.{}",
            hex::encode(Sha256::digest(relay_url.as_bytes()))
        );
        let event_tags = &signing.request.event.tags;
        let expected_challenge_ref = nip_aa_challenge_ref(challenge)?;
        let exact_nip_aa_tags = event_tags.len() == 3
            && event_tags
                .iter()
                .filter(|tag| tag.as_slice() == ["relay", relay_url.as_str()])
                .count()
                == 1
            && event_tags
                .iter()
                .filter(|tag| tag.as_slice() == ["challenge", challenge])
                .count()
                == 1
            && event_tags
                .iter()
                .filter(|tag| tag.as_slice() == profile.owner_auth_tag.as_slice())
                .count()
                == 1;
        if profile.expires_at <= now
            || profile.owner_account_ref != signing.selection.account_ref
            || profile.account_generation != signing.selection.generation
            || signing.signer.public_key_hex != profile.agent_public_key_hex
            || !matches!(
                &signing.signer.principal,
                AssuredSignerPrincipal::Agent {
                    agent_identity_ref,
                    owner_attestation_ref,
                } if agent_identity_ref == &profile.agent_identity_ref
                    && owner_attestation_ref == &profile.owner_attestation_ref
            )
            || connection_generation == 0
            || signing.authorization.calling_subsystem.as_str() != "nostr.relay-auth"
            || signing.authorization.capability_ref.as_str() != "nip-aa.relay-auth"
            || signing.authorization.resource_ref.as_str() != expected_resource
            || signing.authorization.origin != relay_url
            || signing.request.event.kind != 22_242
            || !signing.request.event.content.is_empty()
            || signing.request.event.created_at.abs_diff(now) > NIP_AA_AUTH_EVENT_FRESHNESS_SECONDS
            || challenge.is_empty()
            || challenge.len() > 2_048
            || challenge.chars().any(char::is_control)
            || challenge_ref != expected_challenge_ref
            || !normalized_relay_url(&relay_url)
            || !exact_nip_aa_tags
        {
            return Err(SigningAssuranceError::UnadmittedNipAaProfile);
        }
        Ok(Self {
            profile,
            relay_url,
            connection_generation,
            challenge_ref,
            signing,
        })
    }

    pub fn complete(
        &self,
        current_selection: &AccountSelectionToken,
        result: &SigningResult,
        now: u64,
    ) -> Result<NipAaRelayAuthenticationReceipt, SigningAssuranceError> {
        if self.signing.request.event.created_at.abs_diff(now) > NIP_AA_AUTH_EVENT_FRESHNESS_SECONDS
        {
            return Err(SigningAssuranceError::Expired);
        }
        let event = self
            .signing
            .verify_response(current_selection, result, now)?;
        Ok(NipAaRelayAuthenticationReceipt {
            profile_ref: self.profile.profile_ref.clone(),
            owner_account_ref: self.profile.owner_account_ref.clone(),
            owner_attestation_ref: self.profile.owner_attestation_ref.clone(),
            grant_ref: self.profile.grant_ref.clone(),
            account_generation: self.profile.account_generation,
            grant_expires_at: self.profile.expires_at,
            relay_url: self.relay_url.clone(),
            connection_generation: self.connection_generation,
            challenge_ref: self.challenge_ref.clone(),
            agent_identity_ref: self.profile.agent_identity_ref.clone(),
            agent_public_key_hex: self.profile.agent_public_key_hex.clone(),
            auth_event_id: event.id.to_hex(),
            authenticated_at: now,
        })
    }
}

pub fn nip_aa_challenge_ref(challenge: &str) -> Result<ProofRef, SigningAssuranceError> {
    if challenge.is_empty() || challenge.len() > 2_048 || challenge.chars().any(char::is_control) {
        return Err(SigningAssuranceError::UnadmittedNipAaProfile);
    }
    ProofRef::new(format!(
        "challenge.{}",
        hex::encode(Sha256::digest(challenge.as_bytes()))
    ))
    .map_err(|_| SigningAssuranceError::UnadmittedNipAaProfile)
}

fn normalized_relay_url(relay_url: &str) -> bool {
    let Ok(parsed) = url::Url::parse(relay_url) else {
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
    normalized == relay_url
}

fn valid_owner_auth_tag(
    tag: &[String],
    owner_public_key_hex: &str,
    agent_public_key_hex: &str,
) -> bool {
    if tag.len() != 4
        || tag.first().is_none_or(|value| value != "auth")
        || tag.get(1).is_none_or(|value| value != owner_public_key_hex)
        || tag.get(2).is_none_or(|conditions| {
            conditions.len() > 1_024 || conditions.chars().any(char::is_control)
        })
    {
        return false;
    }
    let Ok(owner) = PublicKey::from_hex(owner_public_key_hex) else {
        return false;
    };
    let Ok(signature) = tag[3].parse::<Signature>() else {
        return false;
    };
    let digest =
        Sha256::digest(format!("nostr:agent-auth:{agent_public_key_hex}:{}", tag[2]).as_bytes());
    let Ok(owner) = owner.xonly() else {
        return false;
    };
    nostr::SECP256K1
        .verify_schnorr(&signature, &Message::from_digest(digest.into()), &owner)
        .is_ok()
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NipAaRelayAuthenticationReceipt {
    pub profile_ref: String,
    pub owner_account_ref: AccountRef,
    pub owner_attestation_ref: ProofRef,
    pub grant_ref: ReceiptRef,
    pub account_generation: u64,
    pub grant_expires_at: u64,
    pub relay_url: String,
    pub connection_generation: u64,
    pub challenge_ref: ProofRef,
    pub agent_identity_ref: AgentIdentityRef,
    pub agent_public_key_hex: NostrPublicKeyHex,
    pub auth_event_id: String,
    pub authenticated_at: u64,
}

impl NipAaRelayAuthenticationReceipt {
    pub fn evidence(&self) -> AuthenticationEvidence {
        AuthenticationEvidence::RelayConnection {
            relay_url: self.relay_url.clone(),
            auth_event_id: self.auth_event_id.clone(),
            authenticated_at: self.authenticated_at,
        }
    }

    pub fn proves(&self, domain: AuthorityDomain) -> bool {
        domain == AuthorityDomain::RelayConnection
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Error)]
pub enum SigningAssuranceError {
    #[error("the signing authorization is invalid")]
    InvalidAuthorization,
    #[error("the signing request is invalid")]
    InvalidSigningRequest,
    #[error("the active account or generation changed")]
    StaleAccount,
    #[error("the admitted signer does not match the request")]
    WrongSigner,
    #[error("person and agent principals must remain distinct")]
    CollapsedPrincipal,
    #[error("the signing scope does not match the admitted operation")]
    ScopeMismatch,
    #[error("the request reference or content digest does not match")]
    RequestMismatch,
    #[error("the bounded agent grant does not match the signing request")]
    GrantMismatch,
    #[error("the signing authorization is not currently valid")]
    Expired,
    #[error("the signer returned a mismatched response")]
    MismatchedResponse,
    #[error("NIP-AA is not admitted for this exact profile and relay")]
    UnadmittedNipAaProfile,
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use nostr::{EventBuilder, Keys, Kind, Tag, Timestamp};
    use omega_agent_identity::{
        AgentAuthorizationRequest, AgentGrantScope, AgentIdentityPlan, AgentIdentityStore,
    };
    use omega_identity::{
        OwnerAttestationResult, PublicIdentity, ReceiptRef, UnsignedEventTemplate,
    };

    use super::*;

    const NOW: u64 = 1_700_000_000;

    fn identity(keys: &Keys, reference: &str) -> PublicIdentity {
        PublicIdentity::from_public_key_hex(
            IdentityRef::new(reference).expect("identity ref"),
            keys.public_key().to_hex(),
        )
        .expect("identity")
    }

    fn selection(keys: &Keys) -> AccountSelectionToken {
        AccountSelectionToken {
            account_ref: AccountRef::new("owner-account").expect("account"),
            identity: identity(keys, "owner-identity"),
            generation: 7,
        }
    }

    fn owner_auth_tag(owner: &Keys, agent: &Keys) -> Vec<String> {
        owner_auth_tag_with_conditions(owner, agent.public_key().to_hex(), "room=sarah")
    }

    fn owner_auth_tag_with_conditions(
        owner: &Keys,
        agent_public_key_hex: String,
        conditions: &str,
    ) -> Vec<String> {
        let digest = Sha256::digest(
            format!("nostr:agent-auth:{agent_public_key_hex}:{conditions}").as_bytes(),
        );
        vec![
            "auth".to_string(),
            owner.public_key().to_hex(),
            conditions.to_string(),
            owner
                .sign_schnorr(&Message::from_digest(digest.into()))
                .to_string(),
        ]
    }

    fn request(identity_ref: IdentityRef, owner_auth_tag: Vec<String>) -> AdmittedSigningRequest {
        AdmittedSigningRequest {
            request_ref: ReceiptRef::new("sign-request-1").expect("request"),
            identity_ref,
            purpose: SigningPurpose::NostrEvent,
            event: UnsignedEventTemplate {
                created_at: NOW,
                kind: 22_242,
                tags: vec![
                    vec!["relay".to_string(), "wss://relay.example".to_string()],
                    vec!["challenge".to_string(), "challenge-1".to_string()],
                    owner_auth_tag,
                ],
                content: String::new(),
            },
        }
    }

    fn scope(relay: &str) -> SigningRequestScope {
        SigningRequestScope {
            calling_subsystem: SubsystemRef::new("nostr.relay-auth").expect("subsystem"),
            purpose: SigningPurpose::NostrEvent,
            room_or_tenant: "openagents.sarah".to_string(),
            destination: ResourceRef::new(format!(
                "relay.{}",
                hex::encode(Sha256::digest(relay.as_bytes()))
            ))
            .expect("resource"),
            origin: relay.to_string(),
            capability_ref: CapabilityRef::new("nip-aa.relay-auth").expect("capability"),
            user_gesture: UserGesture::NotRequired,
        }
    }

    fn agent_signer(keys: &Keys) -> AssuredSigner {
        AssuredSigner {
            signer_ref: SignerRef::new("signer.agent.sarah").expect("signer"),
            identity_ref: IdentityRef::new("agent.sarah").expect("identity"),
            public_key_hex: NostrPublicKeyHex::new(keys.public_key().to_hex()).expect("key"),
            capability_ref: CapabilityRef::new("nip-aa.relay-auth").expect("capability"),
            principal: AssuredSignerPrincipal::Agent {
                agent_identity_ref: AgentIdentityRef::new("agent.sarah").expect("agent"),
                owner_attestation_ref: ProofRef::new("owner-attestation-1").expect("proof"),
            },
        }
    }

    fn authorization(
        selection: &AccountSelectionToken,
        signer: &AssuredSigner,
        request: &AdmittedSigningRequest,
        relay: &str,
    ) -> SigningAuthorizationContext {
        let scope = scope(relay);
        SigningAuthorizationContext {
            authorization_ref: ProofRef::new(request.request_ref.as_str()).expect("proof"),
            account_ref: selection.account_ref.clone(),
            account_generation: selection.generation,
            signer_ref: signer.signer_ref.clone(),
            calling_subsystem: scope.calling_subsystem,
            purpose: request.purpose,
            event_kind: request.event.kind,
            resource_ref: scope.destination,
            origin: relay.to_string(),
            content_digest: signing_content_digest(request).expect("digest"),
            capability_ref: signer.capability_ref.clone(),
            user_gesture: UserGesture::NotRequired,
            issued_at: NOW,
            expires_at: NOW + 60,
        }
    }

    fn signing_result(keys: &Keys, request: &AdmittedSigningRequest) -> SigningResult {
        let signed = EventBuilder::new(
            Kind::from(request.event.kind),
            request.event.content.clone(),
        )
        .tags(
            request
                .event
                .tags
                .iter()
                .map(|tag| Tag::parse(tag.clone()).expect("tag")),
        )
        .custom_created_at(Timestamp::from_secs(request.event.created_at))
        .sign_with_keys(keys)
        .expect("signed");
        SigningResult {
            request_ref: request.request_ref.clone(),
            identity: identity(keys, request.identity_ref.as_str()),
            event_id: signed.id.to_hex(),
            signature: signed.sig.to_string(),
            signed_event_json: signed.as_json(),
        }
    }

    #[test]
    fn assurance_binds_every_request_field_and_exact_response() {
        let owner = Keys::generate();
        let agent = Keys::generate();
        let selection = selection(&owner);
        let signer = agent_signer(&agent);
        let mut request = request(signer.identity_ref.clone(), owner_auth_tag(&owner, &agent));
        request.event.content = "PRIVATE_PROMPT_DO_NOT_LOG".to_string();
        let exact_authorization =
            authorization(&selection, &signer, &request, "wss://relay.example");
        let admission = AssuredSigningAdmission::admit(
            selection.clone(),
            signer,
            exact_authorization,
            request.clone(),
            &scope("wss://relay.example"),
            NOW,
        )
        .expect("admission");
        let debug = format!("{admission:?}");
        assert!(!debug.contains("PRIVATE_PROMPT_DO_NOT_LOG"));
        assert!(!debug.contains(&request.event.tags[2][3]));
        admission
            .verify_response(&selection, &signing_result(&agent, &request), NOW + 1)
            .expect("exact response");

        let mut wrong_origin = scope("wss://other.example");
        wrong_origin.destination = scope("wss://relay.example").destination;
        assert_eq!(
            AssuredSigningAdmission::admit(
                selection.clone(),
                agent_signer(&agent),
                authorization(
                    &selection,
                    &agent_signer(&agent),
                    &request,
                    "wss://relay.example"
                ),
                request.clone(),
                &wrong_origin,
                NOW,
            ),
            Err(SigningAssuranceError::ScopeMismatch)
        );
    }

    #[test]
    fn wrong_generation_digest_request_id_expiry_and_signer_are_refused() {
        let owner = Keys::generate();
        let agent = Keys::generate();
        let selection = selection(&owner);
        let signer = agent_signer(&agent);
        let request = request(signer.identity_ref.clone(), owner_auth_tag(&owner, &agent));
        let exact = authorization(&selection, &signer, &request, "wss://relay.example");

        let mut stale = exact.clone();
        stale.account_generation += 1;
        assert_eq!(
            AssuredSigningAdmission::admit(
                selection.clone(),
                signer.clone(),
                stale,
                request.clone(),
                &scope("wss://relay.example"),
                NOW,
            ),
            Err(SigningAssuranceError::StaleAccount)
        );

        let mut wrong_digest = exact.clone();
        wrong_digest.content_digest = "0".repeat(64);
        assert_eq!(
            AssuredSigningAdmission::admit(
                selection.clone(),
                signer.clone(),
                wrong_digest,
                request.clone(),
                &scope("wss://relay.example"),
                NOW,
            ),
            Err(SigningAssuranceError::RequestMismatch)
        );
        let mut wrong_request_ref = exact.clone();
        wrong_request_ref.authorization_ref = ProofRef::new("different-request").expect("request");
        assert_eq!(
            AssuredSigningAdmission::admit(
                selection.clone(),
                signer.clone(),
                wrong_request_ref,
                request.clone(),
                &scope("wss://relay.example"),
                NOW,
            ),
            Err(SigningAssuranceError::RequestMismatch)
        );

        let admission = AssuredSigningAdmission::admit(
            selection.clone(),
            signer,
            exact,
            request.clone(),
            &scope("wss://relay.example"),
            NOW,
        )
        .expect("admission");
        let mut wrong_response = signing_result(&agent, &request);
        wrong_response.request_ref = ReceiptRef::new("other-request").expect("request");
        assert_eq!(
            admission.verify_response(&selection, &wrong_response, NOW + 1),
            Err(SigningAssuranceError::MismatchedResponse)
        );
        assert_eq!(
            admission.verify_response(&selection, &signing_result(&agent, &request), NOW + 60),
            Err(SigningAssuranceError::Expired)
        );
        assert_eq!(
            admission.verify_response(
                &selection,
                &signing_result(&Keys::generate(), &request),
                NOW + 1
            ),
            Err(SigningAssuranceError::MismatchedResponse)
        );
    }

    #[test]
    fn person_signer_cannot_be_used_as_an_agent() {
        let owner = Keys::generate();
        let selection = selection(&owner);
        let collapsed = AssuredSigner {
            signer_ref: SignerRef::new("signer.agent.sarah").expect("signer"),
            identity_ref: selection.identity.identity_ref().clone(),
            public_key_hex: selection.identity.public_key_hex().clone(),
            capability_ref: CapabilityRef::new("nip-aa.relay-auth").expect("capability"),
            principal: AssuredSignerPrincipal::Agent {
                agent_identity_ref: AgentIdentityRef::new("agent.sarah").expect("agent"),
                owner_attestation_ref: ProofRef::new("attestation").expect("proof"),
            },
        };
        let request = request(
            collapsed.identity_ref.clone(),
            owner_auth_tag(&owner, &owner),
        );
        assert_eq!(
            AssuredSigningAdmission::admit(
                selection.clone(),
                collapsed.clone(),
                authorization(&selection, &collapsed, &request, "wss://relay.example"),
                request,
                &scope("wss://relay.example"),
                NOW,
            ),
            Err(SigningAssuranceError::CollapsedPrincipal)
        );
    }

    #[test]
    fn nip_aa_requires_explicit_profile_and_only_proves_relay_connection() {
        let owner = Keys::generate();
        let selection = selection(&owner);
        let directory = tempfile::tempdir().expect("tempdir");
        let store = AgentIdentityStore::new(directory.path().join("agent-store"));
        let prepared = store
            .prepare_agent_identity(
                &selection,
                AgentIdentityPlan {
                    request_ref: ReceiptRef::new("owner-attestation-1").expect("request"),
                    grant_ref: ReceiptRef::new("agent-grant-1").expect("grant"),
                    owner_account_ref: selection.account_ref.clone(),
                    owner_identity: selection.identity.clone(),
                    agent_identity_ref: AgentIdentityRef::new("agent.sarah").expect("agent"),
                    label: "Sarah".to_string(),
                    scope: AgentGrantScope {
                        methods: BTreeSet::from([AgentMethod::NipAaRelayAuthentication]),
                        event_kinds: BTreeSet::from([22_242]),
                        rooms_or_tenants: BTreeSet::from(["openagents.sarah".to_string()]),
                    },
                    account_generation: selection.generation,
                    issued_at: NOW,
                    expires_at: NOW + 3_600,
                },
            )
            .expect("prepare");
        let owner_tag = owner_auth_tag_with_conditions(
            &owner,
            prepared.agent_public_key_hex.as_str().to_string(),
            &prepared.owner_attestation_request.conditions,
        );
        let projection = store
            .complete_owner_attestation(
                &selection,
                &prepared.request_ref,
                &OwnerAttestationResult {
                    request_ref: prepared.request_ref.clone(),
                    identity: selection.identity.clone(),
                    agent_public_key_hex: prepared.agent_public_key_hex.clone(),
                    auth_tag: owner_tag,
                },
            )
            .expect("complete");
        let signer = AssuredSigner {
            signer_ref: projection.signer_ref.clone(),
            identity_ref: IdentityRef::new(projection.agent_identity_ref.as_str())
                .expect("identity"),
            public_key_hex: projection.agent_public_key_hex.clone(),
            capability_ref: CapabilityRef::new("nip-aa.relay-auth").expect("capability"),
            principal: AssuredSignerPrincipal::Agent {
                agent_identity_ref: projection.agent_identity_ref.clone(),
                owner_attestation_ref: projection.grant.owner_attestation_ref.clone(),
            },
        };
        let request = request(
            signer.identity_ref.clone(),
            projection.owner_auth_tag.clone(),
        );
        let signing_context = authorization(&selection, &signer, &request, "wss://relay.example");
        let grant_authorization = store
            .authorize(
                &selection,
                AgentAuthorizationRequest {
                    request_ref: request.request_ref.clone(),
                    owner_account_ref: selection.account_ref.clone(),
                    account_generation: selection.generation,
                    agent_identity_ref: projection.agent_identity_ref.clone(),
                    grant_ref: projection.grant.grant_ref,
                    method: AgentMethod::NipAaRelayAuthentication,
                    event_kind: Some(22_242),
                    room_or_tenant: "openagents.sarah".to_string(),
                    destination_resource_ref: scope("wss://relay.example").destination,
                    issued_at: NOW,
                    expires_at: NOW + 60,
                    signing_context: Some(signing_context.clone()),
                },
                NOW,
            )
            .expect("grant authorization");
        let mut wrong_room = scope("wss://relay.example");
        wrong_room.room_or_tenant = "openagents.other".to_string();
        assert_eq!(
            AssuredSigningAdmission::admit_agent(
                selection.clone(),
                signer.clone(),
                signing_context.clone(),
                request.clone(),
                &wrong_room,
                &grant_authorization,
                NOW,
            ),
            Err(SigningAssuranceError::GrantMismatch)
        );
        let signing = AssuredSigningAdmission::admit_agent(
            selection.clone(),
            signer.clone(),
            signing_context.clone(),
            request.clone(),
            &scope("wss://relay.example"),
            &grant_authorization,
            NOW,
        )
        .expect("signing");
        assert_eq!(
            admit_nip_aa_profile(&selection, "nip42", &grant_authorization, NOW),
            Err(SigningAssuranceError::UnadmittedNipAaProfile)
        );

        let profile = admit_nip_aa_profile(
            &selection,
            NIP_AA_AGENT_RELAY_PROFILE,
            &grant_authorization,
            NOW,
        )
        .expect("profile");
        let mut stale_request = request.clone();
        stale_request.event.created_at = NOW - NIP_AA_AUTH_EVENT_FRESHNESS_SECONDS - 1;
        let stale_context =
            authorization(&selection, &signer, &stale_request, "wss://relay.example");
        let stale_signing = AssuredSigningAdmission::admit(
            selection.clone(),
            signer,
            stale_context,
            stale_request,
            &scope("wss://relay.example"),
            NOW,
        )
        .expect("base signing admission");
        assert_eq!(
            NipAaRelayAuthenticationAdmission::admit(
                profile.clone(),
                "wss://relay.example",
                3,
                "challenge-1",
                nip_aa_challenge_ref("challenge-1").expect("challenge"),
                stale_signing,
                NOW,
            ),
            Err(SigningAssuranceError::UnadmittedNipAaProfile)
        );
        let admission = NipAaRelayAuthenticationAdmission::admit(
            profile,
            "wss://relay.example",
            3,
            "challenge-1",
            nip_aa_challenge_ref("challenge-1").expect("challenge"),
            signing,
            NOW,
        )
        .expect("NIP-AA admission");
        let result = store
            .sign_authorized_event(
                &selection,
                &grant_authorization,
                &signing_context,
                &request,
                NOW + 1,
            )
            .expect("core sign");
        let receipt = admission
            .complete(&selection, &result, NOW + 1)
            .expect("receipt");
        assert!(receipt.proves(AuthorityDomain::RelayConnection));
        assert!(!receipt.proves(AuthorityDomain::GroupAdmission));
        assert!(!receipt.proves(AuthorityDomain::HostedAccount));
        assert!(!receipt.proves(AuthorityDomain::Action));
        assert_eq!(
            receipt.evidence().domain(),
            AuthorityDomain::RelayConnection
        );
    }
}
