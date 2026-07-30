use std::{
    collections::BTreeSet,
    fmt, fs,
    io::{self, Read as _, Write as _},
    path::{Path, PathBuf},
    sync::{Mutex, MutexGuard, OnceLock},
};

use atomic_write_file::AtomicWriteFile;
use nostr::{Event, FromBech32 as _, JsonUtil as _, nips::nip19::Nip19Coordinate};
use omega_community::Invitation;
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};
use thiserror::Error;
use url::Url;

const MAX_INPUT_BYTES: usize = 16 * 1024;
const MAX_OPAQUE_BYTES: usize = 8 * 1024;
const MAX_RELAY_HINTS: usize = 4;
const MAX_TRANSACTION_BYTES: u64 = 1024 * 1024;
const MAX_SIGNED_EVENT_BYTES: usize = 64 * 1024;
const TRANSACTION_SCHEMA: &str = "openagents.omega.join-transaction.v1";

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum InviteError {
    #[error("the invitation is empty or exceeds its size limit")]
    InvalidSize,
    #[error("the invitation uses a known profile but has an invalid shape")]
    InvalidShape,
    #[error("the invitation contains an invalid or ambiguous relay")]
    InvalidRelay,
    #[error("the NIP-29 group address is invalid")]
    InvalidNip29Group,
    #[error("the Buzz invitation is invalid")]
    InvalidBuzzInvite,
    #[error("the Armada invitation is invalid")]
    InvalidArmadaInvite,
    #[error("the OpenAgents invitation is invalid")]
    InvalidOpenAgentsInvite,
}

#[derive(Debug, Error)]
pub enum JoinStoreError {
    #[error("the join transaction reference is invalid")]
    InvalidReference,
    #[error("the account selection fence is invalid")]
    InvalidFence,
    #[error("the selected account or generation changed")]
    StaleGeneration,
    #[error("the join plan is invalid")]
    InvalidPlan,
    #[error("the requested join step does not exist")]
    UnknownStep,
    #[error("the join step is in an invalid state")]
    InvalidStepState,
    #[error("the prepared request digest changed")]
    RequestChanged,
    #[error("the signed event is invalid or does not match its declared event id")]
    InvalidSignedEvent,
    #[error("the join transaction store is unavailable")]
    Storage,
    #[error("the join transaction is malformed or unsupported")]
    InvalidStoredTransaction,
    #[error("the join transaction has already been completed")]
    Completed,
    #[error("the join mutation failed: {0}")]
    Mutation(String),
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InviteProfile {
    Nip29,
    Buzz,
    ArmadaConcordV1,
    ArmadaConcordV2,
    OpenAgentsV1,
    Unsupported,
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InviteKind {
    RelayQualifiedGroup,
    Relay,
    ServiceInvite,
    SealedCommunityInvite,
    ForgeClaim,
    Opaque,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind", content = "reference")]
pub enum AuthorityLabel {
    Nip29Relay(String),
    BuzzService(String),
    ArmadaControlPlane(String),
    OpenAgentsForge(String),
    Unknown,
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SupportLevel {
    Executable,
    PreviewOnly,
    Unsupported,
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CapabilityBlocker {
    BuzzAuthorityAdapter,
    ArmadaCryptography,
    ArmadaControlPlane,
    OpenAgentsForgeVerification,
    UnknownProfile,
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Visibility {
    Public,
    Private,
    AuthorityDetermined,
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "state")]
pub enum TermsRequirement {
    None,
    ResolveFromAuthority,
    Declared {
        terms_url: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        privacy_url: Option<String>,
        age_attestation_required: bool,
    },
    Unknown,
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RecoveryImplication {
    RelayRecoverable,
    ServiceAccountRequired,
    EncryptedCommunityMaterialRequired,
    LocalClaimOnly,
    Unknown,
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SigningOperation {
    Nip42Authenticate,
    Nip29JoinRequest,
    Nip98InviteClaim,
    UpdateNip29GroupList,
    FetchArmadaControlPlane,
    OpenAgentsGrantProof,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PortabilityMatrix {
    pub omega: bool,
    pub armada: bool,
    pub buzz: bool,
    pub web: bool,
    pub mobile: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OpaqueInviteEvidence {
    pub profile_hint: InviteProfile,
    pub sha256: String,
    pub byte_length: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InvitePreview {
    pub profile: InviteProfile,
    pub kind: InviteKind,
    pub support: SupportLevel,
    pub authority: AuthorityLabel,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub room_reference: Option<String>,
    pub visibility: Visibility,
    pub terms: TermsRequirement,
    pub recovery: RecoveryImplication,
    pub portability: PortabilityMatrix,
    pub signing_operations: BTreeSet<SigningOperation>,
    #[serde(default)]
    pub capability_blockers: BTreeSet<CapabilityBlocker>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub opaque_evidence: Option<OpaqueInviteEvidence>,
}

#[derive(Debug, Error, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ResolveRefusal {
    #[error("the invitation or authority record is stale")]
    Stale,
    #[error("the joining account is banned")]
    Banned,
    #[error("the authority requires terms acceptance")]
    TermsRequired,
    #[error("the authority requires an age attestation")]
    AgeAttestationRequired,
    #[error("the invitation or grant was revoked")]
    Revoked,
    #[error("the Armada control plane could not be read")]
    ControlPlaneUnreadable,
    #[error("the required profile capability is not installed")]
    CapabilityMissing,
    #[error("the authority response could not be verified")]
    VerificationFailed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AuthorityResolution {
    pub authority: AuthorityLabel,
    pub authority_receipt_ref: String,
    pub resolved_at: u64,
    pub expires_at: u64,
    pub terms: TermsRequirement,
}

pub trait InviteAuthorityResolver {
    fn resolve_authority(
        &mut self,
        invite: &ResolvedInvite,
        now: u64,
    ) -> Result<AuthorityResolution, ResolveRefusal>;
}

#[derive(Clone, PartialEq, Eq)]
struct SensitiveInviteMaterial(Vec<u8>);

impl SensitiveInviteMaterial {
    fn new(bytes: impl Into<Vec<u8>>) -> Result<Self, InviteError> {
        let bytes = bytes.into();
        if bytes.is_empty() || bytes.len() > MAX_OPAQUE_BYTES {
            return Err(InviteError::InvalidSize);
        }
        Ok(Self(bytes))
    }

    fn expose(&self) -> &[u8] {
        &self.0
    }
}

impl fmt::Debug for SensitiveInviteMaterial {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("SensitiveInviteMaterial([REDACTED])")
    }
}

#[derive(Clone, PartialEq, Eq)]
pub struct Nip29GroupInvite {
    pub relay_url: String,
    pub relay_public_key_hex: String,
    pub group_id: String,
    invite_code: Option<SensitiveInviteMaterial>,
}

impl Nip29GroupInvite {
    pub fn invite_code(&self) -> Option<&[u8]> {
        self.invite_code
            .as_ref()
            .map(SensitiveInviteMaterial::expose)
    }
}

impl fmt::Debug for Nip29GroupInvite {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Nip29GroupInvite")
            .field("relay_url", &self.relay_url)
            .field("relay_public_key_hex", &self.relay_public_key_hex)
            .field("group_id", &self.group_id)
            .field(
                "invite_code",
                &self.invite_code.as_ref().map(|_| "[REDACTED]"),
            )
            .finish()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Nip29RelayInvite {
    pub relay_url: String,
}

#[derive(Clone, PartialEq, Eq)]
pub struct BuzzInvite {
    pub service_origin: String,
    pub relay_hint_requested: bool,
    code: SensitiveInviteMaterial,
}

impl BuzzInvite {
    pub fn code(&self) -> &[u8] {
        self.code.expose()
    }
}

impl fmt::Debug for BuzzInvite {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("BuzzInvite")
            .field("service_origin", &self.service_origin)
            .field("relay_hint_requested", &self.relay_hint_requested)
            .field("code", &"[REDACTED]")
            .finish()
    }
}

#[derive(Clone, PartialEq, Eq)]
pub struct ArmadaInvite {
    pub version: InviteProfile,
    pub web_origin: String,
    pub control_coordinate: Option<String>,
    pub relay_hints: Vec<String>,
    sealed_fragment: SensitiveInviteMaterial,
}

impl ArmadaInvite {
    pub fn sealed_fragment(&self) -> &[u8] {
        self.sealed_fragment.expose()
    }
}

impl fmt::Debug for ArmadaInvite {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ArmadaInvite")
            .field("version", &self.version)
            .field("web_origin", &self.web_origin)
            .field("control_coordinate", &self.control_coordinate)
            .field("relay_hints", &self.relay_hints)
            .field("sealed_fragment", &"[REDACTED]")
            .finish()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OpenAgentsInvite {
    pub tenant_ref: String,
    pub repository_ref: String,
    pub coordinate: String,
    pub relays: Vec<String>,
    pub name: String,
    pub binding_ref: String,
    pub actor_kind: String,
    pub membership_state: String,
    pub roles: Vec<String>,
}

#[derive(Clone, PartialEq, Eq)]
pub struct UnsupportedInvite {
    pub evidence: OpaqueInviteEvidence,
    material: SensitiveInviteMaterial,
}

impl UnsupportedInvite {
    pub fn material(&self) -> &[u8] {
        self.material.expose()
    }
}

impl fmt::Debug for UnsupportedInvite {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("UnsupportedInvite")
            .field("evidence", &self.evidence)
            .field("material", &"[REDACTED]")
            .finish()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ParsedInvite {
    Nip29Group(Nip29GroupInvite),
    Nip29Relay(Nip29RelayInvite),
    Buzz(BuzzInvite),
    Armada(ArmadaInvite),
    OpenAgents(OpenAgentsInvite),
    Unsupported(UnsupportedInvite),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedInvite {
    pub parsed: ParsedInvite,
    pub preview: InvitePreview,
}

#[derive(Default)]
pub struct InviteResolver;

impl InviteResolver {
    pub fn resolve(&self, input: &str) -> Result<ResolvedInvite, InviteError> {
        let input = input.trim();
        if input.is_empty() || input.len() > MAX_INPUT_BYTES {
            return Err(InviteError::InvalidSize);
        }
        if input.starts_with("omega-invite:") {
            return resolve_openagents(input);
        }
        if input.starts_with("nostr:naddr1") || input.starts_with("naddr1") {
            return resolve_nip29_group(input);
        }
        if input.starts_with("ws://") || input.starts_with("wss://") {
            return resolve_nip29_relay(input);
        }
        if input.starts_with("http://") || input.starts_with("https://") {
            let url = Url::parse(input).map_err(|_| InviteError::InvalidShape)?;
            let segments = path_segments(&url)?;
            if segments.first().map(String::as_str) == Some("invite") {
                if segments.len() == 2 && segments[1].starts_with("naddr1") {
                    return resolve_armada_v2(url, &segments[1]);
                }
                if segments.len() == 1 && url.fragment().is_some() {
                    return resolve_armada_v1(url);
                }
                if segments.len() == 2 {
                    return resolve_buzz(url, &segments[1]);
                }
                return Err(InviteError::InvalidShape);
            }
        }
        unsupported(input)
    }
}

fn resolve_nip29_group(input: &str) -> Result<ResolvedInvite, InviteError> {
    let (address, invite_code) = match input.split_once('?') {
        Some((address, query)) => {
            let mut pairs = query.split('&');
            let pair = pairs.next().ok_or(InviteError::InvalidNip29Group)?;
            if pairs.next().is_some() {
                return Err(InviteError::InvalidNip29Group);
            }
            let (key, value) = pair.split_once('=').ok_or(InviteError::InvalidNip29Group)?;
            if key != "invite" || value.is_empty() {
                return Err(InviteError::InvalidNip29Group);
            }
            (
                address,
                Some(SensitiveInviteMaterial::new(value.as_bytes().to_vec())?),
            )
        }
        None => (input, None),
    };
    let bech32 = address.strip_prefix("nostr:").unwrap_or(address);
    let coordinate =
        Nip19Coordinate::from_bech32(bech32).map_err(|_| InviteError::InvalidNip29Group)?;
    if coordinate.kind.as_u16() != 39_000
        || coordinate.identifier.is_empty()
        || coordinate.identifier.len() > 256
        || coordinate.relays.len() != 1
    {
        return Err(InviteError::InvalidNip29Group);
    }
    let relay_url = normalize_relay(coordinate.relays[0].as_str(), false)?;
    let group_id = coordinate.identifier.clone();
    let parsed = Nip29GroupInvite {
        relay_url: relay_url.clone(),
        relay_public_key_hex: coordinate.public_key.to_hex(),
        group_id: group_id.clone(),
        invite_code,
    };
    Ok(ResolvedInvite {
        parsed: ParsedInvite::Nip29Group(parsed),
        preview: InvitePreview {
            profile: InviteProfile::Nip29,
            kind: InviteKind::RelayQualifiedGroup,
            support: SupportLevel::Executable,
            authority: AuthorityLabel::Nip29Relay(relay_url),
            room_reference: Some(group_id),
            visibility: Visibility::AuthorityDetermined,
            terms: TermsRequirement::None,
            recovery: RecoveryImplication::RelayRecoverable,
            portability: portability(true, true, true, true, true),
            signing_operations: [
                SigningOperation::Nip42Authenticate,
                SigningOperation::Nip29JoinRequest,
                SigningOperation::UpdateNip29GroupList,
            ]
            .into_iter()
            .collect(),
            capability_blockers: BTreeSet::new(),
            opaque_evidence: None,
        },
    })
}

fn resolve_nip29_relay(input: &str) -> Result<ResolvedInvite, InviteError> {
    let relay_url = normalize_relay(input, false)?;
    Ok(ResolvedInvite {
        parsed: ParsedInvite::Nip29Relay(Nip29RelayInvite {
            relay_url: relay_url.clone(),
        }),
        preview: InvitePreview {
            profile: InviteProfile::Nip29,
            kind: InviteKind::Relay,
            support: SupportLevel::Executable,
            authority: AuthorityLabel::Nip29Relay(relay_url),
            room_reference: None,
            visibility: Visibility::AuthorityDetermined,
            terms: TermsRequirement::None,
            recovery: RecoveryImplication::RelayRecoverable,
            portability: portability(true, true, true, true, true),
            signing_operations: [SigningOperation::UpdateNip29GroupList]
                .into_iter()
                .collect(),
            capability_blockers: BTreeSet::new(),
            opaque_evidence: None,
        },
    })
}

fn resolve_buzz(url: Url, code: &str) -> Result<ResolvedInvite, InviteError> {
    if url.scheme() != "https"
        || url.fragment().is_some()
        || !valid_dotted_code(code)
        || has_credentials(&url)
    {
        return Err(InviteError::InvalidBuzzInvite);
    }
    let mut relay_hint_requested = false;
    let mut query_seen = false;
    for (key, value) in url.query_pairs() {
        if query_seen || key != "r" || value != "true-relay-host" {
            return Err(InviteError::InvalidBuzzInvite);
        }
        query_seen = true;
        relay_hint_requested = true;
    }
    let origin = origin(&url)?;
    Ok(ResolvedInvite {
        parsed: ParsedInvite::Buzz(BuzzInvite {
            service_origin: origin.clone(),
            relay_hint_requested,
            code: SensitiveInviteMaterial::new(code.as_bytes().to_vec())?,
        }),
        preview: InvitePreview {
            profile: InviteProfile::Buzz,
            kind: InviteKind::ServiceInvite,
            support: SupportLevel::PreviewOnly,
            authority: AuthorityLabel::BuzzService(origin),
            room_reference: None,
            visibility: Visibility::AuthorityDetermined,
            terms: TermsRequirement::ResolveFromAuthority,
            recovery: RecoveryImplication::ServiceAccountRequired,
            portability: portability(true, true, true, true, true),
            signing_operations: [
                SigningOperation::Nip98InviteClaim,
                SigningOperation::UpdateNip29GroupList,
            ]
            .into_iter()
            .collect(),
            capability_blockers: [CapabilityBlocker::BuzzAuthorityAdapter]
                .into_iter()
                .collect(),
            opaque_evidence: None,
        },
    })
}

fn resolve_armada_v1(url: Url) -> Result<ResolvedInvite, InviteError> {
    if url.scheme() != "https" || has_credentials(&url) {
        return Err(InviteError::InvalidArmadaInvite);
    }
    let fragment = url
        .fragment()
        .filter(|fragment| valid_opaque_fragment(fragment))
        .ok_or(InviteError::InvalidArmadaInvite)?
        .as_bytes()
        .to_vec();
    let mut relay_hints = Vec::new();
    let mut query_seen = false;
    for (key, value) in url.query_pairs() {
        if query_seen || key != "relays" {
            return Err(InviteError::InvalidArmadaInvite);
        }
        query_seen = true;
        relay_hints = parse_relay_list(&value)?;
    }
    let web_origin = origin(&url)?;
    let evidence = opaque_evidence(InviteProfile::ArmadaConcordV1, &fragment);
    Ok(ResolvedInvite {
        parsed: ParsedInvite::Armada(ArmadaInvite {
            version: InviteProfile::ArmadaConcordV1,
            web_origin: web_origin.clone(),
            control_coordinate: None,
            relay_hints,
            sealed_fragment: SensitiveInviteMaterial::new(fragment)?,
        }),
        preview: InvitePreview {
            profile: InviteProfile::ArmadaConcordV1,
            kind: InviteKind::SealedCommunityInvite,
            support: SupportLevel::PreviewOnly,
            authority: AuthorityLabel::ArmadaControlPlane(web_origin),
            room_reference: None,
            visibility: Visibility::Private,
            terms: TermsRequirement::Unknown,
            recovery: RecoveryImplication::EncryptedCommunityMaterialRequired,
            portability: portability(true, true, false, false, false),
            signing_operations: [SigningOperation::FetchArmadaControlPlane]
                .into_iter()
                .collect(),
            capability_blockers: [
                CapabilityBlocker::ArmadaCryptography,
                CapabilityBlocker::ArmadaControlPlane,
            ]
            .into_iter()
            .collect(),
            opaque_evidence: Some(evidence),
        },
    })
}

fn resolve_armada_v2(url: Url, encoded_coordinate: &str) -> Result<ResolvedInvite, InviteError> {
    if url.scheme() != "https" || has_credentials(&url) || url.query().is_some() {
        return Err(InviteError::InvalidArmadaInvite);
    }
    let coordinate = Nip19Coordinate::from_bech32(encoded_coordinate)
        .map_err(|_| InviteError::InvalidArmadaInvite)?;
    if coordinate.kind.as_u16() != 33_301
        || !coordinate.identifier.is_empty()
        || coordinate.relays.len() > MAX_RELAY_HINTS
    {
        return Err(InviteError::InvalidArmadaInvite);
    }
    let relay_hints = coordinate
        .relays
        .iter()
        .map(|relay| normalize_relay(relay.as_str(), true))
        .collect::<Result<Vec<_>, _>>()?;
    let fragment = url
        .fragment()
        .filter(|fragment| valid_opaque_fragment(fragment))
        .ok_or(InviteError::InvalidArmadaInvite)?
        .as_bytes()
        .to_vec();
    let web_origin = origin(&url)?;
    let evidence = opaque_evidence(InviteProfile::ArmadaConcordV2, &fragment);
    Ok(ResolvedInvite {
        parsed: ParsedInvite::Armada(ArmadaInvite {
            version: InviteProfile::ArmadaConcordV2,
            web_origin,
            control_coordinate: Some(encoded_coordinate.to_string()),
            relay_hints,
            sealed_fragment: SensitiveInviteMaterial::new(fragment)?,
        }),
        preview: InvitePreview {
            profile: InviteProfile::ArmadaConcordV2,
            kind: InviteKind::SealedCommunityInvite,
            support: SupportLevel::PreviewOnly,
            authority: AuthorityLabel::ArmadaControlPlane(format!(
                "33301:{}",
                coordinate.public_key.to_hex()
            )),
            room_reference: Some(encoded_coordinate.to_string()),
            visibility: Visibility::Private,
            terms: TermsRequirement::Unknown,
            recovery: RecoveryImplication::EncryptedCommunityMaterialRequired,
            portability: portability(true, true, false, false, false),
            signing_operations: [SigningOperation::FetchArmadaControlPlane]
                .into_iter()
                .collect(),
            capability_blockers: [
                CapabilityBlocker::ArmadaCryptography,
                CapabilityBlocker::ArmadaControlPlane,
            ]
            .into_iter()
            .collect(),
            opaque_evidence: Some(evidence),
        },
    })
}

fn resolve_openagents(input: &str) -> Result<ResolvedInvite, InviteError> {
    let invitation = Invitation::parse(input).map_err(|_| InviteError::InvalidOpenAgentsInvite)?;
    let relays = invitation
        .descriptor
        .relays
        .iter()
        .map(|relay| normalize_relay(relay, true))
        .collect::<Result<Vec<_>, _>>()
        .map_err(|_| InviteError::InvalidOpenAgentsInvite)?;
    if relays.is_empty() || relays.len() > MAX_RELAY_HINTS {
        return Err(InviteError::InvalidOpenAgentsInvite);
    }
    let roles = invitation
        .membership
        .role_refs
        .iter()
        .cloned()
        .map(String::from)
        .collect();
    let actor_kind = match invitation.membership.actor_kind {
        omega_community::ActorKind::Human => "human",
        omega_community::ActorKind::Agent => "agent",
    }
    .to_string();
    let membership_state = match invitation.membership.membership_state {
        omega_community::MembershipState::Active => "active",
        omega_community::MembershipState::Tombstoned => "tombstoned",
    }
    .to_string();
    let tenant = invitation.descriptor.tenant_ref.clone();
    let room_reference = format!("{}:{}", tenant, invitation.descriptor.repository_ref);
    Ok(ResolvedInvite {
        parsed: ParsedInvite::OpenAgents(OpenAgentsInvite {
            tenant_ref: tenant.clone(),
            repository_ref: invitation.descriptor.repository_ref.clone(),
            coordinate: invitation.descriptor.coordinate.to_string(),
            relays,
            name: invitation.descriptor.name.clone(),
            binding_ref: invitation.membership.binding_ref,
            actor_kind,
            membership_state,
            roles,
        }),
        preview: InvitePreview {
            profile: InviteProfile::OpenAgentsV1,
            kind: InviteKind::ForgeClaim,
            support: SupportLevel::PreviewOnly,
            authority: AuthorityLabel::OpenAgentsForge(tenant),
            room_reference: Some(room_reference),
            visibility: Visibility::AuthorityDetermined,
            terms: TermsRequirement::Unknown,
            recovery: RecoveryImplication::LocalClaimOnly,
            portability: portability(true, false, false, false, false),
            signing_operations: [SigningOperation::OpenAgentsGrantProof]
                .into_iter()
                .collect(),
            capability_blockers: [CapabilityBlocker::OpenAgentsForgeVerification]
                .into_iter()
                .collect(),
            opaque_evidence: None,
        },
    })
}

fn unsupported(input: &str) -> Result<ResolvedInvite, InviteError> {
    let material = SensitiveInviteMaterial::new(input.as_bytes().to_vec())?;
    let evidence = opaque_evidence(InviteProfile::Unsupported, material.expose());
    Ok(ResolvedInvite {
        parsed: ParsedInvite::Unsupported(UnsupportedInvite {
            evidence: evidence.clone(),
            material,
        }),
        preview: InvitePreview {
            profile: InviteProfile::Unsupported,
            kind: InviteKind::Opaque,
            support: SupportLevel::Unsupported,
            authority: AuthorityLabel::Unknown,
            room_reference: None,
            visibility: Visibility::Unknown,
            terms: TermsRequirement::Unknown,
            recovery: RecoveryImplication::Unknown,
            portability: portability(false, false, false, false, false),
            signing_operations: BTreeSet::new(),
            capability_blockers: [CapabilityBlocker::UnknownProfile].into_iter().collect(),
            opaque_evidence: Some(evidence),
        },
    })
}

fn path_segments(url: &Url) -> Result<Vec<String>, InviteError> {
    url.path_segments()
        .ok_or(InviteError::InvalidShape)
        .map(|segments| {
            segments
                .filter(|segment| !segment.is_empty())
                .map(str::to_string)
                .collect()
        })
}

fn origin(url: &Url) -> Result<String, InviteError> {
    let host = url.host_str().ok_or(InviteError::InvalidShape)?;
    let mut origin = format!("{}://{host}", url.scheme());
    if let Some(port) = url.port() {
        origin.push(':');
        origin.push_str(&port.to_string());
    }
    Ok(origin)
}

fn has_credentials(url: &Url) -> bool {
    !url.username().is_empty() || url.password().is_some()
}

fn valid_dotted_code(code: &str) -> bool {
    code.len() <= 512
        && code.split('.').count() >= 2
        && code.split('.').all(|segment| {
            !segment.is_empty()
                && segment.len() <= 128
                && segment
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
        })
}

fn valid_opaque_fragment(fragment: &str) -> bool {
    !fragment.is_empty()
        && fragment.len() <= MAX_OPAQUE_BYTES
        && !fragment.bytes().any(|byte| byte.is_ascii_control())
}

fn normalize_relay(value: &str, allow_ws: bool) -> Result<String, InviteError> {
    let relay = Url::parse(value).map_err(|_| InviteError::InvalidRelay)?;
    if !(relay.scheme() == "wss" || (allow_ws && relay.scheme() == "ws"))
        || relay.host_str().is_none()
        || has_credentials(&relay)
        || relay.query().is_some()
        || relay.fragment().is_some()
    {
        return Err(InviteError::InvalidRelay);
    }
    Ok(relay.to_string())
}

fn parse_relay_list(value: &str) -> Result<Vec<String>, InviteError> {
    let values = value
        .split(',')
        .map(str::trim)
        .filter(|relay| !relay.is_empty())
        .map(|relay| normalize_relay(relay, true))
        .collect::<Result<Vec<_>, _>>()?;
    let unique = values.iter().collect::<BTreeSet<_>>();
    if values.is_empty() || values.len() > MAX_RELAY_HINTS || unique.len() != values.len() {
        return Err(InviteError::InvalidArmadaInvite);
    }
    Ok(values)
}

fn opaque_evidence(profile_hint: InviteProfile, bytes: &[u8]) -> OpaqueInviteEvidence {
    OpaqueInviteEvidence {
        profile_hint,
        sha256: hex_digest(bytes),
        byte_length: bytes.len(),
    }
}

fn portability(
    omega: bool,
    armada: bool,
    buzz: bool,
    web: bool,
    mobile: bool,
) -> PortabilityMatrix {
    PortabilityMatrix {
        omega,
        armada,
        buzz,
        web,
        mobile,
    }
}

fn hex_digest(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct JoinAccountFence {
    pub account_ref: String,
    pub account_public_key_hex: String,
    pub generation: u64,
}

impl JoinAccountFence {
    pub fn new(
        account_ref: impl Into<String>,
        account_public_key_hex: impl Into<String>,
        generation: u64,
    ) -> Result<Self, JoinStoreError> {
        let fence = Self {
            account_ref: account_ref.into(),
            account_public_key_hex: account_public_key_hex.into(),
            generation,
        };
        fence.validate()?;
        Ok(fence)
    }

    fn validate(&self) -> Result<(), JoinStoreError> {
        if !valid_reference(&self.account_ref)
            || !valid_hex_64(&self.account_public_key_hex)
            || self.generation == 0
        {
            return Err(JoinStoreError::InvalidFence);
        }
        Ok(())
    }
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum JoinStepKind {
    AddRelay,
    Nip42Authenticate,
    ClaimBuzzInvite,
    RequestNip29Join,
    AwaitNip29Admission,
    UpdateNip29GroupList,
    VerifyOpenAgentsGrant,
    PersistLocalClaim,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum JoinStepStatus {
    Pending,
    Prepared,
    Succeeded,
    FailedRetryable,
    FailedTerminal,
    Skipped,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct JoinStepProjection {
    pub kind: JoinStepKind,
    pub required: bool,
    pub status: JoinStepStatus,
    pub request_digest: String,
    pub idempotency_key: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub receipt_ref: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub failure_code: Option<String>,
    pub attempts: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct JoinTransactionProjection {
    pub transaction_ref: String,
    pub account: JoinAccountFence,
    pub invite_evidence: OpaqueInviteEvidence,
    pub steps: Vec<JoinStepProjection>,
    pub created_at: u64,
    pub updated_at: u64,
    pub complete: bool,
    pub successful: bool,
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
struct SensitiveRequest(Vec<u8>);

impl SensitiveRequest {
    fn new(bytes: Vec<u8>) -> Result<Self, JoinStoreError> {
        if bytes.is_empty() || bytes.len() > MAX_OPAQUE_BYTES {
            return Err(JoinStoreError::InvalidPlan);
        }
        Ok(Self(bytes))
    }
}

impl fmt::Debug for SensitiveRequest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("SensitiveRequest([REDACTED])")
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlannedJoinStep {
    pub kind: JoinStepKind,
    pub required: bool,
    request: SensitiveRequest,
}

impl PlannedJoinStep {
    pub fn new(
        kind: JoinStepKind,
        required: bool,
        exact_request: impl Into<Vec<u8>>,
    ) -> Result<Self, JoinStoreError> {
        Ok(Self {
            kind,
            required,
            request: SensitiveRequest::new(exact_request.into())?,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JoinPlan {
    pub invite_evidence: OpaqueInviteEvidence,
    pub steps: Vec<PlannedJoinStep>,
}

impl JoinPlan {
    pub fn new(
        invite_evidence: OpaqueInviteEvidence,
        steps: Vec<PlannedJoinStep>,
    ) -> Result<Self, JoinStoreError> {
        if steps.is_empty() || steps.len() > 16 {
            return Err(JoinStoreError::InvalidPlan);
        }
        let unique = steps.iter().map(|step| step.kind).collect::<BTreeSet<_>>();
        if unique.len() != steps.len() {
            return Err(JoinStoreError::InvalidPlan);
        }
        Ok(Self {
            invite_evidence,
            steps,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum DurableStepState {
    Pending,
    Prepared,
    Succeeded,
    FailedRetryable,
    FailedTerminal,
    Skipped,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct DurableJoinStep {
    kind: JoinStepKind,
    required: bool,
    exact_request: SensitiveRequest,
    request_digest: String,
    idempotency_key: String,
    state: DurableStepState,
    receipt_ref: Option<String>,
    failure_code: Option<String>,
    attempts: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    signed_event: Option<DurableSignedEvent>,
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct DurableSignedEvent {
    event_id: String,
    event_json: Vec<u8>,
    event_digest: String,
}

impl fmt::Debug for DurableSignedEvent {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("DurableSignedEvent")
            .field("event_id", &self.event_id)
            .field("event_digest", &self.event_digest)
            .field("event_json", &"[REDACTED]")
            .finish()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct DurableJoinTransaction {
    schema: String,
    transaction_ref: String,
    account: JoinAccountFence,
    invite_evidence: OpaqueInviteEvidence,
    steps: Vec<DurableJoinStep>,
    created_at: u64,
    updated_at: u64,
}

impl DurableJoinTransaction {
    fn validate(&self) -> Result<(), JoinStoreError> {
        self.account.validate()?;
        if self.schema != TRANSACTION_SCHEMA
            || !valid_reference(&self.transaction_ref)
            || self.steps.is_empty()
            || self.steps.len() > 16
        {
            return Err(JoinStoreError::InvalidStoredTransaction);
        }
        let mut kinds = BTreeSet::new();
        for step in &self.steps {
            if !kinds.insert(step.kind)
                || !valid_reference(&step.idempotency_key)
                || hex_digest(&step.exact_request.0) != step.request_digest
                || step.exact_request.0.is_empty()
                || step.exact_request.0.len() > MAX_OPAQUE_BYTES
                || step
                    .receipt_ref
                    .as_deref()
                    .is_some_and(|value| !valid_reference(value))
                || step
                    .failure_code
                    .as_deref()
                    .is_some_and(|value| !valid_reference(value))
                || step
                    .signed_event
                    .as_ref()
                    .is_some_and(|event| validate_durable_signed_event(event).is_err())
            {
                return Err(JoinStoreError::InvalidStoredTransaction);
            }
        }
        Ok(())
    }

    fn projection(&self) -> JoinTransactionProjection {
        let complete = self.steps.iter().all(|step| {
            matches!(
                step.state,
                DurableStepState::Succeeded
                    | DurableStepState::FailedTerminal
                    | DurableStepState::Skipped
            )
        });
        let successful = complete
            && self.steps.iter().all(|step| {
                matches!(
                    step.state,
                    DurableStepState::Succeeded | DurableStepState::Skipped
                )
            });
        JoinTransactionProjection {
            transaction_ref: self.transaction_ref.clone(),
            account: self.account.clone(),
            invite_evidence: self.invite_evidence.clone(),
            steps: self
                .steps
                .iter()
                .map(|step| JoinStepProjection {
                    kind: step.kind,
                    required: step.required,
                    status: match step.state {
                        DurableStepState::Pending => JoinStepStatus::Pending,
                        DurableStepState::Prepared => JoinStepStatus::Prepared,
                        DurableStepState::Succeeded => JoinStepStatus::Succeeded,
                        DurableStepState::FailedRetryable => JoinStepStatus::FailedRetryable,
                        DurableStepState::FailedTerminal => JoinStepStatus::FailedTerminal,
                        DurableStepState::Skipped => JoinStepStatus::Skipped,
                    },
                    request_digest: step.request_digest.clone(),
                    idempotency_key: step.idempotency_key.clone(),
                    receipt_ref: step.receipt_ref.clone(),
                    failure_code: step.failure_code.clone(),
                    attempts: step.attempts,
                })
                .collect(),
            created_at: self.created_at,
            updated_at: self.updated_at,
            complete,
            successful,
        }
    }
}

#[derive(Clone, PartialEq, Eq)]
pub struct PreparedJoinMutation {
    pub transaction_ref: String,
    pub account: JoinAccountFence,
    pub kind: JoinStepKind,
    pub request_digest: String,
    pub idempotency_key: String,
    exact_request: SensitiveRequest,
}

impl PreparedJoinMutation {
    pub fn exact_request(&self) -> &[u8] {
        &self.exact_request.0
    }
}

#[derive(Clone, PartialEq, Eq)]
pub struct SignedEventBinding {
    pub transaction_ref: String,
    pub account: JoinAccountFence,
    pub kind: JoinStepKind,
    pub request_digest: String,
    pub idempotency_key: String,
    pub event_id: String,
    pub event_digest: String,
    event_json: Vec<u8>,
}

impl SignedEventBinding {
    pub fn exact_event_json(&self) -> &[u8] {
        &self.event_json
    }
}

#[derive(Clone, PartialEq, Eq)]
pub struct JoinStepPrivateMaterial {
    pub transaction_ref: String,
    pub account: JoinAccountFence,
    pub kind: JoinStepKind,
    pub required: bool,
    pub status: JoinStepStatus,
    pub request_digest: String,
    pub idempotency_key: String,
    pub attempts: u32,
    pub created_at: u64,
    pub updated_at: u64,
    exact_request: SensitiveRequest,
    signed_event: Option<SignedEventBinding>,
}

impl JoinStepPrivateMaterial {
    pub fn exact_request(&self) -> &[u8] {
        &self.exact_request.0
    }

    pub fn signed_event(&self) -> Option<&SignedEventBinding> {
        self.signed_event.as_ref()
    }
}

impl fmt::Debug for JoinStepPrivateMaterial {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("JoinStepPrivateMaterial")
            .field("transaction_ref", &self.transaction_ref)
            .field("account", &self.account)
            .field("kind", &self.kind)
            .field("required", &self.required)
            .field("status", &self.status)
            .field("request_digest", &self.request_digest)
            .field("idempotency_key", &self.idempotency_key)
            .field("attempts", &self.attempts)
            .field("created_at", &self.created_at)
            .field("updated_at", &self.updated_at)
            .field("exact_request", &"[REDACTED]")
            .field("signed_event", &self.signed_event.is_some())
            .finish()
    }
}

impl fmt::Debug for SignedEventBinding {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("SignedEventBinding")
            .field("transaction_ref", &self.transaction_ref)
            .field("account", &self.account)
            .field("kind", &self.kind)
            .field("request_digest", &self.request_digest)
            .field("idempotency_key", &self.idempotency_key)
            .field("event_id", &self.event_id)
            .field("event_digest", &self.event_digest)
            .field("event_json", &"[REDACTED]")
            .finish()
    }
}

impl fmt::Debug for PreparedJoinMutation {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("PreparedJoinMutation")
            .field("transaction_ref", &self.transaction_ref)
            .field("account", &self.account)
            .field("kind", &self.kind)
            .field("request_digest", &self.request_digest)
            .field("idempotency_key", &self.idempotency_key)
            .field("exact_request", &"[REDACTED]")
            .finish()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum JoinMutationOutcome {
    Succeeded { receipt_ref: String },
    FailedRetryable { failure_code: String },
    FailedTerminal { failure_code: String },
    Skipped { receipt_ref: String },
}

pub trait JoinMutationExecutor {
    fn execute(
        &mut self,
        mutation: &PreparedJoinMutation,
    ) -> Result<JoinMutationOutcome, JoinStoreError>;
}

pub struct JoinTransactionStore {
    root: PathBuf,
}

impl JoinTransactionStore {
    pub fn system() -> Self {
        Self::for_data_root(paths::data_dir().to_path_buf())
    }

    pub fn for_data_root(data_root: impl Into<PathBuf>) -> Self {
        Self {
            root: data_root
                .into()
                .join("identity")
                .join("invites")
                .join("accounts"),
        }
    }

    pub fn create(
        &self,
        transaction_ref: &str,
        account: JoinAccountFence,
        plan: JoinPlan,
        now: u64,
    ) -> Result<JoinTransactionProjection, JoinStoreError> {
        let _guard = mutation_guard()?;
        account.validate()?;
        if !valid_reference(transaction_ref) || now == 0 {
            return Err(JoinStoreError::InvalidReference);
        }
        let path = self.transaction_path(&account, transaction_ref)?;
        if path_exists_regular(&path)?.is_some() {
            let existing = self.read_path(&path)?;
            if existing.account != account {
                return Err(JoinStoreError::StaleGeneration);
            }
            let requested_steps = plan
                .steps
                .iter()
                .map(|step| (step.kind, step.required, hex_digest(&step.request.0)))
                .collect::<Vec<_>>();
            let existing_steps = existing
                .steps
                .iter()
                .map(|step| (step.kind, step.required, step.request_digest.clone()))
                .collect::<Vec<_>>();
            if existing.invite_evidence != plan.invite_evidence || existing_steps != requested_steps
            {
                return Err(JoinStoreError::RequestChanged);
            }
            return Ok(existing.projection());
        }
        let steps = plan
            .steps
            .into_iter()
            .map(|step| {
                let request_digest = hex_digest(&step.request.0);
                DurableJoinStep {
                    kind: step.kind,
                    required: step.required,
                    exact_request: step.request,
                    idempotency_key: idempotency_key(transaction_ref, step.kind, &request_digest),
                    request_digest,
                    state: DurableStepState::Pending,
                    receipt_ref: None,
                    failure_code: None,
                    attempts: 0,
                    signed_event: None,
                }
            })
            .collect();
        let transaction = DurableJoinTransaction {
            schema: TRANSACTION_SCHEMA.to_string(),
            transaction_ref: transaction_ref.to_string(),
            account,
            invite_evidence: plan.invite_evidence,
            steps,
            created_at: now,
            updated_at: now,
        };
        transaction.validate()?;
        write_transaction(&path, &transaction, &self.root)?;
        self.verify_private_ancestors(&path)?;
        Ok(transaction.projection())
    }

    pub fn inspect(
        &self,
        transaction_ref: &str,
        account: &JoinAccountFence,
    ) -> Result<JoinTransactionProjection, JoinStoreError> {
        let _guard = mutation_guard()?;
        let transaction = self.read(transaction_ref, account)?;
        Ok(transaction.projection())
    }

    pub fn inspect_optional(
        &self,
        transaction_ref: &str,
        account: &JoinAccountFence,
    ) -> Result<Option<JoinTransactionProjection>, JoinStoreError> {
        let _guard = mutation_guard()?;
        let path = self.transaction_path(account, transaction_ref)?;
        if path_exists_regular(&path)?.is_none() {
            return Ok(None);
        }
        let transaction = self.read_path(&path)?;
        ensure_fence(&transaction, account)?;
        Ok(Some(transaction.projection()))
    }

    pub fn read_step_private_material(
        &self,
        transaction_ref: &str,
        account: &JoinAccountFence,
        kind: JoinStepKind,
    ) -> Result<JoinStepPrivateMaterial, JoinStoreError> {
        let _guard = mutation_guard()?;
        let transaction = self.read(transaction_ref, account)?;
        let step = transaction
            .steps
            .iter()
            .find(|step| step.kind == kind)
            .ok_or(JoinStoreError::UnknownStep)?;
        let signed_event = signed_event_binding(&transaction, kind)?;
        Ok(JoinStepPrivateMaterial {
            transaction_ref: transaction.transaction_ref.clone(),
            account: transaction.account.clone(),
            kind,
            required: step.required,
            status: join_step_status(&step.state),
            request_digest: step.request_digest.clone(),
            idempotency_key: step.idempotency_key.clone(),
            attempts: step.attempts,
            created_at: transaction.created_at,
            updated_at: transaction.updated_at,
            exact_request: step.exact_request.clone(),
            signed_event,
        })
    }

    pub fn prepare_step(
        &self,
        transaction_ref: &str,
        account: &JoinAccountFence,
        kind: JoinStepKind,
        now: u64,
    ) -> Result<PreparedJoinMutation, JoinStoreError> {
        let _guard = mutation_guard()?;
        let path = self.transaction_path(account, transaction_ref)?;
        let mut transaction = self.read_path(&path)?;
        ensure_fence(&transaction, account)?;
        if transaction.projection().complete {
            return Err(JoinStoreError::Completed);
        }
        let step = transaction
            .steps
            .iter_mut()
            .find(|step| step.kind == kind)
            .ok_or(JoinStoreError::UnknownStep)?;
        match step.state {
            DurableStepState::Pending | DurableStepState::FailedRetryable => {
                step.state = DurableStepState::Prepared;
                step.attempts = step.attempts.saturating_add(1);
                step.failure_code = None;
                transaction.updated_at = now;
                write_transaction(&path, &transaction, &self.root)?;
            }
            DurableStepState::Prepared => {}
            DurableStepState::Succeeded
            | DurableStepState::FailedTerminal
            | DurableStepState::Skipped => return Err(JoinStoreError::InvalidStepState),
        }
        prepared_mutation(&transaction, kind)
    }

    pub fn record_outcome(
        &self,
        mutation: &PreparedJoinMutation,
        outcome: JoinMutationOutcome,
        now: u64,
    ) -> Result<JoinTransactionProjection, JoinStoreError> {
        let _guard = mutation_guard()?;
        let path = self.transaction_path(&mutation.account, &mutation.transaction_ref)?;
        let mut transaction = self.read_path(&path)?;
        ensure_fence(&transaction, &mutation.account)?;
        let step = transaction
            .steps
            .iter_mut()
            .find(|step| step.kind == mutation.kind)
            .ok_or(JoinStoreError::UnknownStep)?;
        if step.state != DurableStepState::Prepared
            || step.request_digest != mutation.request_digest
            || step.idempotency_key != mutation.idempotency_key
            || step.exact_request != mutation.exact_request
        {
            return Err(JoinStoreError::RequestChanged);
        }
        match outcome {
            JoinMutationOutcome::Succeeded { receipt_ref } => {
                validate_outcome_reference(&receipt_ref)?;
                step.state = DurableStepState::Succeeded;
                step.receipt_ref = Some(receipt_ref);
                step.failure_code = None;
            }
            JoinMutationOutcome::FailedRetryable { failure_code } => {
                validate_outcome_reference(&failure_code)?;
                step.state = DurableStepState::FailedRetryable;
                step.failure_code = Some(failure_code);
            }
            JoinMutationOutcome::FailedTerminal { failure_code } => {
                validate_outcome_reference(&failure_code)?;
                step.state = DurableStepState::FailedTerminal;
                step.failure_code = Some(failure_code);
            }
            JoinMutationOutcome::Skipped { receipt_ref } => {
                validate_outcome_reference(&receipt_ref)?;
                step.state = DurableStepState::Skipped;
                step.receipt_ref = Some(receipt_ref);
                step.failure_code = None;
            }
        }
        transaction.updated_at = now;
        write_transaction(&path, &transaction, &self.root)?;
        Ok(transaction.projection())
    }

    pub fn bind_signed_event(
        &self,
        mutation: &PreparedJoinMutation,
        event_id: &str,
        exact_event_json: impl Into<Vec<u8>>,
        now: u64,
    ) -> Result<SignedEventBinding, JoinStoreError> {
        let _guard = mutation_guard()?;
        let path = self.transaction_path(&mutation.account, &mutation.transaction_ref)?;
        let mut transaction = self.read_path(&path)?;
        ensure_fence(&transaction, &mutation.account)?;
        let step = transaction
            .steps
            .iter_mut()
            .find(|step| step.kind == mutation.kind)
            .ok_or(JoinStoreError::UnknownStep)?;
        ensure_prepared_step_matches(step, mutation)?;
        let event_json = exact_event_json.into();
        let candidate = validate_signed_event(event_id, event_json)?;
        match &step.signed_event {
            Some(existing) if existing == &candidate => {}
            Some(_) => return Err(JoinStoreError::RequestChanged),
            None => {
                step.signed_event = Some(candidate);
                transaction.updated_at = now;
                write_transaction(&path, &transaction, &self.root)?;
            }
        }
        signed_event_binding(&transaction, mutation.kind)?.ok_or(JoinStoreError::InvalidSignedEvent)
    }

    pub fn read_bound_signed_event(
        &self,
        mutation: &PreparedJoinMutation,
    ) -> Result<Option<SignedEventBinding>, JoinStoreError> {
        let _guard = mutation_guard()?;
        let transaction = self.read(&mutation.transaction_ref, &mutation.account)?;
        let step = transaction
            .steps
            .iter()
            .find(|step| step.kind == mutation.kind)
            .ok_or(JoinStoreError::UnknownStep)?;
        ensure_prepared_step_matches(step, mutation)?;
        signed_event_binding(&transaction, mutation.kind)
    }

    pub fn execute_step(
        &self,
        transaction_ref: &str,
        account: &JoinAccountFence,
        kind: JoinStepKind,
        now: u64,
        executor: &mut impl JoinMutationExecutor,
    ) -> Result<JoinTransactionProjection, JoinStoreError> {
        let mutation = self.prepare_step(transaction_ref, account, kind, now)?;
        let outcome = executor.execute(&mutation)?;
        self.record_outcome(&mutation, outcome, now)
    }

    pub fn prepared_steps(
        &self,
        transaction_ref: &str,
        account: &JoinAccountFence,
    ) -> Result<Vec<PreparedJoinMutation>, JoinStoreError> {
        let _guard = mutation_guard()?;
        let transaction = self.read(transaction_ref, account)?;
        transaction
            .steps
            .iter()
            .filter(|step| step.state == DurableStepState::Prepared)
            .map(|step| prepared_mutation(&transaction, step.kind))
            .collect()
    }

    pub fn purge_account(&self, account: &JoinAccountFence) -> Result<(), JoinStoreError> {
        let _guard = mutation_guard()?;
        account.validate()?;
        let path = self.root.join(&account.account_public_key_hex);
        let transactions_path = path.join("transactions");
        if let Ok(entries) = fs::read_dir(&transactions_path) {
            for entry in entries {
                let entry = entry.map_err(|_| JoinStoreError::Storage)?;
                let metadata = entry.metadata().map_err(|_| JoinStoreError::Storage)?;
                if metadata.is_file() {
                    let transaction = self.read_path(&entry.path())?;
                    if transaction.account != *account {
                        return Err(JoinStoreError::StaleGeneration);
                    }
                } else {
                    return Err(JoinStoreError::Storage);
                }
            }
        }
        match fs::symlink_metadata(&path) {
            Ok(metadata) if metadata.file_type().is_symlink() => {
                return Err(JoinStoreError::Storage);
            }
            Ok(metadata) if metadata.is_dir() => {
                fs::remove_dir_all(&path).map_err(|_| JoinStoreError::Storage)?;
            }
            Ok(_) => return Err(JoinStoreError::Storage),
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
            Err(_) => return Err(JoinStoreError::Storage),
        }
        if path.try_exists().map_err(|_| JoinStoreError::Storage)? {
            return Err(JoinStoreError::Storage);
        }
        Ok(())
    }

    fn read(
        &self,
        transaction_ref: &str,
        account: &JoinAccountFence,
    ) -> Result<DurableJoinTransaction, JoinStoreError> {
        let path = self.transaction_path(account, transaction_ref)?;
        let transaction = self.read_path(&path)?;
        ensure_fence(&transaction, account)?;
        Ok(transaction)
    }

    fn read_path(&self, path: &Path) -> Result<DurableJoinTransaction, JoinStoreError> {
        self.verify_private_ancestors(path)?;
        let Some(metadata) = path_exists_regular(path)? else {
            return Err(JoinStoreError::InvalidStoredTransaction);
        };
        verify_private_file_mode(&metadata)?;
        if metadata.len() > MAX_TRANSACTION_BYTES {
            return Err(JoinStoreError::InvalidStoredTransaction);
        }
        let file = fs::File::open(path).map_err(|_| JoinStoreError::Storage)?;
        let mut bytes = Vec::new();
        file.take(MAX_TRANSACTION_BYTES.saturating_add(1))
            .read_to_end(&mut bytes)
            .map_err(|_| JoinStoreError::Storage)?;
        if bytes.len() as u64 > MAX_TRANSACTION_BYTES {
            return Err(JoinStoreError::InvalidStoredTransaction);
        }
        let transaction: DurableJoinTransaction =
            serde_json::from_slice(&bytes).map_err(|_| JoinStoreError::InvalidStoredTransaction)?;
        transaction.validate()?;
        Ok(transaction)
    }

    fn verify_private_ancestors(&self, path: &Path) -> Result<(), JoinStoreError> {
        let mut cursor = path.parent().ok_or(JoinStoreError::Storage)?;
        loop {
            if !cursor.starts_with(&self.root) {
                break;
            }
            let metadata = fs::symlink_metadata(cursor).map_err(|_| JoinStoreError::Storage)?;
            if !metadata.is_dir() || metadata.file_type().is_symlink() {
                return Err(JoinStoreError::Storage);
            }
            verify_private_directory_mode(&metadata)?;
            if cursor == self.root {
                break;
            }
            cursor = cursor.parent().ok_or(JoinStoreError::Storage)?;
        }
        Ok(())
    }

    fn transaction_path(
        &self,
        account: &JoinAccountFence,
        transaction_ref: &str,
    ) -> Result<PathBuf, JoinStoreError> {
        account.validate()?;
        if !valid_reference(transaction_ref) {
            return Err(JoinStoreError::InvalidReference);
        }
        Ok(self
            .root
            .join(&account.account_public_key_hex)
            .join("transactions")
            .join(format!("{transaction_ref}.json")))
    }
}

fn mutation_guard() -> Result<MutexGuard<'static, ()>, JoinStoreError> {
    static MUTATION: OnceLock<Mutex<()>> = OnceLock::new();
    MUTATION
        .get_or_init(|| Mutex::new(()))
        .lock()
        .map_err(|_| JoinStoreError::Storage)
}

fn ensure_fence(
    transaction: &DurableJoinTransaction,
    account: &JoinAccountFence,
) -> Result<(), JoinStoreError> {
    account.validate()?;
    if &transaction.account != account {
        return Err(JoinStoreError::StaleGeneration);
    }
    Ok(())
}

fn prepared_mutation(
    transaction: &DurableJoinTransaction,
    kind: JoinStepKind,
) -> Result<PreparedJoinMutation, JoinStoreError> {
    let step = transaction
        .steps
        .iter()
        .find(|step| step.kind == kind)
        .ok_or(JoinStoreError::UnknownStep)?;
    if step.state != DurableStepState::Prepared {
        return Err(JoinStoreError::InvalidStepState);
    }
    Ok(PreparedJoinMutation {
        transaction_ref: transaction.transaction_ref.clone(),
        account: transaction.account.clone(),
        kind,
        request_digest: step.request_digest.clone(),
        idempotency_key: step.idempotency_key.clone(),
        exact_request: step.exact_request.clone(),
    })
}

fn ensure_prepared_step_matches(
    step: &DurableJoinStep,
    mutation: &PreparedJoinMutation,
) -> Result<(), JoinStoreError> {
    if step.state != DurableStepState::Prepared
        || step.request_digest != mutation.request_digest
        || step.idempotency_key != mutation.idempotency_key
        || step.exact_request != mutation.exact_request
    {
        return Err(JoinStoreError::RequestChanged);
    }
    Ok(())
}

fn validate_signed_event(
    event_id: &str,
    event_json: Vec<u8>,
) -> Result<DurableSignedEvent, JoinStoreError> {
    if !valid_hex_64(event_id) || event_json.is_empty() || event_json.len() > MAX_SIGNED_EVENT_BYTES
    {
        return Err(JoinStoreError::InvalidSignedEvent);
    }
    let event_text =
        std::str::from_utf8(&event_json).map_err(|_| JoinStoreError::InvalidSignedEvent)?;
    let event = Event::from_json(event_text).map_err(|_| JoinStoreError::InvalidSignedEvent)?;
    if event.verify().is_err() || event.id.to_hex() != event_id {
        return Err(JoinStoreError::InvalidSignedEvent);
    }
    Ok(DurableSignedEvent {
        event_id: event_id.to_string(),
        event_digest: hex_digest(&event_json),
        event_json,
    })
}

fn validate_durable_signed_event(event: &DurableSignedEvent) -> Result<(), JoinStoreError> {
    let validated = validate_signed_event(&event.event_id, event.event_json.clone())?;
    if &validated != event || event.event_digest != hex_digest(&event.event_json) {
        return Err(JoinStoreError::InvalidSignedEvent);
    }
    Ok(())
}

fn signed_event_binding(
    transaction: &DurableJoinTransaction,
    kind: JoinStepKind,
) -> Result<Option<SignedEventBinding>, JoinStoreError> {
    let step = transaction
        .steps
        .iter()
        .find(|step| step.kind == kind)
        .ok_or(JoinStoreError::UnknownStep)?;
    let Some(event) = &step.signed_event else {
        return Ok(None);
    };
    validate_durable_signed_event(event)?;
    Ok(Some(SignedEventBinding {
        transaction_ref: transaction.transaction_ref.clone(),
        account: transaction.account.clone(),
        kind,
        request_digest: step.request_digest.clone(),
        idempotency_key: step.idempotency_key.clone(),
        event_id: event.event_id.clone(),
        event_digest: event.event_digest.clone(),
        event_json: event.event_json.clone(),
    }))
}

fn join_step_status(state: &DurableStepState) -> JoinStepStatus {
    match state {
        DurableStepState::Pending => JoinStepStatus::Pending,
        DurableStepState::Prepared => JoinStepStatus::Prepared,
        DurableStepState::Succeeded => JoinStepStatus::Succeeded,
        DurableStepState::FailedRetryable => JoinStepStatus::FailedRetryable,
        DurableStepState::FailedTerminal => JoinStepStatus::FailedTerminal,
        DurableStepState::Skipped => JoinStepStatus::Skipped,
    }
}

fn write_transaction(
    path: &Path,
    transaction: &DurableJoinTransaction,
    private_root: &Path,
) -> Result<(), JoinStoreError> {
    transaction.validate()?;
    let bytes = serde_json::to_vec_pretty(transaction).map_err(|_| JoinStoreError::InvalidPlan)?;
    if bytes.len() as u64 > MAX_TRANSACTION_BYTES {
        return Err(JoinStoreError::InvalidPlan);
    }
    let parent = path.parent().ok_or(JoinStoreError::Storage)?;
    create_private_directory(parent, private_root)?;
    match path_exists_regular(path)? {
        Some(metadata) => verify_private_file_mode(&metadata)?,
        None if path.try_exists().map_err(|_| JoinStoreError::Storage)? => {
            return Err(JoinStoreError::Storage);
        }
        None => {}
    }
    let mut file = AtomicWriteFile::open(path).map_err(|_| JoinStoreError::Storage)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        file.as_file()
            .set_permissions(fs::Permissions::from_mode(0o600))
            .map_err(|_| JoinStoreError::Storage)?;
    }
    file.write_all(&bytes)
        .map_err(|_| JoinStoreError::Storage)?;
    file.write_all(b"\n").map_err(|_| JoinStoreError::Storage)?;
    file.commit().map_err(|_| JoinStoreError::Storage)?;
    let stored = fs::read(path).map_err(|_| JoinStoreError::Storage)?;
    if stored != [bytes, b"\n".to_vec()].concat() {
        return Err(JoinStoreError::Storage);
    }
    Ok(())
}

fn create_private_directory(path: &Path, private_root: &Path) -> Result<(), JoinStoreError> {
    let mut missing = Vec::new();
    let mut cursor = path;
    loop {
        match fs::symlink_metadata(cursor) {
            Ok(metadata) => {
                if !metadata.is_dir() || metadata.file_type().is_symlink() {
                    return Err(JoinStoreError::Storage);
                }
                if cursor.starts_with(private_root) {
                    verify_private_directory_mode(&metadata)?;
                }
                break;
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                missing.push(cursor.to_path_buf());
                cursor = cursor.parent().ok_or(JoinStoreError::Storage)?;
            }
            Err(_) => return Err(JoinStoreError::Storage),
        }
    }
    for directory in missing.iter().rev() {
        fs::create_dir(directory).map_err(|_| JoinStoreError::Storage)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            fs::set_permissions(directory, fs::Permissions::from_mode(0o700))
                .map_err(|_| JoinStoreError::Storage)?;
        }
    }
    Ok(())
}

#[cfg(unix)]
fn verify_private_directory_mode(metadata: &fs::Metadata) -> Result<(), JoinStoreError> {
    use std::os::unix::fs::PermissionsExt as _;

    if metadata.permissions().mode() & 0o077 != 0 {
        return Err(JoinStoreError::Storage);
    }
    Ok(())
}

#[cfg(not(unix))]
fn verify_private_directory_mode(_metadata: &fs::Metadata) -> Result<(), JoinStoreError> {
    Ok(())
}

#[cfg(unix)]
fn verify_private_file_mode(metadata: &fs::Metadata) -> Result<(), JoinStoreError> {
    use std::os::unix::fs::PermissionsExt as _;

    if metadata.permissions().mode() & 0o177 != 0 {
        return Err(JoinStoreError::Storage);
    }
    Ok(())
}

#[cfg(not(unix))]
fn verify_private_file_mode(_metadata: &fs::Metadata) -> Result<(), JoinStoreError> {
    Ok(())
}

fn path_exists_regular(path: &Path) -> Result<Option<fs::Metadata>, JoinStoreError> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.is_file() && !metadata.file_type().is_symlink() => {
            Ok(Some(metadata))
        }
        Ok(_) => Err(JoinStoreError::Storage),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(_) => Err(JoinStoreError::Storage),
    }
}

fn idempotency_key(transaction_ref: &str, kind: JoinStepKind, digest: &str) -> String {
    let value = format!("{transaction_ref}:{kind:?}:{digest}");
    format!("join-{}", hex_digest(value.as_bytes()))
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
            .all(|byte| byte.is_ascii_digit() || matches!(byte, b'a'..=b'f'))
}

fn validate_outcome_reference(value: &str) -> Result<(), JoinStoreError> {
    if valid_reference(value) {
        Ok(())
    } else {
        Err(JoinStoreError::InvalidReference)
    }
}

#[cfg(test)]
mod tests {
    use nostr::{EventBuilder, Keys, Kind, RelayUrl, ToBech32 as _, nips::nip01::Coordinate};

    use super::*;

    const ACCOUNT_KEY: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";

    fn account(generation: u64) -> JoinAccountFence {
        JoinAccountFence::new("account.alpha", ACCOUNT_KEY, generation).expect("account fence")
    }

    fn evidence() -> OpaqueInviteEvidence {
        opaque_evidence(InviteProfile::Buzz, b"invite.secret")
    }

    fn plan() -> JoinPlan {
        JoinPlan::new(
            evidence(),
            vec![
                PlannedJoinStep::new(JoinStepKind::AddRelay, true, b"relay-request".to_vec())
                    .expect("relay step"),
                PlannedJoinStep::new(
                    JoinStepKind::Nip42Authenticate,
                    false,
                    b"auth-request".to_vec(),
                )
                .expect("auth step"),
            ],
        )
        .expect("join plan")
    }

    fn naddr(kind: u16, identifier: &str, relay: &str) -> String {
        let coordinate = Coordinate {
            kind: Kind::from(kind),
            public_key: Keys::generate().public_key(),
            identifier: identifier.to_string(),
        };
        Nip19Coordinate::new(coordinate, [RelayUrl::parse(relay).expect("relay URL")])
            .to_bech32()
            .expect("naddr")
    }

    fn signed_event(content: &str) -> (String, Vec<u8>) {
        let event = EventBuilder::text_note(content)
            .sign_with_keys(&Keys::generate())
            .expect("signed event");
        (
            event.id.to_hex(),
            event.try_as_json().expect("event JSON").into_bytes(),
        )
    }

    #[test]
    fn resolves_relay_qualified_nip29_group() {
        let address = naddr(39_000, "omega", "wss://relay.example");
        let resolved = InviteResolver.resolve(&address).expect("NIP-29 group");
        let ParsedInvite::Nip29Group(group) = resolved.parsed else {
            panic!("expected NIP-29 group");
        };
        assert_eq!(group.group_id, "omega");
        assert_eq!(group.relay_url, "wss://relay.example/");
        assert_eq!(
            resolved.preview.authority,
            AuthorityLabel::Nip29Relay("wss://relay.example/".into())
        );
        assert_eq!(resolved.preview.support, SupportLevel::Executable);
    }

    #[test]
    fn nip29_invite_code_is_retained_only_in_redacted_material() {
        let address = naddr(39_000, "omega", "wss://relay.example");
        let resolved = InviteResolver
            .resolve(&format!("{address}?invite=group-secret"))
            .expect("NIP-29 invitation");
        let ParsedInvite::Nip29Group(group) = &resolved.parsed else {
            panic!("expected NIP-29 group");
        };
        assert_eq!(group.invite_code(), Some(b"group-secret".as_slice()));
        assert!(!format!("{group:?}").contains("group-secret"));
        assert!(
            !serde_json::to_string(&resolved.preview)
                .expect("preview")
                .contains("group-secret")
        );
    }

    #[test]
    fn rejects_non_group_and_ambiguous_nip29_addresses() {
        let wrong_kind = naddr(33_301, "omega", "wss://relay.example");
        assert_eq!(
            InviteResolver.resolve(&wrong_kind),
            Err(InviteError::InvalidNip29Group)
        );

        let coordinate = Coordinate {
            kind: Kind::from(39_000),
            public_key: Keys::generate().public_key(),
            identifier: "omega".into(),
        };
        let address = Nip19Coordinate::new(
            coordinate,
            [
                RelayUrl::parse("wss://one.example").expect("relay"),
                RelayUrl::parse("wss://two.example").expect("relay"),
            ],
        )
        .to_bech32()
        .expect("naddr");
        assert_eq!(
            InviteResolver.resolve(&address),
            Err(InviteError::InvalidNip29Group)
        );
    }

    #[test]
    fn resolves_nip29_relay_without_promoting_transport_to_membership() {
        let resolved = InviteResolver
            .resolve("wss://relay.example/groups")
            .expect("NIP-29 relay");
        assert_eq!(resolved.preview.kind, InviteKind::Relay);
        assert_eq!(
            resolved.preview.signing_operations,
            [SigningOperation::UpdateNip29GroupList]
                .into_iter()
                .collect()
        );
        assert!(resolved.preview.room_reference.is_none());
    }

    #[test]
    fn resolves_buzz_invite_and_redacts_code() {
        let resolved = InviteResolver
            .resolve("https://buzz.example/invite/alpha.beta_gamma?r=true-relay-host")
            .expect("Buzz invite");
        let ParsedInvite::Buzz(invite) = &resolved.parsed else {
            panic!("expected Buzz invite");
        };
        assert_eq!(invite.code(), b"alpha.beta_gamma");
        assert!(!format!("{invite:?}").contains("alpha.beta_gamma"));
        let public_json = serde_json::to_string(&resolved.preview).expect("public preview");
        assert!(!public_json.contains("alpha.beta_gamma"));
        assert_eq!(resolved.preview.support, SupportLevel::PreviewOnly);
        assert_eq!(
            resolved.preview.capability_blockers,
            [CapabilityBlocker::BuzzAuthorityAdapter]
                .into_iter()
                .collect()
        );
        assert_eq!(
            resolved.preview.terms,
            TermsRequirement::ResolveFromAuthority
        );
    }

    #[test]
    fn refuses_malformed_buzz_invites() {
        for value in [
            "http://buzz.example/invite/alpha.beta",
            "https://buzz.example/invite/notdotted",
            "https://buzz.example/invite/alpha.beta?other=true",
            "https://buzz.example/invite/alpha.beta?r=true-relay-host&r=true-relay-host",
            "https://user@buzz.example/invite/alpha.beta",
        ] {
            assert_eq!(
                InviteResolver.resolve(value),
                Err(InviteError::InvalidBuzzInvite),
                "{value}"
            );
        }
    }

    #[test]
    fn armada_v1_is_preview_only_and_preserves_opaque_evidence() {
        let resolved = InviteResolver
            .resolve(
                "https://armada.example/invite?relays=wss%3A%2F%2Frelay.example#v1-sealed-secret",
            )
            .expect("Armada v1");
        let ParsedInvite::Armada(invite) = &resolved.parsed else {
            panic!("expected Armada invite");
        };
        assert_eq!(invite.sealed_fragment(), b"v1-sealed-secret");
        assert!(!format!("{invite:?}").contains("v1-sealed-secret"));
        assert_eq!(resolved.preview.support, SupportLevel::PreviewOnly);
        assert!(resolved.preview.opaque_evidence.is_some());
    }

    #[test]
    fn armada_v2_requires_the_exact_control_coordinate() {
        let coordinate = naddr(33_301, "", "wss://relay.example");
        let input = format!("https://armada.example/invite/{coordinate}#v4-sealed-secret");
        let resolved = InviteResolver.resolve(&input).expect("Armada v2");
        assert_eq!(resolved.preview.profile, InviteProfile::ArmadaConcordV2);
        assert_eq!(resolved.preview.support, SupportLevel::PreviewOnly);

        let wrong = naddr(39_000, "group", "wss://relay.example");
        assert_eq!(
            InviteResolver.resolve(&format!(
                "https://armada.example/invite/{wrong}#v4-sealed-secret"
            )),
            Err(InviteError::InvalidArmadaInvite)
        );
    }

    #[test]
    fn resolves_exact_openagents_v1_without_claiming_forge_authority() {
        let text = concat!(
            "omega-invite:1;tenant=tenant.openagents;repository=omega;",
            "coordinate=30617:7649603503856e5148d571eac2766b288a8ff1e9e35d380337a1d2b0015b4f92:omega;",
            "relays=wss://relay.openagents.com;",
            "name=Omega;binding=forge_actor.human.1;actor=human;",
            "state=active;roles=forge:member"
        );
        let resolved = InviteResolver.resolve(text).expect("OpenAgents invite");
        assert_eq!(resolved.preview.profile, InviteProfile::OpenAgentsV1);
        assert_eq!(resolved.preview.support, SupportLevel::PreviewOnly);
        assert_eq!(
            resolved.preview.recovery,
            RecoveryImplication::LocalClaimOnly
        );
        assert_eq!(
            resolved.preview.signing_operations,
            [SigningOperation::OpenAgentsGrantProof]
                .into_iter()
                .collect()
        );
    }

    #[test]
    fn rejects_unknown_openagents_fields_and_versions() {
        let unknown = concat!(
            "omega-invite:1;tenant=t;repository=r;coordinate=o/r;",
            "relays=wss://relay.example;name=n;binding=b;actor=human;",
            "state=active;roles=forge:member;future=x"
        );
        assert_eq!(
            InviteResolver.resolve(unknown),
            Err(InviteError::InvalidOpenAgentsInvite)
        );
        assert_eq!(
            InviteResolver.resolve("omega-invite:2;opaque=data"),
            Err(InviteError::InvalidOpenAgentsInvite)
        );
    }

    #[test]
    fn unsupported_input_is_digest_only_in_public_projection() {
        let secret = "future-profile:secret-bearing-material";
        let resolved = InviteResolver.resolve(secret).expect("opaque evidence");
        let ParsedInvite::Unsupported(invite) = &resolved.parsed else {
            panic!("expected unsupported invite");
        };
        assert_eq!(invite.material(), secret.as_bytes());
        assert!(!format!("{invite:?}").contains(secret));
        let encoded = serde_json::to_string(&resolved.preview).expect("preview JSON");
        assert!(!encoded.contains(secret));
        assert!(encoded.contains(&hex_digest(secret.as_bytes())));
    }

    #[test]
    fn transaction_is_durable_before_any_mutation() {
        let directory = tempfile::tempdir().expect("temporary directory");
        let store = JoinTransactionStore::for_data_root(directory.path());
        let projection = store
            .create("join.one", account(1), plan(), 10)
            .expect("create transaction");
        assert_eq!(projection.steps[0].status, JoinStepStatus::Pending);

        let mutation = store
            .prepare_step("join.one", &account(1), JoinStepKind::AddRelay, 11)
            .expect("prepare step");
        assert_eq!(mutation.exact_request(), b"relay-request");
        let resumed = store
            .prepared_steps("join.one", &account(1))
            .expect("resume prepared mutation");
        assert_eq!(resumed, vec![mutation]);
    }

    #[test]
    fn prepared_retry_reuses_exact_request_and_idempotency_key() {
        let directory = tempfile::tempdir().expect("temporary directory");
        let store = JoinTransactionStore::for_data_root(directory.path());
        store
            .create("join.retry", account(1), plan(), 10)
            .expect("create transaction");
        let first = store
            .prepare_step("join.retry", &account(1), JoinStepKind::AddRelay, 11)
            .expect("first prepare");
        store
            .record_outcome(
                &first,
                JoinMutationOutcome::FailedRetryable {
                    failure_code: "relay.offline".into(),
                },
                12,
            )
            .expect("retryable failure");
        let second = store
            .prepare_step("join.retry", &account(1), JoinStepKind::AddRelay, 13)
            .expect("second prepare");
        assert_eq!(first.exact_request(), second.exact_request());
        assert_eq!(first.request_digest, second.request_digest);
        assert_eq!(first.idempotency_key, second.idempotency_key);
        assert_eq!(
            store
                .inspect("join.retry", &account(1))
                .expect("inspect")
                .steps[0]
                .attempts,
            2
        );
    }

    #[test]
    fn two_store_instances_resume_the_same_prepared_bytes() {
        let directory = tempfile::tempdir().expect("temporary directory");
        let first_store = JoinTransactionStore::for_data_root(directory.path());
        let second_store = JoinTransactionStore::for_data_root(directory.path());
        first_store
            .create("join.shared", account(1), plan(), 10)
            .expect("create transaction");
        let prepared = first_store
            .prepare_step("join.shared", &account(1), JoinStepKind::AddRelay, 11)
            .expect("prepare");
        let resumed = second_store
            .prepared_steps("join.shared", &account(1))
            .expect("resume from second store");
        assert_eq!(resumed, vec![prepared]);
    }

    #[test]
    fn transaction_reference_cannot_be_reused_for_different_exact_bytes() {
        let directory = tempfile::tempdir().expect("temporary directory");
        let store = JoinTransactionStore::for_data_root(directory.path());
        store
            .create("join.conflict", account(1), plan(), 10)
            .expect("create transaction");
        let changed = JoinPlan::new(
            evidence(),
            vec![
                PlannedJoinStep::new(JoinStepKind::AddRelay, true, b"different-request".to_vec())
                    .expect("step"),
            ],
        )
        .expect("changed plan");
        assert!(matches!(
            store.create("join.conflict", account(1), changed, 11),
            Err(JoinStoreError::RequestChanged)
        ));
    }

    #[test]
    fn signed_event_binding_survives_restart_with_exact_bytes() {
        let directory = tempfile::tempdir().expect("temporary directory");
        let first_store = JoinTransactionStore::for_data_root(directory.path());
        first_store
            .create("join.signed", account(1), plan(), 10)
            .expect("create transaction");
        let mutation = first_store
            .prepare_step("join.signed", &account(1), JoinStepKind::AddRelay, 11)
            .expect("prepare");
        let (event_id, event_json) = signed_event("join-event-secret");
        let binding = first_store
            .bind_signed_event(&mutation, &event_id, event_json.clone(), 12)
            .expect("bind signed event");
        assert_eq!(binding.exact_event_json(), event_json);
        assert!(!format!("{binding:?}").contains("join-event-secret"));

        let restarted_store = JoinTransactionStore::for_data_root(directory.path());
        let restarted_mutation = restarted_store
            .prepared_steps("join.signed", &account(1))
            .expect("resume transaction")
            .into_iter()
            .next()
            .expect("prepared mutation");
        let restarted_binding = restarted_store
            .read_bound_signed_event(&restarted_mutation)
            .expect("read binding")
            .expect("bound event");
        assert_eq!(restarted_binding, binding);
        assert_eq!(restarted_binding.exact_event_json(), event_json);
        assert_eq!(restarted_mutation.exact_request(), b"relay-request");
        assert_eq!(restarted_mutation.idempotency_key, mutation.idempotency_key);
        let public_json = serde_json::to_string(
            &restarted_store
                .inspect("join.signed", &account(1))
                .expect("projection"),
        )
        .expect("public JSON");
        assert!(!public_json.contains("join-event-secret"));
        assert!(!public_json.contains(&event_id));
    }

    #[test]
    fn succeeded_step_private_material_survives_restart_with_creation_timestamp() {
        let directory = tempfile::tempdir().expect("temporary directory");
        let first_store = JoinTransactionStore::for_data_root(directory.path());
        first_store
            .create("join.succeeded-restart", account(1), plan(), 10)
            .expect("create transaction");
        let mutation = first_store
            .prepare_step(
                "join.succeeded-restart",
                &account(1),
                JoinStepKind::AddRelay,
                11,
            )
            .expect("prepare");
        let (event_id, event_json) = signed_event("succeeded-step-secret");
        first_store
            .bind_signed_event(&mutation, &event_id, event_json.clone(), 12)
            .expect("bind signed event");
        first_store
            .record_outcome(
                &mutation,
                JoinMutationOutcome::Succeeded {
                    receipt_ref: "relay.accepted".into(),
                },
                13,
            )
            .expect("record success");

        let restarted_store = JoinTransactionStore::for_data_root(directory.path());
        let projection = restarted_store
            .inspect_optional("join.succeeded-restart", &account(1))
            .expect("inspect optional")
            .expect("existing transaction");
        assert_eq!(projection.created_at, 10);
        assert_eq!(projection.steps[0].status, JoinStepStatus::Succeeded);
        let material = restarted_store
            .read_step_private_material(
                "join.succeeded-restart",
                &account(1),
                JoinStepKind::AddRelay,
            )
            .expect("read succeeded private material");
        assert_eq!(material.status, JoinStepStatus::Succeeded);
        assert_eq!(material.created_at, 10);
        assert_eq!(material.updated_at, 13);
        assert_eq!(material.exact_request(), b"relay-request");
        let binding = material.signed_event().expect("signed event binding");
        assert_eq!(binding.event_id, event_id);
        assert_eq!(binding.exact_event_json(), event_json);

        let debug = format!("{material:?}");
        assert!(!debug.contains("relay-request"));
        assert!(!debug.contains("succeeded-step-secret"));
        let public_json = serde_json::to_string(&projection).expect("public projection JSON");
        assert!(!public_json.contains("relay-request"));
        assert!(!public_json.contains("succeeded-step-secret"));
        assert!(!public_json.contains(&event_id));
    }

    #[test]
    fn optional_inspection_distinguishes_an_absent_transaction() {
        let directory = tempfile::tempdir().expect("temporary directory");
        let store = JoinTransactionStore::for_data_root(directory.path());
        assert_eq!(
            store
                .inspect_optional("join.absent", &account(1))
                .expect("inspect absent transaction"),
            None
        );
    }

    #[test]
    fn signed_event_binding_is_idempotent_and_conflicts_fail_closed() {
        let directory = tempfile::tempdir().expect("temporary directory");
        let store = JoinTransactionStore::for_data_root(directory.path());
        store
            .create("join.bind-once", account(1), plan(), 10)
            .expect("create transaction");
        let mutation = store
            .prepare_step("join.bind-once", &account(1), JoinStepKind::AddRelay, 11)
            .expect("prepare");
        let (event_id, event_json) = signed_event("first");
        let first = store
            .bind_signed_event(&mutation, &event_id, event_json.clone(), 12)
            .expect("first binding");
        let repeated = store
            .bind_signed_event(&mutation, &event_id, event_json, 13)
            .expect("idempotent binding");
        assert_eq!(repeated, first);

        let (other_id, other_json) = signed_event("second");
        assert!(matches!(
            store.bind_signed_event(&mutation, &other_id, other_json, 14),
            Err(JoinStoreError::RequestChanged)
        ));
        assert_eq!(
            store
                .read_bound_signed_event(&mutation)
                .expect("read binding"),
            Some(first)
        );
    }

    #[test]
    fn invalid_signed_event_is_never_persisted() {
        let directory = tempfile::tempdir().expect("temporary directory");
        let store = JoinTransactionStore::for_data_root(directory.path());
        store
            .create("join.invalid-signature", account(1), plan(), 10)
            .expect("create transaction");
        let mutation = store
            .prepare_step(
                "join.invalid-signature",
                &account(1),
                JoinStepKind::AddRelay,
                11,
            )
            .expect("prepare");
        assert!(matches!(
            store.bind_signed_event(&mutation, ACCOUNT_KEY, b"{}".to_vec(), 12),
            Err(JoinStoreError::InvalidSignedEvent)
        ));
        assert_eq!(
            store
                .read_bound_signed_event(&mutation)
                .expect("read binding"),
            None
        );
    }

    #[test]
    fn independent_step_results_do_not_collapse_authority() {
        let directory = tempfile::tempdir().expect("temporary directory");
        let store = JoinTransactionStore::for_data_root(directory.path());
        store
            .create("join.partial", account(1), plan(), 10)
            .expect("create transaction");
        let relay = store
            .prepare_step("join.partial", &account(1), JoinStepKind::AddRelay, 11)
            .expect("prepare relay");
        store
            .record_outcome(
                &relay,
                JoinMutationOutcome::Succeeded {
                    receipt_ref: "relay.added".into(),
                },
                12,
            )
            .expect("record relay");
        let auth = store
            .prepare_step(
                "join.partial",
                &account(1),
                JoinStepKind::Nip42Authenticate,
                13,
            )
            .expect("prepare auth");
        let projection = store
            .record_outcome(
                &auth,
                JoinMutationOutcome::FailedTerminal {
                    failure_code: "relay.auth_refused".into(),
                },
                14,
            )
            .expect("record auth");
        assert!(projection.complete);
        assert!(!projection.successful);
        assert_eq!(projection.steps[0].status, JoinStepStatus::Succeeded);
        assert_eq!(projection.steps[1].status, JoinStepStatus::FailedTerminal);
    }

    #[test]
    fn account_generation_fence_blocks_a_b_a_reuse() {
        let directory = tempfile::tempdir().expect("temporary directory");
        let store = JoinTransactionStore::for_data_root(directory.path());
        store
            .create("join.fenced", account(1), plan(), 10)
            .expect("create transaction");
        assert!(matches!(
            store.prepare_step("join.fenced", &account(2), JoinStepKind::AddRelay, 11),
            Err(JoinStoreError::StaleGeneration)
        ));
    }

    struct MockExecutor {
        calls: Vec<(JoinStepKind, String, Vec<u8>)>,
        outcome: JoinMutationOutcome,
    }

    impl JoinMutationExecutor for MockExecutor {
        fn execute(
            &mut self,
            mutation: &PreparedJoinMutation,
        ) -> Result<JoinMutationOutcome, JoinStoreError> {
            self.calls.push((
                mutation.kind,
                mutation.idempotency_key.clone(),
                mutation.exact_request().to_vec(),
            ));
            Ok(self.outcome.clone())
        }
    }

    #[test]
    fn orchestrator_executes_only_after_prepared_state_is_readable() {
        let directory = tempfile::tempdir().expect("temporary directory");
        let store = JoinTransactionStore::for_data_root(directory.path());
        store
            .create("join.mock", account(1), plan(), 10)
            .expect("create transaction");
        let mut executor = MockExecutor {
            calls: Vec::new(),
            outcome: JoinMutationOutcome::Succeeded {
                receipt_ref: "relay.added".into(),
            },
        };
        let projection = store
            .execute_step(
                "join.mock",
                &account(1),
                JoinStepKind::AddRelay,
                11,
                &mut executor,
            )
            .expect("execute step");
        assert_eq!(executor.calls.len(), 1);
        assert_eq!(executor.calls[0].2, b"relay-request");
        assert_eq!(projection.steps[0].status, JoinStepStatus::Succeeded);
    }

    #[cfg(unix)]
    #[test]
    fn store_uses_owner_only_modes_and_refuses_symlinks() {
        use std::os::unix::fs::{PermissionsExt as _, symlink};

        let directory = tempfile::tempdir().expect("temporary directory");
        let store = JoinTransactionStore::for_data_root(directory.path());
        store
            .create("join.private", account(1), plan(), 10)
            .expect("create transaction");
        let transaction = directory.path().join(format!(
            "identity/invites/accounts/{ACCOUNT_KEY}/transactions/join.private.json"
        ));
        assert_eq!(
            fs::metadata(transaction.parent().expect("parent"))
                .expect("directory metadata")
                .permissions()
                .mode()
                & 0o777,
            0o700
        );
        assert_eq!(
            fs::metadata(&transaction)
                .expect("file metadata")
                .permissions()
                .mode()
                & 0o777,
            0o600
        );

        let target = directory.path().join("target.json");
        fs::write(&target, b"{}").expect("target");
        let linked = directory.path().join(format!(
            "identity/invites/accounts/{ACCOUNT_KEY}/transactions/join.link.json"
        ));
        symlink(target, linked).expect("symlink");
        assert!(matches!(
            store.inspect("join.link", &account(1)),
            Err(JoinStoreError::Storage)
        ));
    }

    #[cfg(unix)]
    #[test]
    fn store_refuses_weak_existing_directory_and_file_modes() {
        use std::os::unix::fs::PermissionsExt as _;

        let directory = tempfile::tempdir().expect("temporary directory");
        let store = JoinTransactionStore::for_data_root(directory.path());
        store
            .create("join.weak", account(1), plan(), 10)
            .expect("create transaction");
        let transaction = directory.path().join(format!(
            "identity/invites/accounts/{ACCOUNT_KEY}/transactions/join.weak.json"
        ));
        fs::set_permissions(
            transaction.parent().expect("parent"),
            fs::Permissions::from_mode(0o755),
        )
        .expect("weaken directory");
        assert!(matches!(
            store.inspect("join.weak", &account(1)),
            Err(JoinStoreError::Storage)
        ));
        fs::set_permissions(
            transaction.parent().expect("parent"),
            fs::Permissions::from_mode(0o700),
        )
        .expect("restore directory");
        fs::set_permissions(&transaction, fs::Permissions::from_mode(0o644)).expect("weaken file");
        assert!(matches!(
            store.inspect("join.weak", &account(1)),
            Err(JoinStoreError::Storage)
        ));
    }

    struct RefusingAuthorityResolver;

    impl InviteAuthorityResolver for RefusingAuthorityResolver {
        fn resolve_authority(
            &mut self,
            _invite: &ResolvedInvite,
            _now: u64,
        ) -> Result<AuthorityResolution, ResolveRefusal> {
            Err(ResolveRefusal::TermsRequired)
        }
    }

    #[test]
    fn authority_adapter_preserves_typed_policy_refusals() {
        let invite = InviteResolver
            .resolve("https://buzz.example/invite/alpha.beta")
            .expect("Buzz preview");
        let refusal = RefusingAuthorityResolver
            .resolve_authority(&invite, 10)
            .expect_err("terms refusal");
        assert_eq!(refusal, ResolveRefusal::TermsRequired);
    }

    #[test]
    fn purge_is_verified_and_idempotent() {
        let directory = tempfile::tempdir().expect("temporary directory");
        let store = JoinTransactionStore::for_data_root(directory.path());
        store
            .create("join.purge", account(1), plan(), 10)
            .expect("create transaction");
        store.purge_account(&account(1)).expect("first purge");
        store.purge_account(&account(1)).expect("idempotent purge");
        assert!(
            !directory
                .path()
                .join(format!("identity/invites/accounts/{ACCOUNT_KEY}"))
                .exists()
        );
    }

    #[test]
    fn sensitive_requests_are_redacted_from_debug_and_public_projection() {
        let step = PlannedJoinStep::new(
            JoinStepKind::ClaimBuzzInvite,
            true,
            b"buzz-claim-secret".to_vec(),
        )
        .expect("step");
        assert!(!format!("{step:?}").contains("buzz-claim-secret"));
        let directory = tempfile::tempdir().expect("temporary directory");
        let store = JoinTransactionStore::for_data_root(directory.path());
        let projection = store
            .create(
                "join.secret",
                account(1),
                JoinPlan::new(evidence(), vec![step]).expect("plan"),
                10,
            )
            .expect("transaction");
        let public_json = serde_json::to_string(&projection).expect("projection JSON");
        assert!(!public_json.contains("buzz-claim-secret"));
    }
}
