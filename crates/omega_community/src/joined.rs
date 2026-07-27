//! Every room this profile has joined, which is what the composer's selector
//! offers. `OMEGA-DELTA-0113`, omega#108.
//!
//! omega#108 deliverable 2: the community workspace is "visible from the
//! composer, selected through omega#107's control. Local stays the default; the
//! community workspace is something a person chooses."
//!
//! [`crate::roster`] answers that for one room. This is the durable version:
//! the set a profile has actually joined, keyed by the audience key each room
//! derives, serialisable so a restart does not empty the selector, and
//! refusing at the join rather than at the send.
//!
//! # Why the refusal is at the join
//!
//! [`JoinedRooms::join`] asks [`crate::ForgeMembership::admits_reading`] before
//! it records anything. A room that cannot be read is a room that must not
//! appear in the selector, because the selector is a list of places a person
//! can start a conversation and an entry that fails on the first send is worse
//! than no entry: it is an invitation to type the message again.
//!
//! # Why leaving keeps nothing
//!
//! [`JoinedRooms::leave`] removes the room, and that is all it does. The
//! threads recorded in it keep their audience, which
//! [`omega_audience::AudienceRoster::describe`] then reports as
//! `Unresolved` — omega#108 acceptance 5, and the reason it is right: a
//! conversation held in a room this profile has left was never private, and
//! keeping the room in the roster so the threads still resolve would say the
//! person is still in it.

use std::collections::BTreeMap;
use std::fmt;

use omega_audience::{AudienceId, AudienceIdError, AudienceRoster};
use serde::{Deserialize, Serialize};

use crate::{ForgeMembership, ForgeRepository, Invitation, MembershipRefused, RepositoryError};

/// One room this profile is in.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct JoinedRoom {
    /// Where the room is.
    pub repository: ForgeRepository,
    /// What the Forge last said about this person here.
    ///
    /// A snapshot, and it is stored as one rather than as a live answer. The
    /// Forge is the authority; this is the most recent thing it said, and
    /// [`Self::membership_as_of`] is when it said it, so a surface can tell a
    /// person how old the answer is instead of implying it is current.
    pub membership: ForgeMembership,
    /// When this profile joined.
    pub joined_at: u64,
    /// When the membership snapshot was taken.
    pub membership_as_of: u64,
}

impl JoinedRoom {
    /// When the membership snapshot was taken.
    #[must_use]
    pub const fn membership_as_of(&self) -> u64 {
        self.membership_as_of
    }

    /// The stored key of this room's audience.
    #[must_use]
    pub fn audience_key(&self) -> String {
        self.repository.audience_key()
    }

    /// What a person reads for this room.
    #[must_use]
    pub fn name(&self) -> String {
        self.repository
            .audience()
            .map_or_else(|_| self.repository.repository_ref().to_string(), |audience| {
                audience.name().to_string()
            })
    }
}

/// What happened when a room was joined.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum JoinOutcome {
    /// This profile was not in the room, and now is.
    Joined,
    /// This profile was already in the room, and the Forge's answer had not
    /// changed. Nothing was written.
    AlreadyJoined,
    /// This profile was already in the room, and the invitation carried a newer
    /// answer from the Forge — different roles, or a different binding. The
    /// snapshot was replaced.
    ///
    /// A distinct outcome rather than a silent overwrite, because a second
    /// invitation changing what somebody may do in a room is a thing they are
    /// entitled to be told about.
    Refreshed,
}

/// Why a room was not joined.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum JoinRefused {
    /// The invitation described a room that cannot exist.
    Repository(RepositoryError),
    /// The Forge does not admit this person to that room.
    Membership(MembershipRefused),
    /// The room's derived identity was refused.
    ///
    /// Cannot happen for a constructed [`ForgeRepository`], whose key is
    /// non-empty and prefixed. Propagated rather than unwrapped, as
    /// [`ForgeRepository::audience`] is, so that a later change to either rule
    /// surfaces as a refusal instead of a panic in a shipped binary.
    Identity(AudienceIdError),
}

impl fmt::Display for JoinRefused {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Repository(error) => write!(formatter, "{error}"),
            Self::Membership(refusal) => write!(formatter, "{refusal}"),
            Self::Identity(error) => write!(formatter, "{error}"),
        }
    }
}

/// What a caller says after a room was joined.
///
/// Owned rather than a borrow of the stored room, so a caller composing the
/// sentence a person reads does not hold the record open while it does.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct JoinReport {
    /// What happened.
    pub outcome: JoinOutcome,
    /// The room's audience identity, which is what a thread records.
    pub audience: AudienceId,
    /// What a person reads for it.
    pub name: String,
    /// The roles the Forge granted, as the Forge names them.
    pub roles: Vec<String>,
    /// May this person send a message here?
    pub may_write: bool,
}

/// Every room this profile has joined.
///
/// Keyed by the audience key each room derives, so the same repository cannot
/// be in here twice under two names and a thread's recorded audience resolves
/// to exactly one room.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct JoinedRooms {
    rooms: BTreeMap<String, JoinedRoom>,
}

impl JoinedRooms {
    /// A profile that has joined nothing.
    #[must_use]
    pub fn new() -> Self {
        Self {
            rooms: BTreeMap::new(),
        }
    }

    /// Accepts an invitation.
    ///
    /// # Errors
    ///
    /// [`JoinRefused`]. A revoked binding, a binding for another tenant, and a
    /// room whose coordinate names a different repository are all refused here
    /// — before the room can appear in the selector, rather than after somebody
    /// has typed into it.
    pub fn join(&mut self, invitation: Invitation, now: u64) -> Result<JoinReport, JoinRefused> {
        let repository = invitation
            .descriptor
            .into_repository()
            .map_err(JoinRefused::Repository)?;
        invitation
            .membership
            .admits_reading(&repository)
            .map_err(JoinRefused::Membership)?;
        let audience = repository.audience().map_err(JoinRefused::Identity)?;

        let key = repository.audience_key();
        let membership = invitation.membership;
        let report = JoinReport {
            outcome: match self.rooms.get(&key) {
                Some(existing)
                    if existing.repository == repository && existing.membership == membership =>
                {
                    JoinOutcome::AlreadyJoined
                }
                Some(_) => JoinOutcome::Refreshed,
                None => JoinOutcome::Joined,
            },
            audience: audience.id().clone(),
            name: audience.name().to_string(),
            roles: membership
                .role_refs
                .iter()
                .cloned()
                .map(String::from)
                .collect(),
            may_write: membership.admits_writing(&repository).is_ok(),
        };

        if report.outcome == JoinOutcome::AlreadyJoined {
            return Ok(report);
        }

        // A second invitation does not restart the membership. `joined_at` is
        // when this profile first entered the room, and a refresh that reset it
        // would erase the only durable answer to "how long have I been here".
        let joined_at = self
            .rooms
            .get(&key)
            .map_or(now, |existing| existing.joined_at);
        self.rooms.insert(
            key,
            JoinedRoom {
                repository,
                membership,
                joined_at,
                membership_as_of: now,
            },
        );
        Ok(report)
    }

    /// Leaves a room, and keeps nothing.
    ///
    /// `false` when this profile was not in it, which a caller should say
    /// rather than report a success.
    pub fn leave(&mut self, id: &AudienceId) -> bool {
        self.rooms.remove(id.as_key()).is_some()
    }

    /// The room an audience identity names, if this profile is in it.
    #[must_use]
    pub fn room(&self, id: &AudienceId) -> Option<&JoinedRoom> {
        self.rooms.get(id.as_key())
    }

    /// Every room, in a stable order.
    pub fn rooms(&self) -> impl Iterator<Item = &JoinedRoom> + '_ {
        self.rooms.values()
    }

    /// What the composer's selector offers this profile, Local first.
    ///
    /// The roster is built from the rooms rather than stored beside them, so
    /// there is no second list to fall out of step with this one.
    #[must_use]
    pub fn roster(&self) -> AudienceRoster {
        AudienceRoster::new(
            self.rooms
                .values()
                .filter_map(|room| room.repository.audience().ok()),
        )
    }

    /// How many rooms this profile is in. Zero is the ordinary state.
    #[must_use]
    pub fn len(&self) -> usize {
        self.rooms.len()
    }

    /// Has this profile joined nothing?
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.rooms.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::invitation::tests::omega_invitation_text;
    use crate::{MembershipState, RoleRef};
    use omega_audience::ThreadAudience;

    fn invitation() -> Invitation {
        Invitation::parse(&omega_invitation_text()).expect("a well formed invitation")
    }

    fn joined() -> JoinedRooms {
        let mut rooms = JoinedRooms::new();
        rooms.join(invitation(), 1_800_000_000).expect("a member joins");
        rooms
    }

    /// omega#108 acceptance 4, and the state every profile is in until somebody
    /// is invited.
    #[test]
    fn a_profile_that_has_joined_nothing_offers_local_alone() {
        let rooms = JoinedRooms::new();
        let roster = rooms.roster();

        assert!(rooms.is_empty());
        assert_eq!(roster.len(), 1);
        assert!(!roster.has_joined_anything());
        assert!(!roster.is_empty(), "and nothing reads as broken");
    }

    /// omega#108 acceptance 1 and deliverable 2, as far as they can be proved
    /// without a window: the invited person's roster carries the room.
    #[test]
    fn an_accepted_invitation_puts_the_room_in_the_selector_beside_local() {
        let rooms = joined();
        let roster = rooms.roster();

        assert_eq!(roster.len(), 2);
        assert!(
            roster
                .entries()
                .next()
                .expect("a roster always has a first entry")
                .is_local(),
            "Local stays the first thing a person reaches"
        );
        let community = roster
            .entries()
            .nth(1)
            .expect("the room this profile joined");
        assert_eq!(community.name(), "Omega development");
        assert_eq!(community.id().as_key(), "forge:tenant.openagents/omega");
        assert!(!community.is_local());
        assert!(!community.is_preview(), "and it is not the fixture");
    }

    #[test]
    fn joining_the_same_room_twice_is_one_room_and_says_so() {
        let mut rooms = joined();

        let report = rooms
            .join(invitation(), 1_800_000_100)
            .expect("still a member");
        assert_eq!(report.outcome, JoinOutcome::AlreadyJoined);
        assert!(report.may_write);
        assert_eq!(report.roles, vec!["forge:member".to_string()]);
        assert_eq!(rooms.len(), 1);
        assert_eq!(rooms.roster().len(), 2);
        assert_eq!(
            rooms
                .rooms()
                .next()
                .expect("the one room")
                .membership_as_of(),
            1_800_000_000,
            "nothing was written, so the snapshot did not move"
        );
    }

    /// A second invitation that changes what somebody may do is not a silent
    /// overwrite.
    #[test]
    fn a_newer_answer_from_the_forge_refreshes_the_snapshot_and_reports_it() {
        let mut rooms = joined();
        let demoted = Invitation::parse(
            &omega_invitation_text().replace("roles=forge:member", "roles=forge:viewer"),
        )
        .expect("a well formed invitation");

        let report = rooms.join(demoted, 1_800_000_500).expect("still admitted");
        assert_eq!(report.outcome, JoinOutcome::Refreshed);
        assert!(
            !report.may_write,
            "a second invitation that takes away writing says so"
        );

        let room = rooms.rooms().next().expect("the one room");
        assert_eq!(room.membership.role_refs, vec![RoleRef::Viewer]);
        assert_eq!(room.joined_at, 1_800_000_000, "joining did not happen twice");
        assert_eq!(room.membership_as_of(), 1_800_000_500);
        assert_eq!(rooms.len(), 1);
    }

    /// The falsifier this rule exists for: a room a person cannot read must
    /// never reach the selector.
    #[test]
    fn a_revoked_invitation_is_refused_at_the_join_and_not_at_the_send() {
        let mut rooms = JoinedRooms::new();
        let revoked =
            Invitation::parse(&omega_invitation_text().replace("state=active", "state=tombstoned"))
                .expect("a well formed invitation");

        assert_eq!(
            rooms.join(revoked, 1_800_000_000).err(),
            Some(JoinRefused::Membership(MembershipRefused::Tombstoned)),
            "a selector entry that fails on the first send is worse than no \
             entry: it is an invitation to type the message again"
        );
        assert!(rooms.is_empty());
        assert_eq!(rooms.roster().len(), 1);
    }

    #[test]
    fn an_invitation_naming_a_room_that_cannot_exist_is_refused() {
        let mut rooms = JoinedRooms::new();
        let mismatched =
            Invitation::parse(&omega_invitation_text().replace("repository=omega", "repository=vortex"))
                .expect("the fields are well formed");

        assert_eq!(
            rooms.join(mismatched, 1_800_000_000).err(),
            Some(JoinRefused::Repository(
                RepositoryError::CoordinateNamesAnotherRepository {
                    coordinate_identifier: "omega".to_string(),
                    repository_ref: "vortex".to_string(),
                }
            ))
        );
        assert!(rooms.is_empty());
    }

    /// A viewer is in the room. Reading is what a viewer is for, and the
    /// refusal to write happens where writing is attempted.
    #[test]
    fn a_viewer_is_in_the_selector_and_is_refused_at_the_post() {
        let mut rooms = JoinedRooms::new();
        let viewer = Invitation::parse(
            &omega_invitation_text().replace("roles=forge:member", "roles=forge:viewer"),
        )
        .expect("a well formed invitation");

        rooms.join(viewer, 1_800_000_000).expect("a viewer joins");
        assert_eq!(rooms.roster().len(), 2);

        let room = rooms.rooms().next().expect("the one room");
        assert_eq!(
            room.membership.admits_writing(&room.repository),
            Err(MembershipRefused::ReadOnly {
                roles: vec!["forge:viewer".to_string()],
            })
        );
    }

    /// omega#108 acceptance 5: leaving degrades visibly rather than silently.
    #[test]
    fn leaving_removes_the_room_and_leaves_its_threads_unresolved() {
        let mut rooms = joined();
        let community = rooms
            .rooms()
            .next()
            .expect("the one room")
            .repository
            .audience_id()
            .expect("an identity");

        assert!(rooms.leave(&community));
        assert!(!rooms.leave(&community), "and leaving twice is not a success");
        assert_eq!(rooms.roster().len(), 1);

        let described = rooms.roster().describe(&community);
        assert_eq!(described, ThreadAudience::Unresolved(community));
        assert!(
            !described.is_private_to_this_computer(),
            "a conversation held in a room this profile has left was never \
             private, and must not start reading as though it were"
        );
    }

    /// The parity bar: survives restart.
    #[test]
    fn the_rooms_a_profile_joined_survive_being_written_down_and_read_back() {
        let rooms = joined();
        let encoded = serde_json::to_string(&rooms).expect("the rooms encode");
        let decoded: JoinedRooms = serde_json::from_str(&encoded).expect("and decode");

        assert_eq!(decoded, rooms);
        assert_eq!(decoded.roster().len(), 2);
        assert_eq!(
            decoded
                .room(
                    &decoded
                        .rooms()
                        .next()
                        .expect("the one room")
                        .repository
                        .audience_id()
                        .expect("an identity")
                )
                .expect("the room resolves by its own identity")
                .membership
                .membership_state,
            MembershipState::Active
        );
    }

    /// A stored file from a build that had joined nothing, and one that is
    /// simply absent, are the same state.
    #[test]
    fn an_empty_record_reads_as_a_profile_that_has_joined_nothing() {
        let decoded: JoinedRooms = serde_json::from_str("{}").expect("an empty record decodes");

        assert_eq!(decoded, JoinedRooms::new());
        assert_eq!(decoded.roster().len(), 1);
    }
}
