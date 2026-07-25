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
use std::rc::Rc;
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
    RouteDecision, RouteInputs, RouteReason, route,
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
        Self::at(
            paths::data_dir()
                .join("openagents")
                .join(ROUTE_JOURNAL_FILE),
        )
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
    /// Called from a visible control a person operates. Nothing model-facing
    /// reaches this, and nothing here starts a run: a pin decides *where* the
    /// next turn of an existing thread goes, and the Full Auto Start button
    /// remains the only path to engine-lane run authority.
    pub fn pin_session(&self, session_id: &acp::SessionId, pin: ExecutorPin) {
        self.pins.borrow_mut().insert(session_id.0.to_string(), pin);
    }

    /// Pin the executor the next session created through this router will use.
    pub fn pin_next_session(&self, pin: Option<ExecutorPin>) {
        *self.next_pin.borrow_mut() = pin;
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
            self.pins.borrow_mut().extend(pin.map(|pin| (session_id, pin)));
            Ok(thread)
        })
    }

    fn supports_load_session(&self) -> bool {
        self.native.supports_load_session()
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

    fn client_user_message_ids(&self, cx: &App) -> Option<Rc<dyn AgentSessionClientUserMessageIds>> {
        self.native.client_user_message_ids(cx)
    }

    fn prompt(&self, params: acp::PromptRequest, cx: &mut App) -> Task<Result<acp::PromptResponse>> {
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
        assert_eq!(
            reopened.decisions(),
            vec![(session.to_owned(), decision)]
        );
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

        let external = router.decide("s-external", Some(ExecutorPin::new(ExecutorClass::ExternalAcp)));
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

        let unpinned = router.decide("s-unpinned", None);
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
            std::fs::read_to_string(
                directory.path().join("openagents").join(ROUTE_JOURNAL_FILE)
            )
            .unwrap()
            .matches("chosen=").count(),
            1,
            "one session must leave one record, not one per decision"
        );
    }
}
