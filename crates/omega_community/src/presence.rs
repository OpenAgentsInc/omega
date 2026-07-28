//! Who Omega can see in a room, and how it knows. `OMEGA-DELTA-0113`, omega#108.
//!
//! omega#108 deliverable 3 asks that "seeing who is present" be a conversation
//! action. This is the answer that action gives, and the first thing to say
//! about it is what it is not.
//!
//! # It is not a member list
//!
//! The Forge's membership endpoint answers about *one* binding — the person
//! asking. There is no call that returns the roll of a repository, and Omega
//! does not have one to read. So a list of "members" here would be a list this
//! build invented.
//!
//! What Omega does have is signed records. Every one carries a key that
//! verified against its own bytes, so "this person wrote in this room" is a
//! fact rather than a claim, and it is the only fact available. [`RoomPresence`]
//! is therefore a list of **who has been seen writing**, with the count and the
//! most recent time, plus this profile's own binding — which is known because
//! the Forge issued it.
//!
//! [`RoomPresence::describe`] says both halves out loud. An answer that read
//! "3 people are here" would be a confident wrong answer to a question Omega
//! cannot answer, and the parity bar this issue inherits is explicit that a
//! surface must not imply an authority it does not have.
//!
//! # Nothing here reaches a relay
//!
//! The records arrive as values. On a machine with no transport the list is
//! this person alone, and that is a true statement about what Omega has seen
//! rather than a failure state.

use std::collections::BTreeMap;

use nostr::PublicKey;

use crate::{ForgeMembership, ForgeRepository, SignedRecord};

/// One key that has been seen writing in the room.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Participant {
    /// The key that signed, as hex. The person this is attributable to.
    pub author: String,
    /// Is this the person asking?
    pub is_you: bool,
    /// How many records from this key Omega has verified in this room.
    pub records: usize,
    /// The most recent `created_at` among them.
    ///
    /// The author's own clock, not this machine's. It is what the signature
    /// covers, so it is the only time that is attributable — and it is
    /// therefore a time a person could have written to say anything they liked.
    pub last_wrote_at: u64,
}

/// Who Omega has seen in one room.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RoomPresence {
    room: String,
    you: String,
    your_roles: Vec<String>,
    participants: Vec<Participant>,
}

impl RoomPresence {
    /// Builds the answer from what Omega has verified.
    ///
    /// The records are taken as values, and each one is already a
    /// [`SignedRecord`] — which cannot be constructed without a signature that
    /// verified against the author's key and a binding to this room. So no
    /// filtering or trusting happens here: the type is the filter.
    ///
    /// This profile appears whether or not it has written anything, because the
    /// Forge issued it a binding and that is a fact independent of the
    /// transcript.
    #[must_use]
    pub fn observed<'a>(
        repository: &ForgeRepository,
        membership: &ForgeMembership,
        you: PublicKey,
        records: impl IntoIterator<Item = &'a SignedRecord>,
    ) -> Self {
        let you = you.to_hex();
        let mut seen: BTreeMap<String, Participant> = BTreeMap::new();
        seen.insert(
            you.clone(),
            Participant {
                author: you.clone(),
                is_you: true,
                records: 0,
                last_wrote_at: 0,
            },
        );

        for record in records {
            if record.coordinate() != repository.coordinate() {
                continue;
            }
            let author = record.author().to_hex();
            let entry = seen.entry(author.clone()).or_insert(Participant {
                is_you: author == you,
                author,
                records: 0,
                last_wrote_at: 0,
            });
            entry.records = entry.records.saturating_add(1);
            entry.last_wrote_at = entry.last_wrote_at.max(record.created_at());
        }

        // This profile first, then whoever wrote most recently. The order is
        // not alphabetical because a key's hex is not a name, and sorting on it
        // would present an arbitrary order as though it meant something.
        let mut participants: Vec<Participant> = seen.into_values().collect();
        participants.sort_by(|left, right| {
            right
                .is_you
                .cmp(&left.is_you)
                .then(right.last_wrote_at.cmp(&left.last_wrote_at))
                .then(left.author.cmp(&right.author))
        });

        Self {
            room: repository.audience().map_or_else(
                |_| repository.repository_ref().to_string(),
                |audience| audience.name().to_string(),
            ),
            you,
            your_roles: membership
                .role_refs
                .iter()
                .cloned()
                .map(String::from)
                .collect(),
            participants,
        }
    }

    /// Everyone Omega has seen, this profile first.
    #[must_use]
    pub fn participants(&self) -> &[Participant] {
        &self.participants
    }

    /// This profile's own key, as hex.
    #[must_use]
    pub fn you(&self) -> &str {
        &self.you
    }

    /// The sentence the conversation action answers with.
    ///
    /// It names its own basis in the same breath as its content, because the
    /// question a person asked — "who is here" — is not the question this can
    /// answer, and the gap between the two is the whole risk.
    #[must_use]
    pub fn describe(&self) -> String {
        let mut lines = vec![format!(
            "In {}, Omega has verified records from {}.",
            self.room,
            match self.participants.len() {
                1 => "one key".to_string(),
                count => format!("{count} keys"),
            }
        )];

        for participant in &self.participants {
            let who = short(&participant.author);
            let mut line = if participant.is_you {
                format!("- {who} (you, {})", roles(&self.your_roles))
            } else {
                format!("- {who}")
            };
            line.push_str(&match participant.records {
                0 => ", nothing written here yet".to_string(),
                1 => format!(", 1 message, last at {}", participant.last_wrote_at),
                count => format!(", {count} messages, last at {}", participant.last_wrote_at),
            });
            lines.push(line);
        }

        lines.push(
            "This is who Omega has seen sign a record in this room, not the room's member list. \
             The Forge answers about one binding at a time, so there is no roll to read."
                .to_string(),
        );
        lines.join("\n")
    }
}

/// How the roles read in a sentence.
fn roles(granted: &[String]) -> String {
    if granted.is_empty() {
        "no role".to_string()
    } else {
        granted.join(", ")
    }
}

/// Enough of a key to tell two apart, and not so much that a line is unreadable.
///
/// Not a name. Omega has no profile record for a key here, and rendering one
/// would be the same invention this module exists to avoid.
fn short(author: &str) -> String {
    let head: String = author.chars().take(8).collect();
    format!("{head}…")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::record::tests::{another_signed_message_for_tests, signed_message_for_tests};
    use crate::tests::{membership, omega_repository};
    use crate::{MembershipState, RoleRef};
    use nostr::{Keys, SecretKey};

    fn you() -> PublicKey {
        Keys::new(SecretKey::from_slice(&[7u8; 32]).expect("a valid test secret")).public_key()
    }

    fn stranger() -> PublicKey {
        Keys::new(SecretKey::from_slice(&[9u8; 32]).expect("a valid test secret")).public_key()
    }

    /// A person who has joined and written nothing is still here.
    #[test]
    fn a_room_with_no_records_is_this_profile_alone_and_says_why() {
        let presence = RoomPresence::observed(
            &omega_repository(),
            &membership(MembershipState::Active, &[RoleRef::Member]),
            you(),
            [],
        );

        assert_eq!(presence.participants().len(), 1);
        assert!(presence.participants()[0].is_you);
        assert_eq!(presence.participants()[0].records, 0);

        let described = presence.describe();
        assert!(described.contains("one key"));
        assert!(described.contains("nothing written here yet"));
        assert!(described.contains("forge:member"));
        assert!(
            described.contains("not the room's member list"),
            "the answer names its own basis, because the question a person \
             asked is not the question this can answer: {described}"
        );
    }

    #[test]
    fn a_verified_record_puts_its_author_in_the_room() {
        let mine = signed_message_for_tests("mine");
        let theirs = another_signed_message_for_tests("theirs", 1_800_000_900);
        let presence = RoomPresence::observed(
            &omega_repository(),
            &membership(MembershipState::Active, &[RoleRef::Member]),
            you(),
            [&mine, &theirs],
        );

        assert_eq!(presence.participants().len(), 2);
        assert!(presence.participants()[0].is_you, "this profile is first");
        assert_eq!(presence.participants()[0].records, 1);

        let other = &presence.participants()[1];
        assert!(!other.is_you);
        assert_eq!(other.author, stranger().to_hex());
        assert_eq!(other.records, 1);
        assert_eq!(other.last_wrote_at, 1_800_000_900);
    }

    #[test]
    fn the_same_author_twice_is_one_participant_and_the_later_time() {
        let first = another_signed_message_for_tests("first", 1_800_000_100);
        let second = another_signed_message_for_tests("second", 1_800_000_200);
        let presence = RoomPresence::observed(
            &omega_repository(),
            &membership(MembershipState::Active, &[RoleRef::Viewer]),
            you(),
            [&second, &first],
        );

        assert_eq!(presence.participants().len(), 2);
        let other = &presence.participants()[1];
        assert_eq!(other.records, 2);
        assert_eq!(
            other.last_wrote_at, 1_800_000_200,
            "the order the records arrived in must not decide the answer"
        );
        assert!(presence.describe().contains("2 messages"));
        assert!(
            presence.describe().contains("forge:viewer"),
            "a viewer is present, and the line says what they may do"
        );
    }

    /// The room is named on every record, and a record for another room does
    /// not count towards this one even if a caller passes it in.
    #[test]
    fn a_record_bound_to_another_room_is_not_presence_here() {
        let elsewhere = crate::ForgeRepository::new(
            "tenant.openagents",
            "vortex",
            crate::RepositoryCoordinate::parse(
                "30617:7649603503856e5148d571eac2766b288a8ff1e9e35d380337a1d2b0015b4f92:vortex",
            )
            .expect("a well formed coordinate"),
            ["wss://relay.example"],
            "Vortex development",
        )
        .expect("a described room");

        let mine = signed_message_for_tests("mine");
        let presence = RoomPresence::observed(
            &elsewhere,
            &membership(MembershipState::Active, &[RoleRef::Member]),
            you(),
            [&mine],
        );

        assert_eq!(presence.participants().len(), 1);
        assert_eq!(
            presence.participants()[0].records,
            0,
            "a record bound to another room is not evidence about this one"
        );
    }

    #[test]
    fn a_key_is_shortened_and_never_given_a_name() {
        let presence = RoomPresence::observed(
            &omega_repository(),
            &membership(MembershipState::Active, &[RoleRef::Member]),
            you(),
            [],
        );
        let described = presence.describe();

        assert!(described.contains(&short(&you().to_hex())));
        assert!(
            !described.contains(&you().to_hex()),
            "a full key on a line nobody can read is not attribution"
        );
    }

    #[test]
    fn a_binding_with_no_role_reads_as_no_role() {
        let presence = RoomPresence::observed(
            &omega_repository(),
            &membership(MembershipState::Active, &[]),
            you(),
            [],
        );

        assert!(presence.describe().contains("no role"));
    }
}
