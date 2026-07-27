//! Omega Agent, the router. `OMEGA-DELTA-0029`, omega#78.
//!
//! The owner admitted Omega Agent on omega#74 as a **router** that owns
//! routing, disclosure, and receipts and owns **no execution**, sitting at the
//! `AgentConnection` seam above three executor classes: the native agent loop,
//! external ACP agents, and `omega-effectd` engine lanes.
//!
//! This module is the dispatch half of that. The decision half is
//! [`omega_front_door::router`], a leaf with no dependencies, so the routing law
//! can be checked without building GPUI and so no decision is ever made inside
//! a widget.
//!
//! # The router owns no execution
//!
//! [`OmegaAgentConnection`] implements every method of [`AgentConnection`] by
//! handing it to the executor the decision names. There is no turn loop here,
//! no tool call, no model call, and no session state beyond the pin and the
//! decision. `the_router_delegates_every_agent_connection_method` in
//! `crates/omega_deltas` reads this file and fails if a method body stops
//! delegating, because "owns no execution" is a property of the source, not of
//! its author's intent.
//!
//! A thread the router creates carries the **executor's** connection, not the
//! router's, because the executor is what built it. That is deliberate:
//! omega#77's disclosure classifies a thread by downcasting its connection, so
//! a thread that carried the router would disclose the router as its executor —
//! which is exactly the first-party attribution claim omega#77 exists to stop.
//!
//! # `omega-effectd` stays the sole run authority
//!
//! [`EngineReadiness`] here is *read* from a `get_capacity` answer. Nothing is
//! written back, nothing is cached as run state, and the router never starts,
//! pauses, or stops a run. Starting engine-lane authority remains what owner
//! gate 8 requires: an explicit human action on a visible control, which after
//! omega#76 is the Start button on the Full Auto surface.
//!
//! # The decision is recorded
//!
//! Every decision is written to a route journal on disk keyed by session, in
//! [`omega_front_door::RouteDecision::canonical_record`] form, which round-trips.
//! The journal carries no clock: a timestamp would make two identical decisions
//! look different and would put a non-deterministic value beside a decision path
//! whose whole point is that it is reproducible.

use std::any::Any;
use std::cell::RefCell;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::rc::{Rc, Weak};
use std::sync::{LazyLock, Mutex};

use acp_thread::{
    AcpThread, AgentConnection, AgentModelSelector, AgentSessionClientUserMessageIds,
    AgentSessionConfigOptions, AgentSessionList, AgentSessionModes, AgentSessionRetry,
    AgentSessionSetTitle, AgentSessionTruncate, AgentTelemetry, ElicitationStore,
};
use agent_client_protocol::schema::v1 as acp;
use anyhow::Result;
use gpui::{App, Entity, SharedString, Task};
use omega_front_door::{
    EngineLane, EngineReadiness, EngineUnreachable, ExecutorClass, ExecutorPin, LaneState,
    PinGesture, RouteDecision, RouteInputs, RouteReason, route,
};
use project::{AgentId, Project};
use serde_json::Value;
use util::path_list::PathList;

/// The schema the route journal is written under.
const ROUTE_JOURNAL_SCHEMA: &str = "openagents.omega.agent_route_journal.v1";

/// The route journal's file name, under the Omega data directory.
const ROUTE_JOURNAL_FILE: &str = "agent-route-journal.json";

// -------------------------------------------------------------------------
// Reading the engine's answer
// -------------------------------------------------------------------------

/// Read a framed `get_capacity` answer into the typed readiness the router
/// takes.
///
/// Anything missing or misshapen reads as
/// [`EngineUnreachable::ProtocolError`] rather than as a partially trusted
/// answer. An engine whose capacity record this build cannot understand is an
/// engine this build must not route onto: believing half of it is how a router
/// dispatches into a lane on the strength of not understanding the other half.
#[must_use]
pub fn engine_readiness_from_capacity(capacity: &Value) -> EngineReadiness {
    let protocol_error = EngineReadiness::Unreachable(EngineUnreachable::ProtocolError);

    let Some(object) = capacity.as_object() else {
        return protocol_error;
    };
    let (Some(active_run_count), Some(active_run_limit)) = (
        object.get("activeRunCount").and_then(Value::as_u64),
        object.get("activeRunLimit").and_then(Value::as_u64),
    ) else {
        return protocol_error;
    };
    let Some(lanes) = object.get("lanes").and_then(Value::as_array) else {
        return protocol_error;
    };

    let mut parsed = Vec::with_capacity(lanes.len());
    for lane in lanes {
        let (Some(lane_ref), Some(state)) = (
            lane.get("lane").and_then(Value::as_str),
            lane.get("state").and_then(Value::as_str),
        ) else {
            return protocol_error;
        };
        parsed.push(EngineLane::new(lane_ref, LaneState::parse(state)));
    }

    EngineReadiness::Answered {
        active_run_count: u32::try_from(active_run_count).unwrap_or(u32::MAX),
        active_run_limit: u32::try_from(active_run_limit).unwrap_or(u32::MAX),
        lanes: parsed,
    }
}

// -------------------------------------------------------------------------
// The durable record
// -------------------------------------------------------------------------

/// Where routing decisions are written down.
///
/// A record, not a log line: keyed by session, readable back into a typed
/// [`RouteDecision`], and rewritten atomically through a temporary file so a
/// crash mid-write leaves the previous journal rather than a truncated one.
pub struct RouteJournal {
    path: PathBuf,
    /// Session id to canonical record. A `BTreeMap` rather than a `HashMap`
    /// because the file is written from it: hash order would make the same set
    /// of decisions serialise differently on different runs, and a journal that
    /// changes without a decision changing is a journal nobody can diff.
    entries: RefCell<BTreeMap<String, String>>,
}

impl RouteJournal {
    /// The journal at the Omega data directory's usual place.
    #[must_use]
    pub fn at_data_dir() -> Self {
        Self::at(Self::data_dir_path())
    }

    /// Where the durable journal lives.
    ///
    /// Exposed so the *caller* can choose a different path — a stateless run
    /// must not write sessions nobody started into the record an operator
    /// reads. That choice is made in `Agent::server` rather than here, because
    /// reading the environment inside this file is what
    /// `the_routing_law_has_no_clock_no_randomness_and_no_hash_order` forbids:
    /// the router's exit is that the same inputs give the same route, and an
    /// environment read is not an input. The check caught this.
    #[must_use]
    pub fn data_dir_path() -> PathBuf {
        paths::data_dir()
            .join("openagents")
            .join(ROUTE_JOURNAL_FILE)
    }

    /// The journal at an explicit path. Loads what is already there.
    #[must_use]
    pub fn at(path: PathBuf) -> Self {
        let entries = load_journal(&path).unwrap_or_else(|error| {
            log::warn!(
                "OMEGA-DELTA-0029: route journal at {} could not be read ({error:#}); \
                 starting from empty rather than routing without a record",
                path.display()
            );
            BTreeMap::new()
        });
        for (session_id, record) in &entries {
            if let Some(decision) = RouteDecision::parse_canonical_record(record) {
                publish_recorded_route(session_id, decision.disclosed_route());
            }
        }
        Self {
            path,
            entries: RefCell::new(entries),
        }
    }

    /// Write one decision down.
    ///
    /// Publishes to the read-mostly index below in the same call, so the
    /// disclosure line a thread draws cannot disagree with the durable record
    /// it is derived from.
    #[must_use]
    pub fn record(&self, session_id: &str, decision: &RouteDecision) -> bool {
        self.entries
            .borrow_mut()
            .insert(session_id.to_owned(), decision.canonical_record());
        publish_recorded_route(session_id, decision.disclosed_route());
        match self.persist() {
            Ok(()) => true,
            Err(error) => {
                log::error!(
                    "OMEGA-DELTA-0029: route decision for {session_id} could not be \
                     persisted to {}: {error:#}",
                    self.path.display()
                );
                false
            }
        }
    }

    /// Read one decision back.
    ///
    /// `None` for a session with no record *and* for a record that does not
    /// read back as a coherent decision, so a hand-edited journal is rejected
    /// rather than believed.
    #[must_use]
    pub fn decision(&self, session_id: &str) -> Option<RouteDecision> {
        self.entries
            .borrow()
            .get(session_id)
            .and_then(|record| RouteDecision::parse_canonical_record(record))
    }

    /// Every recorded decision, in session order.
    #[must_use]
    pub fn decisions(&self) -> Vec<(String, RouteDecision)> {
        self.entries
            .borrow()
            .iter()
            .filter_map(|(session_id, record)| {
                RouteDecision::parse_canonical_record(record)
                    .map(|decision| (session_id.clone(), decision))
            })
            .collect()
    }

    fn persist(&self) -> anyhow::Result<()> {
        let entries = self.entries.borrow();
        let document = serde_json::json!({
            "schema": ROUTE_JOURNAL_SCHEMA,
            "decisions": entries
                .iter()
                .map(|(session_id, record)| {
                    serde_json::json!({ "sessionId": session_id, "decision": record })
                })
                .collect::<Vec<_>>(),
        });
        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let temporary = self.path.with_extension("json.tmp");
        std::fs::write(&temporary, serde_json::to_vec_pretty(&document)?)?;
        std::fs::rename(&temporary, &self.path)?;
        Ok(())
    }
}

fn load_journal(path: &Path) -> anyhow::Result<BTreeMap<String, String>> {
    if !path.exists() {
        return Ok(BTreeMap::new());
    }
    let document: Value = serde_json::from_slice(&std::fs::read(path)?)?;
    let schema = document.get("schema").and_then(Value::as_str);
    anyhow::ensure!(
        schema == Some(ROUTE_JOURNAL_SCHEMA),
        "unsupported route journal schema {schema:?}"
    );
    let mut entries = BTreeMap::new();
    for entry in document
        .get("decisions")
        .and_then(Value::as_array)
        .unwrap_or(&Vec::new())
    {
        let (Some(session_id), Some(record)) = (
            entry.get("sessionId").and_then(Value::as_str),
            entry.get("decision").and_then(Value::as_str),
        ) else {
            anyhow::bail!("route journal entry is missing sessionId or decision");
        };
        entries.insert(session_id.to_owned(), record.to_owned());
    }
    Ok(entries)
}

/// `OMEGA-DELTA-0029`. The route reason each routed session was recorded with,
/// so a thread surface can disclose it wherever a thread is drawn.
///
/// The same shape as `omega_host_bridge`'s lane index and for the same reason:
/// the router lives behind an `Rc` a render cannot reach, while the disclosure
/// has to be readable from one. It is a read-mostly *projection* of the
/// journal, not a second store — it is filled from the journal when one is
/// opened and on every write, so deleting the journal file empties it.
static RECORDED_ROUTES: LazyLock<Mutex<BTreeMap<String, RouteReason>>> =
    LazyLock::new(|| Mutex::new(BTreeMap::new()));

fn publish_recorded_route(session_id: &str, reason: RouteReason) {
    let mut index = match RECORDED_ROUTES.lock() {
        Ok(index) => index,
        Err(poisoned) => poisoned.into_inner(),
    };
    index.insert(session_id.to_owned(), reason);
}

/// Why Omega Agent routed this session where it did, if it routed it.
///
/// `None` means the session was not routed by the router — a thread from before
/// `OMEGA-DELTA-0029`, or one opened directly on an executor. Saying "not
/// routed" is different from claiming a reason nobody recorded.
#[must_use]
pub fn recorded_route(session_id: &acp::SessionId) -> Option<RouteReason> {
    let index = match RECORDED_ROUTES.lock() {
        Ok(index) => index,
        Err(poisoned) => poisoned.into_inner(),
    };
    index.get(session_id.0.as_ref()).copied()
}

// -------------------------------------------------------------------------
// The router
// -------------------------------------------------------------------------

/// Omega Agent at the `AgentConnection` seam.
///
/// Holds one connection per executor class it can dispatch to, the pins a
/// person set, and the journal it writes decisions to. It holds no run state,
/// no policy state, and no turn state.
pub struct OmegaAgentConnection {
    /// The native agent loop. Required, because it is the fail-closed target:
    /// every route that cannot be honoured lands here.
    native: Rc<dyn AgentConnection>,
    /// The external ACP agent connected for this surface, if one is.
    external_acp: Option<Rc<dyn AgentConnection>>,
    /// The executor registered to serve engine lanes, if one is. See
    /// [`RouteInputs::engine_lane`] for why this is separate from the engine
    /// answering at all.
    engine_lane: Option<Rc<dyn AgentConnection>>,
    /// The engine's last framed answer. Read, never written back.
    engine: RefCell<EngineReadiness>,
    /// Pins, by session. Set by a human gesture; never by a turn.
    pins: RefCell<BTreeMap<String, ExecutorPin>>,
    /// The pin a session that does not exist yet will be created under.
    next_pin: RefCell<Option<ExecutorPin>>,
    /// Decisions, by session, in memory and on disk.
    journal: RouteJournal,
    /// The identity this router presents. `OMEGA-DELTA-0024`.
    agent_id: AgentId,
}

impl OmegaAgentConnection {
    /// A router over the native loop alone.
    ///
    /// The minimum honest configuration: one executor, which is also the
    /// fail-closed target.
    #[must_use]
    pub fn new(native: Rc<dyn AgentConnection>, journal: RouteJournal) -> Self {
        let agent_id = native.agent_id();
        Self {
            native,
            external_acp: None,
            engine_lane: None,
            engine: RefCell::new(EngineReadiness::Unreachable(EngineUnreachable::NotRunning)),
            pins: RefCell::new(BTreeMap::new()),
            next_pin: RefCell::new(None),
            journal,
            agent_id,
        }
    }

    /// Register the external ACP agent this surface can route to.
    #[must_use]
    pub fn with_external_acp(mut self, connection: Rc<dyn AgentConnection>) -> Self {
        self.external_acp = Some(connection);
        self
    }

    /// Register the executor that serves engine lanes.
    #[must_use]
    pub fn with_engine_lane(mut self, connection: Rc<dyn AgentConnection>) -> Self {
        self.engine_lane = Some(connection);
        self
    }

    /// Take the engine's latest framed `get_capacity` answer.
    ///
    /// The engine remains the sole run authority. This is a snapshot the router
    /// reads to decide, and the router never answers back.
    pub fn observe_engine(&self, readiness: EngineReadiness) {
        *self.engine.borrow_mut() = readiness;
    }

    /// Take the engine's latest framed answer, or the fact that it did not
    /// answer.
    pub fn observe_capacity(&self, capacity: Result<&Value, EngineUnreachable>) {
        let readiness = match capacity {
            Ok(capacity) => engine_readiness_from_capacity(capacity),
            Err(cause) => EngineReadiness::Unreachable(cause),
        };
        self.observe_engine(readiness);
    }

    /// Pin an executor for a session that already exists.
    ///
    /// The `gesture` argument is the guard, not a label. A pin is the only way
    /// a thread reaches an engine lane, an engine lane is Full Auto authority,
    /// and owner gate 8 admits only an explicit human action into that
    /// authority. Requiring a [`PinGesture`] means there is no way to set a pin
    /// without naming the human gesture that set it, and `PinGesture` has no
    /// variant for a tool call, a slash command, a restored draft, or a
    /// composer mode flag. Nothing here starts a run: a pin decides *where* the
    /// next turn of a thread goes, and the Full Auto Start button remains the
    /// only path to engine-lane run authority.
    /// Returns the decision the new pin produced, which is what the thread
    /// then discloses — including a pin that could not be honoured, with the
    /// typed reason it could not.
    pub fn pin_session(
        &self,
        session_id: &acp::SessionId,
        pin: ExecutorPin,
        gesture: PinGesture,
    ) -> RouteDecision {
        log::info!(
            "OMEGA-DELTA-0035: session {} pinned to {} by {}",
            session_id.0,
            pin.token(),
            gesture.token()
        );
        self.pins
            .borrow_mut()
            .insert(session_id.0.to_string(), pin.clone());
        // Re-decide *because a person changed the pin*, and only then. Capacity
        // moving underneath a thread never re-decides it — `executor_for` reads
        // the record — so this is the one thing that can move a live thread,
        // and it is a human gesture by construction.
        self.decide(session_id.0.as_ref(), Some(pin))
    }

    /// Clear a session's pin, so its next turn takes the unpinned default.
    ///
    /// Re-decides for the same reason [`pin_session`](Self::pin_session) does:
    /// a cleared pin that left the old decision standing would show an
    /// executor the user had just unpinned.
    pub fn unpin_session(&self, session_id: &acp::SessionId, gesture: PinGesture) -> RouteDecision {
        log::info!(
            "OMEGA-DELTA-0035: session {} unpinned by {}",
            session_id.0,
            gesture.token()
        );
        self.pins.borrow_mut().remove(&session_id.0.to_string());
        self.decide(session_id.0.as_ref(), None)
    }

    /// Pin the executor the next session created through this router will use.
    ///
    /// Carries a [`PinGesture`] for the same reason [`pin_session`] does.
    ///
    /// [`pin_session`]: Self::pin_session
    pub fn pin_next_session(&self, pin: Option<ExecutorPin>, gesture: PinGesture) {
        log::info!(
            "OMEGA-DELTA-0035: the next session is pinned to {} by {}",
            pin.as_ref()
                .map_or("nothing".to_owned(), ExecutorPin::token),
            gesture.token()
        );
        *self.next_pin.borrow_mut() = pin;
    }

    /// Every pin currently in force, in session order. For an inspector.
    #[must_use]
    pub fn pins(&self) -> Vec<(String, ExecutorPin)> {
        self.pins
            .borrow()
            .iter()
            .map(|(session_id, pin)| (session_id.clone(), pin.clone()))
            .collect()
    }

    /// The pin currently in force for a session.
    #[must_use]
    pub fn pin(&self, session_id: &acp::SessionId) -> Option<ExecutorPin> {
        self.pins.borrow().get(&session_id.0.to_string()).cloned()
    }

    /// The typed inputs a decision for this session is made from.
    ///
    /// Everything [`route`] is allowed to read, assembled in one place so a
    /// decision can be re-derived and checked against its record.
    #[must_use]
    pub fn inputs_for(&self, pin: Option<ExecutorPin>) -> RouteInputs {
        RouteInputs {
            pin,
            engine: self.engine.borrow().clone(),
            external_acp: self
                .external_acp
                .as_ref()
                .map(|connection| connection.agent_id().0.to_string()),
            engine_lane: self
                .engine_lane
                .as_ref()
                .map(|connection| connection.agent_id().0.to_string()),
        }
    }

    /// Decide, record, and return the decision for a session.
    ///
    /// Recording happens before dispatch. A turn that ran somewhere the journal
    /// does not name is a route nobody can explain afterwards, which is
    /// omega#78's falsifier.
    #[must_use]
    pub fn decide(&self, session_id: &str, pin: Option<ExecutorPin>) -> RouteDecision {
        let decision = route(&self.inputs_for(pin));
        debug_assert!(decision.is_coherent(), "incoherent decision: {decision:?}");
        if !self.journal.record(session_id, &decision) {
            log::error!(
                "OMEGA-DELTA-0029: session {session_id} routed to {} with no durable \
                 record; the route is explainable only until this process exits",
                decision.explain()
            );
        }
        decision
    }

    /// The decision recorded for a session, if one was.
    #[must_use]
    pub fn recorded_decision(&self, session_id: &acp::SessionId) -> Option<RouteDecision> {
        self.journal.decision(session_id.0.as_ref())
    }

    /// The journal, for an inspector.
    #[must_use]
    pub fn journal(&self) -> &RouteJournal {
        &self.journal
    }

    /// The executor a class names, or the native loop.
    ///
    /// Total by construction: the native loop is required, so there is always
    /// something to hand the turn to. A class whose connection is absent cannot
    /// be reached from [`route`] — `RouteInputs` reports absence, and the
    /// decision falls back — so this arm is a belt on top of a brace, not a
    /// silent substitution.
    #[must_use]
    pub fn executor(&self, class: ExecutorClass) -> Rc<dyn AgentConnection> {
        match class {
            ExecutorClass::NativeLoop => self.native.clone(),
            ExecutorClass::ExternalAcp => self.external_acp.clone().unwrap_or_else(|| {
                log::error!("OMEGA-DELTA-0029: external ACP route with no connection");
                self.native.clone()
            }),
            ExecutorClass::EngineLane => self.engine_lane.clone().unwrap_or_else(|| {
                log::error!("OMEGA-DELTA-0029: engine lane route with no connection");
                self.native.clone()
            }),
        }
    }

    /// The executor a live session's recorded decision names.
    ///
    /// Reads the record rather than re-deciding, so a turn cannot silently move
    /// executors mid-thread because the engine's capacity changed between
    /// turns. A session with no record has not been routed yet and gets the
    /// fail-closed target.
    #[must_use]
    pub fn executor_for(&self, session_id: &acp::SessionId) -> Rc<dyn AgentConnection> {
        match self.recorded_decision(session_id) {
            Some(decision) => self.executor(decision.chosen),
            None => self.native.clone(),
        }
    }
}

impl AgentConnection for OmegaAgentConnection {
    fn agent_id(&self) -> AgentId {
        self.agent_id.clone()
    }

    fn telemetry_id(&self) -> SharedString {
        self.native.telemetry_id()
    }

    fn agent_version(&self) -> Option<SharedString> {
        self.native.agent_version()
    }

    /// Route once, record, and let the executor build the thread.
    ///
    /// The thread comes back carrying the *executor's* connection, which is
    /// what omega#77's disclosure downcasts. A thread carrying the router would
    /// disclose the router as its executor, which is the first-party
    /// attribution claim omega#77 exists to stop.
    fn new_session(
        self: Rc<Self>,
        project: Entity<Project>,
        work_dirs: PathList,
        cx: &mut App,
    ) -> Task<Result<Entity<AcpThread>>> {
        let pin = self.next_pin.borrow().clone();
        let decision = route(&self.inputs_for(pin.clone()));
        let executor = self.executor(decision.chosen);
        let session = executor.new_session(project, work_dirs, cx);
        cx.spawn(async move |cx| {
            let thread = session.await?;
            // Recorded once the session exists, because the record is keyed by
            // the session id the executor minted. The decision itself was made
            // before dispatch and is not re-derived here: re-deciding after the
            // fact would let the record describe a world the turn never saw.
            let session_id = thread.read_with(cx, |thread, _| thread.session_id().0.to_string());
            if !self.journal.record(&session_id, &decision) {
                log::error!(
                    "OMEGA-DELTA-0029: session {session_id} routed to {} with no durable \
                     record; the route is explainable only until this process exits",
                    decision.explain()
                );
            }
            self.pins
                .borrow_mut()
                .extend(pin.map(|pin| (session_id, pin)));
            Ok(thread)
        })
    }

    // omega#100. Reopening a thread goes to the executor that owns it.
    //
    // The router answered `supports_load_session` with the *native*
    // connection's answer and then never implemented `load_session`, so the
    // trait default ran and every restore failed with "Loading sessions is not
    // supported". The router was advertising a capability it did not have.
    //
    // It stayed hidden while an unpinned thread was always native and zero base
    // always opened a fresh thread. `OMEGA-DELTA-0055` routes unpinned threads
    // to an attached external agent, and `OMEGA-DELTA-0054` gives the window a
    // project to restore into — so there is now a session to reopen, and the
    // window came up empty with "Failed to Launch".
    //
    // `executor_for` already resolves a session to the connection that owns it,
    // from the durable route record. Both the capability answer and the call
    // route through it, so they cannot disagree: the thing asked whether it can
    // reopen a session is the thing that will be asked to do it.
    fn supports_load_session(&self) -> bool {
        // Asked without a session in hand — before a thread exists, and by
        // surfaces deciding whether to offer reopening at all. True if any
        // attached executor can, because the answer for a *particular* session
        // is `session_supports_load`.
        self.native.supports_load_session()
            || self
                .external_acp
                .as_ref()
                .is_some_and(|executor| executor.supports_load_session())
            || self
                .engine_lane
                .as_ref()
                .is_some_and(|executor| executor.supports_load_session())
    }

    fn load_session(
        self: Rc<Self>,
        session_id: acp::SessionId,
        project: Entity<Project>,
        work_dirs: PathList,
        title: Option<SharedString>,
        cx: &mut App,
    ) -> Task<Result<Entity<AcpThread>>> {
        // `supports_load_session` above cannot name a session, so it answers
        // for the router rather than for this thread. That leaves a gap: the
        // caller may be told yes because *some* executor can load, and then
        // this session's own executor cannot.
        //
        // So the gap is closed here rather than left to the caller. The
        // executor that owns the session is asked what it can do, in the same
        // order the caller would have tried: load, then resume, then say so.
        // Resuming loses the earlier messages, which is worse than loading and
        // much better than an empty window with "Failed to Launch".
        let executor = self.executor_for(&session_id);
        if executor.supports_load_session() {
            executor.load_session(session_id, project, work_dirs, title, cx)
        } else if executor.supports_resume_session() {
            log::info!(
                "reopening session {} by resume: {} cannot load sessions, so the \
                 earlier messages are not restored",
                session_id.0,
                executor.telemetry_id()
            );
            executor.resume_session(session_id, project, work_dirs, title, cx)
        } else {
            Task::ready(Err(anyhow::anyhow!(
                "{} cannot reopen a thread it ran: it supports neither loading \
                 nor resuming a session",
                executor.telemetry_id()
            )))
        }
    }

    fn supports_resume_session(&self) -> bool {
        self.native.supports_resume_session()
            || self
                .external_acp
                .as_ref()
                .is_some_and(|executor| executor.supports_resume_session())
            || self
                .engine_lane
                .as_ref()
                .is_some_and(|executor| executor.supports_resume_session())
    }

    fn resume_session(
        self: Rc<Self>,
        session_id: acp::SessionId,
        project: Entity<Project>,
        work_dirs: PathList,
        title: Option<SharedString>,
        cx: &mut App,
    ) -> Task<Result<Entity<AcpThread>>> {
        self.executor_for(&session_id)
            .resume_session(session_id, project, work_dirs, title, cx)
    }

    fn auth_methods(&self) -> &[acp::AuthMethod] {
        self.native.auth_methods()
    }

    fn authenticate(&self, method: acp::AuthMethodId, cx: &mut App) -> Task<Result<()>> {
        self.native.authenticate(method, cx)
    }

    fn supports_logout(&self) -> bool {
        self.native.supports_logout()
    }

    fn logout(&self, cx: &mut App) -> Task<Result<()>> {
        self.native.logout(cx)
    }

    fn client_user_message_ids(
        &self,
        cx: &App,
    ) -> Option<Rc<dyn AgentSessionClientUserMessageIds>> {
        self.native.client_user_message_ids(cx)
    }

    fn prompt(
        &self,
        params: acp::PromptRequest,
        cx: &mut App,
    ) -> Task<Result<acp::PromptResponse>> {
        self.executor_for(&params.session_id).prompt(params, cx)
    }

    fn retry(&self, session_id: &acp::SessionId, cx: &App) -> Option<Rc<dyn AgentSessionRetry>> {
        self.executor_for(session_id).retry(session_id, cx)
    }

    fn cancel(&self, session_id: &acp::SessionId, cx: &mut App) {
        self.executor_for(session_id).cancel(session_id, cx);
    }

    fn request_elicitations(&self) -> Option<Entity<ElicitationStore>> {
        self.native.request_elicitations()
    }

    fn truncate(
        &self,
        session_id: &acp::SessionId,
        cx: &App,
    ) -> Option<Rc<dyn AgentSessionTruncate>> {
        self.executor_for(session_id).truncate(session_id, cx)
    }

    fn set_title(
        &self,
        session_id: &acp::SessionId,
        cx: &App,
    ) -> Option<Rc<dyn AgentSessionSetTitle>> {
        self.executor_for(session_id).set_title(session_id, cx)
    }

    fn model_selector(&self, session_id: &acp::SessionId) -> Option<Rc<dyn AgentModelSelector>> {
        self.executor_for(session_id).model_selector(session_id)
    }

    fn telemetry(&self) -> Option<Rc<dyn AgentTelemetry>> {
        self.native.telemetry()
    }

    fn session_modes(
        &self,
        session_id: &acp::SessionId,
        cx: &App,
    ) -> Option<Rc<dyn AgentSessionModes>> {
        self.executor_for(session_id).session_modes(session_id, cx)
    }

    fn session_config_options(
        &self,
        session_id: &acp::SessionId,
        cx: &App,
    ) -> Option<Rc<dyn AgentSessionConfigOptions>> {
        self.executor_for(session_id)
            .session_config_options(session_id, cx)
    }

    fn session_list(&self, cx: &mut App) -> Option<Rc<dyn AgentSessionList>> {
        self.native.session_list(cx)
    }

    fn into_any(self: Rc<Self>) -> Rc<dyn Any> {
        self
    }
}

// -------------------------------------------------------------------------
// Wiring: the router is what the native agent entry resolves to
// -------------------------------------------------------------------------

thread_local! {
    /// `OMEGA-DELTA-0035`. The router this window's native agent entry built.
    ///
    /// A render cannot reach an `Rc<dyn AgentConnection>` held inside the
    /// connection store's async entry, and the pin control has to live on the
    /// thread's own disclosure line — so the router publishes itself here when
    /// it is built, exactly as `omega_host_bridge` publishes its lane index and
    /// as [`RECORDED_ROUTES`] publishes route reasons.
    ///
    /// A `thread_local` rather than a `static` because `Rc` is not `Sync`, and
    /// because the only reader is the GPUI main thread that built it. It is a
    /// *handle*, not state: every decision, pin and journal entry lives on the
    /// connection itself.
    ///
    /// **Weak, deliberately.** A strong reference here would keep the native
    /// agent entity alive for the life of the process, which is a leak in
    /// production and a hard failure in the GPUI test harness — it checks for
    /// leaked entity handles at teardown and caught exactly this. Weak also
    /// gives the right semantics: when the connection store drops the
    /// connection there is no router, and `active_router` says so instead of
    /// handing out a router nothing is dispatching through.
    static ACTIVE_ROUTER: RefCell<Weak<OmegaAgentConnection>> =
        const { RefCell::new(Weak::new()) };
}

fn publish_active_router(router: &Rc<OmegaAgentConnection>) {
    ACTIVE_ROUTER.with(|active| *active.borrow_mut() = Rc::downgrade(router));
}

/// Omega Agent's router, if this window has built one.
///
/// `None` before the native agent connects, and in any process that never
/// connects it. A caller that gets `None` must not invent a route — the
/// disclosure says "not routed", which is the honest reading.
#[must_use]
pub fn active_router() -> Option<Rc<OmegaAgentConnection>> {
    ACTIVE_ROUTER.with(|active| active.borrow().upgrade())
}

/// The native agent, behind Omega Agent's router. `OMEGA-DELTA-0035`.
///
/// omega#78 put `OmegaAgentConnection` at the `AgentConnection` seam and left
/// it unwired: nothing constructed one, so every thread disclosed
/// `route: None` and the journal stayed empty. This is the wire. Omega's
/// native-agent entry resolves to a router over the native connection instead
/// of to the native connection itself, so every new native session is routed
/// on purpose and the decision is written down before the turn exists.
///
/// The thread that comes back still carries the **executor's** connection,
/// because `OmegaAgentConnection::new_session` delegates and returns what the
/// executor built. That is what keeps omega#77's disclosure honest and what
/// keeps every existing `downcast::<NativeAgentConnection>()` on a *thread's*
/// connection working unchanged.
pub struct OmegaRouterServer {
    /// Held by value rather than behind an `Rc`, because `AgentServer` is
    /// `Send` and an `Rc` field would take that away.
    native: agent::NativeAgentServer,
    /// Where decisions are written down. Chosen by the caller; see
    /// [`RouteJournal::data_dir_path`].
    journal_path: PathBuf,
    /// Where the Exo harness lane is configured, if the owner configured one.
    /// `OMEGA-DELTA-0042`. Chosen by the caller for the same reason the journal
    /// path is: a harness must not read the owner's real data directory.
    exo_lane_path: PathBuf,
    /// The coding agents found on this machine, in preference order.
    /// `OMEGA-DELTA-0095`, omega#106. Passed in rather than read here, for the
    /// two reasons the paths above are: this file may not read the
    /// environment, and a rendering harness must be able to attach nothing
    /// without the router knowing what a harness is.
    installed_agents: Vec<omega_agent_detect::DetectedAgent>,
}

impl OmegaRouterServer {
    /// A router server over the native agent server, journalling to `journal_path`.
    #[must_use]
    pub fn new(
        native: agent::NativeAgentServer,
        journal_path: PathBuf,
        exo_lane_path: PathBuf,
        installed_agents: Vec<omega_agent_detect::DetectedAgent>,
    ) -> Self {
        Self {
            native,
            journal_path,
            exo_lane_path,
            installed_agents,
        }
    }

    /// The native agent server underneath.
    #[must_use]
    pub fn native(&self) -> &agent::NativeAgentServer {
        &self.native
    }
}

impl agent_servers::AgentServer for OmegaRouterServer {
    fn agent_id(&self) -> project::AgentId {
        self.native.agent_id()
    }

    fn logo(&self) -> ui::IconName {
        self.native.logo()
    }

    fn connect(
        &self,
        mut delegate: agent_servers::AgentServerDelegate,
        project: Entity<Project>,
        cx: &mut App,
    ) -> Task<Result<Rc<dyn AgentConnection>>> {
        let agent_server_store = delegate.store().downgrade();
        // `OMEGA-DELTA-0114`, omega#106. The panel's loading-status channel,
        // taken before the delegate goes to the native server. The native loop
        // downloads nothing and ignores the delegate entirely, while the
        // detected agent's ACP adapter is an npm package resolved on every
        // connect — so the only part of this sequence a person can be left
        // waiting on was also the only part with no way to say so.
        let loading_status = delegate.take_loading_status();
        let native = self.native.connect(delegate, project.clone(), cx);
        let journal_path = self.journal_path.clone();
        let exo_lane_path = self.exo_lane_path.clone();
        let installed_agents = self.installed_agents.clone();
        cx.spawn(async move |cx| {
            let native = native.await?;
            let mut router = OmegaAgentConnection::new(native, RouteJournal::at(journal_path));
            // `OMEGA-DELTA-0115`. What the person chose in the composer, if
            // they chose anything. `None` is *no choice made* and reproduces
            // this connect exactly as it behaved before the control existed:
            // the Exo lane, then the detected agent.
            //
            // The plan is computed here rather than inside either attach
            // because the choice is between them — the router holds one
            // external-ACP slot, the Exo lane wins it by default, and a person
            // who asked for Claude on a machine with an Exo lane would
            // otherwise never reach Claude however the second attach behaved.
            let plan = crate::omega_executor_selector::attach_plan(
                crate::omega_executor_selector::selected(),
                &installed_agents,
            );
            // `OMEGA-DELTA-0117`. Everything detection found, kept whole while
            // the plan narrows to what this connect may attach: warming asks
            // what the *selector* offers, which is not what this one choice
            // reaches for.
            let detected_agents = installed_agents;
            let warm_project = project.clone();
            let warm_agent_server_store = agent_server_store.clone();
            let installed_agents = plan.agents;
            let exo_lane = if plan.exo {
                crate::omega_exo_connection::connect_configured_lane(
                    &exo_lane_path,
                    project.clone(),
                    agent_server_store.clone(),
                    cx,
                )
                .await?
            } else {
                log::info!(
                    "OMEGA-DELTA-0115: the Exo lane is not attached because a \
                     person chose another executor"
                );
                None
            };
            // `OMEGA-DELTA-0042`, omega#87. The Exo harness lane, when the owner
            // configured one. Registered as the external executor rather than
            // as its own class: see `omega_exo_lane`'s module docs for why an
            // Exo thread reports `ExternalAcp` and why it must not report an
            // engine lane. A machine with no Exo registers nothing, and a pin
            // to the external executor then falls back visibly with
            // `RouteReason::ExternalAcpUnavailable`.
            // `OMEGA-DELTA-0117`. Which agent id ends up in the one external
            // slot, so the warming below does not start a second copy of the
            // adapter that is already running.
            let mut attached_adapter: Option<&'static str> = None;
            if let Some(exo) = exo_lane {
                attached_adapter = Some(omega_exo_lane::EXO_HARNESS_ID);
                router = router.with_external_acp(exo);
            // `OMEGA-DELTA-0095`, omega#106. Otherwise the coding agent that is
            // actually installed — Codex first, then Claude. Second, because an
            // Exo lane is a file the owner wrote and detection is only a
            // binary that happens to be on `PATH`: explicit configuration wins
            // over what was found. A machine with neither attaches nothing and
            // every thread stays on the native loop with its visible fallback
            // reason. A machine where the chosen agent cannot be reached fails
            // here naming it, rather than producing a thread that reports one
            // executor and runs another — unless a person has read that failure
            // and chosen Omega's own loop, which `connect_detected_executor`
            // honours by attaching nothing. `OMEGA-DELTA-0114`.
            } else if let Some(installed) = crate::omega_agent_attach::connect_detected_executor(
                &installed_agents,
                project,
                agent_server_store,
                loading_status,
                cx,
            )
            .await?
            {
                attached_adapter = crate::omega_agent_attach::choose_executor(&installed_agents)
                    .map(|choice| choice.chosen.id);
                router = router.with_external_acp(installed);
            }
            let router = Rc::new(router);
            publish_active_router(&router);
            // `OMEGA-DELTA-0117`, omega#112. The earliest moment a preload is
            // allowed to start: this connect has returned, so a window is up
            // and the wait a person was actually in has ended. Starting the
            // other offerable executors any earlier would move that wait rather
            // than remove it.
            crate::omega_executor_warmth::warm_the_others(
                &detected_agents,
                attached_adapter,
                warm_project,
                warm_agent_server_store,
                cx,
            );
            Ok(router as Rc<dyn AgentConnection>)
        })
    }

    fn into_any(self: Rc<Self>) -> Rc<dyn Any> {
        self
    }

    fn default_mode(&self, cx: &App) -> Option<acp::SessionModeId> {
        self.native.default_mode(cx)
    }

    fn set_default_mode(
        &self,
        mode_id: Option<acp::SessionModeId>,
        fs: std::sync::Arc<dyn fs::Fs>,
        cx: &mut App,
    ) {
        self.native.set_default_mode(mode_id, fs, cx);
    }

    fn default_config_option(
        &self,
        config_id: &str,
        cx: &App,
    ) -> Option<settings::AgentConfigOptionValue> {
        self.native.default_config_option(config_id, cx)
    }

    fn set_default_config_option(
        &self,
        config_id: &str,
        value: Option<settings::AgentConfigOptionValue>,
        fs: std::sync::Arc<dyn fs::Fs>,
        cx: &mut App,
    ) {
        self.native
            .set_default_config_option(config_id, value, fs, cx);
    }

    // `favorite_config_option_value_ids` and
    // `toggle_favorite_config_option_value` are deliberately *not* forwarded.
    // `NativeAgentServer` does not override either, so forwarding would be a
    // no-op against the trait default — and the getter's return type names
    // `HashSet`, which `the_routing_law_has_no_clock_no_randomness_and_no_hash_order`
    // forbids in this file. A no-op override is not worth spending that check
    // on. If the native server ever overrides one, forward it then, with a
    // type alias that does not name hash iteration order.
}

/// Whether a server is Omega's native agent, wrapped or bare.
///
/// Every `downcast::<NativeAgentServer>()` that asked "is this the first-party
/// agent?" has to go through here now, because the answer is yes for a router
/// over it. Missing one of these is not a compile error — it is a silently
/// wrong `false`, which is why `omega_deltas` counts the bare downcasts.
#[must_use]
pub fn is_native_agent_server(server: &Rc<dyn agent_servers::AgentServer>) -> bool {
    server
        .clone()
        .downcast::<agent::NativeAgentServer>()
        .is_some()
        || server.clone().downcast::<OmegaRouterServer>().is_some()
}

/// The native connection behind a connection, unwrapping the router.
///
/// The connection the *store* holds is the router; the connection a *thread*
/// holds is the executor. Callers that reach for the native agent through the
/// store need this; callers that already have a thread's connection do not,
/// and are deliberately left alone.
#[must_use]
pub fn native_connection(
    connection: &Rc<dyn AgentConnection>,
) -> Option<Rc<agent::NativeAgentConnection>> {
    if let Some(native) = connection
        .clone()
        .downcast::<agent::NativeAgentConnection>()
    {
        return Some(native);
    }
    connection
        .clone()
        .downcast::<OmegaAgentConnection>()
        .and_then(|router| {
            router
                .native
                .clone()
                .downcast::<agent::NativeAgentConnection>()
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use acp_thread::StubAgentConnection;
    use omega_front_door::{LaneState, RouteReason};

    fn stub(agent_id: &str) -> Rc<dyn AgentConnection> {
        Rc::new(StubAgentConnection::new().with_agent_id(AgentId::new(agent_id)))
    }

    fn journal_in(directory: &tempfile::TempDir) -> RouteJournal {
        RouteJournal::at(directory.path().join("openagents").join(ROUTE_JOURNAL_FILE))
    }

    fn ready_capacity() -> Value {
        serde_json::json!({
            "activeRunCount": 1,
            "activeRunLimit": 8,
            "lanes": [
                { "lane": "codex-local", "state": "busy" },
                { "lane": "claude-local", "state": "available" },
            ],
        })
    }

    /// The live `get_capacity` fixture the Full Auto roster parses reads into
    /// engine readiness the router can decide from.
    ///
    /// A parser tested only against hand-written JSON proves the parser agrees
    /// with its author. This one is the record a running `omega-effectd`
    /// actually returned.
    #[test]
    fn the_live_capacity_fixture_reads_as_a_ready_engine() {
        let capacity: Value = serde_json::from_str(include_str!(
            "../../full_auto_ui/fixtures/live-omega-effectd.get_capacity.json"
        ))
        .expect("the live capacity fixture is JSON");

        let EngineReadiness::Answered {
            active_run_count,
            active_run_limit,
            lanes,
        } = engine_readiness_from_capacity(&capacity)
        else {
            panic!("the live capacity fixture did not read as an answer");
        };
        assert_eq!((active_run_count, active_run_limit), (1, 8));
        assert_eq!(lanes.len(), 7);
        assert!(
            lanes
                .iter()
                .any(|lane| lane.lane_ref == "codex-local" && lane.state == LaneState::Busy)
        );
        assert!(
            lanes
                .iter()
                .any(|lane| lane.lane_ref == "claude-local" && lane.state == LaneState::Available)
        );
    }

    /// A capacity answer this build cannot read fully is not half-believed.
    #[test]
    fn a_misshapen_capacity_answer_reads_as_unreachable() {
        for broken in [
            serde_json::json!({}),
            serde_json::json!({ "activeRunCount": 0, "lanes": [] }),
            serde_json::json!({ "activeRunCount": 0, "activeRunLimit": 8 }),
            serde_json::json!({
                "activeRunCount": 0,
                "activeRunLimit": 8,
                "lanes": [{ "lane": "codex-local" }],
            }),
            Value::Null,
        ] {
            assert_eq!(
                engine_readiness_from_capacity(&broken),
                EngineReadiness::Unreachable(EngineUnreachable::ProtocolError),
                "{broken}"
            );
        }
        assert!(engine_readiness_from_capacity(&ready_capacity()).answered());
    }

    /// Exit property 3, at the durable layer: a decision written down survives
    /// the process that made it.
    ///
    /// Falsified by making `record` a no-op: the reopened journal is empty and
    /// this fails.
    #[test]
    fn a_recorded_decision_survives_reopening_the_journal() {
        let directory = tempfile::tempdir().expect("temporary directory");
        let session = "session-1";

        let decision = {
            let journal = journal_in(&directory);
            let router = OmegaAgentConnection::new(stub("omega-agent"), journal);
            router.observe_capacity(Err(EngineUnreachable::NotRunning));
            router.decide(session, Some(ExecutorPin::on_lane("claude-local")))
        };
        assert_eq!(decision.chosen, ExecutorClass::NativeLoop);
        assert_eq!(decision.reason, RouteReason::EngineUnreachable);

        let reopened = journal_in(&directory);
        assert_eq!(reopened.decision(session).as_ref(), Some(&decision));
        assert_eq!(reopened.decisions(), vec![(session.to_owned(), decision)]);
        assert_eq!(
            recorded_route(&acp::SessionId::new(session)),
            Some(RouteReason::EngineUnreachable),
            "a decision the journal holds must be disclosable on the thread \
             that made it"
        );
    }

    /// A journal written under a schema this build does not know is not read as
    /// if it were.
    #[test]
    fn an_unknown_journal_schema_is_not_believed() {
        let directory = tempfile::tempdir().expect("temporary directory");
        let path = directory.path().join("agent-route-journal.json");
        std::fs::write(
            &path,
            serde_json::to_vec(&serde_json::json!({
                "schema": "openagents.omega.agent_route_journal.v99",
                "decisions": [{ "sessionId": "s", "decision": "chosen=engine_lane;reason=pin_honored;pin=engine_lane;lane=x" }],
            }))
            .unwrap(),
        )
        .unwrap();
        assert!(RouteJournal::at(path).decision("s").is_none());
    }

    /// Exit property 1, at the dispatch layer: the executor a turn reaches is
    /// the one the recorded decision names.
    ///
    /// Falsified by making `executor_for` always return `self.native`: the
    /// external case fails.
    #[test]
    fn dispatch_follows_the_recorded_decision() {
        let directory = tempfile::tempdir().expect("temporary directory");
        let router = OmegaAgentConnection::new(stub("omega-agent"), journal_in(&directory))
            .with_external_acp(stub("codex-acp"))
            .with_engine_lane(stub("codex-local"));
        router.observe_capacity(Ok(&ready_capacity()));

        let external = router.decide(
            "s-external",
            Some(ExecutorPin::new(ExecutorClass::ExternalAcp)),
        );
        assert_eq!(external.chosen, ExecutorClass::ExternalAcp);
        assert_eq!(
            router.executor(external.chosen).agent_id(),
            AgentId::new("codex-acp")
        );

        let lane = router.decide("s-lane", Some(ExecutorPin::on_lane("claude-local")));
        assert_eq!(lane.chosen, ExecutorClass::EngineLane);
        assert_eq!(lane.lane_ref.as_deref(), Some("claude-local"));
        assert_eq!(
            router.executor(lane.chosen).agent_id(),
            AgentId::new("codex-local")
        );

        // OMEGA-DELTA-0055 changed what unpinned means. This router has an
        // external ACP agent attached, and an unpinned thread now goes to it
        // rather than to the native loop — that is the whole point of removing
        // the pin control: a person no longer has to know the word
        // `external_acp` to reach Codex.
        //
        // The property this test is named for is unchanged: dispatch follows
        // the recorded decision. So the assertion still pairs the chosen class
        // with the executor actually dispatched to.
        let unpinned = router.decide("s-unpinned", None);
        assert_eq!(unpinned.chosen, ExecutorClass::ExternalAcp);
        assert_eq!(
            router.executor(unpinned.chosen).agent_id(),
            AgentId::new("codex-acp")
        );
    }

    /// With nothing attached, an unpinned thread is still the native loop.
    ///
    /// OMEGA-DELTA-0055 routes unpinned threads to an attached external agent.
    /// The case above proves that. This proves the other half: automatic
    /// routing must not invent an executor that is not there, or a machine
    /// with no coding agent installed would dispatch into nothing.
    #[test]
    fn an_unpinned_thread_with_nothing_attached_is_the_native_loop() {
        let directory = tempfile::tempdir().expect("temporary directory");
        let router = OmegaAgentConnection::new(stub("omega-agent"), journal_in(&directory));

        let unpinned = router.decide("s-alone", None);

        assert_eq!(unpinned.chosen, ExecutorClass::NativeLoop);
        assert_eq!(
            router.executor(unpinned.chosen).agent_id(),
            AgentId::new("omega-agent")
        );
    }

    /// Exit property 2, at the dispatch layer: with the engine down, an
    /// engine-lane pin reaches the native loop and the record says why.
    #[test]
    fn an_engine_down_router_dispatches_to_the_native_loop() {
        let directory = tempfile::tempdir().expect("temporary directory");
        let router = OmegaAgentConnection::new(stub("omega-agent"), journal_in(&directory))
            .with_engine_lane(stub("codex-local"));
        router.observe_capacity(Err(EngineUnreachable::Timeout));

        let decision = router.decide("s", Some(ExecutorPin::on_lane("claude-local")));
        assert_eq!(
            router.executor(decision.chosen).agent_id(),
            AgentId::new("omega-agent")
        );
        assert_eq!(decision.reason, RouteReason::EngineUnreachable);
        assert!(
            decision.reason.phrase().contains("fell back"),
            "the fallback must be sayable on the thread's own line"
        );
        assert_eq!(
            router.journal().decision("s").map(|d| d.reason),
            Some(RouteReason::EngineUnreachable)
        );
    }

    /// The engine is read, never written. The router holds no run state, so
    /// observing a *worse* engine cannot rewrite a decision already made.
    ///
    /// This is `omega-effectd` staying the sole run authority, as a test: a
    /// second authority would re-derive and change what the first one recorded.
    #[test]
    fn a_later_engine_answer_does_not_rewrite_a_recorded_decision() {
        let directory = tempfile::tempdir().expect("temporary directory");
        let router = OmegaAgentConnection::new(stub("omega-agent"), journal_in(&directory))
            .with_engine_lane(stub("codex-local"));
        router.observe_capacity(Ok(&ready_capacity()));

        let decision = router.decide("s", Some(ExecutorPin::on_lane("claude-local")));
        assert_eq!(decision.chosen, ExecutorClass::EngineLane);

        router.observe_capacity(Err(EngineUnreachable::NotRunning));
        assert_eq!(router.journal().decision("s").as_ref(), Some(&decision));
        // The dispatch path, not only the journal. An earlier draft asserted
        // on the journal alone, and falsifying `executor_for` into re-deciding
        // on every turn left this test green: the record was intact while the
        // turn went somewhere else.
        assert_eq!(
            router.executor_for(&acp::SessionId::new("s")).agent_id(),
            AgentId::new("codex-local"),
            "the recorded route must keep the turn where it was placed rather \
             than moving mid-thread because capacity changed"
        );
    }

    /// The router presents Omega Agent's identity and never claims to be the
    /// executor. `OMEGA-DELTA-0024`.
    #[test]
    fn the_router_does_not_disclose_itself_as_the_executor() {
        let directory = tempfile::tempdir().expect("temporary directory");
        let native = stub("omega-agent");
        let router = Rc::new(
            OmegaAgentConnection::new(native, journal_in(&directory))
                .with_external_acp(stub("codex-acp")),
        );
        assert_eq!(router.agent_id(), AgentId::new("omega-agent"));

        // The router is not one of the three executor classes, so nothing that
        // classifies a connection can mistake it for one. The dispatch target
        // is what a thread carries.
        let decision = router.decide("s", Some(ExecutorPin::new(ExecutorClass::ExternalAcp)));
        let executor = router.executor(decision.chosen);
        assert_ne!(
            executor.agent_id(),
            router.agent_id(),
            "an externally routed thread must carry the external agent, not \
             the router, or omega#77's disclosure would name the router as the \
             executor"
        );
    }

    /// The same inputs produce the same record through the live layer too, not
    /// only through the pure function.
    #[test]
    fn the_live_router_records_the_same_decision_every_time() {
        let directory = tempfile::tempdir().expect("temporary directory");
        let router = OmegaAgentConnection::new(stub("omega-agent"), journal_in(&directory))
            .with_engine_lane(stub("codex-local"));
        router.observe_capacity(Ok(&ready_capacity()));

        let first = router.decide("s", Some(ExecutorPin::new(ExecutorClass::EngineLane)));
        for _ in 0..16 {
            assert_eq!(
                router.decide("s", Some(ExecutorPin::new(ExecutorClass::EngineLane))),
                first
            );
        }
        assert_eq!(
            std::fs::read_to_string(directory.path().join("openagents").join(ROUTE_JOURNAL_FILE))
                .unwrap()
                .matches("chosen=")
                .count(),
            1,
            "one session must leave one record, not one per decision"
        );
    }

    /// `OMEGA-DELTA-0035`. A human pin moves a live thread, and the record says
    /// why it landed where it did.
    ///
    /// The pin control would be decoration without this: `executor_for` reads
    /// the *recorded* decision so a turn cannot drift between executors on its
    /// own, which means setting a pin has to re-decide or it changes nothing a
    /// turn can see. Falsified by making `pin_session` insert the pin without
    /// re-deciding: the executor stays native and this fails.
    #[test]
    fn a_human_pin_moves_a_live_thread_and_records_why() {
        let directory = tempfile::tempdir().expect("temporary directory");
        let router = OmegaAgentConnection::new(stub("omega-agent"), journal_in(&directory))
            .with_external_acp(stub("codex-acp"));
        // Its own session id; see `an_unhonourable_pin_is_kept_and_explained`.
        let session = acp::SessionId::new("s-human-pin");

        // OMEGA-DELTA-0055. An external agent is attached here, so an unpinned
        // thread is routed to it and the reason recorded is
        // `DetectedExternalAcp`, not `UnpinnedDefault`. What this test is for
        // is the *move* that follows — a human pin relocating a live thread and
        // saying why — and that is unchanged.
        assert_eq!(
            router.decide(session.0.as_ref(), None).reason,
            RouteReason::DetectedExternalAcp
        );
        assert_eq!(
            router.executor_for(&session).agent_id(),
            AgentId::new("codex-acp")
        );

        let decision = router.pin_session(
            &session,
            ExecutorPin::new(ExecutorClass::ExternalAcp),
            PinGesture::ExecutorPinMenuItem,
        );
        assert_eq!(decision.reason, RouteReason::PinHonored);
        assert_eq!(
            router.executor_for(&session).agent_id(),
            AgentId::new("codex-acp"),
            "a pin a person set must move the turn, or the control is decoration"
        );
        assert_eq!(
            router
                .journal()
                .decision(session.0.as_ref())
                .map(|d| d.chosen),
            Some(ExecutorClass::ExternalAcp)
        );
    }

    /// `OMEGA-DELTA-0035`. An unhonourable pin is kept, and its reason is
    /// sayable on the thread's own line.
    ///
    /// This is the case the rendered proof photographs: no engine is running,
    /// so an engine-lane pin falls closed to the native loop. The pin is *not*
    /// forgotten — an unhonoured pin the record dropped is indistinguishable
    /// from no pin at all.
    #[test]
    fn an_unhonourable_pin_is_kept_and_explained() {
        let directory = tempfile::tempdir().expect("temporary directory");
        let router = OmegaAgentConnection::new(stub("omega-agent"), journal_in(&directory));
        // A session id no other test uses. `RECORDED_ROUTES` is a
        // process-wide projection keyed by session id, and the tests in this
        // module run in one process: a shared `"s"` made this assertion read
        // another test's decision, which it caught on the first run. Real
        // session ids are minted per session, so this is a harness concern and
        // not a production one.
        let session = acp::SessionId::new("s-unhonourable-pin");
        router.observe_capacity(Err(EngineUnreachable::NotRunning));

        let decision = router.pin_session(
            &session,
            ExecutorPin::new(ExecutorClass::EngineLane),
            PinGesture::ExecutorPinMenuItem,
        );
        assert_eq!(decision.chosen, ExecutorClass::NativeLoop);
        assert!(decision.reason.is_fallback());
        assert_eq!(
            decision.pin.as_ref().map(ExecutorPin::token).as_deref(),
            Some("engine_lane")
        );
        assert!(
            decision
                .reason
                .phrase()
                .contains("fell back to the native loop")
        );
        assert_eq!(
            recorded_route(&session),
            Some(decision.reason),
            "the thread's disclosure line reads this index, so a reason the \
             journal holds and the index does not is a fallback the user \
             cannot see"
        );

        let cleared = router.unpin_session(&session, PinGesture::ExecutorPinCleared);
        assert_eq!(cleared.reason, RouteReason::UnpinnedDefault);
        assert!(cleared.pin.is_none());
        assert!(router.pin(&session).is_none());
    }

    /// `OMEGA-DELTA-0035`. A bare downcast through the router misses the
    /// native loop — which is the whole reason `native_connection` exists.
    ///
    /// Five call sites downcast a connection to `NativeAgentConnection`. Three
    /// hold a *thread's* connection, which is the executor's and unaffected;
    /// the ones that hold the *store's* connection now hold the router, and a
    /// bare downcast there returns `None` — a silently wrong "this is not the
    /// native agent" rather than a compile error.
    #[test]
    fn a_bare_downcast_through_the_router_misses_the_native_loop() {
        let directory = tempfile::tempdir().expect("temporary directory");
        let native = stub("omega-agent");
        let bare: Rc<dyn AgentConnection> = native.clone();
        assert!(
            native_connection(&bare).is_none(),
            "the stub is not a NativeAgentConnection, so this only shows the \
             helper does not answer yes to anything"
        );

        let router: Rc<dyn AgentConnection> =
            Rc::new(OmegaAgentConnection::new(native, journal_in(&directory)));
        assert!(
            router
                .clone()
                .downcast::<agent::NativeAgentConnection>()
                .is_none(),
            "a bare downcast through the router must not find the native loop; \
             if it did, this check would prove nothing about the unwrapping \
             helper"
        );
        assert_eq!(router.agent_id(), AgentId::new("omega-agent"));
    }
}
