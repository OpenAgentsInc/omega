//! The route decision. `OMEGA-DELTA-0029`, omega#78.
//!
//! Omega Agent is a router. It owns routing, disclosure, and receipts, and it
//! owns no execution (omega#74, admitted by the owner 2026-07-25). This module
//! is the decision half of that: a pure function from typed inputs to a typed
//! decision, with no clock, no randomness, and no map iteration in the path.
//!
//! The dispatch half — the `AgentConnection` implementation that hands the turn
//! to the executor this function names — lives in
//! `crates/agent_ui/src/omega_router.rs`, because it needs GPUI and the three
//! executor crates. Keeping the decision here means the routing law can be
//! checked in a second, and means no decision is ever made inside a widget.
//!
//! # Determinism, stated as three rules
//!
//! 1. [`route`] reads nothing but its argument. No clock, no environment, no
//!    global.
//! 2. Every collection it walks is an ordered slice, and every choice among
//!    equals is resolved by a total order on the lane reference — never by the
//!    order the engine happened to answer in, which is not a stable input.
//! 3. Every decision it can make is recordable, and the record is total: the
//!    canonical form escapes the four characters that carry structure, so no
//!    lane reference is refused for a cosmetic reason. The one lane the router
//!    does refuse is a nameless one, because there is nothing to dispatch to
//!    and nothing to write down.
//!
//! `crates/omega_deltas` asserts (1) against the source text as well, so a
//! later edit that reaches for `SystemTime` or a `HashMap` fails a test rather
//! than quietly making the router non-reproducible.
//!
//! # Why an unpinned thread is never routed to an engine lane
//!
//! Owner gate 8: *no model-initiated path can start Full Auto authority; only
//! an explicit human action can, wherever that action lives.* An engine lane is
//! Full Auto authority. So a router that preferred an engine lane for an
//! unpinned thread would be exactly the model-initiated start the gate forbids,
//! reached through a new door nobody had flagged — which is how
//! `full_auto_enable` survived until today.
//!
//! v1 therefore routes an unpinned thread to the native loop, always. Engine
//! lanes are reachable only through a pin, and a pin is set by a visible
//! control a person operates. Model-advisory routing is out of scope for v1 by
//! the packet's own terms, and this is the shape that keeps it out.

use crate::ExecutorClass;

// -------------------------------------------------------------------------
// Inputs
// -------------------------------------------------------------------------

/// A user's explicit choice of executor for a thread.
///
/// A pin is the *only* way a thread reaches anything but the native loop. It is
/// set by a human gesture on a visible control and never by a turn, a slash
/// command, or a restored draft.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExecutorPin {
    /// The class the user pinned.
    pub class: ExecutorClass,
    /// The engine lane the user pinned, when they named one.
    ///
    /// Meaningful only for [`ExecutorClass::EngineLane`]. A pin with no lane
    /// means "any lane the engine says is ready", resolved by the total order
    /// in [`select_lane`].
    pub lane_ref: Option<String>,
}

impl ExecutorPin {
    /// Pin a class without naming a lane.
    #[must_use]
    pub const fn new(class: ExecutorClass) -> Self {
        Self {
            class,
            lane_ref: None,
        }
    }

    /// Pin one named engine lane.
    #[must_use]
    pub fn on_lane(lane_ref: impl Into<String>) -> Self {
        Self {
            class: ExecutorClass::EngineLane,
            lane_ref: Some(lane_ref.into()),
        }
    }

    /// The stable token this pin is recorded under.
    #[must_use]
    pub fn token(&self) -> String {
        match &self.lane_ref {
            Some(lane_ref) => format!("{}@{}", self.class.token(), encode_field(lane_ref)),
            None => self.class.token().to_owned(),
        }
    }

    /// Read a pin back from [`token`](Self::token).
    #[must_use]
    pub fn parse_token(token: &str) -> Option<Self> {
        let (class_token, lane_ref) = match token.split_once('@') {
            Some((class_token, lane_ref)) => (class_token, Some(decode_field(lane_ref)?)),
            None => (token, None),
        };
        let class = *ExecutorClass::all()
            .iter()
            .find(|class| class.token() == class_token)?;
        Some(Self { class, lane_ref })
    }
}

/// What one engine lane says about itself in a `get_capacity` answer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LaneState {
    /// Ready and idle.
    Available,
    /// Ready, but already serving a run.
    Busy,
    /// Anything else the engine says, including a state this build does not
    /// recognise. Unrecognised is *not* available: a router that read an
    /// unknown state as ready would route into a lane on the strength of not
    /// understanding it.
    Unavailable,
}

impl LaneState {
    /// The stable token for this state.
    #[must_use]
    pub const fn token(self) -> &'static str {
        match self {
            Self::Available => "available",
            Self::Busy => "busy",
            Self::Unavailable => "unavailable",
        }
    }

    /// Read the engine's `state` string.
    ///
    /// Anything unrecognised is [`Unavailable`](Self::Unavailable), which is
    /// the fail-closed direction.
    #[must_use]
    pub fn parse(state: &str) -> Self {
        match state {
            "available" => Self::Available,
            "busy" => Self::Busy,
            _ => Self::Unavailable,
        }
    }

    /// Whether a turn may be routed onto a lane in this state.
    #[must_use]
    pub const fn can_serve(self) -> bool {
        matches!(self, Self::Available)
    }
}

/// One lane of the engine's declared capacity.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EngineLane {
    /// The lane reference, exactly as the engine reports it.
    pub lane_ref: String,
    /// What the engine says about it.
    pub state: LaneState,
}

impl EngineLane {
    /// A lane record.
    #[must_use]
    pub fn new(lane_ref: impl Into<String>, state: LaneState) -> Self {
        Self {
            lane_ref: lane_ref.into(),
            state,
        }
    }
}

/// Why the engine could not be asked.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EngineUnreachable {
    /// No supervised `omega-effectd` process is running.
    NotRunning,
    /// The framed request did not answer in time.
    Timeout,
    /// The engine answered something this build could not read.
    ProtocolError,
}

impl EngineUnreachable {
    /// The stable token for this cause.
    #[must_use]
    pub const fn token(self) -> &'static str {
        match self {
            Self::NotRunning => "not_running",
            Self::Timeout => "timeout",
            Self::ProtocolError => "protocol_error",
        }
    }

    /// Every admitted cause, in declaration order.
    #[must_use]
    pub const fn all() -> &'static [Self] {
        &[Self::NotRunning, Self::Timeout, Self::ProtocolError]
    }
}

/// What `omega-effectd` last said about itself.
///
/// A snapshot the router *reads*. The engine remains the sole run authority:
/// nothing here is written back, and nothing here is treated as run state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EngineReadiness {
    /// The engine answered `get_capacity`.
    Answered {
        /// Runs the engine currently has active.
        active_run_count: u32,
        /// The engine's own ceiling on active runs.
        active_run_limit: u32,
        /// The lanes it declared, in the order it declared them. The order is
        /// deliberately not trusted; see [`select_lane`].
        lanes: Vec<EngineLane>,
    },
    /// The engine could not be asked, or could not be understood.
    Unreachable(EngineUnreachable),
}

impl EngineReadiness {
    /// Whether the engine could be asked at all.
    #[must_use]
    pub const fn answered(&self) -> bool {
        matches!(self, Self::Answered { .. })
    }
}

/// Everything [`route`] is allowed to read.
///
/// If a decision cannot be explained from these fields, it is not a decision
/// this router made — which is omega#78's falsifier, stated as a type.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RouteInputs {
    /// What the user pinned for this thread, if anything.
    pub pin: Option<ExecutorPin>,
    /// What the engine last said about itself.
    pub engine: EngineReadiness,
    /// The external ACP agent connected for this thread, if one is.
    ///
    /// `None` means no external agent is connected — not that none is
    /// configured. A pin to an external agent that is not connected fails
    /// closed rather than waiting.
    pub external_acp: Option<String>,
    /// The executor registered to serve engine lanes, if one is.
    ///
    /// Separate from [`engine`](Self::engine) on purpose, because they are two
    /// different facts. The engine answering `get_capacity` says a lane exists;
    /// this says Omega has something to hand the turn to. In this build they
    /// come apart: engine lanes are started by a person on the Full Auto
    /// surface and driven by the host bridge, not through
    /// `AgentConnection::prompt`, so the router can *decide* an engine-lane
    /// route it cannot itself dispatch. Modelling that as an input keeps the
    /// gap a stated fallback with a reason instead of a panic or a silent
    /// substitution.
    pub engine_lane: Option<String>,
}

impl RouteInputs {
    /// The inputs of a machine with no engine and no external agent.
    #[must_use]
    pub const fn native_only() -> Self {
        Self {
            pin: None,
            engine: EngineReadiness::Unreachable(EngineUnreachable::NotRunning),
            external_acp: None,
            engine_lane: None,
        }
    }

    /// The same inputs with a pin set.
    #[must_use]
    pub fn pinned(mut self, pin: ExecutorPin) -> Self {
        self.pin = Some(pin);
        self
    }

    /// The same inputs with an engine answer.
    #[must_use]
    pub fn with_engine(mut self, engine: EngineReadiness) -> Self {
        self.engine = engine;
        self
    }

    /// The same inputs with an external ACP agent connected.
    #[must_use]
    pub fn with_external_acp(mut self, agent_id: impl Into<String>) -> Self {
        self.external_acp = Some(agent_id.into());
        self
    }

    /// The same inputs with an executor registered for engine lanes.
    #[must_use]
    pub fn with_engine_lane(mut self, agent_id: impl Into<String>) -> Self {
        self.engine_lane = Some(agent_id.into());
        self
    }
}

// -------------------------------------------------------------------------
// The decision
// -------------------------------------------------------------------------

/// Why an executor was chosen.
///
/// A closed set of typed reasons, not a message. A route the user cannot have
/// explained to them is the same defect class as a handoff with no system note.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RouteReason {
    /// Nothing was pinned, so the thread ran on the native loop. v1 never
    /// routes an unpinned thread anywhere else; see the module docs.
    UnpinnedDefault,
    /// The user pinned this executor and it could serve.
    PinHonored,
    /// An engine lane was pinned and the engine could not be asked.
    EngineUnreachable,
    /// An engine lane was pinned and the engine was at its active-run limit.
    EngineAtCapacity,
    /// An engine lane was pinned without naming one, and the engine declared no
    /// lane that could serve.
    EngineHasNoReadyLane,
    /// A named engine lane was pinned and the engine did not declare it ready.
    PinnedLaneUnavailable,
    /// An external ACP agent was pinned and none is connected.
    ExternalAcpUnavailable,
    /// The engine declared a lane the router could have used, and Omega has no
    /// executor registered to hand the turn to. See
    /// [`RouteInputs::engine_lane`].
    EngineLaneNotConnected,
    /// The lane that would have been chosen carries a reference the decision
    /// record cannot hold. A decision that cannot be written down is not one
    /// the router makes.
    UnrecordableLane,
}

impl RouteReason {
    /// The stable token this reason is recorded under.
    #[must_use]
    pub const fn token(self) -> &'static str {
        match self {
            Self::UnpinnedDefault => "unpinned_default",
            Self::PinHonored => "pin_honored",
            Self::EngineUnreachable => "engine_unreachable",
            Self::EngineAtCapacity => "engine_at_capacity",
            Self::EngineHasNoReadyLane => "engine_has_no_ready_lane",
            Self::PinnedLaneUnavailable => "pinned_lane_unavailable",
            Self::ExternalAcpUnavailable => "external_acp_unavailable",
            Self::EngineLaneNotConnected => "engine_lane_not_connected",
            Self::UnrecordableLane => "unrecordable_lane",
        }
    }

    /// Every admitted reason, in declaration order.
    #[must_use]
    pub const fn all() -> &'static [Self] {
        &[
            Self::UnpinnedDefault,
            Self::PinHonored,
            Self::EngineUnreachable,
            Self::EngineAtCapacity,
            Self::EngineHasNoReadyLane,
            Self::PinnedLaneUnavailable,
            Self::ExternalAcpUnavailable,
            Self::EngineLaneNotConnected,
            Self::UnrecordableLane,
        ]
    }

    /// Read a reason back from [`token`](Self::token).
    #[must_use]
    pub fn parse_token(token: &str) -> Option<Self> {
        Self::all()
            .iter()
            .find(|reason| reason.token() == token)
            .copied()
    }

    /// Whether this reason means a pin could not be honoured.
    ///
    /// Every fail-closed reason lands on the native loop. That is asserted in
    /// [`RouteDecision::is_coherent`] rather than merely intended.
    #[must_use]
    pub const fn is_fallback(self) -> bool {
        match self {
            Self::UnpinnedDefault | Self::PinHonored => false,
            Self::EngineUnreachable
            | Self::EngineAtCapacity
            | Self::EngineHasNoReadyLane
            | Self::PinnedLaneUnavailable
            | Self::ExternalAcpUnavailable
            | Self::EngineLaneNotConnected
            | Self::UnrecordableLane => true,
        }
    }

    /// What the user is told, in their terms.
    ///
    /// Derived on every call. Nothing stores it — the reason is the record, and
    /// this is one rendering of it. A fallback says it fell back, because a
    /// fallback the user cannot see is the defect this packet exists to avoid.
    #[must_use]
    pub const fn phrase(self) -> &'static str {
        match self {
            Self::UnpinnedDefault => "unpinned",
            Self::PinHonored => "pinned",
            Self::EngineUnreachable => "engine unreachable, fell back to the native loop",
            Self::EngineAtCapacity => "engine at capacity, fell back to the native loop",
            Self::EngineHasNoReadyLane => "engine has no ready lane, fell back to the native loop",
            Self::PinnedLaneUnavailable => {
                "pinned engine lane unavailable, fell back to the native loop"
            }
            Self::ExternalAcpUnavailable => {
                "pinned external agent not connected, fell back to the native loop"
            }
            Self::EngineLaneNotConnected => {
                "no executor is connected for engine lanes, fell back to the native loop"
            }
            Self::UnrecordableLane => {
                "engine lane could not be recorded, fell back to the native loop"
            }
        }
    }
}

/// One routing decision, in full.
///
/// Durable and inspectable through [`canonical_record`](Self::canonical_record),
/// which round-trips. It carries no timestamp on purpose: a clock in the record
/// would make two identical decisions look different, and would put a
/// non-deterministic value in the decision path this packet exists to keep
/// reproducible.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RouteDecision {
    /// The class the turn was dispatched to.
    pub chosen: ExecutorClass,
    /// Why.
    pub reason: RouteReason,
    /// What was pinned when the decision was made, if anything. Kept even when
    /// the pin could not be honoured — an unhonoured pin the record forgot is
    /// indistinguishable from no pin at all.
    pub pin: Option<ExecutorPin>,
    /// The engine lane the turn was dispatched to.
    ///
    /// `Some` exactly when [`chosen`](Self::chosen) is
    /// [`ExecutorClass::EngineLane`].
    pub lane_ref: Option<String>,
}

/// Characters the canonical record escapes, and the escape character itself.
///
/// The record is a flat `key=value;…` line in which a pin may name a lane after
/// an `@`, so these three would otherwise give it a second reading. `%` is
/// escaped too, which is what makes the encoding a bijection rather than merely
/// a substitution — without it, a literal `%3B` in a lane reference and an
/// escaped `;` would decode to the same thing.
pub const RESERVED_RECORD_CHARACTERS: &[char] = &['%', ';', '=', '@'];

/// Write one field of the canonical record.
///
/// Percent-escapes the four reserved characters and nothing else, so a lane
/// reference such as `acp:cursor-agent` is written verbatim and stays legible
/// to a person reading the journal.
#[must_use]
pub fn encode_field(value: &str) -> String {
    let mut encoded = String::with_capacity(value.len());
    for character in value.chars() {
        match character {
            '%' => encoded.push_str("%25"),
            ';' => encoded.push_str("%3B"),
            '=' => encoded.push_str("%3D"),
            '@' => encoded.push_str("%40"),
            other => encoded.push(other),
        }
    }
    encoded
}

/// Read one field of the canonical record.
///
/// Returns `None` for a truncated or unknown escape rather than guessing, so a
/// half-written journal entry is rejected instead of decoded into something
/// nobody wrote.
#[must_use]
pub fn decode_field(value: &str) -> Option<String> {
    let mut decoded = String::with_capacity(value.len());
    let mut characters = value.chars();
    while let Some(character) = characters.next() {
        if character != '%' {
            decoded.push(character);
            continue;
        }
        let escape: String = characters.by_ref().take(2).collect();
        match escape.as_str() {
            "25" => decoded.push('%'),
            "3B" => decoded.push(';'),
            "3D" => decoded.push('='),
            "40" => decoded.push('@'),
            _ => return None,
        }
    }
    Some(decoded)
}

/// Whether a lane reference can be routed to.
///
/// Every non-empty reference can: the record escapes anything that would make
/// it ambiguous. An *empty* one cannot, and that is not a cosmetic problem —
/// there is nothing to dispatch to and nothing to write down. The router refuses
/// it rather than routing into a lane it cannot name.
#[must_use]
pub fn lane_ref_is_recordable(lane_ref: &str) -> bool {
    !lane_ref.is_empty()
}

impl RouteDecision {
    /// A decision that landed on the native loop.
    fn native(reason: RouteReason, pin: Option<ExecutorPin>) -> Self {
        Self {
            chosen: ExecutorClass::NativeLoop,
            reason,
            pin,
            lane_ref: None,
        }
    }

    /// Whether this decision is internally consistent.
    ///
    /// Every clause here is a way the router could have lied about itself: a
    /// fallback that did not land on the native loop, a lane reference on a
    /// non-engine route, an honoured pin that names a different class from the
    /// one that ran.
    #[must_use]
    pub fn is_coherent(&self) -> bool {
        let lane_matches_class = match self.chosen {
            ExecutorClass::EngineLane => self
                .lane_ref
                .as_deref()
                .is_some_and(lane_ref_is_recordable),
            ExecutorClass::NativeLoop | ExecutorClass::ExternalAcp => self.lane_ref.is_none(),
        };
        let fallback_lands_native = !self.reason.is_fallback()
            || (self.chosen == ExecutorClass::NativeLoop && self.pin.is_some());
        let unpinned_is_unpinned =
            self.reason != RouteReason::UnpinnedDefault || self.pin.is_none();
        let honoured_pin_names_the_class = self.reason != RouteReason::PinHonored
            || self
                .pin
                .as_ref()
                .is_some_and(|pin| pin.class == self.chosen);

        lane_matches_class
            && fallback_lands_native
            && unpinned_is_unpinned
            && honoured_pin_names_the_class
    }

    /// The typed reason this decision contributes to a thread's disclosure.
    #[must_use]
    pub const fn disclosed_route(&self) -> RouteReason {
        self.reason
    }

    /// The durable form. Deterministic, total, and round-trippable.
    ///
    /// Written to the route journal verbatim. An empty value is the absent
    /// marker, which is unambiguous because no pin token and no routable lane
    /// reference is ever empty — a sentinel such as `-` would collide with a
    /// lane genuinely named `-`, and escaping `-` would make `claude-local`
    /// unreadable.
    #[must_use]
    pub fn canonical_record(&self) -> String {
        let pin = match &self.pin {
            Some(pin) => pin.token(),
            None => String::new(),
        };
        let lane = match &self.lane_ref {
            Some(lane_ref) => encode_field(lane_ref),
            None => String::new(),
        };
        format!(
            "chosen={};reason={};pin={pin};lane={lane}",
            self.chosen.token(),
            self.reason.token(),
        )
    }

    /// Read a decision back from [`canonical_record`](Self::canonical_record).
    ///
    /// Returns `None` for anything it cannot read *or* for a record that is
    /// readable but incoherent, so a hand-edited or half-written journal entry
    /// is rejected rather than believed. Every one of the four keys must be
    /// present exactly once: a partial record is a truncated write, and reading
    /// one as a decision would invent the missing parts.
    #[must_use]
    pub fn parse_canonical_record(record: &str) -> Option<Self> {
        let mut chosen = None;
        let mut reason = None;
        let mut pin = None;
        let mut lane = None;
        let mut keys = 0usize;
        for field in record.split(';') {
            let (key, value) = field.split_once('=')?;
            keys += 1;
            match key {
                "chosen" if chosen.is_none() => {
                    chosen = Some(
                        *ExecutorClass::all()
                            .iter()
                            .find(|class| class.token() == value)?,
                    );
                }
                "reason" if reason.is_none() => reason = Some(RouteReason::parse_token(value)?),
                "pin" if pin.is_none() => {
                    pin = Some(if value.is_empty() {
                        None
                    } else {
                        Some(ExecutorPin::parse_token(value)?)
                    });
                }
                "lane" if lane.is_none() => {
                    lane = Some(if value.is_empty() {
                        None
                    } else {
                        Some(decode_field(value)?)
                    });
                }
                _ => return None,
            }
        }
        if keys != 4 {
            return None;
        }
        let decision = Self {
            chosen: chosen?,
            reason: reason?,
            pin: pin?,
            lane_ref: lane?,
        };
        decision.is_coherent().then_some(decision)
    }

    /// One sentence explaining the decision from its parts.
    ///
    /// Derived, never stored. Used in logs and in the inspector.
    #[must_use]
    pub fn explain(&self) -> String {
        let mut line = format!("{} ({})", self.chosen.token(), self.reason.phrase());
        if let Some(lane_ref) = &self.lane_ref {
            line.push_str(" on ");
            line.push_str(lane_ref);
        }
        if let Some(pin) = &self.pin {
            line.push_str("; pin ");
            line.push_str(&pin.token());
            if self.reason.is_fallback() {
                line.push_str(" could not be honoured");
            }
        }
        line
    }
}

// -------------------------------------------------------------------------
// The decision itself
// -------------------------------------------------------------------------

/// Choose the lane a decision would use.
///
/// Two rules, both total:
///
/// * A pin that names a lane matches that lane and no other, and only when the
///   engine declared it able to serve.
/// * A pin that names no lane takes the **lexicographically smallest** lane
///   able to serve. Not the first the engine listed: the engine's array order
///   is not a stable input, so trusting it would make the same capacity answer
///   route two ways on two runs. A total order on the reference is the cheapest
///   thing that cannot do that.
#[must_use]
pub fn select_lane(lanes: &[EngineLane], pinned: Option<&str>) -> Option<String> {
    match pinned {
        Some(pinned) => lanes
            .iter()
            .find(|lane| lane.lane_ref == pinned && lane.state.can_serve())
            .map(|lane| lane.lane_ref.clone()),
        None => lanes
            .iter()
            .filter(|lane| lane.state.can_serve())
            .map(|lane| lane.lane_ref.as_str())
            .min()
            .map(str::to_owned),
    }
}

/// Decide where a turn runs.
///
/// The whole routing law. Pure: same inputs, same decision, every time.
#[must_use]
pub fn route(inputs: &RouteInputs) -> RouteDecision {
    let Some(pin) = inputs.pin.clone() else {
        // Nothing pinned. The native loop, always — see the module docs on
        // owner gate 8.
        return RouteDecision::native(RouteReason::UnpinnedDefault, None);
    };

    match pin.class {
        // Pinning the first-party loop is always honourable: it is the
        // fail-closed target, so it is available whenever Omega is running.
        ExecutorClass::NativeLoop => RouteDecision {
            chosen: ExecutorClass::NativeLoop,
            reason: RouteReason::PinHonored,
            pin: Some(pin),
            lane_ref: None,
        },

        ExecutorClass::ExternalAcp => {
            if inputs.external_acp.is_some() {
                RouteDecision {
                    chosen: ExecutorClass::ExternalAcp,
                    reason: RouteReason::PinHonored,
                    pin: Some(pin),
                    lane_ref: None,
                }
            } else {
                RouteDecision::native(RouteReason::ExternalAcpUnavailable, Some(pin))
            }
        }

        ExecutorClass::EngineLane => match &inputs.engine {
            // Engine down. Fail closed to the native loop, and say which way it
            // was down: `EngineUnreachable` is one reason, but the cause it
            // carries is what an operator needs.
            EngineReadiness::Unreachable(_) => {
                RouteDecision::native(RouteReason::EngineUnreachable, Some(pin))
            }
            EngineReadiness::Answered {
                active_run_count,
                active_run_limit,
                lanes,
            } => {
                if active_run_count >= active_run_limit {
                    return RouteDecision::native(RouteReason::EngineAtCapacity, Some(pin));
                }
                match select_lane(lanes, pin.lane_ref.as_deref()) {
                    Some(lane_ref) if !lane_ref_is_recordable(&lane_ref) => {
                        RouteDecision::native(RouteReason::UnrecordableLane, Some(pin))
                    }
                    // The lane exists and the engine says it is ready. Whether
                    // Omega can hand a turn to it is a separate fact, checked
                    // last so an operator hears about the engine first.
                    Some(_) if inputs.engine_lane.is_none() => {
                        RouteDecision::native(RouteReason::EngineLaneNotConnected, Some(pin))
                    }
                    Some(lane_ref) => RouteDecision {
                        chosen: ExecutorClass::EngineLane,
                        reason: RouteReason::PinHonored,
                        pin: Some(pin),
                        lane_ref: Some(lane_ref),
                    },
                    None if pin.lane_ref.is_some() => {
                        RouteDecision::native(RouteReason::PinnedLaneUnavailable, Some(pin))
                    }
                    None => RouteDecision::native(RouteReason::EngineHasNoReadyLane, Some(pin)),
                }
            }
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ready_engine() -> EngineReadiness {
        EngineReadiness::Answered {
            active_run_count: 1,
            active_run_limit: 8,
            lanes: vec![
                EngineLane::new("codex-local", LaneState::Busy),
                EngineLane::new("claude-local", LaneState::Available),
                EngineLane::new("acp:cursor-agent", LaneState::Available),
            ],
        }
    }

    // ---------------------------------------------------------------------
    // Exit property 1: pins honoured
    // ---------------------------------------------------------------------

    /// An explicitly pinned, available executor is used. Every time, for every
    /// class, regardless of what else is ready.
    ///
    /// Falsified by making `route` prefer an available engine lane over a
    /// native pin: this test fails on the native case.
    #[test]
    fn a_pin_to_an_available_executor_is_always_used() {
        let base = RouteInputs::native_only()
            .with_engine(ready_engine())
            .with_external_acp("codex-acp")
            .with_engine_lane("codex-local");

        let native = route(&base.clone().pinned(ExecutorPin::new(ExecutorClass::NativeLoop)));
        assert_eq!(native.chosen, ExecutorClass::NativeLoop);
        assert_eq!(native.reason, RouteReason::PinHonored);

        let external = route(
            &base
                .clone()
                .pinned(ExecutorPin::new(ExecutorClass::ExternalAcp)),
        );
        assert_eq!(external.chosen, ExecutorClass::ExternalAcp);
        assert_eq!(external.reason, RouteReason::PinHonored);

        let lane = route(&base.pinned(ExecutorPin::on_lane("claude-local")));
        assert_eq!(lane.chosen, ExecutorClass::EngineLane);
        assert_eq!(lane.reason, RouteReason::PinHonored);
        assert_eq!(lane.lane_ref.as_deref(), Some("claude-local"));

        for decision in [native, external, lane] {
            assert!(decision.is_coherent(), "{decision:?}");
        }
    }

    /// A pin outranks everything the engine is offering. The engine being ready
    /// and idle is not a reason to move a turn the user placed.
    #[test]
    fn a_ready_engine_never_takes_a_thread_away_from_its_pin() {
        let inputs = RouteInputs::native_only()
            .with_engine(ready_engine())
            .with_external_acp("codex-acp")
            .pinned(ExecutorPin::new(ExecutorClass::NativeLoop));
        assert_eq!(route(&inputs).chosen, ExecutorClass::NativeLoop);
        assert!(route(&inputs).lane_ref.is_none());
    }

    /// Owner gate 8, as a routing law: nothing but a pin reaches an engine
    /// lane, so no model-initiated path can start Full Auto authority through
    /// the router.
    #[test]
    fn an_unpinned_thread_never_reaches_an_engine_lane() {
        let inputs = RouteInputs::native_only()
            .with_engine(ready_engine())
            .with_external_acp("codex-acp")
            // Everything an engine-lane route needs is present, so the native
            // answer below is a decision rather than an absence.
            .with_engine_lane("codex-local");
        assert_eq!(
            route(&inputs.clone().pinned(ExecutorPin::on_lane("claude-local"))).chosen,
            ExecutorClass::EngineLane,
            "an engine lane is reachable from these inputs, so the unpinned \
             case below is not passing by default"
        );
        let decision = route(&inputs);
        assert_eq!(decision.chosen, ExecutorClass::NativeLoop);
        assert_eq!(decision.reason, RouteReason::UnpinnedDefault);
        assert!(decision.lane_ref.is_none());
        assert!(decision.pin.is_none());
        assert!(decision.is_coherent());
    }

    // ---------------------------------------------------------------------
    // Exit property 2: engine-down fails closed to native, visibly
    // ---------------------------------------------------------------------

    /// Every way the engine can be unavailable lands on the native loop, keeps
    /// the pin it could not honour, and says so with a fallback reason.
    ///
    /// Falsified by returning `EngineLane` from the `Unreachable` arm: this
    /// test fails on `chosen`, and `a_fallback_is_never_silent` fails too.
    #[test]
    fn an_unavailable_engine_falls_closed_to_the_native_loop() {
        let pin = ExecutorPin::on_lane("claude-local");

        let mut cases: Vec<(EngineReadiness, RouteReason)> = EngineUnreachable::all()
            .iter()
            .map(|cause| {
                (
                    EngineReadiness::Unreachable(*cause),
                    RouteReason::EngineUnreachable,
                )
            })
            .collect();
        cases.push((
            EngineReadiness::Answered {
                active_run_count: 8,
                active_run_limit: 8,
                lanes: vec![EngineLane::new("claude-local", LaneState::Available)],
            },
            RouteReason::EngineAtCapacity,
        ));
        cases.push((
            EngineReadiness::Answered {
                active_run_count: 0,
                active_run_limit: 8,
                lanes: vec![EngineLane::new("claude-local", LaneState::Busy)],
            },
            RouteReason::PinnedLaneUnavailable,
        ));
        cases.push((
            EngineReadiness::Answered {
                active_run_count: 0,
                active_run_limit: 8,
                lanes: vec![],
            },
            RouteReason::PinnedLaneUnavailable,
        ));

        for (engine, expected) in cases {
            let decision = route(&RouteInputs::native_only().with_engine(engine.clone()).pinned(pin.clone()));
            assert_eq!(
                decision.chosen,
                ExecutorClass::NativeLoop,
                "engine {engine:?} did not fail closed"
            );
            assert_eq!(decision.reason, expected, "engine {engine:?}");
            assert!(decision.reason.is_fallback());
            assert_eq!(
                decision.pin.as_ref(),
                Some(&pin),
                "a fallback that forgets the pin it could not honour is \
                 indistinguishable from an unpinned thread"
            );
            assert!(decision.is_coherent(), "{decision:?}");
        }
    }

    /// An engine lane pinned without naming one, with nothing ready, is a
    /// different fallback from a named lane that is busy — and the record says
    /// which.
    #[test]
    fn an_engine_with_no_ready_lane_is_its_own_reason() {
        let decision = route(
            &RouteInputs::native_only()
                .with_engine(EngineReadiness::Answered {
                    active_run_count: 0,
                    active_run_limit: 8,
                    lanes: vec![EngineLane::new("codex-local", LaneState::Busy)],
                })
                .pinned(ExecutorPin::new(ExecutorClass::EngineLane)),
        );
        assert_eq!(decision.reason, RouteReason::EngineHasNoReadyLane);
        assert_eq!(decision.chosen, ExecutorClass::NativeLoop);
    }

    /// A pinned external agent that is not connected fails closed the same way
    /// the engine does, rather than waiting or erroring.
    #[test]
    fn a_disconnected_external_agent_falls_closed_to_the_native_loop() {
        let decision = route(
            &RouteInputs::native_only().pinned(ExecutorPin::new(ExecutorClass::ExternalAcp)),
        );
        assert_eq!(decision.chosen, ExecutorClass::NativeLoop);
        assert_eq!(decision.reason, RouteReason::ExternalAcpUnavailable);
        assert!(decision.is_coherent());
    }

    /// A ready engine with nothing to dispatch onto is its own fallback, and
    /// the operator hears about the engine first.
    ///
    /// This is the gap between "the engine has a lane" and "Omega can hand this
    /// turn to it". In this build they come apart, and the honest answer is a
    /// named reason rather than a panic, a hang, or a substitution nobody sees.
    #[test]
    fn a_ready_lane_with_no_executor_behind_it_falls_closed() {
        let decision = route(
            &RouteInputs::native_only()
                .with_engine(ready_engine())
                .pinned(ExecutorPin::on_lane("claude-local")),
        );
        assert_eq!(decision.chosen, ExecutorClass::NativeLoop);
        assert_eq!(decision.reason, RouteReason::EngineLaneNotConnected);
        assert!(decision.is_coherent());

        // The engine being down outranks it: an operator needs the runtime
        // fact before the wiring fact.
        let engine_down = route(
            &RouteInputs::native_only()
                .with_engine(EngineReadiness::Unreachable(EngineUnreachable::Timeout))
                .pinned(ExecutorPin::on_lane("claude-local")),
        );
        assert_eq!(engine_down.reason, RouteReason::EngineUnreachable);
    }

    /// A fallback is always disclosable, because every fallback reason renders
    /// a phrase that says it fell back.
    ///
    /// A fallback the user cannot see is the same defect class as a handoff
    /// with no system note, which shipped in rc11.
    #[test]
    fn a_fallback_is_never_silent() {
        for reason in RouteReason::all() {
            if !reason.is_fallback() {
                continue;
            }
            assert!(
                reason.phrase().contains("fell back to the native loop"),
                "{} renders as {:?}, which does not tell the reader a pin was \
                 not honoured",
                reason.token(),
                reason.phrase()
            );
        }
    }

    /// An unrecognised lane state is not a ready lane.
    #[test]
    fn an_unrecognised_lane_state_is_not_available() {
        assert_eq!(LaneState::parse("available"), LaneState::Available);
        assert_eq!(LaneState::parse("busy"), LaneState::Busy);
        assert_eq!(LaneState::parse("draining"), LaneState::Unavailable);
        assert!(!LaneState::parse("draining").can_serve());

        let decision = route(
            &RouteInputs::native_only()
                .with_engine(EngineReadiness::Answered {
                    active_run_count: 0,
                    active_run_limit: 8,
                    lanes: vec![EngineLane::new("codex-local", LaneState::parse("draining"))],
                })
                .pinned(ExecutorPin::on_lane("codex-local")),
        );
        assert_eq!(decision.chosen, ExecutorClass::NativeLoop);
    }

    // ---------------------------------------------------------------------
    // Determinism
    // ---------------------------------------------------------------------

    /// The same inputs give the same route, whatever order the engine listed
    /// its lanes in.
    ///
    /// The engine's array order is not a stable input: the same logical
    /// capacity can come back ordered differently. A router that took "the
    /// first available lane" would route the same thread two ways on two runs
    /// and nothing would be wrong with either.
    ///
    /// Falsified by replacing `.min()` with `.next()` in `select_lane`: this
    /// test fails on the reversed permutation.
    #[test]
    fn routing_is_invariant_under_lane_order() {
        let lanes = vec![
            EngineLane::new("harness:opencode", LaneState::Available),
            EngineLane::new("claude-local", LaneState::Available),
            EngineLane::new("acp:cursor-agent", LaneState::Available),
            EngineLane::new("codex-local", LaneState::Busy),
        ];
        let pin = ExecutorPin::new(ExecutorClass::EngineLane);
        let connected = RouteInputs::native_only().with_engine_lane("codex-local");

        let mut permutations = vec![lanes.clone()];
        let mut reversed = lanes.clone();
        reversed.reverse();
        permutations.push(reversed);
        let mut rotated = lanes.clone();
        rotated.rotate_left(2);
        permutations.push(rotated);
        let mut swapped = lanes;
        swapped.swap(0, 1);
        permutations.push(swapped);

        let expected = RouteDecision {
            chosen: ExecutorClass::EngineLane,
            reason: RouteReason::PinHonored,
            pin: Some(pin.clone()),
            // Lexicographically smallest available lane. `codex-local` sorts
            // after it but is busy; `harness:opencode` sorts after it too.
            lane_ref: Some("acp:cursor-agent".to_owned()),
        };

        for lanes in permutations {
            let decision = route(
                &connected
                    .clone()
                    .with_engine(EngineReadiness::Answered {
                        active_run_count: 0,
                        active_run_limit: 8,
                        lanes: lanes.clone(),
                    })
                    .pinned(pin.clone()),
            );
            assert_eq!(decision, expected, "lane order {lanes:?} changed the route");
        }
    }

    /// Routing the same inputs repeatedly gives byte-identical records.
    ///
    /// This is the property a clock in the record would break, which is why
    /// there is not one.
    #[test]
    fn the_same_inputs_give_the_same_record_every_time() {
        let inputs = RouteInputs::native_only()
            .with_engine(ready_engine())
            .with_engine_lane("codex-local")
            .pinned(ExecutorPin::on_lane("claude-local"));
        let first = route(&inputs).canonical_record();
        for _ in 0..64 {
            assert_eq!(route(&inputs).canonical_record(), first);
        }
        assert_eq!(
            first,
            "chosen=engine_lane;reason=pin_honored;pin=engine_lane@claude-local;lane=claude-local"
        );
    }

    // ---------------------------------------------------------------------
    // Exit property 3: the decision is recorded
    // ---------------------------------------------------------------------

    /// Every decision the router can reach survives being written down and read
    /// back.
    ///
    /// Falsified by dropping the `pin` field from `canonical_record`: this test
    /// fails on every fallback, because the pin comes back `None`.
    #[test]
    fn every_reachable_decision_round_trips_through_its_record() {
        let engines = [
            EngineReadiness::Unreachable(EngineUnreachable::NotRunning),
            EngineReadiness::Unreachable(EngineUnreachable::Timeout),
            EngineReadiness::Unreachable(EngineUnreachable::ProtocolError),
            EngineReadiness::Answered {
                active_run_count: 8,
                active_run_limit: 8,
                lanes: vec![],
            },
            ready_engine(),
            EngineReadiness::Answered {
                active_run_count: 0,
                active_run_limit: 8,
                lanes: vec![EngineLane::new("codex-local", LaneState::Busy)],
            },
            EngineReadiness::Answered {
                active_run_count: 0,
                active_run_limit: 8,
                lanes: vec![EngineLane::new("", LaneState::Available)],
            },
        ];
        let pins = [
            None,
            Some(ExecutorPin::new(ExecutorClass::NativeLoop)),
            Some(ExecutorPin::new(ExecutorClass::ExternalAcp)),
            Some(ExecutorPin::new(ExecutorClass::EngineLane)),
            Some(ExecutorPin::on_lane("claude-local")),
            Some(ExecutorPin::on_lane("a;b=c@d")),
        ];

        let mut seen_reasons = Vec::new();
        for engine in &engines {
            for pin in &pins {
                for (external, engine_lane) in [
                    (None, None),
                    (Some("codex-acp".to_owned()), None),
                    (None, Some("codex-local".to_owned())),
                    (Some("codex-acp".to_owned()), Some("codex-local".to_owned())),
                ] {
                    let inputs = RouteInputs {
                        pin: pin.clone(),
                        engine: engine.clone(),
                        external_acp: external,
                        engine_lane,
                    };
                    let decision = route(&inputs);
                    assert!(decision.is_coherent(), "{decision:?} from {inputs:?}");

                    let record = decision.canonical_record();
                    assert_eq!(
                        RouteDecision::parse_canonical_record(&record).as_ref(),
                        Some(&decision),
                        "record {record:?} did not read back"
                    );
                    if !seen_reasons.contains(&decision.reason) {
                        seen_reasons.push(decision.reason);
                    }
                }
            }
        }

        // A round-trip suite that never reaches a reason proves nothing about
        // it. Every admitted reason has to appear.
        for reason in RouteReason::all() {
            assert!(
                seen_reasons.contains(reason),
                "no input in this suite produced {}; the round-trip check is \
                 vacuous for it",
                reason.token()
            );
        }
    }

    /// A lane whose reference contains a character the record uses structurally
    /// is still routable, because the record escapes it and reads it back
    /// unchanged.
    ///
    /// The first draft of this packet refused such lanes instead. The
    /// round-trip suite falsified that immediately: an *unhonoured pin* can
    /// carry the same reference, and a refusal at the routing step does not
    /// stop it reaching the record, so a decision the router really made could
    /// not be written down. Escaping is total; refusal was not.
    #[test]
    fn a_lane_with_a_structural_character_still_round_trips() {
        for lane_ref in ["bad;lane", "bad=lane", "bad@lane", "100%lane", "-"] {
            assert!(lane_ref_is_recordable(lane_ref), "{lane_ref:?}");

            let decision = route(
                &RouteInputs::native_only()
                    .with_engine_lane("codex-local")
                    .with_engine(EngineReadiness::Answered {
                        active_run_count: 0,
                        active_run_limit: 8,
                        lanes: vec![EngineLane::new(lane_ref, LaneState::Available)],
                    })
                    .pinned(ExecutorPin::on_lane(lane_ref)),
            );
            assert_eq!(decision.chosen, ExecutorClass::EngineLane, "{lane_ref:?}");
            assert_eq!(decision.lane_ref.as_deref(), Some(lane_ref));

            let record = decision.canonical_record();
            assert_eq!(record.split(';').count(), 4, "{record:?} lost its shape");
            assert_eq!(
                RouteDecision::parse_canonical_record(&record).as_ref(),
                Some(&decision),
                "{record:?}"
            );
        }
    }

    /// A lane with no name cannot be dispatched to and cannot be written down,
    /// so the router refuses it instead of routing into a lane it cannot name.
    #[test]
    fn a_nameless_lane_is_not_routed_to() {
        assert!(!lane_ref_is_recordable(""));
        let decision = route(
            &RouteInputs::native_only()
                .with_engine(EngineReadiness::Answered {
                    active_run_count: 0,
                    active_run_limit: 8,
                    lanes: vec![EngineLane::new("", LaneState::Available)],
                })
                .pinned(ExecutorPin::new(ExecutorClass::EngineLane)),
        );
        assert_eq!(decision.chosen, ExecutorClass::NativeLoop);
        assert_eq!(decision.reason, RouteReason::UnrecordableLane);
        assert!(decision.is_coherent());
    }

    /// The field encoding is a bijection, which is the property that stops an
    /// escaped `;` and a literal `%3B` decoding to the same lane.
    #[test]
    fn the_field_encoding_round_trips_and_rejects_bad_escapes() {
        for value in [
            "claude-local",
            "acp:cursor-agent",
            "a;b=c@d",
            "%3B",
            "%",
            "%%",
            "",
        ] {
            assert_eq!(
                decode_field(&encode_field(value)).as_deref(),
                Some(value),
                "{value:?}"
            );
        }
        assert!(encode_field("%3B") != encode_field(";"));
        for broken in ["%", "%3", "%zz", "%3b"] {
            assert!(decode_field(broken).is_none(), "{broken:?}");
        }
    }

    /// A record that reads cleanly but describes an impossible decision is
    /// rejected, not believed.
    #[test]
    fn an_incoherent_record_does_not_read_back() {
        // A fallback that claims an engine lane.
        assert!(
            RouteDecision::parse_canonical_record(
                "chosen=engine_lane;reason=engine_unreachable;pin=engine_lane;lane=claude-local"
            )
            .is_none()
        );
        // An honoured pin that names a different class from the one that ran.
        assert!(
            RouteDecision::parse_canonical_record(
                "chosen=native_loop;reason=pin_honored;pin=external_acp;lane="
            )
            .is_none()
        );
        // An unpinned decision that carries a pin.
        assert!(
            RouteDecision::parse_canonical_record(
                "chosen=native_loop;reason=unpinned_default;pin=native_loop;lane="
            )
            .is_none()
        );
        // A partial record, as a truncated write leaves behind.
        assert!(
            RouteDecision::parse_canonical_record("chosen=native_loop;reason=unpinned_default")
                .is_none()
        );
        // A duplicated key, where the second value would silently win.
        assert!(
            RouteDecision::parse_canonical_record(
                "chosen=native_loop;chosen=engine_lane;reason=unpinned_default;pin=;lane="
            )
            .is_none()
        );
        // A native route carrying a lane.
        assert!(
            RouteDecision::parse_canonical_record(
                "chosen=native_loop;reason=pin_honored;pin=native_loop;lane=claude-local"
            )
            .is_none()
        );
        // Junk.
        assert!(RouteDecision::parse_canonical_record("chosen=wat;reason=pin_honored").is_none());
        assert!(RouteDecision::parse_canonical_record("").is_none());
    }

    /// The record holds no rendered explanation, only parts.
    ///
    /// Same law as `ExecutorDisclosure`: a record of parts can be re-rendered,
    /// re-signed, or re-read by a later reader. A stored sentence cannot.
    #[test]
    fn the_decision_record_holds_no_rendered_explanation() {
        let decision = route(&RouteInputs::native_only());
        let dumped = format!("{decision:?}");
        for caption in ["explain", "label", "line", "text", "summary", "message"] {
            assert!(
                !dumped.contains(caption),
                "RouteDecision grew a `{caption}` field: {dumped}"
            );
        }
        assert!(!decision.canonical_record().contains(' '));
    }

    /// The explanation is derived from the parts, and names both the pin and
    /// the fact it was not honoured.
    #[test]
    fn a_fallback_explains_itself_from_its_parts() {
        let decision = route(
            &RouteInputs::native_only()
                .with_engine(EngineReadiness::Unreachable(EngineUnreachable::NotRunning))
                .pinned(ExecutorPin::on_lane("claude-local")),
        );
        let explanation = decision.explain();
        assert!(explanation.contains("native_loop"), "{explanation}");
        assert!(explanation.contains("engine unreachable"), "{explanation}");
        assert!(
            explanation.contains("engine_lane@claude-local"),
            "{explanation}"
        );
        assert!(
            explanation.contains("could not be honoured"),
            "{explanation}"
        );
    }

    /// Pin tokens round-trip, including the lane a pin names.
    #[test]
    fn pin_tokens_round_trip() {
        for pin in [
            ExecutorPin::new(ExecutorClass::NativeLoop),
            ExecutorPin::new(ExecutorClass::ExternalAcp),
            ExecutorPin::new(ExecutorClass::EngineLane),
            ExecutorPin::on_lane("acp:cursor-agent"),
        ] {
            assert_eq!(ExecutorPin::parse_token(&pin.token()).as_ref(), Some(&pin));
        }
        assert!(ExecutorPin::parse_token("zed_agent").is_none());
    }

    /// The reason set is closed, and every reason has a distinct token.
    #[test]
    fn the_reason_set_is_closed_and_distinct() {
        let mut tokens: Vec<&str> = RouteReason::all().iter().map(|r| r.token()).collect();
        let count = tokens.len();
        tokens.sort_unstable();
        tokens.dedup();
        assert_eq!(tokens.len(), count, "two reasons share a token");
        assert_eq!(
            count,
            9,
            "the reason set changed. Every reason is a thing the router can \
             tell a user; adding one is a deliberate edit, and removing one \
             means a route it used to explain is now unexplained."
        );
    }
}
