//! A message in the community room, from the authorization to the signature.
//!
//! The path is one way and each step consumes the last:
//!
//! ```text
//! ThreadAudience --may_post--> AuthorizedMessage --into_unsigned--> UnsignedRecord
//!                                                                        |
//!                                              somebody's own signer -----+
//!                                                                        v
//!                                                                  SignedRecord --> Outbox
//! ```
//!
//! That shape is the whole of omega#108's "authorization and audience checks
//! happen **before** an effect, not after". [`AuthorizedMessage::prepare`] is
//! the only constructor of an [`AuthorizedMessage`], and it calls
//! [`omega_audience::may_publish`]; [`UnsignedRecord`] can only come from one;
//! [`SignedRecord`] can only come from an [`UnsignedRecord`]. There is no value
//! in this module that a caller can build by skipping a step, so the refusal
//! cannot be forgotten at a call site — it is the type that is missing.
//!
//! The signature is never produced here. Omega composes the bytes and accepts a
//! signature over exactly those bytes; the key stays wherever its owner keeps
//! it. omega#108 deliverable 4 is "a person's identity is theirs, not Omega's",
//! and a crate that could sign would be a crate that had to hold one.

use std::fmt;

use nostr::{Event, EventId, Kind, PublicKey, Tag, Timestamp, UnsignedEvent};
use omega_audience::{Audience, PublishRefused, ThreadAudience, may_publish};

use crate::{ForgeMembership, ForgeRepository, MembershipRefused, RepositoryCoordinate};

/// NIP-34's repository announcement kind, which a room's records are rooted on.
pub const REPOSITORY_ANNOUNCEMENT_KIND: u16 = 30617;

/// The kind a message in the room is published as: NIP-22's comment.
///
/// A standard kind rather than an Omega one, for the reason the parity
/// recommendation gives — "prefer a standard NIP when it expresses the required
/// behavior" — and for a second, harder reason: the Forge relay admits
/// `1111` already, alongside the NIP-34 patch, issue and status kinds. A custom
/// kind would need an admission change on the relay before the first message
/// could be sent, and would be invisible to every other client that can already
/// read a comment on this repository.
pub const MESSAGE_KIND: u16 = 1111;

/// The tag naming the root of a NIP-22 comment thread.
const ROOT_ADDRESS_TAG: &str = "A";
const ROOT_KIND_TAG: &str = "K";
const ROOT_AUTHOR_TAG: &str = "P";
const PARENT_ADDRESS_TAG: &str = "a";
const PARENT_KIND_TAG: &str = "k";
const PARENT_AUTHOR_TAG: &str = "p";

/// Why a message may not be sent.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PostRefused {
    /// The thread's own audience refused, before membership was considered.
    ///
    /// [`omega_audience::PublishRefused::ThreadIsLocal`] is omega#108's second
    /// falsifier, and it is this variant. The rule is not restated here; it is
    /// carried.
    Audience(PublishRefused),
    /// The thread belongs to a shared audience, and not to this repository's.
    ///
    /// Refused rather than redirected. A message composed for one room and
    /// delivered to another is the disclosure defect `omega_audience` exists to
    /// prevent, arriving one layer down.
    AnotherRoom {
        /// The audience the thread is recorded in.
        thread: String,
        /// The audience this repository backs.
        repository: String,
    },
    /// The Forge does not admit this person here, or admits them read-only.
    Membership(MembershipRefused),
}

impl fmt::Display for PostRefused {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Audience(refusal) => write!(formatter, "{refusal}"),
            Self::AnotherRoom { thread, repository } => write!(
                formatter,
                "this thread belongs to `{thread}`, and this room is `{repository}`."
            ),
            Self::Membership(refusal) => write!(formatter, "{refusal}"),
        }
    }
}

/// May this person send a message from this thread into this room?
///
/// Three questions in the order they have to be asked. The audience comes
/// first because it is the fact recorded on the thread and the one a person is
/// looking at; membership comes last because it is the one that can change
/// under them. Answering membership first would refuse a local thread with
/// "you are not a member", which is both wrong and an invitation to fix it by
/// joining.
///
/// # Errors
///
/// [`PostRefused`], each variant of which is a sentence a person can read.
pub fn may_post<'a>(
    repository: &ForgeRepository,
    membership: &ForgeMembership,
    audience: &'a ThreadAudience,
) -> Result<&'a Audience, PostRefused> {
    let shared = may_publish(audience).map_err(PostRefused::Audience)?;
    let repository_key = repository.audience_key();
    if shared.id().as_key() != repository_key {
        return Err(PostRefused::AnotherRoom {
            thread: shared.id().as_key().to_string(),
            repository: repository_key,
        });
    }
    membership
        .admits_writing(repository)
        .map_err(PostRefused::Membership)?;
    Ok(shared)
}

/// A message that has passed every check and has not been written down yet.
///
/// Holds the repository rather than a copy of its coordinate so that the
/// binding on the wire and the audience that authorized it cannot drift apart.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AuthorizedMessage {
    coordinate: RepositoryCoordinate,
    relay: String,
    author: PublicKey,
    text: String,
}

impl AuthorizedMessage {
    /// Authorizes a message, or refuses it.
    ///
    /// The only way to make one.
    ///
    /// # Errors
    ///
    /// [`PostRefused`], from [`may_post`].
    pub fn prepare(
        repository: &ForgeRepository,
        membership: &ForgeMembership,
        audience: &ThreadAudience,
        author: PublicKey,
        text: impl Into<String>,
    ) -> Result<Self, PostRefused> {
        may_post(repository, membership, audience)?;
        Ok(Self {
            coordinate: repository.coordinate().clone(),
            relay: repository.relay().to_string(),
            author,
            text: text.into(),
        })
    }

    /// The bytes to sign.
    ///
    /// `created_at` is a parameter because a rule that reads a clock can only
    /// be tested on a machine that happens to be at the right time.
    #[must_use]
    pub fn into_unsigned(self, created_at: u64) -> UnsignedRecord {
        let coordinate = self.coordinate.to_string();
        let root_kind = REPOSITORY_ANNOUNCEMENT_KIND.to_string();
        let maintainer = self.coordinate.maintainer().to_string();

        // Built with `Tag::custom` rather than `Tag::parse`, because `parse`
        // standardises what it recognises and a standardised tag is re-rendered
        // from the parsed value on the way out. `binding_of` reads these back
        // as strings, and a tag that survives the round trip only because both
        // ends agree on a normalisation is a binding that can be lost to a
        // dependency bump.
        let tags = vec![
            Tag::custom(
                nostr::TagKind::custom(ROOT_ADDRESS_TAG),
                [coordinate.clone(), self.relay.clone()],
            ),
            Tag::custom(nostr::TagKind::custom(ROOT_KIND_TAG), [root_kind.clone()]),
            Tag::custom(
                nostr::TagKind::custom(ROOT_AUTHOR_TAG),
                [maintainer.clone()],
            ),
            Tag::custom(
                nostr::TagKind::custom(PARENT_ADDRESS_TAG),
                [coordinate, self.relay],
            ),
            Tag::custom(nostr::TagKind::custom(PARENT_KIND_TAG), [root_kind]),
            Tag::custom(nostr::TagKind::custom(PARENT_AUTHOR_TAG), [maintainer]),
        ];

        let mut event = UnsignedEvent::new(
            self.author,
            Timestamp::from_secs(created_at),
            Kind::from_u16(MESSAGE_KIND),
            tags,
            self.text,
        );
        event.ensure_id();

        UnsignedRecord {
            coordinate: self.coordinate,
            event,
        }
    }
}

/// The exact bytes a signature has to cover.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UnsignedRecord {
    coordinate: RepositoryCoordinate,
    event: UnsignedEvent,
}

impl UnsignedRecord {
    /// The event as a signer takes it.
    #[must_use]
    pub fn event(&self) -> &UnsignedEvent {
        &self.event
    }

    /// The identity these bytes hash to, which is also the idempotency key a
    /// retry reuses.
    #[must_use]
    pub fn id(&mut self) -> EventId {
        self.event.id()
    }

    /// The room this record is bound to.
    #[must_use]
    pub fn coordinate(&self) -> &RepositoryCoordinate {
        &self.coordinate
    }

    /// Accepts a signature over exactly these bytes.
    ///
    /// The only constructor of a [`SignedRecord`], and it re-checks everything
    /// rather than trusting the caller, because between [`Self::event`] and
    /// here the bytes have been outside this process. In particular it checks
    /// that the returned event *is* this record: a signer that returned a
    /// valid, correctly signed event for different content would otherwise
    /// publish something nobody authorized, and every individual check below
    /// would pass.
    ///
    /// # Errors
    ///
    /// [`RecordRefused`]. Nothing here logs and continues: an event that fails
    /// any of these is not published.
    pub fn accept_signature(mut self, signed: Event) -> Result<SignedRecord, RecordRefused> {
        let expected = self.event.id();
        if signed.id != expected {
            return Err(RecordRefused::NotTheAuthorizedBytes {
                authorized: expected.to_hex(),
                signed: signed.id.to_hex(),
            });
        }
        if signed.pubkey != self.event.pubkey {
            return Err(RecordRefused::AnotherAuthor {
                authorized: self.event.pubkey.to_hex(),
                signed: signed.pubkey.to_hex(),
            });
        }
        verify(&self.coordinate, &signed)?;
        Ok(SignedRecord {
            coordinate: self.coordinate,
            event: signed,
        })
    }
}

/// Why a signed event was refused.
///
/// Every variant is a refusal to publish or to display, never a warning. "The
/// signature did not check out, so it went out unsigned" is not a state this
/// module can be in, because there is no constructor that produces one.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RecordRefused {
    /// The signed event is not the event that was authorized.
    NotTheAuthorizedBytes {
        /// The identity of the authorized bytes.
        authorized: String,
        /// The identity of what came back.
        signed: String,
    },
    /// The signature is by a different key from the one that composed it.
    AnotherAuthor {
        /// The key the record was composed for.
        authorized: String,
        /// The key that signed.
        signed: String,
    },
    /// The event identity does not match its own content.
    ContentDoesNotMatchItsIdentity,
    /// The signature does not verify against the author's key.
    SignatureDoesNotVerify,
    /// The event is not a room message.
    NotAMessage {
        /// The kind it carries.
        kind: u16,
    },
    /// The event carries no room binding at all.
    ///
    /// omega#108's first falsifier: "remove the workspace binding from an
    /// outbound event: it must fail to publish rather than defaulting to some
    /// workspace". There is no default here to fall back to.
    BindingMissing,
    /// The event carries a binding that is not a repository coordinate.
    BindingMalformed(String),
    /// The event is bound to a different repository.
    BindingNamesAnotherRepository {
        /// The room it was offered to.
        room: String,
        /// The room it names.
        binding: String,
    },
}

impl fmt::Display for RecordRefused {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotTheAuthorizedBytes { authorized, signed } => write!(
                formatter,
                "the signer returned a different event: authorized `{authorized}`, signed `{signed}`."
            ),
            Self::AnotherAuthor { authorized, signed } => write!(
                formatter,
                "this message was composed for `{authorized}` and signed by `{signed}`."
            ),
            Self::ContentDoesNotMatchItsIdentity => {
                formatter.write_str("this event's content does not match its identity.")
            }
            Self::SignatureDoesNotVerify => {
                formatter.write_str("this event's signature does not verify against its author.")
            }
            Self::NotAMessage { kind } => {
                write!(formatter, "kind {kind} is not a message in this room.")
            }
            Self::BindingMissing => formatter.write_str(
                "this event names no room, and there is no room to assume. It will not be published.",
            ),
            Self::BindingMalformed(value) => {
                write!(formatter, "`{value}` is not a repository coordinate.")
            }
            Self::BindingNamesAnotherRepository { room, binding } => write!(
                formatter,
                "this event is bound to `{binding}`, and this room is `{room}`."
            ),
        }
    }
}

/// The room an event says it belongs to.
///
/// # Errors
///
/// [`RecordRefused::BindingMissing`] when there is no root address tag, and
/// [`RecordRefused::BindingMalformed`] when there is one that is not a
/// coordinate. There is deliberately no third answer: a caller cannot receive
/// "probably this room".
pub fn binding_of(event: &Event) -> Result<RepositoryCoordinate, RecordRefused> {
    let value = event
        .tags
        .iter()
        .filter_map(|tag| {
            let fields = tag.as_slice();
            match (fields.first(), fields.get(1)) {
                (Some(name), Some(value)) if name == ROOT_ADDRESS_TAG => Some(value.clone()),
                _ => None,
            }
        })
        .next()
        .ok_or(RecordRefused::BindingMissing)?;

    RepositoryCoordinate::parse(&value).map_err(|_| RecordRefused::BindingMalformed(value))
}

fn verify(coordinate: &RepositoryCoordinate, event: &Event) -> Result<(), RecordRefused> {
    if event.kind != Kind::from_u16(MESSAGE_KIND) {
        return Err(RecordRefused::NotAMessage {
            kind: event.kind.as_u16(),
        });
    }
    let binding = binding_of(event)?;
    if &binding != coordinate {
        return Err(RecordRefused::BindingNamesAnotherRepository {
            room: coordinate.to_string(),
            binding: binding.to_string(),
        });
    }
    if !event.verify_id() {
        return Err(RecordRefused::ContentDoesNotMatchItsIdentity);
    }
    if !event.verify_signature() {
        return Err(RecordRefused::SignatureDoesNotVerify);
    }
    Ok(())
}

/// A message that is signed, bound to this room, and ready to be sent.
///
/// The value the outbox takes. It cannot be constructed from a bare event: it
/// comes from an [`UnsignedRecord`], which comes from an
/// [`AuthorizedMessage`], which comes from [`may_post`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SignedRecord {
    coordinate: RepositoryCoordinate,
    event: Event,
}

impl SignedRecord {
    /// Reads a record somebody else published into this room.
    ///
    /// The inbound half, and the reason it returns a [`SignedRecord`] rather
    /// than a distinct type is that "signed, bound to this room, verified" is
    /// the same fact whichever direction it travelled. What it does *not* share
    /// with the outbound path is authorization: a record read from a relay was
    /// authorized by its own author, on their machine, and Omega's job is to
    /// check the signature rather than to relitigate their membership.
    ///
    /// # Errors
    ///
    /// [`RecordRefused`]. An unverifiable event is not displayed.
    pub fn verify_received(
        repository: &ForgeRepository,
        event: Event,
    ) -> Result<Self, RecordRefused> {
        verify(repository.coordinate(), &event)?;
        Ok(Self {
            coordinate: repository.coordinate().clone(),
            event,
        })
    }

    /// The key that signed this, which is the person it is attributable to.
    #[must_use]
    pub fn author(&self) -> PublicKey {
        self.event.pubkey
    }

    /// The identity of this record, and the idempotency key for any retry.
    #[must_use]
    pub fn id(&self) -> EventId {
        self.event.id
    }

    /// When its author says they wrote it.
    #[must_use]
    pub fn created_at(&self) -> u64 {
        self.event.created_at.as_secs()
    }

    /// What it says.
    #[must_use]
    pub fn text(&self) -> &str {
        &self.event.content
    }

    /// The room it is bound to.
    #[must_use]
    pub fn coordinate(&self) -> &RepositoryCoordinate {
        &self.coordinate
    }

    /// The event as a relay takes it.
    #[must_use]
    pub fn event(&self) -> &Event {
        &self.event
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::tests::{membership, omega_repository};
    use crate::{MembershipState, RoleRef};
    use nostr::{Keys, SecretKey};
    use omega_audience::{AudienceId, AudienceRoster};

    /// A key that exists only in this file. Nothing signs with a person's key
    /// here, and nothing in the shipped crate signs at all.
    fn test_keys() -> Keys {
        let secret = SecretKey::from_slice(&[7u8; 32]).expect("a valid test secret");
        Keys::new(secret)
    }

    fn roster_with_room() -> AudienceRoster {
        crate::roster(
            &omega_repository(),
            Some(&membership(MembershipState::Active, &[RoleRef::Member])),
        )
    }

    pub(crate) fn signed_message_for_tests(text: &str) -> SignedRecord {
        signed_message(text)
    }

    fn signed_message(text: &str) -> SignedRecord {
        let repository = omega_repository();
        let keys = test_keys();
        let audience = roster_with_room().describe(&repository.audience_id().expect("an identity"));

        let unsigned = AuthorizedMessage::prepare(
            &repository,
            &membership(MembershipState::Active, &[RoleRef::Member]),
            &audience,
            keys.public_key(),
            text,
        )
        .expect("a member of the room may post")
        .into_unsigned(1_800_000_000);

        let event = sign(&keys, unsigned.event().clone());
        unsigned
            .accept_signature(event)
            .expect("a signature over exactly those bytes")
    }

    fn sign(keys: &Keys, mut unsigned: UnsignedEvent) -> Event {
        use nostr::secp256k1::Message;

        let id = unsigned.id();
        let signature = keys.sign_schnorr(&Message::from_digest(id.to_bytes()));
        Event::new(
            id,
            unsigned.pubkey,
            unsigned.created_at,
            unsigned.kind,
            unsigned.tags.clone().to_vec(),
            unsigned.content.clone(),
            signature,
        )
    }

    /// omega#108's second falsifier, carried rather than restated.
    #[test]
    fn a_local_thread_is_refused_before_anything_is_composed() {
        let repository = omega_repository();
        let local = roster_with_room().describe(&AudienceId::local());

        assert_eq!(
            AuthorizedMessage::prepare(
                &repository,
                &membership(MembershipState::Active, &[RoleRef::Admin]),
                &local,
                test_keys().public_key(),
                "this must not become bytes",
            ),
            Err(PostRefused::Audience(PublishRefused::ThreadIsLocal)),
            "the refusal is `omega_audience`'s, not a second copy of it"
        );
    }

    #[test]
    fn a_thread_in_another_room_is_not_redirected_into_this_one() {
        let repository = omega_repository();
        let elsewhere = AudienceRoster::new([omega_audience::Audience::joined(
            "forge:tenant.openagents/vortex",
            "Vortex",
        )
        .expect("a joined audience")]);
        let audience = elsewhere
            .describe(&AudienceId::joined("forge:tenant.openagents/vortex").expect("an identity"));

        assert_eq!(
            AuthorizedMessage::prepare(
                &repository,
                &membership(MembershipState::Active, &[RoleRef::Admin]),
                &audience,
                test_keys().public_key(),
                "meant for another room",
            ),
            Err(PostRefused::AnotherRoom {
                thread: "forge:tenant.openagents/vortex".to_string(),
                repository: "forge:tenant.openagents/omega".to_string(),
            })
        );
    }

    #[test]
    fn a_revoked_member_is_refused_at_the_audience_before_the_membership() {
        let repository = omega_repository();
        let revoked = membership(MembershipState::Tombstoned, &[RoleRef::Member]);
        // The roster is rebuilt from the revoked membership, so the room is no
        // longer in it and the thread's recorded audience no longer resolves.
        let audience = crate::roster(&repository, Some(&revoked))
            .describe(&repository.audience_id().expect("an identity"));

        assert_eq!(
            AuthorizedMessage::prepare(
                &repository,
                &revoked,
                &audience,
                test_keys().public_key(),
                "after being removed",
            ),
            Err(PostRefused::Audience(PublishRefused::AudienceUnresolved(
                repository.audience_id().expect("an identity")
            ))),
            "a person removed from the room is told they cannot see it, which \
             is the true thing, rather than being told their message failed"
        );
    }

    #[test]
    fn a_viewer_is_refused_by_the_forges_own_role() {
        let repository = omega_repository();
        let viewer = membership(MembershipState::Active, &[RoleRef::Viewer]);
        let audience = crate::roster(&repository, Some(&viewer))
            .describe(&repository.audience_id().expect("an identity"));

        assert_eq!(
            AuthorizedMessage::prepare(
                &repository,
                &viewer,
                &audience,
                test_keys().public_key(),
                "a viewer's message",
            ),
            Err(PostRefused::Membership(MembershipRefused::ReadOnly {
                roles: vec!["forge:viewer".to_string()],
            }))
        );
    }

    /// omega#108 deliverable 4, executed.
    #[test]
    fn a_sent_message_carries_its_authors_signature_and_this_rooms_binding() {
        let record = signed_message("the first message in the room");
        let repository = omega_repository();

        assert_eq!(record.author(), test_keys().public_key());
        assert_eq!(record.coordinate(), repository.coordinate());
        assert_eq!(record.text(), "the first message in the room");
        assert_eq!(record.event().kind.as_u16(), MESSAGE_KIND);
        assert_eq!(
            binding_of(record.event()).expect("a bound record"),
            *repository.coordinate()
        );
        assert!(
            record.event().verify().is_ok(),
            "the record a relay receives must verify on its own terms"
        );
    }

    /// omega#108's first falsifier, executed: strip the binding and watch it
    /// refuse rather than default.
    #[test]
    fn an_event_with_its_room_binding_removed_is_refused_and_defaults_to_nothing() {
        let repository = omega_repository();
        let keys = test_keys();
        let signed = signed_message("bound, for now");

        let stripped: Vec<Tag> = signed
            .event()
            .tags
            .iter()
            .filter(|tag| tag.as_slice().first().map(String::as_str) != Some(ROOT_ADDRESS_TAG))
            .cloned()
            .collect();
        let unbound = sign(
            &keys,
            UnsignedEvent::new(
                keys.public_key(),
                signed.event().created_at,
                signed.event().kind,
                stripped,
                signed.text(),
            ),
        );

        assert_eq!(binding_of(&unbound), Err(RecordRefused::BindingMissing));
        assert_eq!(
            SignedRecord::verify_received(&repository, unbound),
            Err(RecordRefused::BindingMissing),
            "an event that names no room must fail rather than be delivered to \
             whichever room happens to be selected"
        );
    }

    #[test]
    fn an_event_bound_to_another_room_is_refused_by_this_one() {
        let repository = omega_repository();
        let keys = test_keys();
        let signed = signed_message("for the neighbours");

        let retagged: Vec<Tag> = signed
            .event()
            .tags
            .iter()
            .map(|tag| {
                if tag.as_slice().first().map(String::as_str) == Some(ROOT_ADDRESS_TAG) {
                    Tag::custom(
                        nostr::TagKind::custom(ROOT_ADDRESS_TAG),
                        [
                            "30617:7649603503856e5148d571eac2766b288a8ff1e9e35d380337a1d2b0015b4f92:vortex"
                                .to_string(),
                        ],
                    )
                } else {
                    tag.clone()
                }
            })
            .collect();
        let elsewhere = sign(
            &keys,
            UnsignedEvent::new(
                keys.public_key(),
                signed.event().created_at,
                signed.event().kind,
                retagged,
                signed.text(),
            ),
        );

        assert!(matches!(
            SignedRecord::verify_received(&repository, elsewhere),
            Err(RecordRefused::BindingNamesAnotherRepository { .. })
        ));
    }

    /// omega#108's third falsifier: break the signature path, and watch the
    /// event be rejected rather than published unsigned.
    #[test]
    fn a_broken_signature_is_rejected_and_never_published_unsigned() {
        let repository = omega_repository();
        let good = signed_message("honestly signed");
        let impostor = Keys::new(SecretKey::from_slice(&[9u8; 32]).expect("a second test secret"));

        let forged = Event::new(
            good.event().id,
            good.event().pubkey,
            good.event().created_at,
            good.event().kind,
            good.event().tags.clone().to_vec(),
            good.text(),
            sign(&impostor, {
                let mut same_bytes = UnsignedEvent::new(
                    impostor.public_key(),
                    good.event().created_at,
                    good.event().kind,
                    good.event().tags.clone().to_vec(),
                    good.text(),
                );
                same_bytes.ensure_id();
                same_bytes
            })
            .sig,
        );

        assert_eq!(
            SignedRecord::verify_received(&repository, forged),
            Err(RecordRefused::SignatureDoesNotVerify),
            "a signature by another key over the same words is not this \
             author's message"
        );
    }

    #[test]
    fn an_event_whose_content_was_edited_after_signing_is_rejected() {
        let repository = omega_repository();
        let good = signed_message("what was actually said");

        let edited = Event::new(
            good.event().id,
            good.event().pubkey,
            good.event().created_at,
            good.event().kind,
            good.event().tags.clone().to_vec(),
            "what somebody wishes had been said",
            good.event().sig,
        );

        assert_eq!(
            SignedRecord::verify_received(&repository, edited),
            Err(RecordRefused::ContentDoesNotMatchItsIdentity)
        );
    }

    /// The gap between "authorized" and "signed" is the one place a signer can
    /// substitute something, and it is checked.
    #[test]
    fn a_signer_that_returns_a_different_event_does_not_get_it_published() {
        let repository = omega_repository();
        let keys = test_keys();
        let audience = roster_with_room().describe(&repository.audience_id().expect("an identity"));
        let authorized = AuthorizedMessage::prepare(
            &repository,
            &membership(MembershipState::Active, &[RoleRef::Member]),
            &audience,
            keys.public_key(),
            "what the person typed",
        )
        .expect("a member may post")
        .into_unsigned(1_800_000_000);

        let substituted = signed_message("what the signer sent instead");

        assert!(matches!(
            authorized.accept_signature(substituted.event().clone()),
            Err(RecordRefused::NotTheAuthorizedBytes { .. })
        ));
    }

    #[test]
    fn a_signature_by_another_key_over_the_authorized_bytes_is_refused() {
        let repository = omega_repository();
        let keys = test_keys();
        let impostor = Keys::new(SecretKey::from_slice(&[11u8; 32]).expect("a third test secret"));
        let audience = roster_with_room().describe(&repository.audience_id().expect("an identity"));
        let authorized = AuthorizedMessage::prepare(
            &repository,
            &membership(MembershipState::Active, &[RoleRef::Member]),
            &audience,
            keys.public_key(),
            "composed for one key",
        )
        .expect("a member may post")
        .into_unsigned(1_800_000_000);

        let mut theirs = UnsignedEvent::new(
            impostor.public_key(),
            authorized.event().created_at,
            authorized.event().kind,
            authorized.event().tags.clone().to_vec(),
            authorized.event().content.clone(),
        );
        theirs.ensure_id();
        let signed_by_them = sign(&impostor, theirs);

        assert!(
            matches!(
                authorized.accept_signature(signed_by_them),
                Err(RecordRefused::NotTheAuthorizedBytes { .. })
            ),
            "a different author is different bytes, and it is caught at the \
             first check rather than the author one"
        );
    }
}
