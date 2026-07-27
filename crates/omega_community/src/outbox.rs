//! What is waiting to reach the relay, and what stopped.
//!
//! omega#108's fourth falsifier: "simulate an offline relay: pending work must
//! be visible as pending, not lost and not silently retried forever." Three
//! requirements in one sentence, and they pull against each other, so each is
//! answered by a different part of this module:
//!
//! - **Not lost.** The outbox is a record, not a queue of in-flight futures. It
//!   serialises, so a message composed before a restart is still here after
//!   one, in the state it was in.
//! - **Visible as pending.** [`Delivery::Pending`] carries when it was first
//!   queued and how many attempts it has had, which is what a person needs to
//!   tell "sending" from "stuck".
//! - **Not retried forever.** [`MAX_DELIVERY_ATTEMPTS`] is a ceiling, and
//!   [`Delivery::GaveUp`] is a terminal state a person is shown. A relay
//!   refusal that will never change its mind — [`TERMINAL_OK_PREFIXES`] — skips
//!   the ceiling entirely and fails on the first answer, because retrying
//!   `invalid:` is a progress spinner over a thing that already finished.
//!
//! Nothing here connects to anything. The relay's answer arrives as a
//! [`RelayOutcome`] and the clock arrives as a parameter, so every state
//! transition is a pure function that a test can drive without a socket.

use std::collections::BTreeMap;
use std::fmt;

use nostr::EventId;
use serde::{Deserialize, Serialize};

use crate::SignedRecord;

/// How many times a retryable failure is attempted before the outbox stops and
/// says so.
///
/// A small number on purpose. The alternative that looks kinder — retry until
/// it works — is the failure omega#108 names, because a person watching
/// "sending…" for an hour has been told nothing, and the honest thing is a
/// message they can act on.
pub const MAX_DELIVERY_ATTEMPTS: u32 = 5;

/// NIP-01 `OK` reason prefixes that will not change on a retry.
///
/// A closed list of the answers that are *final*, rather than a list of the
/// ones that are retryable. The direction matters: an unknown prefix from a
/// newer relay is treated as retryable and hits [`MAX_DELIVERY_ATTEMPTS`],
/// which wastes four attempts. Treating an unknown prefix as final would
/// instead discard a message over a word this build had not heard of.
pub const TERMINAL_OK_PREFIXES: &[&str] = &["invalid", "blocked", "restricted", "pow"];

/// What a relay said about one event.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RelayOutcome {
    /// `["OK", <id>, true, ""]`. The relay has it.
    Accepted,
    /// `["OK", <id>, false, "<prefix>: <message>"]`.
    ///
    /// The prefix is kept separate because it is the part with meaning, and
    /// the message is kept because it is the part a person can act on. Neither
    /// is rewritten.
    Refused {
        /// The machine-readable prefix, without its colon.
        prefix: String,
        /// The relay's own words.
        message: String,
    },
    /// The relay could not be reached, or the connection ended before an `OK`
    /// arrived.
    ///
    /// Distinct from a refusal because it is a statement about the network and
    /// not about the event, and because a lost `OK` means the relay may already
    /// have it — which is why the retry re-sends the same signed bytes rather
    /// than composing new ones.
    Unreachable {
        /// What went wrong, in whatever words the transport had.
        message: String,
    },
}

impl RelayOutcome {
    /// Reads a NIP-01 `OK` frame's reason into a refusal.
    ///
    /// A reason with no prefix becomes an empty prefix rather than being
    /// guessed at, and an empty prefix is not in [`TERMINAL_OK_PREFIXES`], so
    /// it retries.
    #[must_use]
    pub fn refused(reason: &str) -> Self {
        match reason.split_once(':') {
            Some((prefix, message)) => Self::Refused {
                prefix: prefix.trim().to_string(),
                message: message.trim().to_string(),
            },
            None => Self::Refused {
                prefix: String::new(),
                message: reason.trim().to_string(),
            },
        }
    }

    fn is_terminal_refusal(&self) -> bool {
        match self {
            Self::Refused { prefix, .. } => TERMINAL_OK_PREFIXES.contains(&prefix.as_str()),
            _ => false,
        }
    }

    /// A `duplicate:` refusal means the relay already has the event, which is
    /// the outcome the sender wanted.
    fn is_already_there(&self) -> bool {
        matches!(self, Self::Refused { prefix, .. } if prefix == "duplicate")
    }
}

impl fmt::Display for RelayOutcome {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Accepted => formatter.write_str("accepted"),
            Self::Refused { prefix, message } if prefix.is_empty() => formatter.write_str(message),
            Self::Refused { prefix, message } => write!(formatter, "{prefix}: {message}"),
            Self::Unreachable { message } => {
                write!(formatter, "the relay is unreachable: {message}")
            }
        }
    }
}

/// Where one message has got to.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum Delivery {
    /// Waiting, or being tried.
    Pending {
        /// How many attempts have been made.
        attempts: u32,
        /// The last thing that went wrong, if anything has yet.
        last_failure: Option<String>,
    },
    /// The relay has it.
    Delivered {
        /// When, by the clock the caller passed.
        at: u64,
    },
    /// The relay refused in a way that will not change.
    Failed {
        /// The relay's answer, preserved.
        reason: String,
        /// When it answered.
        at: u64,
    },
    /// Retryable failures ran out of attempts.
    ///
    /// Terminal for the outbox, not for the person: the record is still here
    /// and still signed, so a caller can requeue it deliberately. What it will
    /// not do is retry on its own.
    GaveUp {
        /// The last thing that went wrong.
        reason: String,
        /// How many attempts were made.
        attempts: u32,
        /// When it stopped.
        at: u64,
    },
}

impl Delivery {
    /// Is this message still going to be tried?
    #[must_use]
    pub const fn is_pending(&self) -> bool {
        matches!(self, Self::Pending { .. })
    }

    /// Has this message stopped, one way or another, without arriving?
    #[must_use]
    pub const fn needs_attention(&self) -> bool {
        matches!(self, Self::Failed { .. } | Self::GaveUp { .. })
    }

    /// What a person reads beside the message.
    #[must_use]
    pub fn label(&self) -> String {
        match self {
            Self::Pending { attempts: 0, .. } => "Waiting to send".to_string(),
            Self::Pending {
                attempts,
                last_failure: None,
            } => format!("Sending (attempt {attempts})"),
            Self::Pending {
                attempts,
                last_failure: Some(failure),
            } => format!("Retrying after {failure} (attempt {attempts})"),
            Self::Delivered { .. } => "Sent".to_string(),
            Self::Failed { reason, .. } => format!("Not sent: {reason}"),
            Self::GaveUp {
                reason, attempts, ..
            } => format!("Not sent after {attempts} attempts: {reason}"),
        }
    }
}

/// One message and where it has got to.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct OutboxEntry {
    /// The signed event, as NIP-01 JSON. Kept in the form a retry re-sends,
    /// because a retry must not sign new bytes: the event identity is the
    /// idempotency key, and re-signing would produce a second message.
    pub event: nostr::Event,
    /// When it was first queued.
    pub queued_at: u64,
    /// Where it has got to.
    pub delivery: Delivery,
}

/// The result of queueing something.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum QueueOutcome {
    /// It was not here, and now it is.
    Queued,
    /// It was already here, and nothing changed.
    ///
    /// Not an error. omega#108's parity bar includes "survives restart and
    /// replay and duplicate delivery", and the honest answer to queueing the
    /// same signed event twice is that there is still one message.
    AlreadyQueued,
}

/// A caller referred to a record the outbox does not have.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UnknownRecord(pub String);

impl fmt::Display for UnknownRecord {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "no queued record with id `{}`", self.0)
    }
}

/// Everything composed for the room that the relay has not confirmed.
///
/// Keyed by event id, which is a hash of the signed bytes, so the same message
/// cannot be in here twice and a retry after a lost `OK` is the same entry
/// rather than a second one.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Outbox {
    entries: BTreeMap<String, OutboxEntry>,
}

impl Outbox {
    /// An empty outbox.
    #[must_use]
    pub fn new() -> Self {
        Self {
            entries: BTreeMap::new(),
        }
    }

    /// Puts a signed record in the outbox.
    ///
    /// Takes a [`SignedRecord`] and not an event, which is what makes the
    /// authorization structural: the only way to get one for the outbound path
    /// is through `AuthorizedMessage::prepare`, so a message cannot be queued
    /// without having been authorized.
    pub fn queue(&mut self, record: &SignedRecord, now: u64) -> QueueOutcome {
        let key = record.id().to_hex();
        if self.entries.contains_key(&key) {
            return QueueOutcome::AlreadyQueued;
        }
        self.entries.insert(
            key,
            OutboxEntry {
                event: record.event().clone(),
                queued_at: now,
                delivery: Delivery::Pending {
                    attempts: 0,
                    last_failure: None,
                },
            },
        );
        QueueOutcome::Queued
    }

    /// Records what the relay said about one attempt.
    ///
    /// # Errors
    ///
    /// [`UnknownRecord`] if the id is not in the outbox. An error rather than a
    /// silent insert: a caller reporting an outcome for something that was
    /// never queued is a caller that has lost track of which message it sent,
    /// and inventing an entry would hide that.
    pub fn record_attempt(
        &mut self,
        id: EventId,
        outcome: &RelayOutcome,
        now: u64,
    ) -> Result<&OutboxEntry, UnknownRecord> {
        let key = id.to_hex();
        let entry = self
            .entries
            .get_mut(&key)
            .ok_or_else(|| UnknownRecord(key.clone()))?;

        let attempts = match entry.delivery {
            Delivery::Pending { attempts, .. } => attempts,
            // A settled entry does not move. Reporting another outcome for a
            // delivered message is a duplicate `OK`, which the parity bar
            // requires be survivable rather than surprising.
            _ => return Ok(entry),
        }
        .saturating_add(1);

        entry.delivery = if matches!(outcome, RelayOutcome::Accepted) || outcome.is_already_there()
        {
            Delivery::Delivered { at: now }
        } else if outcome.is_terminal_refusal() {
            Delivery::Failed {
                reason: outcome.to_string(),
                at: now,
            }
        } else if attempts >= MAX_DELIVERY_ATTEMPTS {
            Delivery::GaveUp {
                reason: outcome.to_string(),
                attempts,
                at: now,
            }
        } else {
            Delivery::Pending {
                attempts,
                last_failure: Some(outcome.to_string()),
            }
        };

        Ok(entry)
    }

    /// Everything still going to be tried, oldest first.
    pub fn pending(&self) -> impl Iterator<Item = &OutboxEntry> + '_ {
        self.sorted().filter(|entry| entry.delivery.is_pending())
    }

    /// Everything that stopped without arriving, oldest first.
    pub fn needing_attention(&self) -> impl Iterator<Item = &OutboxEntry> + '_ {
        self.sorted()
            .filter(|entry| entry.delivery.needs_attention())
    }

    /// Every entry, oldest first.
    pub fn sorted(&self) -> impl Iterator<Item = &OutboxEntry> + '_ {
        let mut entries: Vec<&OutboxEntry> = self.entries.values().collect();
        entries.sort_by_key(|entry| entry.queued_at);
        entries.into_iter()
    }

    /// One entry.
    #[must_use]
    pub fn entry(&self, id: EventId) -> Option<&OutboxEntry> {
        self.entries.get(&id.to_hex())
    }

    /// Forgets a delivered message.
    ///
    /// Only a delivered one: an outbox that could drop a pending or failed
    /// entry is an outbox that can lose a message quietly, which is what the
    /// falsifier is about.
    pub fn forget_delivered(&mut self, id: EventId) -> bool {
        let key = id.to_hex();
        match self.entries.get(&key) {
            Some(entry) if matches!(entry.delivery, Delivery::Delivered { .. }) => {
                self.entries.remove(&key).is_some()
            }
            _ => false,
        }
    }

    /// How many messages the outbox is holding.
    #[must_use]
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Is there nothing outstanding?
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::record::tests::signed_message_for_tests;

    fn outbox_with_one_message() -> (Outbox, SignedRecord) {
        let record = signed_message_for_tests("into the room");
        let mut outbox = Outbox::new();
        assert_eq!(outbox.queue(&record, 100), QueueOutcome::Queued);
        (outbox, record)
    }

    #[test]
    fn a_queued_message_is_visible_as_pending_before_anything_is_tried() {
        let (outbox, record) = outbox_with_one_message();

        let entry = outbox.entry(record.id()).expect("the queued record");
        assert_eq!(
            entry.delivery,
            Delivery::Pending {
                attempts: 0,
                last_failure: None
            }
        );
        assert_eq!(entry.delivery.label(), "Waiting to send");
        assert_eq!(outbox.pending().count(), 1);
        assert_eq!(outbox.needing_attention().count(), 0);
    }

    /// omega#108's fourth falsifier: an offline relay must be visible, and must
    /// stop.
    #[test]
    fn an_offline_relay_shows_pending_then_stops_rather_than_retrying_forever() {
        let (mut outbox, record) = outbox_with_one_message();
        let offline = RelayOutcome::Unreachable {
            message: "connection refused".to_string(),
        };

        for attempt in 1..MAX_DELIVERY_ATTEMPTS {
            let entry = outbox
                .record_attempt(record.id(), &offline, 100 + u64::from(attempt))
                .expect("a queued record");
            assert_eq!(
                entry.delivery,
                Delivery::Pending {
                    attempts: attempt,
                    last_failure: Some("the relay is unreachable: connection refused".to_string()),
                },
                "attempt {attempt} must still read as pending, with a reason"
            );
        }

        let entry = outbox
            .record_attempt(record.id(), &offline, 200)
            .expect("a queued record");
        assert_eq!(
            entry.delivery,
            Delivery::GaveUp {
                reason: "the relay is unreachable: connection refused".to_string(),
                attempts: MAX_DELIVERY_ATTEMPTS,
                at: 200,
            },
            "the outbox stops on its own, and says so"
        );
        assert_eq!(outbox.pending().count(), 0);
        assert_eq!(
            outbox.needing_attention().count(),
            1,
            "and the message is still here, not lost"
        );
        assert_eq!(outbox.len(), 1);
    }

    #[test]
    fn a_refusal_that_will_not_change_fails_on_the_first_answer() {
        for prefix in TERMINAL_OK_PREFIXES {
            let (mut outbox, record) = outbox_with_one_message();
            let outcome = RelayOutcome::refused(&format!("{prefix}: not happening"));

            let entry = outbox
                .record_attempt(record.id(), &outcome, 101)
                .expect("a queued record");
            assert_eq!(
                entry.delivery,
                Delivery::Failed {
                    reason: format!("{prefix}: not happening"),
                    at: 101,
                },
                "retrying `{prefix}` is a progress spinner over a finished thing"
            );
        }
    }

    #[test]
    fn an_unknown_refusal_prefix_is_retried_rather_than_discarded() {
        let (mut outbox, record) = outbox_with_one_message();
        let outcome = RelayOutcome::refused("newer-relay-word: try again later");

        let entry = outbox
            .record_attempt(record.id(), &outcome, 101)
            .expect("a queued record");
        assert!(
            entry.delivery.is_pending(),
            "a word this build has not heard of must not discard somebody's \
             message"
        );
    }

    #[test]
    fn a_relay_that_already_has_it_counts_as_delivered() {
        let (mut outbox, record) = outbox_with_one_message();

        let entry = outbox
            .record_attempt(
                record.id(),
                &RelayOutcome::refused("duplicate: have this already"),
                101,
            )
            .expect("a queued record");
        assert_eq!(entry.delivery, Delivery::Delivered { at: 101 });
    }

    /// A lost `OK` is retried with the same bytes, not with new ones.
    #[test]
    fn queueing_the_same_signed_event_twice_is_still_one_message() {
        let (mut outbox, record) = outbox_with_one_message();

        assert_eq!(outbox.queue(&record, 500), QueueOutcome::AlreadyQueued);
        assert_eq!(outbox.len(), 1);
        assert_eq!(
            outbox.entry(record.id()).expect("the entry").queued_at,
            100,
            "the second queueing must not reset when the message was composed"
        );
    }

    #[test]
    fn a_duplicate_ok_after_delivery_does_not_move_a_settled_message() {
        let (mut outbox, record) = outbox_with_one_message();
        outbox
            .record_attempt(record.id(), &RelayOutcome::Accepted, 101)
            .expect("a queued record");

        let entry = outbox
            .record_attempt(record.id(), &RelayOutcome::Accepted, 900)
            .expect("a queued record");
        assert_eq!(entry.delivery, Delivery::Delivered { at: 101 });
    }

    #[test]
    fn an_outcome_for_something_never_queued_is_an_error_not_an_insert() {
        let mut outbox = Outbox::new();
        let record = signed_message_for_tests("never queued");

        assert_eq!(
            outbox.record_attempt(record.id(), &RelayOutcome::Accepted, 1),
            Err(UnknownRecord(record.id().to_hex()))
        );
        assert!(outbox.is_empty());
    }

    #[test]
    fn only_a_delivered_message_can_be_forgotten() {
        let (mut outbox, record) = outbox_with_one_message();

        assert!(
            !outbox.forget_delivered(record.id()),
            "a pending message must not be droppable"
        );
        outbox
            .record_attempt(record.id(), &RelayOutcome::refused("blocked: no"), 101)
            .expect("a queued record");
        assert!(
            !outbox.forget_delivered(record.id()),
            "and neither must a failed one"
        );
        assert_eq!(outbox.len(), 1);
    }

    /// "Survives restart": the outbox is a record, not a set of in-flight
    /// futures.
    #[test]
    fn an_outbox_survives_the_round_trip_through_storage() {
        let (mut outbox, record) = outbox_with_one_message();
        outbox
            .record_attempt(
                record.id(),
                &RelayOutcome::Unreachable {
                    message: "offline".to_string(),
                },
                101,
            )
            .expect("a queued record");

        let encoded = serde_json::to_string(&outbox).expect("an outbox encodes");
        let decoded: Outbox = serde_json::from_str(&encoded).expect("an outbox decodes");

        assert_eq!(decoded, outbox);
        let entry = decoded.entry(record.id()).expect("the entry survived");
        assert!(entry.delivery.is_pending());
        assert_eq!(
            entry.event,
            *record.event(),
            "the bytes a retry re-sends must be the bytes that were signed"
        );
    }

    #[test]
    fn a_reason_with_no_prefix_is_not_guessed_at() {
        assert_eq!(
            RelayOutcome::refused("something went wrong"),
            RelayOutcome::Refused {
                prefix: String::new(),
                message: "something went wrong".to_string(),
            }
        );
        assert_eq!(
            RelayOutcome::refused("something went wrong").to_string(),
            "something went wrong"
        );
    }
}
