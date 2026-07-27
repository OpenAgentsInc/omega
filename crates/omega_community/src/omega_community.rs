//! The community audience: a Forge repository, the people invited to it, and
//! the signed records they send each other.
//!
//! omega#108 asks for "a public workspace for Omega development that the owner
//! can invite people into". [`omega_audience`] already answers *who can read a
//! thread*; this crate answers *what a shared audience actually is* — an
//! OpenAgents Forge repository — and *who is admitted to it* — whoever the
//! Forge already bound an npub to.
//!
//! # Three things this crate deliberately does not do
//!
//! **It does not restate the audience rule.** [`omega_audience::may_publish`]
//! is the authorization, and [`may_post`] calls it rather than reimplementing
//! it. A second copy of "a local thread may not publish" is a second place for
//! that rule to be wrong, and the two copies would disagree on the day one of
//! them was edited.
//!
//! **It does not invent a membership system.** `FORGE-04` (openagents#9246)
//! already binds an OpenAgents account to one npub per tenant and issues the
//! roles; [`ForgeMembership`] is the shape that authority already serves at
//! `GET /api/forge/membership`, decoded. This crate cannot admit anybody. It
//! can only read what the Forge decided and refuse to exceed it.
//!
//! **It does not open a socket or touch a key.** Everything here is a value or
//! a function of values, including the clock, which arrives as a parameter.
//! The signing is somebody's own — a [`SignedRecord`] is *accepted*, never
//! produced, so a person's key stays theirs and Omega never holds one to hold.
//!
//! # What `OMEGA-DELTA-0113` added
//!
//! The first cut of this crate described a room and could not be used to enter
//! one. Four modules close that: [`Invitation`] is a room and the Forge's
//! answer in a line somebody can paste, [`JoinedRooms`] is the durable set that
//! becomes the composer's selector, [`RoomPresence`] is who Omega has actually
//! verified writing here, and [`parse_command`] is the grammar of an
//! instruction typed into a conversation rather than clicked in a pane.
//!
//! All four keep this crate's rule: they are values and functions of values.
//! The clock, the key, the storage and the transport all arrive from the edge,
//! which is `agent_ui::omega_community_control`.
//!
//! # GitHub is still where the code goes
//!
//! This is worth stating in the crate that names the Forge, because the Forge
//! epic's *target* is often read as its current state. It is not. Development
//! for this repository happens on GitHub today; the Forge carries the community
//! audience's conversation, and the contribution path the built-in
//! `omega-contributing` skill describes is the GitHub one. Nothing here
//! requires a migration that has not happened, and nothing here breaks while
//! GitHub is authoritative.

#![deny(missing_docs)]

mod command;
mod invitation;
mod joined;
mod outbox;
mod presence;
mod record;

use std::fmt;

use omega_audience::{Audience, AudienceId, AudienceIdError, AudienceRoster};
use serde::{Deserialize, Serialize};

pub use command::{
    COMMAND_HELP, COMMAND_PREFIX, COMMAND_VERBS, Command, CommandRefused, JOIN, LEAVE, POST,
    STATUS, WHO, parse as parse_command,
};
pub use invitation::{INVITATION_FIELDS, INVITATION_TAG, Invitation, InvitationRefused};
pub use joined::{JoinOutcome, JoinRefused, JoinReport, JoinedRoom, JoinedRooms};
pub use outbox::{
    Delivery, MAX_DELIVERY_ATTEMPTS, Outbox, OutboxEntry, QueueOutcome, RelayOutcome,
    TERMINAL_OK_PREFIXES, UnknownRecord,
};
pub use presence::{Participant, RoomPresence};
pub use record::{
    AuthorizedMessage, MESSAGE_KIND, PostRefused, REPOSITORY_ANNOUNCEMENT_KIND, RecordRefused,
    SignedRecord, UnsignedRecord, binding_of, may_post,
};

/// The prefix every Forge-backed audience identifier carries.
///
/// It is a namespace rather than decoration: an [`AudienceId`] is an opaque
/// validated string, so without a prefix a Forge repository named `local-notes`
/// and some future audience kind named `local-notes` would be the same
/// audience.
pub const FORGE_AUDIENCE_PREFIX: &str = "forge:";

/// Why a Forge repository was refused.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RepositoryError {
    /// A tenant or repository reference was empty.
    EmptyReference,
    /// A tenant or repository reference contained `/`, which separates them.
    SeparatorInReference(String),
    /// The NIP-34 coordinate was not `30617:<64 hex>:<identifier>`.
    MalformedCoordinate(String),
    /// No relay was given.
    ///
    /// Refused at construction rather than at publication. A shared audience
    /// with nowhere to publish would appear in the selector, accept a thread,
    /// accept a message, and then fail at the last step with nothing useful to
    /// say about why.
    NoRelay,
    /// The coordinate named a different repository from the one it was given
    /// for.
    ///
    /// The defect this catches is a coordinate copied from a neighbouring
    /// repository, which would bind every outbound record to somebody else's
    /// room while every screen kept saying this one.
    CoordinateNamesAnotherRepository {
        /// The `d` identifier the coordinate carries.
        coordinate_identifier: String,
        /// The repository it was offered for.
        repository_ref: String,
    },
}

impl fmt::Display for RepositoryError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EmptyReference => {
                formatter.write_str("a Forge tenant and repository reference cannot be empty")
            }
            Self::SeparatorInReference(value) => write!(
                formatter,
                "`{value}` cannot contain `/`, which separates the tenant from the repository"
            ),
            Self::NoRelay => formatter.write_str(
                "a shared audience needs at least one relay, or there is nowhere for a \
                 message in it to go",
            ),
            Self::MalformedCoordinate(value) => write!(
                formatter,
                "`{value}` is not a NIP-34 repository coordinate of the form \
                 `{REPOSITORY_ANNOUNCEMENT_KIND}:<64 hex>:<identifier>`"
            ),
            Self::CoordinateNamesAnotherRepository {
                coordinate_identifier,
                repository_ref,
            } => write!(
                formatter,
                "this coordinate announces `{coordinate_identifier}`, not `{repository_ref}`"
            ),
        }
    }
}

/// A NIP-34 repository announcement coordinate: `30617:<maintainer>:<name>`.
///
/// The addressable identity of a repository on the Forge, and the value that
/// goes in the `A`/`a` tag of every record published to it. `FORGE-10` gives
/// omega's as
/// `30617:7649603503856e5148d571eac2766b288a8ff1e9e35d380337a1d2b0015b4f92:omega`.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct RepositoryCoordinate {
    maintainer: String,
    identifier: String,
}

impl RepositoryCoordinate {
    /// Reads a coordinate.
    ///
    /// # Errors
    ///
    /// [`RepositoryError::MalformedCoordinate`] for anything that is not the
    /// announcement kind, a 64-character lowercase hex key, and a non-empty
    /// identifier. Strict rather than lenient because a coordinate that parses
    /// loosely is one that can be wrong in a way nothing reports: the identifier
    /// is allowed to contain `:` (NIP-01 says the remainder is the `d` value),
    /// so only the first two fields are fixed.
    pub fn parse(value: &str) -> Result<Self, RepositoryError> {
        let malformed = || RepositoryError::MalformedCoordinate(value.to_string());
        let mut fields = value.splitn(3, ':');
        let kind = fields.next().ok_or_else(malformed)?;
        let maintainer = fields.next().ok_or_else(malformed)?;
        let identifier = fields.next().ok_or_else(malformed)?;

        if kind != REPOSITORY_ANNOUNCEMENT_KIND.to_string() {
            return Err(malformed());
        }
        if maintainer.len() != 64
            || !maintainer
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        {
            return Err(malformed());
        }
        if identifier.is_empty() {
            return Err(malformed());
        }

        Ok(Self {
            maintainer: maintainer.to_string(),
            identifier: identifier.to_string(),
        })
    }

    /// The key that announced the repository.
    #[must_use]
    pub fn maintainer(&self) -> &str {
        &self.maintainer
    }

    /// The repository's `d` identifier, which is its name on the Forge.
    #[must_use]
    pub fn identifier(&self) -> &str {
        &self.identifier
    }
}

impl fmt::Display for RepositoryCoordinate {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "{REPOSITORY_ANNOUNCEMENT_KIND}:{}:{}",
            self.maintainer, self.identifier
        )
    }
}

impl TryFrom<String> for RepositoryCoordinate {
    type Error = RepositoryError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::parse(&value)
    }
}

impl From<RepositoryCoordinate> for String {
    fn from(value: RepositoryCoordinate) -> Self {
        value.to_string()
    }
}

/// What an invitation hands somebody.
///
/// A described repository rather than a compiled-in one. `OMEGA-DELTA-0070`
/// made the same call for the public chat skill and gave the reason: a relay
/// host name, a group identifier, and an operator's choice of infrastructure
/// are configuration, and putting them in the code makes the code work for
/// exactly one deployment. Omega ships the ability to join a Forge repository;
/// it does not ship the address of one.
///
/// The field names are the Forge's own, so this decodes what the Forge already
/// serves rather than a shape somebody transcribed.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CommunityDescriptor {
    /// The Forge tenant.
    pub tenant_ref: String,
    /// The repository within it.
    pub repository_ref: String,
    /// The NIP-34 announcement coordinate.
    pub coordinate: RepositoryCoordinate,
    /// The relays this repository's records live on, most preferred first.
    pub relays: Vec<String>,
    /// What a person reads in the audience selector.
    pub name: String,
}

impl CommunityDescriptor {
    /// Reads a descriptor into a repository.
    ///
    /// # Errors
    ///
    /// [`RepositoryError`], as [`ForgeRepository::new`].
    pub fn into_repository(self) -> Result<ForgeRepository, RepositoryError> {
        ForgeRepository::new(
            self.tenant_ref,
            self.repository_ref,
            self.coordinate,
            self.relays,
            self.name,
        )
    }
}

/// A repository on the OpenAgents Forge, and the audience behind it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ForgeRepository {
    tenant_ref: String,
    repository_ref: String,
    coordinate: RepositoryCoordinate,
    relays: Vec<String>,
    name: String,
}

impl ForgeRepository {
    /// Describes a repository the Forge already hosts.
    ///
    /// `name` is what a person reads in the audience selector — "Omega
    /// development", not `tenant.openagents/omega`. The identity and the label
    /// are separate because the label is allowed to change and the identity is
    /// not.
    ///
    /// `relays` is a list rather than a single address from the first contract.
    /// Buzz assumes one workspace relay; the accepted parity direction is that
    /// relay choice and replacement are first-class, and widening this later
    /// would be a stored-record migration for every profile that had joined
    /// anything. The first entry is the one a record's relay hint names.
    ///
    /// # Errors
    ///
    /// [`RepositoryError`], including the case where the coordinate announces a
    /// different repository from `repository_ref`, and the case where no relay
    /// is given — a shared audience with nowhere to publish is a room that
    /// cannot say why it is failing.
    pub fn new(
        tenant_ref: impl Into<String>,
        repository_ref: impl Into<String>,
        coordinate: RepositoryCoordinate,
        relays: impl IntoIterator<Item = impl Into<String>>,
        name: impl Into<String>,
    ) -> Result<Self, RepositoryError> {
        let tenant_ref = tenant_ref.into().trim().to_string();
        let repository_ref = repository_ref.into().trim().to_string();

        for reference in [&tenant_ref, &repository_ref] {
            if reference.is_empty() {
                return Err(RepositoryError::EmptyReference);
            }
            if reference.contains('/') {
                return Err(RepositoryError::SeparatorInReference(reference.clone()));
            }
        }
        if coordinate.identifier() != repository_ref {
            return Err(RepositoryError::CoordinateNamesAnotherRepository {
                coordinate_identifier: coordinate.identifier().to_string(),
                repository_ref,
            });
        }

        let relays: Vec<String> = relays
            .into_iter()
            .map(|relay| relay.into().trim().to_string())
            .filter(|relay| !relay.is_empty())
            .collect();
        if relays.is_empty() {
            return Err(RepositoryError::NoRelay);
        }

        Ok(Self {
            tenant_ref,
            repository_ref,
            coordinate,
            relays,
            name: name.into(),
        })
    }

    /// The Forge tenant, as `GET /api/forge/membership?tenantRef=` takes it.
    #[must_use]
    pub fn tenant_ref(&self) -> &str {
        &self.tenant_ref
    }

    /// The repository's reference within the tenant.
    #[must_use]
    pub fn repository_ref(&self) -> &str {
        &self.repository_ref
    }

    /// The NIP-34 coordinate every published record is bound to.
    #[must_use]
    pub fn coordinate(&self) -> &RepositoryCoordinate {
        &self.coordinate
    }

    /// The relay a record's hint names, which is the first admitted one.
    #[must_use]
    pub fn relay(&self) -> &str {
        self.relays.first().map_or("", String::as_str)
    }

    /// Every relay admitted for this repository, most preferred first.
    #[must_use]
    pub fn relays(&self) -> &[String] {
        &self.relays
    }

    /// The stored key of this repository's audience.
    ///
    /// Derived, never chosen. Two repositories cannot collide on it unless they
    /// are the same repository, and a build that had joined the audience under
    /// a hand-written key would fail to resolve threads recorded under the
    /// derived one — which is why nothing here takes a key as a parameter.
    #[must_use]
    pub fn audience_key(&self) -> String {
        format!(
            "{FORGE_AUDIENCE_PREFIX}{}/{}",
            self.tenant_ref, self.repository_ref
        )
    }

    /// This repository's audience.
    ///
    /// Always [`omega_audience::Reach::Shared`], because
    /// [`Audience::joined`] cannot produce anything else.
    ///
    /// # Errors
    ///
    /// [`AudienceIdError`], which cannot happen for a constructed
    /// [`ForgeRepository`] — the key is non-empty and prefixed — and is
    /// propagated rather than unwrapped so that a later change to either rule
    /// surfaces as an error instead of a panic in a shipped binary.
    pub fn audience(&self) -> Result<Audience, AudienceIdError> {
        Audience::joined(self.audience_key(), self.name.clone())
    }

    /// This repository's audience identity.
    ///
    /// # Errors
    ///
    /// As [`Self::audience`].
    pub fn audience_id(&self) -> Result<AudienceId, AudienceIdError> {
        AudienceId::joined(self.audience_key())
    }
}

/// What kind of actor a Forge binding is for.
///
/// `FORGE-04` admits exactly two, and an agent cannot bind itself: it carries
/// its owner's attestation. Omega does not decide this and does not extend it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ActorKind {
    /// A person.
    Human,
    /// An agent, attached under a human owner's attestation.
    Agent,
}

/// Whether a Forge binding is still live.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MembershipState {
    /// Admitted.
    Active,
    /// Revoked. The row survives so that revocation survives replay.
    Tombstoned,
}

/// A role the Forge granted.
///
/// The unknown case is a variant rather than a parse failure, and it grants
/// nothing. A role this build has not heard of is a role a newer Forge added,
/// and the two safe readings of it are "refuse the whole membership" and "keep
/// the membership, grant nothing extra". The second is chosen because the first
/// would lock a person out of a room they are in over a field they never see.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(from = "String", into = "String")]
pub enum RoleRef {
    /// `forge:admin`. Any scope.
    Admin,
    /// `forge:member`. Read and write.
    Member,
    /// `forge:viewer`. Read only.
    Viewer,
    /// A role this build does not know. Grants nothing.
    Unknown(String),
}

impl RoleRef {
    /// May somebody holding this role send a message into the room?
    ///
    /// The mapping is the Forge's own: `forge:viewer` gets `git:upload-pack`
    /// and nothing else, so a viewer reads and does not write. Omega does not
    /// get to be more generous than the credential the same person would be
    /// issued for a push.
    #[must_use]
    pub const fn may_write(&self) -> bool {
        matches!(self, Self::Admin | Self::Member)
    }
}

impl From<String> for RoleRef {
    fn from(value: String) -> Self {
        match value.as_str() {
            "forge:admin" => Self::Admin,
            "forge:member" => Self::Member,
            "forge:viewer" => Self::Viewer,
            _ => Self::Unknown(value),
        }
    }
}

impl From<RoleRef> for String {
    fn from(value: RoleRef) -> Self {
        match value {
            RoleRef::Admin => "forge:admin".to_string(),
            RoleRef::Member => "forge:member".to_string(),
            RoleRef::Viewer => "forge:viewer".to_string(),
            RoleRef::Unknown(other) => other,
        }
    }
}

/// What `GET /api/forge/membership?tenantRef=…` says about this person.
///
/// Decoded from the Forge's own response rather than mirrored into a second
/// store, so there is exactly one authority on whether somebody is a member and
/// this is not it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ForgeMembership {
    /// Whether the binding is for a person or an agent.
    pub actor_kind: ActorKind,
    /// The Forge's deterministic identifier for the binding.
    pub binding_ref: String,
    /// Whether the binding is live.
    pub membership_state: MembershipState,
    /// The roles the Forge granted.
    pub role_refs: Vec<RoleRef>,
    /// The tenant the binding is in. One npub per tenant.
    pub tenant_ref: String,
}

/// Why somebody is not in this room, or may not write to it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MembershipRefused {
    /// The Forge revoked the binding.
    Tombstoned,
    /// The binding is for a different tenant.
    WrongTenant {
        /// The tenant the repository is in.
        expected: String,
        /// The tenant the binding names.
        found: String,
    },
    /// The binding is live and read-only.
    ReadOnly {
        /// The roles it carries, rendered as the Forge names them.
        roles: Vec<String>,
    },
}

impl fmt::Display for MembershipRefused {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Tombstoned => formatter.write_str(
                "this Forge membership has been revoked. Ask the owner for a new invitation.",
            ),
            Self::WrongTenant { expected, found } => write!(
                formatter,
                "this membership is in `{found}`, and this repository is in `{expected}`."
            ),
            Self::ReadOnly { roles } => write!(
                formatter,
                "this Forge membership can read this repository and not write to it ({}).",
                roles.join(", ")
            ),
        }
    }
}

impl ForgeMembership {
    /// Is this binding live, and for this repository's tenant?
    ///
    /// # Errors
    ///
    /// [`MembershipRefused::Tombstoned`] or
    /// [`MembershipRefused::WrongTenant`].
    pub fn admits_reading(&self, repository: &ForgeRepository) -> Result<(), MembershipRefused> {
        if self.tenant_ref != repository.tenant_ref() {
            return Err(MembershipRefused::WrongTenant {
                expected: repository.tenant_ref().to_string(),
                found: self.tenant_ref.clone(),
            });
        }
        if self.membership_state == MembershipState::Tombstoned {
            return Err(MembershipRefused::Tombstoned);
        }
        Ok(())
    }

    /// May this binding send a message into this repository's room?
    ///
    /// # Errors
    ///
    /// [`MembershipRefused`], including [`MembershipRefused::ReadOnly`] for a
    /// live viewer.
    pub fn admits_writing(&self, repository: &ForgeRepository) -> Result<(), MembershipRefused> {
        self.admits_reading(repository)?;
        if self.role_refs.iter().any(RoleRef::may_write) {
            Ok(())
        } else {
            Err(MembershipRefused::ReadOnly {
                roles: self
                    .role_refs
                    .iter()
                    .cloned()
                    .map(String::from)
                    .collect::<Vec<_>>(),
            })
        }
    }
}

/// What the audience selector offers this profile.
///
/// A membership that is absent, revoked, or in another tenant produces the
/// roster a profile that has joined nothing has: Local, alone, and not broken.
/// That is omega#108 acceptance 4 and half of acceptance 5 — leaving does not
/// remove a thread, it makes the thread's recorded audience one this profile
/// can no longer resolve, which
/// [`omega_audience::AudienceRoster::describe`] already renders as
/// `Unknown audience` rather than as private.
#[must_use]
pub fn roster(
    repository: &ForgeRepository,
    membership: Option<&ForgeMembership>,
) -> AudienceRoster {
    let admitted = membership
        .filter(|membership| membership.admits_reading(repository).is_ok())
        .and_then(|_| repository.audience().ok());

    match admitted {
        Some(audience) => AudienceRoster::new([audience]),
        None => AudienceRoster::default(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use omega_audience::{Reach, ThreadAudience};

    /// `FORGE-10`'s receipt, quoted.
    pub(crate) const OMEGA_COORDINATE: &str =
        "30617:7649603503856e5148d571eac2766b288a8ff1e9e35d380337a1d2b0015b4f92:omega";

    pub(crate) fn omega_repository() -> ForgeRepository {
        omega_descriptor()
            .into_repository()
            .expect("the receipted repository")
    }

    /// The descriptor an invitation to omega's own community audience carries.
    /// Written out here, in a test, rather than compiled into the crate: the
    /// crate ships the ability to join a Forge repository, not the address of
    /// one.
    fn omega_descriptor() -> CommunityDescriptor {
        CommunityDescriptor {
            tenant_ref: "tenant.openagents".to_string(),
            repository_ref: "omega".to_string(),
            coordinate: RepositoryCoordinate::parse(OMEGA_COORDINATE)
                .expect("the receipted coordinate"),
            relays: vec!["wss://relay.openagents.com".to_string()],
            name: "Omega development".to_string(),
        }
    }

    pub(crate) fn membership(state: MembershipState, roles: &[RoleRef]) -> ForgeMembership {
        ForgeMembership {
            actor_kind: ActorKind::Human,
            binding_ref: "forge_actor.human.7e921936b0fabb102e39383971789420".to_string(),
            membership_state: state,
            role_refs: roles.to_vec(),
            tenant_ref: "tenant.openagents".to_string(),
        }
    }

    #[test]
    fn a_repository_audience_is_derived_from_the_forge_and_is_always_shared() {
        let repository = omega_repository();

        assert_eq!(
            repository.audience_key(),
            "forge:tenant.openagents/omega",
            "the key is derived from the tenant and repository, so two builds \
             cannot join the same room under two different keys"
        );
        let audience = repository.audience().expect("a joined audience");
        assert_eq!(audience.reach(), Reach::Shared);
        assert!(!audience.is_local());
        assert_eq!(audience.name(), "Omega development");
    }

    /// A descriptor is what an invitation hands somebody, and it has to survive
    /// the trip.
    #[test]
    fn a_descriptor_becomes_a_room_and_back_again() {
        let descriptor = omega_descriptor();
        let encoded = serde_json::to_string(&descriptor).expect("a descriptor encodes");
        let decoded: CommunityDescriptor =
            serde_json::from_str(&encoded).expect("a descriptor decodes");

        assert_eq!(decoded, descriptor);
        let repository = decoded.into_repository().expect("a described room");
        assert_eq!(repository.audience_key(), "forge:tenant.openagents/omega");
        assert_eq!(repository.relay(), "wss://relay.openagents.com");
        assert_eq!(repository.relays().len(), 1);
    }

    #[test]
    fn a_room_with_nowhere_to_publish_is_refused_at_construction() {
        let no_relays: Vec<String> = Vec::new();

        assert_eq!(
            ForgeRepository::new(
                "tenant.openagents",
                "omega",
                RepositoryCoordinate::parse(OMEGA_COORDINATE).expect("the coordinate"),
                no_relays,
                "Omega development",
            ),
            Err(RepositoryError::NoRelay),
            "a room that appeared in the selector and then failed at the last \
             step would have nothing useful to say about why"
        );
    }

    #[test]
    fn more_than_one_relay_is_admitted_from_the_first_contract() {
        let mut descriptor = omega_descriptor();
        descriptor.relays.push("wss://relay.example".to_string());
        let repository = descriptor.into_repository().expect("a described room");

        assert_eq!(repository.relays().len(), 2);
        assert_eq!(
            repository.relay(),
            "wss://relay.openagents.com",
            "the hint on a record names the first admitted relay"
        );
    }

    #[test]
    fn a_coordinate_for_another_repository_is_refused() {
        let neighbour = RepositoryCoordinate::parse(
            "30617:7649603503856e5148d571eac2766b288a8ff1e9e35d380337a1d2b0015b4f92:vortex",
        )
        .expect("a well formed coordinate");

        assert_eq!(
            ForgeRepository::new(
                "tenant.openagents",
                "omega",
                neighbour,
                ["wss://relay.openagents.com"],
                "Omega development",
            ),
            Err(RepositoryError::CoordinateNamesAnotherRepository {
                coordinate_identifier: "vortex".to_string(),
                repository_ref: "omega".to_string(),
            }),
            "a coordinate copied from a neighbouring repository would bind \
             every outbound record to somebody else's room"
        );
    }

    #[test]
    fn a_coordinate_is_read_strictly_or_not_at_all() {
        for malformed in [
            "",
            "omega",
            "30617:omega",
            "30618:7649603503856e5148d571eac2766b288a8ff1e9e35d380337a1d2b0015b4f92:omega",
            "30617:7649603503856E5148D571EAC2766B288A8FF1E9E35D380337A1D2B0015B4F92:omega",
            "30617:7649603503856e5148d571eac2766b288a8ff1e9e35d380337a1d2b0015b4f9:omega",
            "30617:7649603503856e5148d571eac2766b288a8ff1e9e35d380337a1d2b0015b4f92:",
        ] {
            assert_eq!(
                RepositoryCoordinate::parse(malformed),
                Err(RepositoryError::MalformedCoordinate(malformed.to_string())),
                "`{malformed}` must not read as a repository coordinate"
            );
        }

        let coordinate = RepositoryCoordinate::parse(OMEGA_COORDINATE).expect("the receipted one");
        assert_eq!(coordinate.identifier(), "omega");
        assert_eq!(coordinate.to_string(), OMEGA_COORDINATE);
    }

    /// omega#108 acceptance 4, and the state every profile is in before an
    /// invitation.
    #[test]
    fn a_profile_that_has_joined_nothing_sees_local_alone() {
        let roster = roster(&omega_repository(), None);

        assert_eq!(roster.len(), 1);
        assert!(!roster.has_joined_anything());
        assert!(!roster.is_empty(), "and nothing reads as broken");
    }

    #[test]
    fn an_invited_person_sees_the_community_audience_beside_local() {
        let repository = omega_repository();
        let roster = roster(
            &repository,
            Some(&membership(MembershipState::Active, &[RoleRef::Member])),
        );

        assert_eq!(roster.len(), 2);
        assert!(roster.has_joined_anything());
        assert!(
            roster
                .entries()
                .next()
                .expect("a roster always has a first entry")
                .is_local(),
            "Local stays the first thing a person reaches"
        );
        assert_eq!(
            roster
                .resolve(&repository.audience_id().expect("an identity"))
                .expect("the community audience is in the selector")
                .name(),
            "Omega development"
        );
    }

    /// omega#108 acceptance 5: leaving degrades visibly.
    #[test]
    fn a_revoked_membership_leaves_its_threads_unresolved_and_never_private() {
        let repository = omega_repository();
        let community = repository.audience_id().expect("an identity");

        let after_revocation = roster(
            &repository,
            Some(&membership(MembershipState::Tombstoned, &[RoleRef::Member])),
        );

        assert_eq!(
            after_revocation.len(),
            1,
            "the room is gone from the selector"
        );
        let described = after_revocation.describe(&community);
        assert_eq!(described, ThreadAudience::Unresolved(community));
        assert!(
            !described.is_private_to_this_computer(),
            "a thread held in a room this profile has been removed from was \
             never private, and must not start reading as though it were"
        );
    }

    #[test]
    fn a_membership_in_another_tenant_is_not_a_membership_here() {
        let repository = omega_repository();
        let mut elsewhere = membership(MembershipState::Active, &[RoleRef::Admin]);
        elsewhere.tenant_ref = "tenant.someone-else".to_string();

        assert_eq!(
            elsewhere.admits_reading(&repository),
            Err(MembershipRefused::WrongTenant {
                expected: "tenant.openagents".to_string(),
                found: "tenant.someone-else".to_string(),
            })
        );
        assert_eq!(roster(&repository, Some(&elsewhere)).len(), 1);
    }

    #[test]
    fn a_viewer_reads_and_does_not_write() {
        let repository = omega_repository();
        let viewer = membership(MembershipState::Active, &[RoleRef::Viewer]);

        assert_eq!(viewer.admits_reading(&repository), Ok(()));
        assert_eq!(
            viewer.admits_writing(&repository),
            Err(MembershipRefused::ReadOnly {
                roles: vec!["forge:viewer".to_string()],
            }),
            "the Forge issues a viewer `git:upload-pack` and nothing else, and \
             Omega does not get to be more generous than that"
        );
        assert_eq!(roster(&repository, Some(&viewer)).len(), 2);
    }

    #[test]
    fn a_role_this_build_does_not_know_grants_nothing_and_loses_nothing() {
        let repository = omega_repository();
        let future = membership(
            MembershipState::Active,
            &[RoleRef::Unknown("forge:reviewer".to_string())],
        );

        assert_eq!(
            future.admits_reading(&repository),
            Ok(()),
            "a role a newer Forge added must not lock somebody out of a room \
             they are in"
        );
        assert_eq!(
            future.admits_writing(&repository),
            Err(MembershipRefused::ReadOnly {
                roles: vec!["forge:reviewer".to_string()],
            }),
            "and it must not grant anything either"
        );
        assert_eq!(
            String::from(RoleRef::Unknown("forge:reviewer".to_string())),
            "forge:reviewer",
            "an unknown role survives the round trip under its own name"
        );
    }

    /// The Forge's response, decoded from the shape it actually serves.
    #[test]
    fn the_membership_response_is_read_as_the_forge_serves_it() {
        let served = r#"{
            "actorKind": "human",
            "bindingRef": "forge_actor.human.7e921936b0fabb102e39383971789420",
            "membershipState": "active",
            "roleRefs": ["forge:admin"],
            "tenantRef": "tenant.openagents"
        }"#;

        let decoded: ForgeMembership =
            serde_json::from_str(served).expect("the Forge membership response decodes");

        assert_eq!(decoded.actor_kind, ActorKind::Human);
        assert_eq!(decoded.membership_state, MembershipState::Active);
        assert_eq!(decoded.role_refs, vec![RoleRef::Admin]);
        assert_eq!(decoded.admits_writing(&omega_repository()), Ok(()));
    }
}
