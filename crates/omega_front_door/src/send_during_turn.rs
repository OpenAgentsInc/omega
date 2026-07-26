//! What pressing send during a running turn does, per executor class (omega#79).
//!
//! ## The hole this closes at design time
//!
//! Every incumbent has the same concurrency defect: a second send arriving
//! while a turn is running is quietly reinterpreted. Sometimes it steers the
//! provider, sometimes it cancels and restarts, sometimes it is dropped — and
//! the user cannot tell which happened, because the three look identical from
//! the composer.
//!
//! Omega inherited a partial fix. `MessageQueue` already distinguishes a steer
//! from an enqueue, but only the **native loop** ever learned about it:
//! `sync_queue_flag_to_native_thread` sets the boundary flag on a
//! `agent::Thread` and no-ops for anything else. An external ACP thread and an
//! engine lane both fell through to cancel-then-send, which is the silent
//! reinterpretation this packet exists to end. Two classes worked and one
//! dropped.
//!
//! ## The shape
//!
//! [`disposition`] is a total function from (what the user asked for, which
//! class is running it, what the peer said it can do) to a **declared** outcome.
//! There is no `None`, no fallthrough, and no variant meaning "whatever the
//! executor does". Every class has a stated answer, and every answer a user
//! would experience as different is a different variant.
//!
//! Where a class cannot do what was asked, the answer is a typed refusal
//! carrying its declared fallback ([`SendDisposition::Refused`]) rather than a
//! quiet substitution. The user is told the steer was not available *and* what
//! happened instead.
//!
//! ## Why an engine lane refuses a steer
//!
//! An engine lane **is** Full Auto authority (`OMEGA-DELTA-0029`), and its
//! controls are bound to the run generation the Full Auto surface minted them
//! for (`OMEGA-DELTA-0030`). A composer that could interrupt a run mid-flight
//! would be a second place that believes it can command a run, reading a
//! projection. So the engine lane's declared answer to a steer is a refusal
//! with a durable-hold fallback, and its answer to an enqueue is the hold. That
//! is a stated behavior, not an omission — which is the whole difference the
//! exit asks about.
//!
//! This module is pure and clock-free, like [`super::router`]: same inputs,
//! same disposition, every time. It starts nothing, and it holds no state.

use crate::ExecutorClass;

/// What the person asked for when they pressed send during a running turn.
///
/// These are two different intentions, and conflating them is the defect.
/// "Interrupt what you are doing and take this into account" is not "run this
/// next".
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SendCommand {
    /// Reach the running turn. The user wants the executor to change course.
    Steer,
    /// Do not reach the running turn. Run this after it finishes.
    Enqueue,
}

impl SendCommand {
    #[must_use]
    pub const fn token(self) -> &'static str {
        match self {
            Self::Steer => "steer",
            Self::Enqueue => "enqueue",
        }
    }

    #[must_use]
    pub const fn all() -> &'static [Self] {
        &[Self::Steer, Self::Enqueue]
    }

    #[must_use]
    pub fn parse_token(token: &str) -> Option<Self> {
        match token {
            "steer" => Some(Self::Steer),
            "enqueue" => Some(Self::Enqueue),
            _ => None,
        }
    }
}

/// What an executor said it can do with a send that arrives mid-turn.
///
/// Only [`ExecutorClass::ExternalAcp`] negotiates: the native loop's answer is
/// fixed by its own turn loop, and an engine lane's is fixed by where run
/// authority lives. An external peer is asked, and a peer that has not answered
/// is [`Unknown`](Self::Unknown) — which is deliberately *not* the same as
/// [`CannotSteer`](Self::CannotSteer). Treating silence as a capability is how
/// a steer becomes a cancel.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SteerCapability {
    /// The peer declared it can take a mid-turn message.
    CanSteer,
    /// The peer declared it cannot.
    CannotSteer,
    /// The peer has not been asked, or did not answer.
    Unknown,
}

impl SteerCapability {
    #[must_use]
    pub const fn token(self) -> &'static str {
        match self {
            Self::CanSteer => "can_steer",
            Self::CannotSteer => "cannot_steer",
            Self::Unknown => "unknown",
        }
    }

    #[must_use]
    pub const fn all() -> &'static [Self] {
        &[Self::CanSteer, Self::CannotSteer, Self::Unknown]
    }
}

/// Why a steer was not performed as asked.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SteerRefusal {
    /// The peer declared it cannot take a mid-turn message.
    PeerCannotSteer,
    /// The peer has not declared a steer capability. Not the same as declaring
    /// that it cannot: Omega refuses rather than guessing, because guessing
    /// wrong means cancelling somebody's turn.
    PeerCapabilityUnknown,
    /// An engine lane is Full Auto authority. Its run is commanded from the
    /// surface that minted its controls, not from a thread composer.
    EngineLaneIsRunAuthority,
}

impl SteerRefusal {
    #[must_use]
    pub const fn token(self) -> &'static str {
        match self {
            Self::PeerCannotSteer => "peer_cannot_steer",
            Self::PeerCapabilityUnknown => "peer_capability_unknown",
            Self::EngineLaneIsRunAuthority => "engine_lane_is_run_authority",
        }
    }

    /// The sentence a user is shown. Derived, never stored — the same rule
    /// [`crate::ExecutorDisclosure`] holds to.
    #[must_use]
    pub const fn phrase(self) -> &'static str {
        match self {
            Self::PeerCannotSteer => "this agent cannot take a message mid-turn",
            Self::PeerCapabilityUnknown => {
                "this agent has not said whether it can take a message mid-turn"
            }
            Self::EngineLaneIsRunAuthority => {
                "a Full Auto run is steered from the run surface, not from here"
            }
        }
    }

    #[must_use]
    pub const fn all() -> &'static [Self] {
        &[
            Self::PeerCannotSteer,
            Self::PeerCapabilityUnknown,
            Self::EngineLaneIsRunAuthority,
        ]
    }
}

/// What actually happens to a send that arrives during a running turn.
///
/// Every variant is something a user would experience as different. There is no
/// variant meaning "it depends on the executor" — that is the state this type
/// replaces.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SendDisposition {
    /// The running turn is ended at its next message boundary and the message
    /// is delivered. The native loop's `end_turn_at_next_boundary`.
    SteerAtMessageBoundary,
    /// The message is handed to the running turn without ending it. What an
    /// external peer that declared the capability does.
    SteerInFlight,
    /// The message is held until the prior turn is proven quiescent, then
    /// promoted. Nothing reaches the running turn.
    HeldUntilQuiescent,
    /// The steer was not available. The fallback is stated, not implied.
    Refused {
        refusal: SteerRefusal,
        fallback: SendFallback,
    },
}

/// What happens instead, when a steer is refused.
///
/// A separate closed type so a refusal cannot be constructed without saying
/// what it did with the message. "Refused" on its own would be the drop this
/// packet exists to prevent.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SendFallback {
    /// Admitted to the durable queue and promoted after quiescence.
    HeldUntilQuiescent,
}

impl SendFallback {
    #[must_use]
    pub const fn token(self) -> &'static str {
        match self {
            Self::HeldUntilQuiescent => "held_until_quiescent",
        }
    }
}

impl SendDisposition {
    /// The stable token. Persisted and compared; the user sees
    /// [`phrase`](Self::phrase).
    #[must_use]
    pub fn token(self) -> String {
        match self {
            Self::SteerAtMessageBoundary => "steer_at_message_boundary".to_owned(),
            Self::SteerInFlight => "steer_in_flight".to_owned(),
            Self::HeldUntilQuiescent => "held_until_quiescent".to_owned(),
            Self::Refused { refusal, fallback } => {
                format!("refused:{}:{}", refusal.token(), fallback.token())
            }
        }
    }

    /// Whether the running turn is reached at all.
    ///
    /// The single question the falsifier for this packet asks: "a second send
    /// reaches a running provider turn without a guard".
    #[must_use]
    pub const fn reaches_running_turn(self) -> bool {
        matches!(self, Self::SteerAtMessageBoundary | Self::SteerInFlight)
    }

    /// The line the composer shows. Derived from the parts on every call.
    #[must_use]
    pub fn phrase(self) -> String {
        match self {
            Self::SteerAtMessageBoundary => {
                "Steering: the current turn ends at its next step.".to_owned()
            }
            Self::SteerInFlight => "Steering: sent to the running turn.".to_owned(),
            Self::HeldUntilQuiescent => "Queued: sends after this turn finishes.".to_owned(),
            Self::Refused { refusal, fallback } => match fallback {
                SendFallback::HeldUntilQuiescent => {
                    format!(
                        "Not steered — {}. Queued: sends after this turn finishes.",
                        refusal.phrase()
                    )
                }
            },
        }
    }
}

/// The law. Total over every (command, class, capability) triple.
///
/// A caller cannot reach a case this does not answer, which is what makes
/// "declared visible behavior on every executor class" checkable rather than
/// asserted.
#[must_use]
pub const fn disposition(
    command: SendCommand,
    class: ExecutorClass,
    capability: SteerCapability,
) -> SendDisposition {
    match command {
        // An enqueue never reaches the running turn, on any class. This is the
        // half that was already honest, and it stays uniform on purpose: a
        // queued message that sometimes interrupts is the original defect.
        SendCommand::Enqueue => SendDisposition::HeldUntilQuiescent,
        SendCommand::Steer => match class {
            // The native loop stops at a message boundary. It does not need to
            // negotiate, because Omega owns both sides of that loop.
            ExecutorClass::NativeLoop => SendDisposition::SteerAtMessageBoundary,
            ExecutorClass::ExternalAcp => match capability {
                SteerCapability::CanSteer => SendDisposition::SteerInFlight,
                SteerCapability::CannotSteer => SendDisposition::Refused {
                    refusal: SteerRefusal::PeerCannotSteer,
                    fallback: SendFallback::HeldUntilQuiescent,
                },
                // Silence is not consent. A peer that never answered is not
                // assumed able, because the cost of assuming wrong is the
                // user's running turn.
                SteerCapability::Unknown => SendDisposition::Refused {
                    refusal: SteerRefusal::PeerCapabilityUnknown,
                    fallback: SendFallback::HeldUntilQuiescent,
                },
            },
            // An engine lane is Full Auto authority. Its capability is not
            // consulted, because the answer does not depend on what the engine
            // can do — it depends on where run authority lives.
            ExecutorClass::EngineLane => SendDisposition::Refused {
                refusal: SteerRefusal::EngineLaneIsRunAuthority,
                fallback: SendFallback::HeldUntilQuiescent,
            },
        },
    }
}

/// Where a queued message is in its life.
///
/// Distinct from a draft (not yet sent) and from a steer (reaches the turn).
/// The exit asks for these to be visible and distinct, so they are a closed
/// enum rather than a pair of booleans.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QueueItemState {
    /// Admitted durably. Acknowledged to the user only once this holds.
    Queued,
    /// Promoted to a turn after the prior turn was proven quiescent.
    Promoted,
    /// Withdrawn by the user before promotion.
    Cancelled,
    /// Promotion was attempted and did not start a turn.
    Failed,
}

impl QueueItemState {
    #[must_use]
    pub const fn token(self) -> &'static str {
        match self {
            Self::Queued => "queued",
            Self::Promoted => "promoted",
            Self::Cancelled => "cancelled",
            Self::Failed => "failed",
        }
    }

    #[must_use]
    pub fn parse_token(token: &str) -> Option<Self> {
        match token {
            "queued" => Some(Self::Queued),
            "promoted" => Some(Self::Promoted),
            "cancelled" => Some(Self::Cancelled),
            "failed" => Some(Self::Failed),
            _ => None,
        }
    }

    #[must_use]
    pub const fn all() -> &'static [Self] {
        &[
            Self::Queued,
            Self::Promoted,
            Self::Cancelled,
            Self::Failed,
        ]
    }

    /// Whether this item can still be promoted.
    ///
    /// A terminal item that promoted again is the duplicate the falsifier for
    /// reconnect and restart names.
    #[must_use]
    pub const fn is_open(self) -> bool {
        matches!(self, Self::Queued)
    }
}

/// Whether the prior turn is proven finished, or merely believed to be.
///
/// A scheduler that promotes on "believed" is how a queued message races the
/// turn it was supposed to follow. Only [`Proven`](Self::Proven) promotes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Quiescence {
    /// The executor reported the turn stopped.
    Proven,
    /// A turn is running.
    Running,
    /// The connection dropped and no stop was observed. Not proof of anything.
    Unknown,
}

impl Quiescence {
    #[must_use]
    pub const fn token(self) -> &'static str {
        match self {
            Self::Proven => "proven",
            Self::Running => "running",
            Self::Unknown => "unknown",
        }
    }

    #[must_use]
    pub const fn all() -> &'static [Self] {
        &[Self::Proven, Self::Running, Self::Unknown]
    }
}

/// Whether the head of the queue may be promoted right now.
///
/// One thread-owned scheduler asks this. It is deliberately conservative about
/// [`Quiescence::Unknown`]: after a reconnect Omega has not seen the prior turn
/// stop, and promoting there is exactly the duplicate the falsifier describes.
#[must_use]
pub const fn may_promote(state: QueueItemState, quiescence: Quiescence) -> bool {
    matches!(
        (state, quiescence),
        (QueueItemState::Queued, Quiescence::Proven)
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The exit, as a check. Every class answers every command, and the answers
    /// are not all the same — a uniform answer would mean the classes were not
    /// really consulted.
    #[test]
    fn every_executor_class_has_a_declared_answer_to_a_send_during_a_turn() {
        let mut dispositions = Vec::new();
        for command in SendCommand::all() {
            for class in ExecutorClass::all() {
                for capability in SteerCapability::all() {
                    let decided = disposition(*command, *class, *capability);
                    assert!(
                        !decided.phrase().is_empty(),
                        "{}/{}/{} has no phrase to show",
                        command.token(),
                        class.token(),
                        capability.token()
                    );
                    dispositions.push(decided);
                }
            }
        }
        assert_eq!(dispositions.len(), 2 * 3 * 3);
        // Each class contributes at least one answer no other class gives, so a
        // reader cannot conclude the class was ignored.
        for class in ExecutorClass::all() {
            let mine: Vec<_> = SteerCapability::all()
                .iter()
                .map(|capability| disposition(SendCommand::Steer, *class, *capability))
                .collect();
            let others: Vec<_> = ExecutorClass::all()
                .iter()
                .filter(|other| *other != class)
                .flat_map(|other| {
                    SteerCapability::all()
                        .iter()
                        .map(move |capability| disposition(SendCommand::Steer, *other, *capability))
                })
                .collect();
            assert!(
                mine.iter().any(|decided| !others.contains(decided)),
                "{} gives no answer distinct from the other classes",
                class.token()
            );
        }
    }

    /// The named falsifier: "a second send reaches a running provider turn
    /// without a guard".
    #[test]
    fn nothing_reaches_a_running_turn_without_an_explicit_steer() {
        for class in ExecutorClass::all() {
            for capability in SteerCapability::all() {
                assert!(
                    !disposition(SendCommand::Enqueue, *class, *capability).reaches_running_turn(),
                    "an enqueue reached the running turn on {}",
                    class.token()
                );
            }
        }
    }

    /// Silence is not a capability. This is the case that turns a steer into a
    /// cancelled turn on a peer nobody asked.
    #[test]
    fn an_undeclared_peer_is_refused_rather_than_assumed_able() {
        let decided = disposition(
            SendCommand::Steer,
            ExecutorClass::ExternalAcp,
            SteerCapability::Unknown,
        );
        assert!(!decided.reaches_running_turn());
        assert_eq!(
            decided,
            SendDisposition::Refused {
                refusal: SteerRefusal::PeerCapabilityUnknown,
                fallback: SendFallback::HeldUntilQuiescent,
            }
        );
    }

    /// An engine lane is Full Auto authority. `OMEGA-DELTA-0029` reaches it only
    /// through an explicit pin, and `OMEGA-DELTA-0030` keeps run commands bound
    /// to the generation the run surface minted. A composer steer would be a
    /// second commanding surface.
    #[test]
    fn an_engine_lane_steer_is_refused_whatever_the_engine_can_do() {
        for capability in SteerCapability::all() {
            let decided = disposition(
                SendCommand::Steer,
                ExecutorClass::EngineLane,
                *capability,
            );
            assert_eq!(
                decided,
                SendDisposition::Refused {
                    refusal: SteerRefusal::EngineLaneIsRunAuthority,
                    fallback: SendFallback::HeldUntilQuiescent,
                },
                "capability {} changed an engine lane's answer",
                capability.token()
            );
            assert!(!decided.reaches_running_turn());
        }
    }

    /// A refusal that did not say what happened to the message is a drop with
    /// better manners. The type has no way to express one, and the rendered
    /// line always carries both halves.
    #[test]
    fn every_refusal_states_what_happened_to_the_message() {
        for class in ExecutorClass::all() {
            for capability in SteerCapability::all() {
                let decided = disposition(SendCommand::Steer, *class, *capability);
                if let SendDisposition::Refused { refusal, fallback } = decided {
                    let phrase = decided.phrase();
                    assert!(phrase.contains(refusal.phrase()), "{phrase} hides the reason");
                    assert!(
                        phrase.contains("Queued"),
                        "{phrase} does not say the message was queued"
                    );
                    assert_eq!(fallback, SendFallback::HeldUntilQuiescent);
                }
            }
        }
    }

    /// Only a proven stop promotes. Unknown is what a reconnect leaves behind,
    /// and promoting there is the duplicate the acceptance names.
    #[test]
    fn only_a_proven_stop_promotes_the_queue_head() {
        assert!(may_promote(QueueItemState::Queued, Quiescence::Proven));
        for quiescence in Quiescence::all() {
            for state in QueueItemState::all() {
                let promoted = may_promote(*state, *quiescence);
                if promoted {
                    assert_eq!(*state, QueueItemState::Queued);
                    assert_eq!(*quiescence, Quiescence::Proven);
                }
                if !state.is_open() {
                    assert!(
                        !promoted,
                        "{} promoted a second time",
                        state.token()
                    );
                }
            }
        }
    }

    #[test]
    fn every_token_round_trips_and_is_distinct() {
        for command in SendCommand::all() {
            assert_eq!(SendCommand::parse_token(command.token()), Some(*command));
        }
        for state in QueueItemState::all() {
            assert_eq!(QueueItemState::parse_token(state.token()), Some(*state));
        }
        let mut tokens = Vec::new();
        for command in SendCommand::all() {
            for class in ExecutorClass::all() {
                for capability in SteerCapability::all() {
                    tokens.push(disposition(*command, *class, *capability).token());
                }
            }
        }
        tokens.sort();
        let before = tokens.len();
        tokens.dedup();
        assert!(tokens.len() < before, "the triples should collapse to a smaller answer set");
        assert!(
            tokens.contains(&"steer_at_message_boundary".to_owned()),
            "the native loop's boundary stop is not reachable"
        );
    }
}
