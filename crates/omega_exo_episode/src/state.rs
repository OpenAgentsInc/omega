//! Comparing two episodes, rather than asserting they are the same.
//! `OMEGA-DELTA-0090`, omega#103.
//!
//! # Two forks of one event are not byte-identical, and cannot be
//!
//! omega#103 asks for a "byte-identical start". That is not what
//! `conversation_fork` produces, and an episode that checked for it would fail
//! on every correct fork. `BasicConversationHandle::fork` replays the source
//! events and rewrites three fields of each one on the way through:
//!
//! ```text
//! event.id = new_event_id;              // a fresh Uuid7
//! event.conversation_id = record.id;    // the fork's own id
//! event.created_at = new_event_id.timestamp();
//! ```
//!
//! then appends one `conversation_forked` event naming the source and the fork
//! point. Two forks taken from one event therefore differ in every event id,
//! in every `conversation_id`, and in every `created_at`, always — those are
//! the identity of the copy, not the content of the episode.
//!
//! Everything else is preserved verbatim, including `session_id`, `turn_id`,
//! and the whole `data` payload. And the appended `conversation_forked` event
//! carries `source_conversation_id` and `up_to_inclusive`, which are equal for
//! two siblings of one fork point — so it needs no special handling and gets
//! none.
//!
//! So the comparison is over the events with exactly [`IDENTITY_FIELDS`]
//! removed. That set is small, closed, and justified line by line against the
//! fork implementation. It is also the dangerous part of this file: an
//! exclusion set that grew would make more and more episodes compare equal, and
//! the check would go green by ignoring more rather than by matching more.
//! `an_exclusion_that_swallowed_the_payload_would_be_caught` is the guard, and
//! [`IDENTITY_FIELDS`] is asserted to be those three and nothing else.
//!
//! # Why a digest and a diff
//!
//! [`EpisodeState::digest`] answers "are these the same" in one comparable
//! value, which is what a receipt or a log line wants. [`EpisodeState::diff`]
//! answers "where did they stop being the same", which is what a person
//! debugging wants. A digest alone would turn every divergence into one bit.

use sha2::{Digest as _, Sha256};

/// The fields `fork` rewrites, which are therefore identity rather than
/// content.
///
/// Read off the three assignments in `BasicConversationHandle::fork`
/// (`crates/exoharness/src/basic.rs`, the replay loop) at
/// [`crate::EXO_PROTOCOL_PIN`]. `session_id` and `turn_id` are deliberately
/// **not** here: fork preserves them, so two siblings agree on them, and
/// excluding them would hide a real divergence.
pub const IDENTITY_FIELDS: &[&str] = &["id", "conversation_id", "created_at"];

/// Why a state could not be read.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum StateReadError {
    /// The server said `ok: false`, and this is what it said.
    Refused(String),
    /// The envelope did not answer the request this reader was given.
    WrongRequestId {
        /// The request id the reader expected.
        expected: u64,
        /// The request id the envelope carried, if any.
        found: Option<u64>,
    },
    /// The response was not the `events` shape a read answers with.
    NotAnEventsResponse,
    /// An event was not a JSON object.
    NotAnEvent {
        /// Its position in the array.
        at: usize,
    },
    /// The read stopped at a page boundary, so this is a prefix of the episode
    /// rather than the episode.
    ///
    /// Refused rather than truncated: a comparison of two different prefixes of
    /// two episodes is a green check that read less than it thought.
    Truncated {
        /// The cursor Exo returned to resume from.
        cursor: String,
    },
}

impl std::fmt::Display for StateReadError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Refused(error) => write!(formatter, "Exo refused the read: {error}"),
            Self::WrongRequestId { expected, found } => write!(
                formatter,
                "that answer is for request {found:?}, not for request {expected}"
            ),
            Self::NotAnEventsResponse => {
                formatter.write_str("a conversation read answers with events, and that is not one")
            }
            Self::NotAnEvent { at } => {
                write!(formatter, "the event at position {at} is not an object")
            }
            Self::Truncated { cursor } => write!(
                formatter,
                "the episode continues past this page (resume at {cursor}); comparing prefixes \
                 is not comparing episodes"
            ),
        }
    }
}

impl std::error::Error for StateReadError {}

/// One episode's durable state, with identity removed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EpisodeState {
    events: Vec<serde_json::Value>,
}

/// Where two episodes stop agreeing.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Divergence {
    /// The same state.
    Identical,
    /// The same prefix, then a different event.
    FirstDifferenceAt {
        /// The position of the first event that differs.
        at: usize,
    },
    /// One episode is a prefix of the other.
    LengthDiffers {
        /// How many events the left episode has.
        left: usize,
        /// How many events the right episode has.
        right: usize,
    },
}

impl Divergence {
    /// Whether the two episodes are the same state.
    #[must_use]
    pub const fn is_identical(&self) -> bool {
        matches!(self, Self::Identical)
    }
}

impl std::fmt::Display for Divergence {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Identical => formatter.write_str("identical"),
            Self::FirstDifferenceAt { at } => write!(formatter, "first difference at event {at}"),
            Self::LengthDiffers { left, right } => {
                write!(formatter, "{left} events against {right}")
            }
        }
    }
}

impl EpisodeState {
    /// Read the answer to a `conversation_get_events`.
    ///
    /// # Errors
    ///
    /// [`StateReadError`] for a refusal, a mismatched request id, a shape this
    /// build cannot read, or a page that does not reach the end of the episode.
    pub fn read_events_response(
        request_id: u64,
        envelope: &serde_json::Value,
    ) -> Result<Self, StateReadError> {
        let answered = envelope.get("id").and_then(serde_json::Value::as_u64);
        if answered != Some(request_id) {
            return Err(StateReadError::WrongRequestId {
                expected: request_id,
                found: answered,
            });
        }
        if envelope.get("ok").and_then(serde_json::Value::as_bool) != Some(true) {
            let error = envelope
                .get("error")
                .and_then(serde_json::Value::as_str)
                .unwrap_or("no reason given")
                .to_owned();
            return Err(StateReadError::Refused(error));
        }
        let response = envelope
            .get("response")
            .ok_or(StateReadError::NotAnEventsResponse)?;
        if response.get("type").and_then(serde_json::Value::as_str) != Some("events") {
            return Err(StateReadError::NotAnEventsResponse);
        }
        let result = response
            .get("result")
            .ok_or(StateReadError::NotAnEventsResponse)?;
        if let Some(cursor) = result.get("cursor").and_then(serde_json::Value::as_str) {
            return Err(StateReadError::Truncated {
                cursor: cursor.to_owned(),
            });
        }
        let events = result
            .get("events")
            .and_then(serde_json::Value::as_array)
            .ok_or(StateReadError::NotAnEventsResponse)?;
        Self::read_events(events)
    }

    /// Read an array of events directly.
    ///
    /// # Errors
    ///
    /// [`StateReadError::NotAnEvent`] when an element is not an object.
    pub fn read_events(events: &[serde_json::Value]) -> Result<Self, StateReadError> {
        events
            .iter()
            .enumerate()
            .map(|(at, event)| {
                let object = event.as_object().ok_or(StateReadError::NotAnEvent { at })?;
                let stripped: serde_json::Map<String, serde_json::Value> = object
                    .iter()
                    .filter(|(key, _)| !IDENTITY_FIELDS.contains(&key.as_str()))
                    .map(|(key, value)| (key.clone(), value.clone()))
                    .collect();
                Ok(serde_json::Value::Object(stripped))
            })
            .collect::<Result<Vec<_>, _>>()
            .map(|events| Self { events })
    }

    /// How many events the episode carries.
    #[must_use]
    pub fn len(&self) -> usize {
        self.events.len()
    }

    /// Whether the episode carries no events.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.events.is_empty()
    }

    /// A comparable digest of the whole episode.
    ///
    /// Object keys are sorted before hashing, so two readings of one episode
    /// that happened to preserve key order differently still agree. That is not
    /// hypothetical: whether `serde_json` preserves insertion order is a
    /// feature flag, and a digest that depended on it would be a divergence
    /// report that changed with a `Cargo.toml`.
    #[must_use]
    pub fn digest(&self) -> String {
        let mut hasher = Sha256::new();
        hasher.update(b"omega.exo.episode.state.v1");
        for event in &self.events {
            hasher.update(b"\x1e");
            hasher.update(canonical(event).as_bytes());
        }
        let digest = hasher.finalize();
        digest.iter().fold(String::new(), |mut rendered, byte| {
            use std::fmt::Write as _;
            let _ = write!(rendered, "{byte:02x}");
            rendered
        })
    }

    /// Where this episode stops agreeing with another.
    #[must_use]
    pub fn diff(&self, other: &Self) -> Divergence {
        for (at, (left, right)) in self.events.iter().zip(other.events.iter()).enumerate() {
            if canonical(left) != canonical(right) {
                return Divergence::FirstDifferenceAt { at };
            }
        }
        if self.events.len() == other.events.len() {
            Divergence::Identical
        } else {
            Divergence::LengthDiffers {
                left: self.events.len(),
                right: other.events.len(),
            }
        }
    }
}

/// Render a value with every object key sorted, so the bytes are a function of
/// the content alone.
fn canonical(value: &serde_json::Value) -> String {
    match value {
        serde_json::Value::Object(object) => {
            let mut keys: Vec<&String> = object.keys().collect();
            keys.sort_unstable();
            let mut rendered = String::from("{");
            for (index, key) in keys.into_iter().enumerate() {
                if index > 0 {
                    rendered.push(',');
                }
                rendered.push_str(&serde_json::Value::String(key.clone()).to_string());
                rendered.push(':');
                rendered.push_str(&canonical(&object[key]));
            }
            rendered.push('}');
            rendered
        }
        serde_json::Value::Array(items) => {
            let mut rendered = String::from("[");
            for (index, item) in items.iter().enumerate() {
                if index > 0 {
                    rendered.push(',');
                }
                rendered.push_str(&canonical(item));
            }
            rendered.push(']');
            rendered
        }
        other => other.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// One event as Exo serializes it, with the fields a fork rewrites.
    fn event(
        id: &str,
        conversation: &str,
        created_at: &str,
        data: serde_json::Value,
    ) -> serde_json::Value {
        serde_json::json!({
            "id": id,
            "conversation_id": conversation,
            "session_id": "019e5782-0000-7000-8000-00000000aaaa",
            "turn_id": serde_json::Value::Null,
            "created_at": created_at,
            "data": data,
        })
    }

    fn a_conversation(conversation: &str, id_prefix: &str, minute: u32) -> Vec<serde_json::Value> {
        vec![
            event(
                &format!("019e5782-0000-7000-8000-{id_prefix}00001"),
                conversation,
                &format!("2026-07-26T00:{minute:02}:01Z"),
                serde_json::json!({"type": "conversation_created", "slug": "work", "name": "Work"}),
            ),
            event(
                &format!("019e5782-0000-7000-8000-{id_prefix}00002"),
                conversation,
                &format!("2026-07-26T00:{minute:02}:02Z"),
                serde_json::json!({
                    "type": "tool_requested",
                    "tool": "shell",
                    "arguments": {"command": "cargo test -p omega_deltas"},
                }),
            ),
            event(
                &format!("019e5782-0000-7000-8000-{id_prefix}00003"),
                conversation,
                &format!("2026-07-26T00:{minute:02}:03Z"),
                serde_json::json!({
                    "type": "conversation_forked",
                    "source_conversation_id": "019e5782-0000-7000-8000-000000000002",
                    "up_to_inclusive": "019e5782-0000-7000-8000-00000000000e",
                }),
            ),
        ]
    }

    #[test]
    fn two_forks_of_one_event_compare_equal_although_no_byte_of_their_identity_matches() {
        // Every id, every conversation id, and every timestamp differs, which
        // is what a real pair of forks looks like.
        let candidate = a_conversation("019e5782-0000-7000-8000-0000000000c1", "aaa", 10);
        let control = a_conversation("019e5782-0000-7000-8000-0000000000c2", "bbb", 11);
        assert_ne!(candidate, control, "the raw events differ in identity");

        let candidate = EpisodeState::read_events(&candidate).expect("events");
        let control = EpisodeState::read_events(&control).expect("events");
        assert_eq!(candidate.diff(&control), Divergence::Identical);
        assert_eq!(
            candidate.digest(),
            control.digest(),
            "two forks from one event are one starting state"
        );
        assert_eq!(candidate.len(), 3);
        assert!(!candidate.is_empty());
    }

    #[test]
    fn a_mutation_in_one_fork_is_absent_from_its_sibling() {
        let baseline = a_conversation("019e5782-0000-7000-8000-0000000000c1", "aaa", 10);
        let control = EpisodeState::read_events(&a_conversation(
            "019e5782-0000-7000-8000-0000000000c2",
            "bbb",
            11,
        ))
        .expect("events");

        let mut mutated = baseline.clone();
        mutated.push(event(
            "019e5782-0000-7000-8000-aaa00000004",
            "019e5782-0000-7000-8000-0000000000c1",
            "2026-07-26T00:10:04Z",
            serde_json::json!({
                "type": "tool_requested",
                "tool": "edit",
                "arguments": {"path": "crates/omega_deltas/src/omega_deltas.rs"},
            }),
        ));

        let baseline = EpisodeState::read_events(&baseline).expect("events");
        let mutated = EpisodeState::read_events(&mutated).expect("events");

        assert_eq!(
            mutated.diff(&baseline),
            Divergence::LengthDiffers { left: 4, right: 3 },
            "the candidate moved"
        );
        assert_ne!(mutated.digest(), baseline.digest());
        assert_eq!(
            control.diff(&baseline),
            Divergence::Identical,
            "the sibling did not move"
        );
        assert_eq!(control.digest(), baseline.digest());
    }

    #[test]
    fn an_exclusion_that_swallowed_the_payload_would_be_caught() {
        assert_eq!(
            IDENTITY_FIELDS,
            &["id", "conversation_id", "created_at"],
            "these are the three fields fork rewrites; a fourth would start hiding \
             real divergence"
        );
        // The same identity, different content, must not compare equal — this
        // is the property an over-broad exclusion set destroys.
        let left = vec![event(
            "019e5782-0000-7000-8000-aaa00000001",
            "019e5782-0000-7000-8000-0000000000c1",
            "2026-07-26T00:10:01Z",
            serde_json::json!({"type": "tool_requested", "tool": "shell"}),
        )];
        let right = vec![event(
            "019e5782-0000-7000-8000-aaa00000001",
            "019e5782-0000-7000-8000-0000000000c1",
            "2026-07-26T00:10:01Z",
            serde_json::json!({"type": "tool_requested", "tool": "edit"}),
        )];
        let left = EpisodeState::read_events(&left).expect("events");
        let right = EpisodeState::read_events(&right).expect("events");
        assert_eq!(left.diff(&right), Divergence::FirstDifferenceAt { at: 0 });
        assert_ne!(left.digest(), right.digest());
    }

    #[test]
    fn session_and_turn_ids_are_content_because_a_fork_preserves_them() {
        let with_session = vec![serde_json::json!({
            "id": "019e5782-0000-7000-8000-aaa00000001",
            "conversation_id": "019e5782-0000-7000-8000-0000000000c1",
            "created_at": "2026-07-26T00:10:01Z",
            "session_id": "019e5782-0000-7000-8000-00000000aaaa",
            "data": {"type": "session_started"},
        })];
        let with_other_session = vec![serde_json::json!({
            "id": "019e5782-0000-7000-8000-bbb00000001",
            "conversation_id": "019e5782-0000-7000-8000-0000000000c2",
            "created_at": "2026-07-26T00:11:01Z",
            "session_id": "019e5782-0000-7000-8000-00000000bbbb",
            "data": {"type": "session_started"},
        })];
        let left = EpisodeState::read_events(&with_session).expect("events");
        let right = EpisodeState::read_events(&with_other_session).expect("events");
        assert_eq!(
            left.diff(&right),
            Divergence::FirstDifferenceAt { at: 0 },
            "a fork copies session ids verbatim, so a different one is a different episode"
        );
    }

    #[test]
    fn the_digest_does_not_depend_on_the_order_keys_arrived_in() {
        let one = serde_json::json!({"data": {"a": 1, "b": 2}, "session_id": null});
        let other = serde_json::json!({"session_id": null, "data": {"b": 2, "a": 1}});
        assert_eq!(canonical(&one), canonical(&other));
        let one = EpisodeState::read_events(&[one]).expect("events");
        let other = EpisodeState::read_events(&[other]).expect("events");
        assert_eq!(one.digest(), other.digest());
    }

    #[test]
    fn the_digest_is_over_the_order_as_well_as_the_content() {
        // An event log is a sequence. Two episodes that ran the same events in
        // a different order are two different episodes, and a digest that
        // sorted or set-ified its input would call them one.
        //
        // The record separator in `digest` is not what this proves, and an
        // earlier version of this test claimed it was. Deleting the separator
        // left the test green, because canonical JSON renderings are
        // self-delimiting — every string is quoted and every object is braced,
        // so no two different sequences concatenate to the same bytes. The
        // separator is defence in depth against a future rendering that is not
        // self-delimiting. Falsifying this test is what found that; the claim
        // is now the one the test can actually keep.
        let first = event(
            "019e5782-0000-7000-8000-aaa00000001",
            "019e5782-0000-7000-8000-0000000000c1",
            "2026-07-26T00:10:01Z",
            serde_json::json!({"type": "tool_requested", "tool": "shell"}),
        );
        let second = event(
            "019e5782-0000-7000-8000-aaa00000002",
            "019e5782-0000-7000-8000-0000000000c1",
            "2026-07-26T00:10:02Z",
            serde_json::json!({"type": "tool_requested", "tool": "edit"}),
        );
        let forwards = EpisodeState::read_events(&[first.clone(), second.clone()]).expect("events");
        let backwards = EpisodeState::read_events(&[second, first]).expect("events");
        assert_ne!(
            forwards.digest(),
            backwards.digest(),
            "the same events in a different order are a different episode"
        );
        assert_eq!(
            forwards.diff(&backwards),
            Divergence::FirstDifferenceAt { at: 0 }
        );
    }

    #[test]
    fn a_truncated_read_is_refused_rather_than_compared() {
        let envelope = serde_json::json!({
            "kind": "response",
            "id": 2,
            "ok": true,
            "response": {
                "type": "events",
                "result": {
                    "events": a_conversation("019e5782-0000-7000-8000-0000000000c1", "aaa", 10),
                    "cursor": "019e5782-0000-7000-8000-aaa00000003",
                },
            },
            "error": null,
        });
        assert_eq!(
            EpisodeState::read_events_response(2, &envelope),
            Err(StateReadError::Truncated {
                cursor: "019e5782-0000-7000-8000-aaa00000003".to_owned()
            })
        );
    }

    #[test]
    fn a_complete_read_is_accepted() {
        let envelope = serde_json::json!({
            "kind": "response",
            "id": 2,
            "ok": true,
            "response": {
                "type": "events",
                "result": {
                    "events": a_conversation("019e5782-0000-7000-8000-0000000000c1", "aaa", 10),
                    "cursor": null,
                },
            },
            "error": null,
        });
        let state = EpisodeState::read_events_response(2, &envelope).expect("a complete page");
        assert_eq!(state.len(), 3);
    }

    #[test]
    fn a_read_that_is_not_a_read_is_refused() {
        assert_eq!(
            EpisodeState::read_events_response(
                1,
                &serde_json::json!({"kind":"response","id":1,"ok":false,"error":"conversation not found","response":null})
            ),
            Err(StateReadError::Refused("conversation not found".to_owned()))
        );
        assert_eq!(
            EpisodeState::read_events_response(
                1,
                &serde_json::json!({"kind":"response","id":9,"ok":true,"response":{"type":"events"},"error":null})
            ),
            Err(StateReadError::WrongRequestId {
                expected: 1,
                found: Some(9)
            })
        );
        assert_eq!(
            EpisodeState::read_events_response(
                1,
                &serde_json::json!({"kind":"response","id":1,"ok":true,"response":{"type":"unit"},"error":null})
            ),
            Err(StateReadError::NotAnEventsResponse)
        );
        assert_eq!(
            EpisodeState::read_events(&[serde_json::json!("not an event")]),
            Err(StateReadError::NotAnEvent { at: 0 })
        );
    }
}
