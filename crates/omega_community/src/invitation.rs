//! What an invitation carries, in a form a person can paste into a sentence. `OMEGA-DELTA-0113`, omega#108.
//!
//! omega#108 deliverable 3: joining is a conversation action. So an invitation
//! has to survive being pasted into a chat line — one line, no attachment, no
//! settings page, and legible enough that somebody can read what they are being
//! offered before they accept it.
//!
//! # Why the invitation carries the Forge's answer, and what that does not mean
//!
//! [`ForgeMembership`] is the Forge's own decision, served at
//! `GET /api/forge/membership`. Omega has no transport, so an invitation
//! carries the binding the owner's Forge already issued rather than a client
//! calling for it. That is the same record, delivered by the person who has it.
//!
//! **It is a claim, not an authority.** Anybody can type one of these. Nothing
//! is granted by having one: the roster entry it produces is local, the
//! credentials a push would need are still the Forge's to issue, and a relay
//! still refuses an event from somebody it does not admit. What the invitation
//! decides is which room appears in *this* person's selector, which is a
//! decision they are entitled to make about their own machine. When there is a
//! transport, the same record is refreshed from the Forge and the refusal
//! surfaces there.
//!
//! # Why an unrecognised field is refused rather than ignored
//!
//! The opposite call from [`crate::RoleRef`], deliberately. An unknown *role*
//! grants nothing, so ignoring it costs a person nothing and refusing it would
//! lock them out of a room they are in. An unknown *field* in an invitation may
//! be part of the room's address — a second coordinate, a signer hint, a
//! successor relay — and joining a room while discarding part of its
//! description is joining something other than what was sent.

use std::fmt;

use crate::{
    ActorKind, CommunityDescriptor, ForgeMembership, MembershipState, RepositoryCoordinate,
    RepositoryError, RoleRef,
};

/// The first field of every invitation, which is also how one is recognised.
///
/// Versioned from the first line so that a later shape can be told apart from
/// this one by a build that has never heard of it, rather than being parsed
/// halfway and joined.
pub const INVITATION_TAG: &str = "omega-invite:1";

/// What separates one field from the next.
const FIELD: char = ';';
/// What separates a field's name from its value.
const ASSIGN: char = '=';
/// What separates the entries of a list-valued field.
const LIST: char = ',';

const TENANT: &str = "tenant";
const REPOSITORY: &str = "repository";
const COORDINATE: &str = "coordinate";
const RELAYS: &str = "relays";
const NAME: &str = "name";
const BINDING: &str = "binding";
const ACTOR: &str = "actor";
const STATE: &str = "state";
const ROLES: &str = "roles";

/// Every field an invitation of this version carries, all of them required.
///
/// Required rather than defaulted: each of these is either the room's address
/// or the Forge's answer about a person, and a default for either is a guess
/// presented as a fact.
pub const INVITATION_FIELDS: &[&str] = &[
    TENANT, REPOSITORY, COORDINATE, RELAYS, NAME, BINDING, ACTOR, STATE, ROLES,
];

/// Why an invitation was not read.
///
/// Every variant is a sentence, because these are read by somebody who has just
/// pasted something into a chat line and needs to know whether to ask for
/// another one.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum InvitationRefused {
    /// The text does not begin with [`INVITATION_TAG`].
    NotAnInvitation,
    /// A field had no `=`.
    FieldHasNoValue(String),
    /// A field appeared twice, and there is no rule for which one wins.
    Repeated(String),
    /// A field this version does not know about.
    Unrecognised(String),
    /// A required field was absent.
    Missing(&'static str),
    /// A required field was present and empty.
    Empty(&'static str),
    /// `actor` was neither `human` nor `agent`.
    UnknownActorKind(String),
    /// `state` was neither `active` nor `tombstoned`.
    UnknownMembershipState(String),
    /// A value carried the field separator, so it could not be written down.
    SeparatorInValue(&'static str),
    /// The room the invitation describes was refused.
    Repository(RepositoryError),
}

impl fmt::Display for InvitationRefused {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotAnInvitation => write!(
                formatter,
                "this is not an invitation. One begins `{INVITATION_TAG}`."
            ),
            Self::FieldHasNoValue(field) => {
                write!(formatter, "`{field}` is not a `name{ASSIGN}value` field.")
            }
            Self::Repeated(field) => write!(
                formatter,
                "this invitation gives `{field}` twice, and there is no rule for which one wins."
            ),
            Self::Unrecognised(field) => write!(
                formatter,
                "this invitation carries `{field}`, which this version of Omega does not \
                 understand. Joining while ignoring part of a room's description would be \
                 joining something other than what was sent."
            ),
            Self::Missing(field) => write!(formatter, "this invitation has no `{field}`."),
            Self::Empty(field) => write!(formatter, "this invitation's `{field}` is empty."),
            Self::UnknownActorKind(value) => write!(
                formatter,
                "`{value}` is not an actor kind. The Forge issues `human` and `agent`."
            ),
            Self::UnknownMembershipState(value) => write!(
                formatter,
                "`{value}` is not a membership state. The Forge issues `active` and `tombstoned`."
            ),
            Self::SeparatorInValue(field) => write!(
                formatter,
                "`{field}` contains `{FIELD}`, which separates one field from the next, so this \
                 invitation cannot be written down without changing what it says."
            ),
            Self::Repository(error) => write!(formatter, "{error}"),
        }
    }
}

/// A room, and the Forge's answer about the person being invited to it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Invitation {
    /// Where the room is.
    pub descriptor: CommunityDescriptor,
    /// What the Forge said about the person this was written for.
    pub membership: ForgeMembership,
}

impl Invitation {
    /// Reads an invitation.
    ///
    /// # Errors
    ///
    /// [`InvitationRefused`], each variant of which is a sentence.
    pub fn parse(text: &str) -> Result<Self, InvitationRefused> {
        let text = text.trim();
        let mut parts = text.split(FIELD).map(str::trim);
        if parts.next() != Some(INVITATION_TAG) {
            return Err(InvitationRefused::NotAnInvitation);
        }

        let mut fields: Vec<(String, String)> = Vec::new();
        for part in parts.filter(|part| !part.is_empty()) {
            let (name, value) = part
                .split_once(ASSIGN)
                .ok_or_else(|| InvitationRefused::FieldHasNoValue(part.to_string()))?;
            let name = name.trim().to_string();
            if !INVITATION_FIELDS.contains(&name.as_str()) {
                return Err(InvitationRefused::Unrecognised(name));
            }
            if fields.iter().any(|(seen, _)| *seen == name) {
                return Err(InvitationRefused::Repeated(name));
            }
            fields.push((name, value.trim().to_string()));
        }

        let field = |wanted: &'static str| -> Result<String, InvitationRefused> {
            let value = fields
                .iter()
                .find(|(name, _)| name == wanted)
                .map(|(_, value)| value.clone())
                .ok_or(InvitationRefused::Missing(wanted))?;
            if value.is_empty() {
                return Err(InvitationRefused::Empty(wanted));
            }
            Ok(value)
        };

        let tenant_ref = field(TENANT)?;
        let repository_ref = field(REPOSITORY)?;
        let coordinate = RepositoryCoordinate::parse(&field(COORDINATE)?)
            .map_err(InvitationRefused::Repository)?;
        let relays: Vec<String> = field(RELAYS)?
            .split(LIST)
            .map(|relay| relay.trim().to_string())
            .filter(|relay| !relay.is_empty())
            .collect();
        let name = field(NAME)?;
        let binding_ref = field(BINDING)?;

        let actor = field(ACTOR)?;
        let actor_kind = match actor.as_str() {
            "human" => ActorKind::Human,
            "agent" => ActorKind::Agent,
            _ => return Err(InvitationRefused::UnknownActorKind(actor)),
        };

        let state = field(STATE)?;
        let membership_state = match state.as_str() {
            "active" => MembershipState::Active,
            "tombstoned" => MembershipState::Tombstoned,
            _ => return Err(InvitationRefused::UnknownMembershipState(state)),
        };

        // Roles are the one list where an entry this build does not know is
        // kept rather than refused. `RoleRef::Unknown` grants nothing, so a
        // newer Forge's role costs the holder nothing and refusing it would
        // lock somebody out of a room they are in.
        let role_refs: Vec<RoleRef> = field(ROLES)?
            .split(LIST)
            .map(|role| role.trim().to_string())
            .filter(|role| !role.is_empty())
            .map(RoleRef::from)
            .collect();

        let descriptor = CommunityDescriptor {
            tenant_ref: tenant_ref.clone(),
            repository_ref,
            coordinate,
            relays,
            name,
        };
        let membership = ForgeMembership {
            actor_kind,
            binding_ref,
            membership_state,
            role_refs,
            // One field, used for both halves. The tenant a room is in and the
            // tenant a binding is in cannot disagree in an invitation, because
            // there is only one place to write it.
            tenant_ref,
        };

        Ok(Self {
            descriptor,
            membership,
        })
    }

    /// Writes an invitation.
    ///
    /// # Errors
    ///
    /// [`InvitationRefused::SeparatorInValue`] when a value carries the field
    /// separator. Refused rather than escaped: an escape is a second grammar,
    /// and a room name with a semicolon in it is a thing the owner can change.
    pub fn to_text(&self) -> Result<String, InvitationRefused> {
        let relays = self.descriptor.relays.join(&LIST.to_string());
        let roles = self
            .membership
            .role_refs
            .iter()
            .cloned()
            .map(String::from)
            .collect::<Vec<_>>()
            .join(&LIST.to_string());
        let actor = match self.membership.actor_kind {
            ActorKind::Human => "human",
            ActorKind::Agent => "agent",
        };
        let state = match self.membership.membership_state {
            MembershipState::Active => "active",
            MembershipState::Tombstoned => "tombstoned",
        };

        let written = [
            (TENANT, self.descriptor.tenant_ref.clone()),
            (REPOSITORY, self.descriptor.repository_ref.clone()),
            (COORDINATE, self.descriptor.coordinate.to_string()),
            (RELAYS, relays),
            (NAME, self.descriptor.name.clone()),
            (BINDING, self.membership.binding_ref.clone()),
            (ACTOR, actor.to_string()),
            (STATE, state.to_string()),
            (ROLES, roles),
        ];

        let mut text = String::from(INVITATION_TAG);
        for (name, value) in written {
            if value.contains(FIELD) {
                return Err(InvitationRefused::SeparatorInValue(name));
            }
            text.push(FIELD);
            text.push_str(name);
            text.push(ASSIGN);
            text.push_str(&value);
        }
        Ok(text)
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::MembershipRefused;
    use crate::tests::{OMEGA_COORDINATE, membership};

    /// The invitation the owner would send for omega's own room. Written here,
    /// in a test, because the crate ships the ability to join a Forge
    /// repository and not the address of one.
    pub(crate) fn omega_invitation_text() -> String {
        format!(
            "omega-invite:1;tenant=tenant.openagents;repository=omega;\
             coordinate={OMEGA_COORDINATE};relays=wss://relay.openagents.com;\
             name=Omega development;binding=forge_actor.human.7e921936b0fabb102e39383971789420;\
             actor=human;state=active;roles=forge:member"
        )
    }

    #[test]
    fn an_invitation_is_one_line_a_person_can_paste() {
        let invitation = Invitation::parse(&omega_invitation_text()).expect("a well formed one");

        assert_eq!(invitation.descriptor.tenant_ref, "tenant.openagents");
        assert_eq!(invitation.descriptor.repository_ref, "omega");
        assert_eq!(invitation.descriptor.name, "Omega development");
        assert_eq!(
            invitation.descriptor.relays,
            vec!["wss://relay.openagents.com".to_string()]
        );
        assert_eq!(invitation.membership.role_refs, vec![RoleRef::Member]);
        assert_eq!(
            invitation.membership.membership_state,
            MembershipState::Active
        );
        assert_eq!(invitation.membership.actor_kind, ActorKind::Human);

        let room = invitation
            .descriptor
            .clone()
            .into_repository()
            .expect("a described room");
        assert_eq!(room.audience_key(), "forge:tenant.openagents/omega");
        assert_eq!(invitation.membership.admits_writing(&room), Ok(()));
    }

    #[test]
    fn an_invitation_survives_the_round_trip_it_was_written_for() {
        let invitation = Invitation::parse(&omega_invitation_text()).expect("a well formed one");
        let written = invitation.to_text().expect("it can be written down");

        assert_eq!(
            Invitation::parse(&written).expect("and read back"),
            invitation
        );
    }

    #[test]
    fn a_line_that_is_not_an_invitation_is_not_read_as_one() {
        for text in [
            "",
            "hello",
            "omega-invite:2;tenant=tenant.openagents",
            "tenant=tenant.openagents;repository=omega",
        ] {
            assert_eq!(
                Invitation::parse(text),
                Err(InvitationRefused::NotAnInvitation),
                "`{text}` must not read as an invitation"
            );
        }
    }

    #[test]
    fn a_missing_or_empty_field_is_named_rather_than_defaulted() {
        let full = omega_invitation_text();

        let without_relays = full.replace(";relays=wss://relay.openagents.com", "");
        assert_eq!(
            Invitation::parse(&without_relays),
            Err(InvitationRefused::Missing("relays")),
            "a default relay would be a guess presented as the room's address"
        );

        let empty_name = full.replace("name=Omega development", "name=");
        assert_eq!(
            Invitation::parse(&empty_name),
            Err(InvitationRefused::Empty("name"))
        );
    }

    #[test]
    fn a_field_this_version_does_not_understand_is_refused_and_not_dropped() {
        let text = format!("{};successor=wss://elsewhere", omega_invitation_text());

        assert_eq!(
            Invitation::parse(&text),
            Err(InvitationRefused::Unrecognised("successor".to_string())),
            "an unknown field may be part of the room's address, and joining \
             while discarding it would be joining something else"
        );
    }

    #[test]
    fn a_repeated_field_is_refused_rather_than_resolved() {
        let text = format!("{};repository=vortex", omega_invitation_text());

        assert_eq!(
            Invitation::parse(&text),
            Err(InvitationRefused::Repeated("repository".to_string())),
            "first-wins and last-wins are both defensible, which is why \
             neither may be chosen silently"
        );
    }

    #[test]
    fn a_malformed_field_says_which_one() {
        let text = format!("{};roles", omega_invitation_text());

        assert_eq!(
            Invitation::parse(&text),
            Err(InvitationRefused::FieldHasNoValue("roles".to_string()))
        );
    }

    #[test]
    fn an_actor_kind_or_state_the_forge_does_not_issue_is_refused() {
        let full = omega_invitation_text();

        assert_eq!(
            Invitation::parse(&full.replace("actor=human", "actor=service")),
            Err(InvitationRefused::UnknownActorKind("service".to_string()))
        );
        assert_eq!(
            Invitation::parse(&full.replace("state=active", "state=pending")),
            Err(InvitationRefused::UnknownMembershipState(
                "pending".to_string()
            ))
        );
    }

    /// The role list keeps the opposite rule from every other field here, and
    /// this is where that is visible.
    #[test]
    fn an_unknown_role_is_kept_and_grants_nothing() {
        let text = omega_invitation_text().replace("roles=forge:member", "roles=forge:reviewer");
        let invitation = Invitation::parse(&text).expect("an invitation with a newer role");
        let room = invitation
            .descriptor
            .clone()
            .into_repository()
            .expect("a described room");

        assert_eq!(
            invitation.membership.role_refs,
            vec![RoleRef::Unknown("forge:reviewer".to_string())]
        );
        assert_eq!(invitation.membership.admits_reading(&room), Ok(()));
        assert_eq!(
            invitation.membership.admits_writing(&room),
            Err(MembershipRefused::ReadOnly {
                roles: vec!["forge:reviewer".to_string()],
            })
        );
    }

    #[test]
    fn a_coordinate_for_another_repository_is_refused_at_the_invitation() {
        let text = omega_invitation_text().replace("repository=omega", "repository=vortex");
        let invitation = Invitation::parse(&text).expect("the fields are all well formed");

        assert_eq!(
            invitation.descriptor.into_repository(),
            Err(RepositoryError::CoordinateNamesAnotherRepository {
                coordinate_identifier: "omega".to_string(),
                repository_ref: "vortex".to_string(),
            })
        );
    }

    #[test]
    fn a_value_carrying_the_separator_cannot_be_written_down() {
        let mut invitation =
            Invitation::parse(&omega_invitation_text()).expect("a well formed one");
        invitation.descriptor.name = "Omega; and friends".to_string();

        assert_eq!(
            invitation.to_text(),
            Err(InvitationRefused::SeparatorInValue("name")),
            "escaping would be a second grammar, and the name is the owner's \
             to change"
        );
    }

    /// A tombstoned invitation parses. It is refused at the join, which is
    /// where the refusal is a sentence about a room rather than about a field.
    #[test]
    fn a_revoked_binding_still_reads_as_an_invitation() {
        let text = omega_invitation_text().replace("state=active", "state=tombstoned");
        let invitation = Invitation::parse(&text).expect("it is well formed");

        assert_eq!(
            invitation.membership.membership_state,
            MembershipState::Tombstoned
        );
        assert_eq!(
            invitation.membership.admits_reading(
                &invitation
                    .descriptor
                    .clone()
                    .into_repository()
                    .expect("a room")
            ),
            Err(MembershipRefused::Tombstoned)
        );
    }

    /// The tenant is written once, so the two halves cannot disagree.
    #[test]
    fn the_tenant_is_one_field_for_the_room_and_the_binding() {
        let invitation = Invitation::parse(&omega_invitation_text()).expect("a well formed one");

        assert_eq!(
            invitation.descriptor.tenant_ref,
            invitation.membership.tenant_ref
        );
        assert_eq!(
            membership(MembershipState::Active, &[RoleRef::Member]).tenant_ref,
            invitation.membership.tenant_ref,
            "and it is the tenant the rest of this crate's tests use"
        );
    }
}
