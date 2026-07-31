//! Fail-closed desktop projection for Sarah in an authenticated community call.
//!
//! The server owns room epochs, participant mappings, Sarah presence, and the
//! speaking floor. This model only renders a snapshot after all of those
//! bindings agree. It never derives authority from LiveKit display names or
//! participant metadata.

use anyhow::{Result, bail};
use collections::HashSet;
use serde_json::{Value, json};

pub const ROOM_AUTHORITY_SCHEMA: &str = "openagents.sarah.livekit-room-authority.v1";
pub const SARAH_PRINCIPAL: &str = "principal.sarah";
pub const COMMUNITY_CAPABILITY_PROFILE: &str = "community_member_v1";
pub const PROCESSOR_DISCLOSURE: &str = "sarah_openagents_openai_v1";
pub const COMMUNITY_COHORT_POLICY: &str = "authenticated_allowlisted";
pub const FLOOR_LEASE_MS: u64 = 30_000;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum CommunityCallLifecycle {
    #[default]
    Unavailable,
    ReadyToJoin,
    Joining,
    Joined,
    Leaving,
    Failed,
}

impl CommunityCallLifecycle {
    pub fn label(self) -> &'static str {
        match self {
            Self::Unavailable => "Room voice unavailable",
            Self::ReadyToJoin => "Room voice ready",
            Self::Joining => "Joining room voice",
            Self::Joined => "In room voice",
            Self::Leaving => "Leaving room voice",
            Self::Failed => "Room voice failed",
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum CommunitySarahState {
    #[default]
    Absent,
    Summoning,
    Idle,
    Listening,
    Speaking,
    Failed,
}

impl CommunitySarahState {
    pub fn label(self) -> &'static str {
        match self {
            Self::Absent => "Sarah absent",
            Self::Summoning => "Summoning Sarah",
            Self::Idle => "Sarah idle",
            Self::Listening => "Sarah listening",
            Self::Speaking => "Sarah speaking",
            Self::Failed => "Sarah failed",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CommunityRoomRole {
    Member,
    Moderator,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CommunityRoomContext {
    pub community_ref: String,
    pub channel_ref: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct VerifiedParticipantMapping {
    pub user_ref_digest: String,
    pub pubkey: String,
    pub participant_ref: String,
    pub membership_revision: String,
    pub room_ref: String,
    pub room_epoch: u64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CommunityFloorLease {
    pub schema: String,
    pub lease_ref: String,
    pub presence_lease_ref: String,
    pub community_ref: String,
    pub channel_ref: String,
    pub membership_revision: String,
    pub room_ref: String,
    pub room_epoch: u64,
    pub session_ref: String,
    pub generation: u32,
    pub issuance: u64,
    pub holder_user_ref_digest: String,
    pub holder_pubkey: String,
    pub holder_participant_ref: String,
    pub holder_safety_identifier: String,
    pub nonce_digest: String,
    pub issued_at_ms: u64,
    pub expires_at_ms: u64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum CommunityFloorState {
    Available {
        presence_lease_ref: String,
        issuance: u64,
    },
    Held(CommunityFloorLease),
    Stopped {
        presence_lease_ref: String,
        issuance: u64,
        reason: CommunityFloorStopReason,
    },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CommunityFloorStopReason {
    ModeratorStop,
    Timeout,
    MemberRemoved,
    SarahRemoved,
    MembershipChanged,
    PresenceExpired,
}

impl CommunityFloorState {
    pub fn label(&self, local_user_ref_digest: &str) -> String {
        match self {
            Self::Available { .. } => "Floor available".into(),
            Self::Held(lease) if lease.holder_user_ref_digest == local_user_ref_digest => {
                "You have the floor".into()
            }
            Self::Held(_) => "Floor held by another member".into(),
            Self::Stopped { .. } => "Floor stopped".into(),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CommunityRoomAuthority {
    pub schema: String,
    pub principal: String,
    pub capability_profile: String,
    pub processor_disclosure: String,
    pub cohort_policy: String,
    pub revision: u64,
    pub sarah_pubkey: String,
    pub presence_lease_ref: String,
    pub community_ref: String,
    pub channel_ref: String,
    pub membership_revision: String,
    pub e2ee_key_revision: String,
    pub room_ref: String,
    pub room_epoch: u64,
    pub sarah_participant_ref: String,
    pub dispatch_ref: String,
    pub session_ref: String,
    pub generation: u32,
    pub admission_digest: String,
    pub issued_at_ms: u64,
    pub expires_at_ms: u64,
    pub presence_active: bool,
    pub local_participant: VerifiedParticipantMapping,
    pub verified_participants: Vec<VerifiedParticipantMapping>,
    pub floor: CommunityFloorState,
}

impl CommunityRoomAuthority {
    pub fn validate(&self, context: &CommunityRoomContext, now_ms: u64) -> Result<()> {
        for value in [
            self.presence_lease_ref.as_str(),
            self.community_ref.as_str(),
            self.channel_ref.as_str(),
            self.room_ref.as_str(),
            self.dispatch_ref.as_str(),
            self.session_ref.as_str(),
            self.local_participant.participant_ref.as_str(),
        ] {
            validate_ref(value)?;
        }
        for digest in [
            self.membership_revision.as_str(),
            self.e2ee_key_revision.as_str(),
            self.admission_digest.as_str(),
            self.local_participant.user_ref_digest.as_str(),
            self.local_participant.membership_revision.as_str(),
        ] {
            validate_digest(digest)?;
        }
        validate_pubkey(&self.sarah_pubkey)?;
        validate_pubkey(&self.local_participant.pubkey)?;
        if self.schema != ROOM_AUTHORITY_SCHEMA
            || self.principal != SARAH_PRINCIPAL
            || self.capability_profile != COMMUNITY_CAPABILITY_PROFILE
            || self.processor_disclosure != PROCESSOR_DISCLOSURE
            || self.cohort_policy != COMMUNITY_COHORT_POLICY
            || self.revision == 0
            || self.room_epoch == 0
            || self.generation == 0
            || !self.presence_active
            || self.expires_at_ms <= now_ms
            || self.issued_at_ms >= self.expires_at_ms
            || self.community_ref != context.community_ref
            || self.channel_ref != context.channel_ref
            || self.sarah_participant_ref != SARAH_PRINCIPAL
            || self.membership_revision != self.local_participant.membership_revision
            || self.room_ref != self.local_participant.room_ref
            || self.room_epoch != self.local_participant.room_epoch
            || self.local_participant.participant_ref == self.sarah_participant_ref
        {
            bail!("community Sarah authority did not match the current room");
        }
        self.validate_participants()?;
        self.validate_floor(now_ms)
    }

    fn validate_participants(&self) -> Result<()> {
        if self.verified_participants.is_empty() || self.verified_participants.len() > 200 {
            bail!("community Sarah participant roster was empty or too large");
        }
        let mut user_refs = HashSet::default();
        let mut participant_refs = HashSet::default();
        for participant in &self.verified_participants {
            validate_digest(&participant.user_ref_digest)?;
            validate_digest(&participant.membership_revision)?;
            validate_pubkey(&participant.pubkey)?;
            validate_ref(&participant.participant_ref)?;
            if participant.membership_revision != self.membership_revision
                || participant.room_ref != self.room_ref
                || participant.room_epoch != self.room_epoch
                || participant.participant_ref == self.sarah_participant_ref
                || !user_refs.insert(participant.user_ref_digest.as_str())
                || !participant_refs.insert(participant.participant_ref.as_str())
            {
                bail!("community Sarah participant mapping was forged or stale");
            }
        }
        if !self
            .verified_participants
            .iter()
            .any(|participant| participant == &self.local_participant)
        {
            bail!("community Sarah local participant mapping was not verified");
        }
        Ok(())
    }

    fn validate_floor(&self, now_ms: u64) -> Result<()> {
        match &self.floor {
            CommunityFloorState::Available {
                presence_lease_ref, ..
            }
            | CommunityFloorState::Stopped {
                presence_lease_ref, ..
            } if presence_lease_ref != &self.presence_lease_ref => {
                bail!("community Sarah floor used another presence lease")
            }
            CommunityFloorState::Held(lease) => {
                for digest in [
                    lease.membership_revision.as_str(),
                    lease.holder_user_ref_digest.as_str(),
                    lease.holder_safety_identifier.as_str(),
                    lease.nonce_digest.as_str(),
                ] {
                    validate_digest(digest)?;
                }
                validate_pubkey(&lease.holder_pubkey)?;
                if lease.schema != ROOM_AUTHORITY_SCHEMA
                    || lease.presence_lease_ref != self.presence_lease_ref
                    || lease.community_ref != self.community_ref
                    || lease.channel_ref != self.channel_ref
                    || lease.membership_revision != self.membership_revision
                    || lease.room_ref != self.room_ref
                    || lease.room_epoch != self.room_epoch
                    || lease.session_ref != self.session_ref
                    || lease.generation != self.generation
                    || lease.issuance == 0
                    || lease.issued_at_ms >= lease.expires_at_ms
                    || lease.expires_at_ms <= now_ms
                    || !self.verified_participants.iter().any(|participant| {
                        participant.user_ref_digest == lease.holder_user_ref_digest
                            && participant.pubkey == lease.holder_pubkey
                            && participant.participant_ref == lease.holder_participant_ref
                    })
                {
                    bail!("community Sarah floor lease was stale or mismatched");
                }
                validate_ref(&lease.lease_ref)?;
                validate_ref(&lease.holder_participant_ref)?;
            }
            _ => {}
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum CommunitySarahIntent {
    Join,
    Leave,
    SetMuted(bool),
    Summon,
    Remove,
    AcquireFloor { body: Value },
    TransferFloor { body: Value },
    ModeratorStop { body: Value },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CommunitySarahControl {
    Join,
    Leave,
    Mute,
    Summon,
    Remove,
    Talk,
    ModeratorStop,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CommunityAuthorityLoss {
    MembershipStale,
    ParticipantRemoved,
    RoleLost,
    Revoked,
    ServerStopped,
}

#[derive(Clone, Debug)]
pub struct CommunitySarahRoom {
    pub context: Option<CommunityRoomContext>,
    pub lifecycle: CommunityCallLifecycle,
    pub sarah_state: CommunitySarahState,
    pub role: CommunityRoomRole,
    pub muted: bool,
    pub disclosure_acknowledged: bool,
    pub authority: Option<CommunityRoomAuthority>,
    pub failure: Option<String>,
    control_contract_available: bool,
    last_authority_revision: u64,
}

impl Default for CommunitySarahRoom {
    fn default() -> Self {
        Self {
            context: None,
            lifecycle: CommunityCallLifecycle::Unavailable,
            sarah_state: CommunitySarahState::Absent,
            role: CommunityRoomRole::Member,
            muted: false,
            disclosure_acknowledged: false,
            authority: None,
            failure: Some("Room voice service unavailable.".into()),
            control_contract_available: false,
            last_authority_revision: 0,
        }
    }
}

impl CommunitySarahRoom {
    pub fn configure(&mut self, context: CommunityRoomContext, control_contract_available: bool) {
        self.context = Some(context);
        self.control_contract_available = control_contract_available;
        self.lifecycle = if control_contract_available {
            CommunityCallLifecycle::ReadyToJoin
        } else {
            CommunityCallLifecycle::Unavailable
        };
        self.failure =
            (!control_contract_available).then(|| "Room voice service unavailable.".to_string());
    }

    pub fn apply_authority(
        &mut self,
        authority: CommunityRoomAuthority,
        role: CommunityRoomRole,
        sarah_state: CommunitySarahState,
        now_ms: u64,
    ) -> Result<()> {
        let Some(context) = self.context.clone() else {
            self.fail_closed("Room voice context is unavailable.");
            bail!("community room context is not configured");
        };
        if let Err(error) = authority.validate(&context, now_ms) {
            self.fail_closed("Room voice authority is invalid.");
            return Err(error);
        }
        if authority.revision <= self.last_authority_revision {
            self.fail_closed("Room voice state is stale.");
            bail!("community Sarah authority revision was stale or replayed");
        }
        self.last_authority_revision = authority.revision;
        self.role = role;
        self.lifecycle = CommunityCallLifecycle::Joined;
        self.sarah_state = sarah_state;
        self.failure = None;
        self.authority = Some(authority);
        Ok(())
    }

    pub fn fail_closed(&mut self, reason: impl Into<String>) {
        self.lifecycle = CommunityCallLifecycle::Failed;
        self.sarah_state = CommunitySarahState::Failed;
        self.muted = true;
        self.authority = None;
        self.failure = Some(reason.into());
    }

    pub fn expire(&mut self, now_ms: u64) {
        if self
            .authority
            .as_ref()
            .is_some_and(|authority| authority.expires_at_ms <= now_ms)
        {
            self.fail_closed("Room voice authority expired.");
        }
    }

    pub fn lose_authority(&mut self, loss: CommunityAuthorityLoss) {
        let reason = match loss {
            CommunityAuthorityLoss::MembershipStale => "Room membership changed.",
            CommunityAuthorityLoss::ParticipantRemoved => "You were removed from room voice.",
            CommunityAuthorityLoss::RoleLost => "Room voice role changed.",
            CommunityAuthorityLoss::Revoked => "Room voice authority was revoked.",
            CommunityAuthorityLoss::ServerStopped => "Room voice was stopped by the server.",
        };
        self.fail_closed(reason);
    }

    pub fn control_enabled(&self, control: CommunitySarahControl) -> bool {
        if !self.control_contract_available || self.context.is_none() {
            return false;
        }
        match control {
            CommunitySarahControl::Join => self.lifecycle == CommunityCallLifecycle::ReadyToJoin,
            CommunitySarahControl::Leave => self.lifecycle == CommunityCallLifecycle::Joined,
            CommunitySarahControl::Mute => self.lifecycle == CommunityCallLifecycle::Joined,
            CommunitySarahControl::Summon => {
                self.lifecycle == CommunityCallLifecycle::Joined
                    && self.disclosure_acknowledged
                    && matches!(
                        self.sarah_state,
                        CommunitySarahState::Absent | CommunitySarahState::Failed
                    )
            }
            CommunitySarahControl::Remove => {
                self.lifecycle == CommunityCallLifecycle::Joined
                    && self.role == CommunityRoomRole::Moderator
                    && !matches!(self.sarah_state, CommunitySarahState::Absent)
            }
            CommunitySarahControl::Talk => {
                self.lifecycle == CommunityCallLifecycle::Joined
                    && self.disclosure_acknowledged
                    && self.sarah_state == CommunitySarahState::Idle
                    && matches!(
                        self.authority.as_ref().map(|authority| &authority.floor),
                        Some(CommunityFloorState::Available { .. })
                    )
            }
            CommunitySarahControl::ModeratorStop => {
                self.lifecycle == CommunityCallLifecycle::Joined
                    && self.role == CommunityRoomRole::Moderator
                    && matches!(
                        self.authority.as_ref().map(|authority| &authority.floor),
                        Some(CommunityFloorState::Held(_))
                    )
            }
        }
    }

    pub fn begin(
        &mut self,
        control: CommunitySarahControl,
        nonce: Option<&str>,
    ) -> Result<CommunitySarahIntent> {
        if !self.control_enabled(control) {
            bail!("community Sarah control is not currently authorized");
        }
        let intent =
            match control {
                CommunitySarahControl::Join => {
                    self.lifecycle = CommunityCallLifecycle::Joining;
                    CommunitySarahIntent::Join
                }
                CommunitySarahControl::Leave => {
                    self.lifecycle = CommunityCallLifecycle::Leaving;
                    CommunitySarahIntent::Leave
                }
                CommunitySarahControl::Mute => {
                    self.muted = !self.muted;
                    CommunitySarahIntent::SetMuted(self.muted)
                }
                CommunitySarahControl::Summon => {
                    self.sarah_state = CommunitySarahState::Summoning;
                    CommunitySarahIntent::Summon
                }
                CommunitySarahControl::Remove => CommunitySarahIntent::Remove,
                CommunitySarahControl::Talk => {
                    let authority = self.authority.as_ref().ok_or_else(|| {
                        anyhow::anyhow!("community Sarah authority is unavailable")
                    })?;
                    let nonce = nonce.ok_or_else(|| anyhow::anyhow!("floor nonce is required"))?;
                    validate_nonce(nonce)?;
                    CommunitySarahIntent::AcquireFloor {
                        body: json!({
                            "action": "acquire",
                            "presenceLeaseRef": authority.presence_lease_ref,
                            "expectedRevision": authority.revision,
                            "nonce": nonce,
                            "requestedLeaseMs": FLOOR_LEASE_MS,
                        }),
                    }
                }
                CommunitySarahControl::ModeratorStop => {
                    let authority = self.authority.as_ref().ok_or_else(|| {
                        anyhow::anyhow!("community Sarah authority is unavailable")
                    })?;
                    let nonce = nonce.ok_or_else(|| anyhow::anyhow!("floor nonce is required"))?;
                    validate_nonce(nonce)?;
                    CommunitySarahIntent::ModeratorStop {
                        body: json!({
                            "action": "stop",
                            "presenceLeaseRef": authority.presence_lease_ref,
                            "expectedRevision": authority.revision,
                            "nonce": nonce,
                        }),
                    }
                }
            };
        Ok(intent)
    }

    pub fn transfer_floor(
        &self,
        target_user_ref_digest: &str,
        nonce: &str,
    ) -> Result<CommunitySarahIntent> {
        validate_digest(target_user_ref_digest)?;
        validate_nonce(nonce)?;
        if !self.control_contract_available
            || self.lifecycle != CommunityCallLifecycle::Joined
            || !self.disclosure_acknowledged
        {
            bail!("community Sarah floor transfer is not currently authorized");
        }
        let authority = self
            .authority
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("community Sarah authority is unavailable"))?;
        let CommunityFloorState::Held(lease) = &authority.floor else {
            bail!("community Sarah floor is not held");
        };
        if lease.holder_user_ref_digest != authority.local_participant.user_ref_digest
            || target_user_ref_digest == authority.local_participant.user_ref_digest
            || !authority
                .verified_participants
                .iter()
                .any(|participant| participant.user_ref_digest == target_user_ref_digest)
        {
            bail!("community Sarah floor transfer target is not verified");
        }
        Ok(CommunitySarahIntent::TransferFloor {
            body: json!({
                "action": "transfer",
                "presenceLeaseRef": authority.presence_lease_ref,
                "expectedRevision": authority.revision,
                "nonce": nonce,
                "targetUserRefDigest": target_user_ref_digest,
            }),
        })
    }

    pub fn floor_label(&self) -> String {
        let Some(authority) = &self.authority else {
            return "Floor unavailable".into();
        };
        authority
            .floor
            .label(&authority.local_participant.user_ref_digest)
    }

    pub fn has_private_authority(&self) -> bool {
        false
    }
}

fn validate_ref(value: &str) -> Result<()> {
    if value.is_empty() || value != value.trim() || value.len() > 256 {
        bail!("invalid community Sarah reference");
    }
    Ok(())
}

fn validate_digest(value: &str) -> Result<()> {
    if value.len() != 64
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        bail!("invalid community Sarah digest");
    }
    Ok(())
}

fn validate_pubkey(value: &str) -> Result<()> {
    validate_digest(value)
}

fn validate_nonce(value: &str) -> Result<()> {
    if !(32..=256).contains(&value.len())
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
    {
        bail!("invalid community Sarah floor nonce");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOW: u64 = 1_000;

    fn digest(byte: char) -> String {
        byte.to_string().repeat(64)
    }

    fn participant(byte: char, participant_ref: &str) -> VerifiedParticipantMapping {
        VerifiedParticipantMapping {
            user_ref_digest: digest(byte),
            pubkey: digest(byte),
            participant_ref: participant_ref.into(),
            membership_revision: digest('a'),
            room_ref: "room:testers:voice".into(),
            room_epoch: 7,
        }
    }

    fn authority() -> CommunityRoomAuthority {
        let local = participant('b', "participant:local");
        CommunityRoomAuthority {
            schema: ROOM_AUTHORITY_SCHEMA.into(),
            principal: SARAH_PRINCIPAL.into(),
            capability_profile: COMMUNITY_CAPABILITY_PROFILE.into(),
            processor_disclosure: PROCESSOR_DISCLOSURE.into(),
            cohort_policy: COMMUNITY_COHORT_POLICY.into(),
            revision: 12,
            sarah_pubkey: digest('c'),
            presence_lease_ref: "presence:testers:7".into(),
            community_ref: "community:testers".into(),
            channel_ref: "channel:agent-chat".into(),
            membership_revision: digest('a'),
            e2ee_key_revision: digest('d'),
            room_ref: "room:testers:voice".into(),
            room_epoch: 7,
            sarah_participant_ref: SARAH_PRINCIPAL.into(),
            dispatch_ref: "dispatch:sarah".into(),
            session_ref: "session:sarah".into(),
            generation: 1,
            admission_digest: digest('e'),
            issued_at_ms: 900,
            expires_at_ms: 20_000,
            presence_active: true,
            local_participant: local.clone(),
            verified_participants: vec![local],
            floor: CommunityFloorState::Available {
                presence_lease_ref: "presence:testers:7".into(),
                issuance: 3,
            },
        }
    }

    fn room() -> CommunitySarahRoom {
        let mut room = CommunitySarahRoom::default();
        room.configure(
            CommunityRoomContext {
                community_ref: "community:testers".into(),
                channel_ref: "channel:agent-chat".into(),
            },
            true,
        );
        room.disclosure_acknowledged = true;
        room
    }

    #[test]
    fn canonical_floor_request_uses_the_verified_presence_and_revision() {
        let mut room = room();
        room.apply_authority(
            authority(),
            CommunityRoomRole::Member,
            CommunitySarahState::Idle,
            NOW,
        )
        .expect("apply authority");
        let nonce = "n".repeat(32);
        assert_eq!(
            room.begin(CommunitySarahControl::Talk, Some(&nonce))
                .expect("acquire floor"),
            CommunitySarahIntent::AcquireFloor {
                body: json!({
                    "action": "acquire",
                    "presenceLeaseRef": "presence:testers:7",
                    "expectedRevision": 12,
                    "nonce": nonce,
                    "requestedLeaseMs": 30_000,
                })
            }
        );
        assert!(!room.has_private_authority());
    }

    #[test]
    fn canonical_floor_transfer_requires_a_verified_target_and_local_lease() {
        let local = participant('b', "participant:local");
        let target = participant('f', "participant:target");
        let mut authority = authority();
        authority.verified_participants.push(target.clone());
        authority.floor = CommunityFloorState::Held(CommunityFloorLease {
            schema: ROOM_AUTHORITY_SCHEMA.into(),
            lease_ref: "lease:local".into(),
            presence_lease_ref: authority.presence_lease_ref.clone(),
            community_ref: authority.community_ref.clone(),
            channel_ref: authority.channel_ref.clone(),
            membership_revision: authority.membership_revision.clone(),
            room_ref: authority.room_ref.clone(),
            room_epoch: authority.room_epoch,
            session_ref: authority.session_ref.clone(),
            generation: authority.generation,
            issuance: 4,
            holder_user_ref_digest: local.user_ref_digest,
            holder_pubkey: local.pubkey,
            holder_participant_ref: local.participant_ref,
            holder_safety_identifier: digest('b'),
            nonce_digest: digest('b'),
            issued_at_ms: 950,
            expires_at_ms: 10_000,
        });
        let mut room = room();
        room.apply_authority(
            authority,
            CommunityRoomRole::Member,
            CommunitySarahState::Listening,
            NOW,
        )
        .expect("apply held floor");
        let nonce = "n".repeat(32);
        assert_eq!(
            room.transfer_floor(&target.user_ref_digest, &nonce)
                .expect("transfer floor"),
            CommunitySarahIntent::TransferFloor {
                body: json!({
                    "action": "transfer",
                    "presenceLeaseRef": "presence:testers:7",
                    "expectedRevision": 12,
                    "nonce": nonce,
                    "targetUserRefDigest": target.user_ref_digest,
                })
            }
        );
        assert!(room.transfer_floor(&digest('9'), &"n".repeat(32)).is_err());
    }

    #[test]
    fn stale_replay_and_unverified_floor_holder_fail_closed() {
        let mut replay_room = room();
        replay_room
            .apply_authority(
                authority(),
                CommunityRoomRole::Member,
                CommunitySarahState::Idle,
                NOW,
            )
            .expect("apply authority");
        assert!(
            replay_room
                .apply_authority(
                    authority(),
                    CommunityRoomRole::Member,
                    CommunitySarahState::Idle,
                    NOW,
                )
                .is_err()
        );
        assert_eq!(replay_room.lifecycle, CommunityCallLifecycle::Failed);
        assert!(replay_room.authority.is_none());

        let mut forged = authority();
        forged.floor = CommunityFloorState::Held(CommunityFloorLease {
            schema: ROOM_AUTHORITY_SCHEMA.into(),
            lease_ref: "lease:forged".into(),
            presence_lease_ref: forged.presence_lease_ref.clone(),
            community_ref: forged.community_ref.clone(),
            channel_ref: forged.channel_ref.clone(),
            membership_revision: forged.membership_revision.clone(),
            room_ref: forged.room_ref.clone(),
            room_epoch: forged.room_epoch,
            session_ref: forged.session_ref.clone(),
            generation: forged.generation,
            issuance: 4,
            holder_user_ref_digest: digest('f'),
            holder_pubkey: digest('f'),
            holder_participant_ref: "participant:other-room".into(),
            holder_safety_identifier: digest('f'),
            nonce_digest: digest('f'),
            issued_at_ms: 950,
            expires_at_ms: 10_000,
        });
        let mut forged_room = room();
        assert!(
            forged_room
                .apply_authority(
                    forged,
                    CommunityRoomRole::Member,
                    CommunitySarahState::Idle,
                    NOW,
                )
                .is_err()
        );
        assert_eq!(forged_room.lifecycle, CommunityCallLifecycle::Failed);
    }

    #[test]
    fn removal_role_loss_expiry_and_server_stop_revoke_every_control() {
        for loss in [
            CommunityAuthorityLoss::MembershipStale,
            CommunityAuthorityLoss::ParticipantRemoved,
            CommunityAuthorityLoss::RoleLost,
            CommunityAuthorityLoss::Revoked,
            CommunityAuthorityLoss::ServerStopped,
        ] {
            let mut room = room();
            room.apply_authority(
                authority(),
                CommunityRoomRole::Moderator,
                CommunitySarahState::Idle,
                NOW,
            )
            .expect("apply authority");
            room.lose_authority(loss);
            assert!(room.muted);
            assert!(room.authority.is_none());
            for control in [
                CommunitySarahControl::Join,
                CommunitySarahControl::Leave,
                CommunitySarahControl::Mute,
                CommunitySarahControl::Summon,
                CommunitySarahControl::Remove,
                CommunitySarahControl::Talk,
                CommunitySarahControl::ModeratorStop,
            ] {
                assert!(!room.control_enabled(control));
            }
        }

        let mut room = room();
        room.apply_authority(
            authority(),
            CommunityRoomRole::Member,
            CommunitySarahState::Idle,
            NOW,
        )
        .expect("apply authority");
        room.expire(20_000);
        assert_eq!(room.lifecycle, CommunityCallLifecycle::Failed);
    }
}
