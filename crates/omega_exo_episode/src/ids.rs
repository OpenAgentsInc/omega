//! Exo's identifiers, kept apart from each other. `OMEGA-DELTA-0090`, omega#103.
//!
//! Exo's own type aliases put six different meanings on one underlying type:
//! `AgentId`, `ConversationId`, `SessionId`, `TurnId`, `EventId` and
//! `SnapshotId` are all `Uuid7`, and `SandboxId` is a bare `String`
//! (`crates/exoharness/src/types.rs:935-946` at [`crate::EXO_PROTOCOL_PIN`]).
//! Inside Exo that is fine — the request enum names each field. On Omega's side
//! of the wire the fields are positional in the reader's head, and
//! `start_sandbox` takes a sandbox id **and** a snapshot id next to each other.
//! Swapping those two is a plausible mistake that produces a request Exo will
//! reject with a message about a missing snapshot, which reads like the episode
//! problem this crate exists to explain rather than like a typo.
//!
//! So each meaning is its own type here, none of them convert into each other,
//! and every one of them is parsed rather than assumed.
//!
//! # Why parse at all
//!
//! Omega does not mint Exo ids. Every id this crate carries came out of Exo —
//! from a response, a log line, or a person reading one off a pane. That is
//! exactly the population that contains typos, truncations, and the empty
//! string. A shape check is cheap and it is the difference between a refusal
//! Omega can name and a request that reaches Exo carrying nonsense.

/// Why an Exo identifier was refused.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ExoIdError {
    /// The string was empty, or whitespace only.
    Empty,
    /// The string is not a UUID at all: wrong length, or wrong separators.
    NotAUuid,
    /// The string is a UUID and its version nibble is not 7.
    ///
    /// Every Exo id of this kind is a UUIDv7, which means it is also a
    /// timestamp and a sort key. `up_to_inclusive` compares ids directly
    /// (`events.retain(|event| event.id <= limit)`), so a v4 here would order
    /// against real history by luck.
    NotVersion7,
    /// The string carries a character an identifier cannot carry — a control
    /// character, a quote, or whitespace inside the value.
    Unprintable,
}

impl std::fmt::Display for ExoIdError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self {
            Self::Empty => "an Exo identifier cannot be empty",
            Self::NotAUuid => "that is not the shape of an Exo UUID identifier",
            Self::NotVersion7 => {
                "Exo identifiers are UUIDv7, which is what makes them orderable; this one is not"
            }
            Self::Unprintable => "that identifier carries a character an identifier cannot carry",
        })
    }
}

impl std::error::Error for ExoIdError {}

/// Read a UUIDv7 in Exo's rendering, lowercased.
fn parse_uuid7(value: &str) -> Result<String, ExoIdError> {
    let value = value.trim();
    if value.is_empty() {
        return Err(ExoIdError::Empty);
    }
    if value.len() != 36 {
        return Err(ExoIdError::NotAUuid);
    }
    let bytes = value.as_bytes();
    for (offset, byte) in bytes.iter().enumerate() {
        let expected_hyphen = matches!(offset, 8 | 13 | 18 | 23);
        if expected_hyphen {
            if *byte != b'-' {
                return Err(ExoIdError::NotAUuid);
            }
        } else if !byte.is_ascii_hexdigit() {
            return Err(ExoIdError::NotAUuid);
        }
    }
    // The version nibble is the first character of the third group.
    if !bytes[14].eq_ignore_ascii_case(&b'7') {
        return Err(ExoIdError::NotVersion7);
    }
    Ok(value.to_ascii_lowercase())
}

macro_rules! exo_uuid7_id {
    ($name:ident, $what:literal) => {
        #[doc = concat!("Exo's ", $what, ", a UUIDv7.")]
        #[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
        pub struct $name(String);

        impl $name {
            #[doc = concat!("Read an Exo ", $what, ".")]
            ///
            /// # Errors
            ///
            /// [`ExoIdError`] when the string is not a UUIDv7.
            pub fn parse(value: &str) -> Result<Self, ExoIdError> {
                parse_uuid7(value).map(Self)
            }

            /// The identifier, for putting on the wire.
            #[must_use]
            pub fn as_str(&self) -> &str {
                &self.0
            }
        }

        impl std::fmt::Display for $name {
            fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str(&self.0)
            }
        }
    };
}

exo_uuid7_id!(AgentId, "agent id");
exo_uuid7_id!(ConversationId, "conversation id");
exo_uuid7_id!(EventId, "event id");
exo_uuid7_id!(SnapshotId, "snapshot id");

/// Exo's sandbox id, which is **not** a UUID.
///
/// `SandboxId = String` upstream, and the shipped ids look like
/// `sandbox-019e5782-2a46-7970-a5bf-62900a2233e8` — a prefix and a UUID, by
/// convention rather than by type. Omega does not invent a stricter shape than
/// Exo has, because a stricter shape would refuse ids Exo really issues. What
/// it refuses is the set that could not be an identifier at all: empty, or
/// carrying whitespace, quotes, or control characters.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SandboxId(String);

impl SandboxId {
    /// Read an Exo sandbox id.
    ///
    /// # Errors
    ///
    /// [`ExoIdError`] for an empty or unprintable id.
    pub fn parse(value: &str) -> Result<Self, ExoIdError> {
        let value = value.trim();
        if value.is_empty() {
            return Err(ExoIdError::Empty);
        }
        if value
            .chars()
            .any(|character| character.is_control() || character.is_whitespace())
        {
            return Err(ExoIdError::Unprintable);
        }
        if value.contains(['"', '\'', '\\']) {
            return Err(ExoIdError::Unprintable);
        }
        Ok(Self(value.to_owned()))
    }

    /// The identifier, for putting on the wire.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for SandboxId {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const A_UUID7: &str = "019e5782-7c6b-72a2-b4fa-a81bf56eb37e";

    #[test]
    fn a_uuid7_is_read_and_lowercased() {
        let id = ConversationId::parse(A_UUID7).expect("a v7 uuid");
        assert_eq!(id.as_str(), A_UUID7);
        let upper = ConversationId::parse(&A_UUID7.to_ascii_uppercase()).expect("a v7 uuid");
        assert_eq!(upper, id, "one id written two ways is one id");
    }

    #[test]
    fn the_shapes_that_are_not_ids_are_refused() {
        assert_eq!(EventId::parse(""), Err(ExoIdError::Empty));
        assert_eq!(EventId::parse("   "), Err(ExoIdError::Empty));
        assert_eq!(EventId::parse("019e5782"), Err(ExoIdError::NotAUuid));
        assert_eq!(
            EventId::parse("019e5782-7c6b-72a2-b4fa-a81bf56eb37"),
            Err(ExoIdError::NotAUuid),
            "a truncated id is refused rather than padded"
        );
        assert_eq!(
            EventId::parse("019e5782_7c6b_72a2_b4fa_a81bf56eb37e"),
            Err(ExoIdError::NotAUuid)
        );
        assert_eq!(
            EventId::parse("019e5782-7c6b-72g2-b4fa-a81bf56eb37e"),
            Err(ExoIdError::NotAUuid),
            "`g` is not hexadecimal"
        );
    }

    #[test]
    fn a_uuid_that_is_not_version_7_is_refused() {
        // A real v4 UUID. Exo would order this against real history by luck,
        // because `up_to_inclusive` compares ids rather than timestamps.
        assert_eq!(
            EventId::parse("f47ac10b-58cc-4372-a567-0e02b2c3d479"),
            Err(ExoIdError::NotVersion7)
        );
    }

    #[test]
    fn a_sandbox_id_is_not_a_uuid_because_exo_says_it_is_a_string() {
        let id = SandboxId::parse("sandbox-019e5782-2a46-7970-a5bf-62900a2233e8")
            .expect("Exo's own shipped shape");
        assert_eq!(id.as_str(), "sandbox-019e5782-2a46-7970-a5bf-62900a2233e8");
        assert_eq!(SandboxId::parse(""), Err(ExoIdError::Empty));
        assert_eq!(
            SandboxId::parse("sandbox one"),
            Err(ExoIdError::Unprintable),
            "an id with a space in it would split into two arguments somewhere"
        );
        assert_eq!(
            SandboxId::parse("sandbox\"one"),
            Err(ExoIdError::Unprintable)
        );
    }

    #[test]
    fn the_id_kinds_do_not_convert_into_one_another() {
        // Not a runtime property: this test exists so the source records the
        // intent. `SnapshotId` and `SandboxId` are the pair `start_sandbox`
        // takes together, and they are different types with no `From` between
        // them, so the two cannot be swapped at a call site.
        let snapshot = SnapshotId::parse(A_UUID7).expect("a v7 uuid");
        let sandbox = SandboxId::parse(A_UUID7).expect("a string id");
        assert_eq!(snapshot.as_str(), sandbox.as_str());
        assert!(
            std::any::TypeId::of::<SnapshotId>() != std::any::TypeId::of::<SandboxId>(),
            "the same characters must still be two different values"
        );
    }
}
