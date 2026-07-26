//! Omega Agent, served over ACP on a loopback socket. `OMEGA-DELTA-0041`,
//! omega#82.
//!
//! Omega is already an ACP **client**: `crates/agent_servers` reaches out to
//! external agents. This crate is the other direction. An external ACP host —
//! stock Zed, another fork, any conformant client — can attach to Omega and get
//! Omega Agent, the router, with the same disclosed routing an in-app thread
//! gets. Recursive composability, which is what omega#82 asks for.
//!
//! Four properties hold this surface down, and each is structural rather than
//! a runtime check something could forget to call.
//!
//! # 1. Default off
//!
//! [`enablement`] is an **exact** match on `"1"`. `"true"`, `"yes"`, `"on"`,
//! `" 1"`, and an unset variable are all off, and the caller gets a typed
//! [`OffReason`] rather than a bool. A listener that is on by default is a
//! different product from one that is off by default, so the default is
//! asserted rather than assumed.
//!
//! # 2. Loopback only
//!
//! [`LoopbackHost`] is a **construction invariant**, not a configuration check.
//! There is no way to hold a bind address that is not `127.0.0.1` or `::1`,
//! because the only constructor refuses anything else, and
//! [`LoopbackAcpServer::bind`] takes a `LoopbackHost` rather than a string. A
//! routable interface cannot be reached by editing a setting; it needs a new
//! type.
//!
//! # 3. Read-only, inside the authority partition rather than beside it
//!
//! This is an **unauthenticated** model-driven surface: [`AUTH_METHODS`] is
//! empty, so it carries no bearer at all and is structurally weaker than the
//! Desktop MCP surface. Owner gate 8 — *no model-initiated path can start Full
//! Auto authority; only an explicit human action can, wherever that action
//! lives* — therefore reaches it directly.
//!
//! So the partition is the design. [`SERVED_SURFACE`] is every method an
//! attached host can reach, each classified, and every one of them is an
//! [`SurfaceAuthority::Observation`]. [`UNEXPOSED_AUTHORITY`] is the other
//! half: every control in Omega that grants Full Auto authority or mutates run
//! state, listed with the typed refusal a host attempting it receives. That
//! list is checked **against the existing ledgers** — every
//! [`omega_front_door::FULL_AUTO_AFFORDANCES`] element id and every
//! [`omega_front_door::PinGesture`] must appear in it — so a new Full Auto
//! control fails this crate's tests until somebody classifies it. Adding a
//! control cannot silently open a door here.
//!
//! Anything not in [`SERVED_SURFACE`] is refused: [`served_method`] is the only
//! dispatch table, so deny-by-default is the absence of an entry rather than a
//! branch someone has to remember to write.
//!
//! A served prompt is answered by **disclosing**, not by executing. Nothing is
//! dispatched to any executor, so the answer's stop reason is the protocol's
//! own word for it, `refusal`.
//!
//! # 4. The served agent is the router, and it can never reach an engine lane
//!
//! [`served_route`] calls [`omega_front_door::route`] — the same routing law an
//! in-app thread is decided by — with a pin of `None`. A pin is the only door
//! to an engine lane, an engine lane *is* Full Auto authority, and setting one
//! requires an `omega_front_door::PinGesture`, every variant of which is a
//! visible control a person operates. There is no variant for "an external host
//! asked", so the served surface cannot take a pin, and
//! `a_served_session_can_never_reach_an_engine_lane` proves the consequence
//! against every engine state the router can be shown.
//!
//! # Where the socket lives
//!
//! Not in a GPUI crate. This crate depends on `omega_front_door` and the pinned
//! ACP schema and on **no** part of GPUI, and a check in `crates/omega_deltas`
//! fails if a listener appears in `crates/agent_ui`, `crates/full_auto_ui`, or
//! `crates/zed`. The lifecycle owner in a shipped build is `omega-effectd`:
//! [`start_if_enabled`] is called from `crates/omega_effectd`, which is the
//! supervisor layer, and nowhere else.
//!
//! **What this does not do,** stated rather than discovered: the listener runs
//! in the Omega process under the supervisor's control, not inside the packaged
//! `@openagentsinc/omega-effectd` daemon. The daemon lives in the openagents
//! repository and this packet is scoped to omega. The property omega#82's
//! falsifier names — *GPUI owns the socket* — is what is enforced here, by
//! keeping the bind in a crate GPUI cannot reach and the start in the
//! supervisor.

use std::io::{BufRead, BufReader, Write};
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr, TcpListener, TcpStream};

use agent_client_protocol::schema::v1::{AGENT_METHOD_NAMES, CLIENT_METHOD_NAMES};
use omega_front_door::{
    EngineReadiness, EngineUnreachable, ExecutorDisclosure, RouteDecision, RouteInputs,
    SessionOrigin, route,
};
use serde_json::{Value, json};

// -------------------------------------------------------------------------
// Default off
// -------------------------------------------------------------------------

/// The environment variable that turns the loopback ACP server on.
pub const ENABLE_FLAG: &str = "OMEGA_ACP_SERVER";

/// The environment variable that pins the port. Optional; `0` means ephemeral.
pub const PORT_FLAG: &str = "OMEGA_ACP_SERVER_PORT";

/// The one value of [`ENABLE_FLAG`] that turns the server on.
///
/// Exact. `"true"`, `"yes"`, `"on"`, `"01"` and `" 1"` are all off. A flag that
/// accepts anything truthy is a flag whose default nobody can state.
pub const ENABLE_VALUE: &str = "1";

/// Why the server is off.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OffReason {
    /// The flag is not set at all. This is the shipped default.
    FlagUnset,
    /// The flag is set to something other than exactly [`ENABLE_VALUE`].
    FlagNotExactlyOne,
}

impl OffReason {
    /// The stable token this reason is recorded under.
    #[must_use]
    pub const fn token(self) -> &'static str {
        match self {
            Self::FlagUnset => "flag_unset",
            Self::FlagNotExactlyOne => "flag_not_exactly_one",
        }
    }
}

/// Whether the loopback ACP server runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Enablement {
    /// Off, with the reason.
    Off(OffReason),
    /// On, because the flag said exactly [`ENABLE_VALUE`].
    On,
}

impl Enablement {
    /// Whether a listener should be opened.
    #[must_use]
    pub const fn is_on(self) -> bool {
        matches!(self, Self::On)
    }
}

/// Read the enablement from the flag's value.
///
/// Takes the value rather than reading the environment, so the default can be
/// asserted without a process.
#[must_use]
pub fn enablement(flag: Option<&str>) -> Enablement {
    match flag {
        None => Enablement::Off(OffReason::FlagUnset),
        Some(ENABLE_VALUE) => Enablement::On,
        Some(_) => Enablement::Off(OffReason::FlagNotExactlyOne),
    }
}

// -------------------------------------------------------------------------
// Loopback only
// -------------------------------------------------------------------------

/// A bind address that is loopback, proven by construction.
///
/// There is no way to build one that is not, so nothing downstream has to
/// check. `LoopbackAcpServer::bind` takes this type rather than a string, which
/// is what makes "never binds a routable interface" a property of the type
/// system instead of a code review.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LoopbackHost(IpAddr);

/// Why an address was refused as a bind target.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BindRefusal {
    /// The text is not an IP address at all. A hostname is refused rather than
    /// resolved: resolution is the step where `localhost` can be made to mean
    /// something routable.
    NotAnAddress(String),
    /// The address parses and is not loopback.
    NotLoopback(IpAddr),
}

impl BindRefusal {
    /// The stable token this refusal is recorded under.
    #[must_use]
    pub const fn token(&self) -> &'static str {
        match self {
            Self::NotAnAddress(_) => "not_an_address",
            Self::NotLoopback(_) => "not_loopback",
        }
    }
}

impl std::fmt::Display for BindRefusal {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotAnAddress(raw) => write!(
                formatter,
                "{raw:?} is not an IP address. The Omega ACP server binds a \
                 literal loopback address and never resolves a name."
            ),
            Self::NotLoopback(address) => write!(
                formatter,
                "{address} is not a loopback address. The Omega ACP server is \
                 an unauthenticated surface and never binds a routable \
                 interface."
            ),
        }
    }
}

impl LoopbackHost {
    /// The shipped bind address.
    pub const DEFAULT: Self = Self(IpAddr::V4(Ipv4Addr::LOCALHOST));

    /// Take a loopback address, or refuse.
    ///
    /// # Errors
    ///
    /// [`BindRefusal`] when the text is not an IP address, or is an address
    /// that is not loopback.
    pub fn new(raw: &str) -> Result<Self, BindRefusal> {
        let address: IpAddr = raw
            .parse()
            .map_err(|_| BindRefusal::NotAnAddress(raw.to_owned()))?;
        if address == IpAddr::V4(Ipv4Addr::LOCALHOST) || address == IpAddr::V6(Ipv6Addr::LOCALHOST)
        {
            Ok(Self(address))
        } else {
            Err(BindRefusal::NotLoopback(address))
        }
    }

    /// The address itself.
    #[must_use]
    pub const fn address(self) -> IpAddr {
        self.0
    }
}

// -------------------------------------------------------------------------
// The authority partition
// -------------------------------------------------------------------------

/// What an attached host is refused, and why.
///
/// Typed rather than a sentence, so a caller can branch on it and so the same
/// refusal reads the same on the wire and in a test.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ServedRefusal {
    /// A prompt was answered by disclosing where it would route, and dispatched
    /// to nothing.
    ExecutesNothing,
    /// A run may not be started over this surface.
    StartsNoRun,
    /// A run's state may not be changed over this surface.
    MutatesNoRun,
    /// An executor pin may not be set or cleared over this surface.
    SetsNoPin,
    /// An account or credential may not be connected or revoked over this
    /// surface.
    GrantsNoCredential,
    /// The method is not part of the served surface at all.
    NotExposed,
}

impl ServedRefusal {
    /// Every refusal, in declaration order.
    #[must_use]
    pub const fn all() -> &'static [Self] {
        &[
            Self::ExecutesNothing,
            Self::StartsNoRun,
            Self::MutatesNoRun,
            Self::SetsNoPin,
            Self::GrantsNoCredential,
            Self::NotExposed,
        ]
    }

    /// The stable token this refusal is recorded under.
    #[must_use]
    pub const fn token(self) -> &'static str {
        match self {
            Self::ExecutesNothing => "served_executes_nothing",
            Self::StartsNoRun => "served_starts_no_run",
            Self::MutatesNoRun => "served_mutates_no_run",
            Self::SetsNoPin => "served_sets_no_pin",
            Self::GrantsNoCredential => "served_grants_no_credential",
            Self::NotExposed => "served_method_not_exposed",
        }
    }

    /// What the attached host is told, in its operator's terms.
    ///
    /// Derived on every call; nothing stores it.
    #[must_use]
    pub const fn message(self) -> &'static str {
        match self {
            Self::ExecutesNothing => {
                "Omega Agent served over ACP is read-only. This turn was routed \
                 and disclosed, and dispatched to no executor."
            }
            Self::StartsNoRun => {
                "A run cannot be started over the served ACP surface. Only an \
                 explicit human action in Omega starts Full Auto authority."
            }
            Self::MutatesNoRun => {
                "Run state cannot be changed over the served ACP surface. The \
                 engine remains the sole run authority and Omega's own controls \
                 are the only way to reach it."
            }
            Self::SetsNoPin => {
                "An executor pin cannot be set over the served ACP surface. A \
                 pin is the only door to an engine lane and it is set by a \
                 human gesture on a visible control."
            }
            Self::GrantsNoCredential => {
                "An account cannot be connected or revoked over the served ACP \
                 surface. This surface is unauthenticated and grants nothing."
            }
            Self::NotExposed => {
                "This method is not part of the served ACP surface. The surface \
                 is deny-by-default: what is not listed is refused."
            }
        }
    }
}

/// What an attached host may do with a served method.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SurfaceAuthority {
    /// Reads a record or mints one. Grants nothing and changes no run.
    ///
    /// Every entry of [`SERVED_SURFACE`] is this, and
    /// `the_whole_served_surface_is_observation` fails if one is not.
    Observation,
}

/// Which handler a served method reaches.
///
/// The dispatch is driven by [`SERVED_SURFACE`] rather than by a `match` on a
/// string, so a method with no entry in the table has no handler to reach and
/// deny-by-default is the absence of a row.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ServedMethodKind {
    /// `initialize`.
    Initialize,
    /// `session/new`.
    NewSession,
    /// `session/prompt`.
    Prompt,
    /// `session/cancel`, a notification.
    Cancel,
}

/// One method an attached host may reach, and what it is allowed to do.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ServedMethod {
    /// The ACP method name, taken from the pinned schema's own constants so a
    /// protocol rename is a compile error rather than a silent 404.
    pub method: &'static str,
    /// Which handler it reaches.
    pub kind: ServedMethodKind,
    /// What it is allowed to do.
    pub authority: SurfaceAuthority,
}

/// Every method the served surface exposes.
///
/// Deliberately short. `session/load`, `session/resume`, `session/fork`,
/// `session/set_mode`, `session/set_config_option`, `session/delete`,
/// `authenticate`, and `logout` are all absent, and absence is the refusal:
/// [`served_method`] returns `None` and the caller answers
/// [`ServedRefusal::NotExposed`].
pub const SERVED_SURFACE: &[ServedMethod] = &[
    ServedMethod {
        method: AGENT_METHOD_NAMES.initialize,
        kind: ServedMethodKind::Initialize,
        authority: SurfaceAuthority::Observation,
    },
    ServedMethod {
        method: AGENT_METHOD_NAMES.session_new,
        kind: ServedMethodKind::NewSession,
        authority: SurfaceAuthority::Observation,
    },
    ServedMethod {
        method: AGENT_METHOD_NAMES.session_prompt,
        kind: ServedMethodKind::Prompt,
        authority: SurfaceAuthority::Observation,
    },
    ServedMethod {
        method: AGENT_METHOD_NAMES.session_cancel,
        kind: ServedMethodKind::Cancel,
        authority: SurfaceAuthority::Observation,
    },
];

/// The entry for a method, or `None` if the surface does not expose it.
#[must_use]
pub fn served_method(method: &str) -> Option<&'static ServedMethod> {
    SERVED_SURFACE.iter().find(|entry| entry.method == method)
}

/// One Omega control that grants authority or mutates run state, and is
/// deliberately not on the served surface.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UnexposedControl {
    /// The control's own identifier: a `full_auto_ui` element id, or an
    /// `omega_front_door::PinGesture` token.
    pub control: &'static str,
    /// What an attached host attempting it is told.
    pub refusal: ServedRefusal,
}

/// Every authority-bearing control in Omega, and the refusal a served host gets
/// for it.
///
/// This is the half of the partition that makes the served surface *inside* it
/// rather than beside it. `every_authority_bearing_control_is_classified`
/// checks this list against the two ledgers that already exist —
/// [`omega_front_door::FULL_AUTO_AFFORDANCES`] and
/// [`omega_front_door::PinGesture::all`] — in both directions, so adding a Full
/// Auto control or a pin gesture fails this crate until it is classified here,
/// and removing one fails until the stale row goes.
pub const UNEXPOSED_AUTHORITY: &[UnexposedControl] = &[
    UnexposedControl {
        control: "full-auto-panel",
        refusal: ServedRefusal::NotExposed,
    },
    UnexposedControl {
        control: "full-auto-openagents-connect",
        refusal: ServedRefusal::GrantsNoCredential,
    },
    UnexposedControl {
        control: "full-auto-openagents-disconnect",
        refusal: ServedRefusal::GrantsNoCredential,
    },
    UnexposedControl {
        control: "full-auto-provider-account",
        refusal: ServedRefusal::GrantsNoCredential,
    },
    UnexposedControl {
        control: "full-auto-advanced-toggle",
        refusal: ServedRefusal::NotExposed,
    },
    UnexposedControl {
        control: "full-auto-start",
        refusal: ServedRefusal::StartsNoRun,
    },
    UnexposedControl {
        control: "full-auto-cancel",
        refusal: ServedRefusal::NotExposed,
    },
    UnexposedControl {
        control: "full-auto-pause",
        refusal: ServedRefusal::MutatesNoRun,
    },
    UnexposedControl {
        control: "full-auto-resume",
        refusal: ServedRefusal::MutatesNoRun,
    },
    UnexposedControl {
        control: "full-auto-handoff",
        refusal: ServedRefusal::MutatesNoRun,
    },
    UnexposedControl {
        control: "full-auto-retry",
        refusal: ServedRefusal::MutatesNoRun,
    },
    UnexposedControl {
        control: "full-auto-stop",
        refusal: ServedRefusal::MutatesNoRun,
    },
    UnexposedControl {
        control: "full-auto-new",
        refusal: ServedRefusal::StartsNoRun,
    },
    UnexposedControl {
        control: "full-auto-evidence-chain",
        refusal: ServedRefusal::NotExposed,
    },
    UnexposedControl {
        control: "full-auto-monitor",
        refusal: ServedRefusal::NotExposed,
    },
    UnexposedControl {
        control: "full-auto-monitor-new",
        refusal: ServedRefusal::StartsNoRun,
    },
    UnexposedControl {
        control: "full-auto-run-row",
        refusal: ServedRefusal::NotExposed,
    },
    UnexposedControl {
        control: "executor_pin_menu_item",
        refusal: ServedRefusal::SetsNoPin,
    },
    UnexposedControl {
        control: "executor_pin_cleared",
        refusal: ServedRefusal::SetsNoPin,
    },
];

// -------------------------------------------------------------------------
// The served session
// -------------------------------------------------------------------------

/// The agent identity the served surface presents.
///
/// The same identity an in-app thread discloses, because it is the same agent.
/// `the_served_surface_presents_the_first_party_agent_id` in `omega_deltas`
/// checks this against `crates/agent`'s own `OMEGA_AGENT_ID`, so the two cannot
/// drift into a served surface that claims to be something else.
pub const SERVED_AGENT_ID: &str = "Omega Agent";

/// The authentication methods the served surface offers.
///
/// Empty, and empty on purpose. There is no credential to present, so there is
/// no credential to steal or to mistake for authority. That is also why the
/// surface must be read-only: an unauthenticated surface that could act would
/// be a hole with no gate in front of it.
pub const AUTH_METHODS: &[&str] = &[];

/// The routing inputs a served session is decided from.
///
/// The pin is `None` and there is no way to make it anything else from this
/// crate: setting a pin requires an `omega_front_door::PinGesture`, and no
/// variant of that enum is reachable over a socket. The engine is reported
/// unreachable because the served surface does not hold the supervisor's
/// answer — and it does not matter, which
/// `a_served_session_can_never_reach_an_engine_lane` proves by showing the
/// decision is identical for every engine state.
#[must_use]
pub fn served_inputs() -> RouteInputs {
    RouteInputs {
        pin: None,
        engine: EngineReadiness::Unreachable(EngineUnreachable::NotRunning),
        external_acp: None,
        engine_lane: None,
    }
}

/// The route decision a served session gets.
///
/// The same [`omega_front_door::route`] law an in-app thread is decided by, so
/// an attached host gets Omega Agent's disclosed routing rather than a raw
/// provider.
#[must_use]
pub fn served_route() -> RouteDecision {
    route(&served_inputs())
}

/// The disclosure a served session carries.
///
/// A typed record. The provider and model are `None` — *not disclosed*, which
/// is the honest answer, because the served surface dispatches to no executor
/// and so no provider served anything.
#[must_use]
pub fn served_disclosure(decision: &RouteDecision) -> ExecutorDisclosure {
    ExecutorDisclosure {
        class: decision.chosen,
        agent_id: SERVED_AGENT_ID.to_owned(),
        provider: None,
        model: None,
        run_ref: None,
        route: Some(decision.disclosed_route()),
    }
}

/// One session an external host reached over the loopback server.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServedSession {
    /// The ACP session id this session was minted under.
    pub session_id: String,
    /// Where it was reached from. A separate record from the disclosure,
    /// because ingress and execution are different facts.
    pub origin: SessionOrigin,
    /// Where a turn on it would be routed.
    pub decision: RouteDecision,
    /// What it says about the executor. Typed; a label renders it.
    pub disclosure: ExecutorDisclosure,
}

/// The `_meta` key the served records are published under.
pub const SERVED_SESSION_META_KEY: &str = "openagents.omega.served_session.v1";

/// The disclosure record as it goes on the wire.
///
/// Keys are exactly `omega_front_door::EXECUTOR_DISCLOSURE_FIELDS`, and
/// `the_served_disclosure_is_a_record_not_a_label` asserts that exactly. A
/// rendered line on the wire under any name — `label`, `line`, `text`,
/// `summary` — fails, because the binding condition of the owner's identity
/// decision is that disclosure travels as a record a label renders.
#[must_use]
pub fn disclosure_meta(disclosure: &ExecutorDisclosure) -> Value {
    json!({
        "class": disclosure.class.token(),
        "agent_id": disclosure.agent_id,
        "provider": disclosure.provider,
        "model": disclosure.model,
        "run_ref": disclosure.run_ref,
        "route": disclosure.route.map(|route| route.token()),
    })
}

/// The origin record as it goes on the wire. Keys exactly
/// `omega_front_door::SESSION_ORIGIN_FIELDS`.
#[must_use]
pub fn origin_meta(origin: &SessionOrigin) -> Value {
    json!({
        "ingress": origin.ingress.token(),
        "host_name": origin.host_name,
        "host_version": origin.host_version,
        "authenticated": origin.authenticated,
    })
}

/// The `_meta` block a served **turn** publishes.
///
/// The session's records, plus the typed refusal saying the turn reached no
/// executor. The refusal lives here rather than in the stop reason on purpose:
/// ACP's `refusal` stop reason means *the prompt and everything after it will
/// not be included in the next prompt*, and stock Zed implements that by
/// dropping the turn from the thread — which took the disclosure with it, so
/// the operator saw a refusal banner and no disclosure at all. That was watched
/// happening against Zed 1.12.0 before this shape was chosen. The turn genuinely
/// ended, so it says `end_turn`, and what did **not** happen is stated in the
/// message the operator reads and in the record beside it.
#[must_use]
pub fn served_turn_meta(session: &ServedSession) -> Value {
    let mut meta = served_meta(session);
    if let Some(block) = meta
        .get_mut(SERVED_SESSION_META_KEY)
        .and_then(Value::as_object_mut)
    {
        block.insert(
            "refusal".to_owned(),
            Value::String(ServedRefusal::ExecutesNothing.token().to_owned()),
        );
    }
    meta
}

/// The `_meta` block a served session publishes.
#[must_use]
pub fn served_meta(session: &ServedSession) -> Value {
    json!({
        SERVED_SESSION_META_KEY: {
            "disclosure": disclosure_meta(&session.disclosure),
            "origin": origin_meta(&session.origin),
            "readOnly": true,
            "refusals": ServedRefusal::all()
                .iter()
                .map(|refusal| refusal.token())
                .collect::<Vec<_>>(),
        }
    })
}

/// What a served turn says back, rendered from the typed records.
///
/// Derived on every call. Nothing stores it, which is the same discipline
/// `ExecutorDisclosure::label` is held to — the wire carries the record in
/// `_meta` and this rendering in the message the operator reads.
#[must_use]
pub fn served_turn_text(session: &ServedSession) -> String {
    format!(
        "Omega Agent, served over ACP.\n\n\
         executor: {}\n\
         origin: {}\n\n\
         {}",
        session.disclosure.label(),
        session.origin.label(),
        ServedRefusal::ExecutesNothing.message(),
    )
}

// -------------------------------------------------------------------------
// The connection
// -------------------------------------------------------------------------

/// The JSON-RPC error code for a method the surface does not expose.
const METHOD_NOT_FOUND: i64 = -32601;

/// The JSON-RPC error code for a request this surface cannot read.
const INVALID_REQUEST: i64 = -32600;

/// The ACP protocol version this surface speaks.
const PROTOCOL_VERSION: u16 = 1;

/// One attached host.
///
/// Holds the sessions it minted and what it said about itself. It holds no run
/// state, no policy state, and no credential, because there is none to hold.
#[derive(Debug)]
pub struct ServedConnection {
    /// A stable label for this connection, used to mint session ids without a
    /// clock or a random source.
    connection_ref: String,
    /// Sessions minted on this connection, in mint order.
    sessions: Vec<ServedSession>,
    /// What the host said about itself at `initialize`.
    host_name: Option<String>,
    /// The host's version, where it gave one.
    host_version: Option<String>,
}

impl ServedConnection {
    /// A connection with nothing on it yet.
    #[must_use]
    pub fn new(connection_ref: impl Into<String>) -> Self {
        Self {
            connection_ref: connection_ref.into(),
            sessions: Vec::new(),
            host_name: None,
            host_version: None,
        }
    }

    /// The sessions minted on this connection.
    #[must_use]
    pub fn sessions(&self) -> &[ServedSession] {
        &self.sessions
    }

    /// One session by id.
    #[must_use]
    pub fn session(&self, session_id: &str) -> Option<&ServedSession> {
        self.sessions
            .iter()
            .find(|session| session.session_id == session_id)
    }

    /// The origin every session on this connection carries.
    #[must_use]
    pub fn origin(&self) -> SessionOrigin {
        SessionOrigin::loopback_acp(self.host_name.clone(), self.host_version.clone())
    }

    /// Handle one newline-framed JSON-RPC message, and return the lines to
    /// write back in order.
    ///
    /// Notifications the surface emits come before the response they belong to,
    /// which is the order ACP requires for a prompt turn.
    pub fn handle_line(&mut self, line: &str) -> Vec<String> {
        let Ok(message) = serde_json::from_str::<Value>(line) else {
            return vec![error_line(
                &Value::Null,
                INVALID_REQUEST,
                "The Omega ACP server reads newline-framed JSON-RPC.",
                ServedRefusal::NotExposed,
            )];
        };
        let id = message.get("id").cloned();
        let Some(method) = message.get("method").and_then(Value::as_str) else {
            // A response to a request. This surface sends none, so there is
            // nothing this can be an answer to and nothing to say back.
            return Vec::new();
        };
        let params = message.get("params").cloned().unwrap_or(Value::Null);

        let Some(entry) = served_method(method) else {
            log::info!(
                "OMEGA-DELTA-0041: served surface refused {method}: {}",
                ServedRefusal::NotExposed.token()
            );
            return match id {
                Some(id) => vec![error_line(
                    &id,
                    METHOD_NOT_FOUND,
                    ServedRefusal::NotExposed.message(),
                    ServedRefusal::NotExposed,
                )],
                // A notification gets no answer, by JSON-RPC. It is still
                // refused: nothing happened.
                None => Vec::new(),
            };
        };

        match entry.kind {
            ServedMethodKind::Initialize => {
                self.remember_host(&params);
                id.map(|id| vec![result_line(&id, self.initialize_result())])
                    .unwrap_or_default()
            }
            ServedMethodKind::NewSession => {
                let session = self.mint_session();
                let result = json!({
                    "sessionId": session.session_id,
                    "_meta": served_meta(&session),
                });
                self.sessions.push(session);
                id.map(|id| vec![result_line(&id, result)])
                    .unwrap_or_default()
            }
            ServedMethodKind::Prompt => {
                let session_id = params
                    .get("sessionId")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_owned();
                let Some(session) = self.session(&session_id).cloned() else {
                    return match id {
                        Some(id) => vec![error_line(
                            &id,
                            INVALID_REQUEST,
                            "No such session on this connection.",
                            ServedRefusal::NotExposed,
                        )],
                        None => Vec::new(),
                    };
                };
                let mut lines = vec![notification_line(
                    CLIENT_METHOD_NAMES.session_update,
                    json!({
                        "sessionId": session.session_id,
                        "update": {
                            "sessionUpdate": "agent_message_chunk",
                            "content": { "type": "text", "text": served_turn_text(&session) },
                        },
                        "_meta": served_turn_meta(&session),
                    }),
                )];
                if let Some(id) = id {
                    lines.push(result_line(
                        &id,
                        json!({
                            // The turn ended, having disclosed. Not ACP's
                            // `refusal`, which means the turn is dropped from
                            // the thread — stock Zed implements that literally
                            // and the disclosure went with it. What did not
                            // happen is in the message and in `_meta`.
                            "stopReason": "end_turn",
                            "_meta": served_turn_meta(&session),
                        }),
                    ));
                }
                lines
            }
            // Nothing is ever in flight, because nothing is ever dispatched.
            // A cancel is therefore already satisfied, and a notification gets
            // no answer.
            ServedMethodKind::Cancel => Vec::new(),
        }
    }

    fn remember_host(&mut self, params: &Value) {
        let info = params.get("clientInfo");
        self.host_name = info
            .and_then(|info| info.get("name"))
            .and_then(Value::as_str)
            .filter(|name| !name.is_empty())
            .map(str::to_owned);
        self.host_version = info
            .and_then(|info| info.get("version"))
            .and_then(Value::as_str)
            .filter(|version| !version.is_empty())
            .map(str::to_owned);
    }

    fn initialize_result(&self) -> Value {
        json!({
            "protocolVersion": PROTOCOL_VERSION,
            "agentCapabilities": {
                "loadSession": false,
                "promptCapabilities": {
                    "image": false,
                    "audio": false,
                    "embeddedContext": false,
                },
            },
            "authMethods": AUTH_METHODS,
            "agentInfo": { "name": SERVED_AGENT_ID, "version": env!("CARGO_PKG_VERSION") },
        })
    }

    fn mint_session(&self) -> ServedSession {
        let decision = served_route();
        // No clock and no random source: the id is the connection's own label
        // and a count. Two identical runs mint identical ids, which is the same
        // discipline the route journal is held to.
        let session_id = format!(
            "omega-served-{}-{}",
            self.connection_ref,
            self.sessions.len()
        );
        ServedSession {
            session_id,
            origin: self.origin(),
            disclosure: served_disclosure(&decision),
            decision,
        }
    }
}

fn result_line(id: &Value, result: Value) -> String {
    json!({ "jsonrpc": "2.0", "id": id, "result": result }).to_string()
}

fn notification_line(method: &str, params: Value) -> String {
    json!({ "jsonrpc": "2.0", "method": method, "params": params }).to_string()
}

fn error_line(id: &Value, code: i64, message: &str, refusal: ServedRefusal) -> String {
    json!({
        "jsonrpc": "2.0",
        "id": id,
        "error": {
            "code": code,
            "message": message,
            "data": { "refusal": refusal.token() },
        },
    })
    .to_string()
}

// -------------------------------------------------------------------------
// The listener
// -------------------------------------------------------------------------

/// A bound loopback ACP listener.
#[derive(Debug)]
pub struct LoopbackAcpServer {
    listener: TcpListener,
}

impl LoopbackAcpServer {
    /// Bind the listener.
    ///
    /// Takes a [`LoopbackHost`], so there is no way to ask this for a routable
    /// interface.
    ///
    /// # Errors
    ///
    /// The underlying bind error.
    pub fn bind(host: LoopbackHost, port: u16) -> std::io::Result<Self> {
        let listener = TcpListener::bind(SocketAddr::new(host.address(), port))?;
        Ok(Self { listener })
    }

    /// Where it is listening.
    ///
    /// # Errors
    ///
    /// The underlying socket error.
    pub fn local_addr(&self) -> std::io::Result<SocketAddr> {
        self.listener.local_addr()
    }

    /// Serve connections until the listener fails.
    ///
    /// One thread per attached host. There is no shared mutable state between
    /// connections, because a served session holds nothing an operator could
    /// change.
    pub fn serve(self) {
        let mut accepted = 0usize;
        loop {
            match self.listener.accept() {
                Ok((stream, peer)) => {
                    let connection_ref = format!("{accepted}");
                    accepted += 1;
                    log::info!("OMEGA-DELTA-0041: served ACP connection from {peer}");
                    std::thread::spawn(move || serve_stream(stream, &connection_ref));
                }
                Err(error) => {
                    log::error!("OMEGA-DELTA-0041: the served ACP listener stopped: {error:#}");
                    return;
                }
            }
        }
    }
}

/// Serve one attached host over an already-accepted stream.
pub fn serve_stream(stream: TcpStream, connection_ref: &str) {
    let Ok(write_half) = stream.try_clone() else {
        return;
    };
    let mut connection = ServedConnection::new(connection_ref);
    let reader = BufReader::new(stream);
    let mut writer = write_half;
    for line in reader.lines() {
        let Ok(line) = line else { return };
        if line.trim().is_empty() {
            continue;
        }
        for outgoing in connection.handle_line(&line) {
            if writeln!(writer, "{outgoing}").is_err() || writer.flush().is_err() {
                return;
            }
        }
    }
}

/// What [`start_if_enabled`] did.
#[derive(Debug)]
pub enum StartOutcome {
    /// Nothing was bound. The shipped default.
    NotStarted(OffReason),
    /// A listener is bound at this address and is serving on its own thread.
    Listening(SocketAddr),
    /// The flag asked for a listener and one could not be opened.
    Failed(String),
}

/// Start the server if, and only if, the flag says exactly `1`.
///
/// The only production call site is `crates/omega_effectd`, which is the
/// supervisor layer. GPUI never calls this and `omega_deltas` fails if it
/// starts.
pub fn start_if_enabled() -> StartOutcome {
    let flag = std::env::var(ENABLE_FLAG).ok();
    match enablement(flag.as_deref()) {
        Enablement::Off(reason) => StartOutcome::NotStarted(reason),
        Enablement::On => {
            let port = std::env::var(PORT_FLAG)
                .ok()
                .and_then(|port| port.parse::<u16>().ok())
                .unwrap_or(0);
            match LoopbackAcpServer::bind(LoopbackHost::DEFAULT, port) {
                Ok(server) => match server.local_addr() {
                    Ok(address) => {
                        log::info!(
                            "OMEGA-DELTA-0041: Omega Agent is served over ACP on {address} \
                             (loopback, unauthenticated, read-only)"
                        );
                        std::thread::spawn(move || server.serve());
                        StartOutcome::Listening(address)
                    }
                    Err(error) => StartOutcome::Failed(error.to_string()),
                },
                Err(error) => StartOutcome::Failed(error.to_string()),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use agent_client_protocol::schema::v1::{
        InitializeResponse, NewSessionResponse, PromptResponse, SessionNotification, StopReason,
    };
    use omega_front_door::{
        EXECUTOR_DISCLOSURE_FIELDS, EngineLane, ExecutorClass, FULL_AUTO_AFFORDANCES, Ingress,
        LaneState, PinGesture, RouteReason, SESSION_ORIGIN_FIELDS,
    };
    use std::collections::BTreeSet;
    use std::io::{BufRead, BufReader, Write};
    use std::net::TcpStream;

    fn initialized(host: &str) -> ServedConnection {
        let mut connection = ServedConnection::new("test");
        connection.handle_line(
            &json!({
                "jsonrpc": "2.0", "id": 0, "method": "initialize",
                "params": {
                    "protocolVersion": 1,
                    "clientInfo": { "name": host, "version": "1.12.0" },
                },
            })
            .to_string(),
        );
        connection
    }

    fn new_session(connection: &mut ServedConnection) -> Value {
        let lines = connection.handle_line(
            &json!({
                "jsonrpc": "2.0", "id": 1, "method": "session/new",
                "params": { "cwd": "/tmp", "mcpServers": [] },
            })
            .to_string(),
        );
        assert_eq!(lines.len(), 1);
        serde_json::from_str::<Value>(&lines[0]).expect("a JSON line")
    }

    /// The shipped default is off, and the flag is exact.
    ///
    /// A listener that is on by default is a different product. This is the
    /// check that says which product this is.
    #[test]
    fn the_server_is_off_unless_the_flag_says_exactly_one() {
        assert_eq!(enablement(None), Enablement::Off(OffReason::FlagUnset));
        assert!(!enablement(None).is_on());

        for truthy_looking in ["true", "TRUE", "yes", "on", "0", "01", " 1", "1 ", "", "11"] {
            assert_eq!(
                enablement(Some(truthy_looking)),
                Enablement::Off(OffReason::FlagNotExactlyOne),
                "{truthy_looking:?} turned the loopback ACP server on. The \
                 flag is an exact match on {ENABLE_VALUE:?} so that the \
                 default can be stated rather than guessed."
            );
        }

        assert_eq!(enablement(Some(ENABLE_VALUE)), Enablement::On);
    }

    /// A routable bind address is refused by construction.
    #[test]
    fn a_routable_bind_address_is_refused_by_construction() {
        for routable in ["0.0.0.0", "192.168.1.10", "10.0.0.1", "::", "8.8.8.8"] {
            let refused = LoopbackHost::new(routable).expect_err(
                "the Omega ACP server is unauthenticated and must never bind a \
                 routable interface",
            );
            assert_eq!(refused.token(), "not_loopback");
        }

        // A name is refused rather than resolved: resolution is the step where
        // `localhost` can be pointed somewhere else.
        assert_eq!(
            LoopbackHost::new("localhost")
                .expect_err("names are refused")
                .token(),
            "not_an_address"
        );

        assert_eq!(
            LoopbackHost::new("127.0.0.1").expect("loopback").address(),
            IpAddr::V4(Ipv4Addr::LOCALHOST)
        );
        assert_eq!(
            LoopbackHost::new("::1").expect("loopback").address(),
            IpAddr::V6(Ipv6Addr::LOCALHOST)
        );
        assert_eq!(
            LoopbackHost::DEFAULT.address(),
            IpAddr::V4(Ipv4Addr::LOCALHOST)
        );
    }

    /// Every method the surface exposes is observation, and the surface is
    /// exactly these four.
    #[test]
    fn the_whole_served_surface_is_observation() {
        let methods: Vec<&str> = SERVED_SURFACE.iter().map(|entry| entry.method).collect();
        assert_eq!(
            methods,
            [
                "initialize",
                "session/new",
                "session/prompt",
                "session/cancel"
            ],
            "the served surface grew or lost a method. It is deny-by-default, \
             so every addition is a deliberate edit to this list — and an \
             addition that can mutate run state or grant authority is owner \
             gate 8 broken through a socket."
        );
        for entry in SERVED_SURFACE {
            assert_eq!(
                entry.authority,
                SurfaceAuthority::Observation,
                "{} is on an unauthenticated surface and is not observation",
                entry.method
            );
        }
    }

    /// The served surface is inside the authority partition, not beside it.
    ///
    /// Checked against the ledgers that already exist, in both directions, so a
    /// new Full Auto control or a new pin gesture fails here until somebody
    /// says what a served host attempting it is told.
    #[test]
    fn every_authority_bearing_control_is_classified() {
        let classified: BTreeSet<&str> = UNEXPOSED_AUTHORITY
            .iter()
            .map(|control| control.control)
            .collect();
        let mut required: BTreeSet<&str> = FULL_AUTO_AFFORDANCES
            .iter()
            .map(|affordance| affordance.element_id)
            .collect();
        required.extend(PinGesture::all().iter().map(|gesture| gesture.token()));

        assert!(
            !required.is_empty(),
            "the ledgers reached nothing; this partition check would be vacuous"
        );
        let unclassified: Vec<&&str> = required.difference(&classified).collect();
        assert!(
            unclassified.is_empty(),
            "authority-bearing controls with no entry in UNEXPOSED_AUTHORITY: \
             {unclassified:?}. The loopback ACP surface is an unauthenticated \
             model-driven surface and must sit *inside* the authority \
             partition. A control nobody classified is a fourth hole."
        );
        let stale: Vec<&&str> = classified.difference(&required).collect();
        assert!(
            stale.is_empty(),
            "UNEXPOSED_AUTHORITY names controls that no longer exist: \
             {stale:?}. A partition that outlives its source stops being \
             evidence."
        );

        // Nothing that grants authority may be answerable over the socket.
        let served: BTreeSet<&str> = SERVED_SURFACE.iter().map(|entry| entry.method).collect();
        for control in UNEXPOSED_AUTHORITY {
            assert!(
                !served.contains(control.control),
                "{} is both classified as unexposed and served",
                control.control
            );
        }
    }

    /// A served session is routed by the router, and never reaches an engine
    /// lane — whatever the engine says.
    ///
    /// This is owner gate 8 at the socket. A pin is the only door to an engine
    /// lane, setting one requires a `PinGesture`, and no variant of that enum
    /// is reachable from a socket. The consequence is checked against every
    /// engine state the router can be shown, so "at capacity" or "no lanes" is
    /// not what is doing the work.
    #[test]
    fn a_served_session_can_never_reach_an_engine_lane() {
        let decision = served_route();
        assert_eq!(decision.chosen, ExecutorClass::NativeLoop);
        assert_eq!(decision.reason, RouteReason::UnpinnedDefault);
        assert!(decision.is_coherent());

        for engine in [
            EngineReadiness::Unreachable(EngineUnreachable::NotRunning),
            EngineReadiness::Unreachable(EngineUnreachable::Timeout),
            EngineReadiness::Unreachable(EngineUnreachable::ProtocolError),
            EngineReadiness::Answered {
                active_run_count: 0,
                active_run_limit: 8,
                lanes: vec![
                    EngineLane::new("claude-local", LaneState::Available),
                    EngineLane::new("codex-local", LaneState::Available),
                ],
            },
        ] {
            let inputs = RouteInputs {
                engine,
                external_acp: Some("codex-acp".to_owned()),
                engine_lane: Some("codex-local".to_owned()),
                ..served_inputs()
            };
            // OMEGA-DELTA-0055. This used to assert `NativeLoop`, and the
            // property it is named for is about engine lanes. An unpinned
            // thread now runs on an attached external ACP agent, which is not
            // Full Auto authority and is not reachable from anything a socket
            // can say — the connection is made at startup from what is
            // installed. The engine lane is what the socket must never reach,
            // and that is what is asserted.
            let decided = route(&inputs);
            assert_ne!(
                decided.chosen,
                ExecutorClass::EngineLane,
                "a served session reached an engine lane. Nothing over the \
                 socket may take a pin, and a pin is the only door to Full \
                 Auto authority."
            );
            assert!(decided.lane_ref.is_none());
        }
    }

    /// The disclosure crosses the wire as a record with exactly the fields the
    /// record has, and the rendered line is derived beside it.
    #[test]
    fn the_served_disclosure_is_a_record_not_a_label() {
        let mut connection = initialized("Zed");
        let response = new_session(&mut connection);
        let meta = &response["result"]["_meta"][SERVED_SESSION_META_KEY];

        let disclosure = meta["disclosure"]
            .as_object()
            .expect("the disclosure travels as an object");
        let mut keys: Vec<&str> = disclosure.keys().map(String::as_str).collect();
        keys.sort_unstable();
        let mut expected: Vec<&str> = EXECUTOR_DISCLOSURE_FIELDS.to_vec();
        expected.sort_unstable();
        assert_eq!(
            keys, expected,
            "the disclosure on the wire must carry exactly the record's \
             fields. A rendered line under any name — label, line, text, \
             summary — breaks the condition the owner admitted the identity \
             decision on, and an exact key set is what catches the names \
             nobody anticipated."
        );

        let origin = meta["origin"]
            .as_object()
            .expect("the origin travels as an object");
        let mut origin_keys: Vec<&str> = origin.keys().map(String::as_str).collect();
        origin_keys.sort_unstable();
        let mut expected_origin: Vec<&str> = SESSION_ORIGIN_FIELDS.to_vec();
        expected_origin.sort_unstable();
        assert_eq!(origin_keys, expected_origin);

        assert_eq!(disclosure["class"], "native_loop");
        assert_eq!(disclosure["agent_id"], SERVED_AGENT_ID);
        assert!(disclosure["provider"].is_null());
        assert!(disclosure["model"].is_null());
        assert_eq!(disclosure["route"], "unpinned_default");
        assert_eq!(origin["ingress"], "loopback_acp");
        assert_eq!(origin["host_name"], "Zed");
        assert_eq!(origin["authenticated"], false);
    }

    /// A served prompt is answered by disclosing, and dispatched to nothing.
    #[test]
    fn a_served_prompt_executes_nothing_and_says_so() {
        let mut connection = initialized("Zed");
        let session = new_session(&mut connection);
        let session_id = session["result"]["sessionId"]
            .as_str()
            .expect("a session id");

        let lines = connection.handle_line(
            &json!({
                "jsonrpc": "2.0", "id": 2, "method": "session/prompt",
                "params": {
                    "sessionId": session_id,
                    "prompt": [{ "type": "text", "text": "run the build and push it" }],
                },
            })
            .to_string(),
        );
        assert_eq!(lines.len(), 2, "one update then one response");

        let update: Value = serde_json::from_str(&lines[0]).expect("JSON");
        assert_eq!(update["method"], "session/update");
        let text = update["params"]["update"]["content"]["text"]
            .as_str()
            .expect("the disclosure is rendered for the operator");
        // The label lost its class token and its `routed:` fragment when the
        // owner asked for the line to stop leading with a wire token and to
        // stop saying "routed: unpinned" on every ordinary turn. What a person
        // is entitled to know is who ran the turn and on what model, so that is
        // what is asserted. `OMEGA-DELTA-0055` is why the route fragment would
        // now be meaningless anyway: there is no pin to be unpinned from.
        assert!(text.contains("executor: Omega Agent"), "{text}");
        assert!(!text.contains("native_loop"), "{text}");
        assert!(!text.contains("routed:"), "{text}");
        assert!(
            text.contains("loopback_acp · Zed 1.12.0 · unauthenticated"),
            "{text}"
        );
        assert!(text.contains("dispatched to no executor"), "{text}");

        let response: Value = serde_json::from_str(&lines[1]).expect("JSON");
        assert_eq!(
            response["result"]["stopReason"], "end_turn",
            "the turn ended having disclosed. ACP's `refusal` stop reason \
             means the turn is dropped from the thread, and stock Zed 1.12.0 \
             implements that literally — the operator saw a refusal banner \
             and no disclosure at all, which is the exact failure this packet \
             exists to avoid."
        );
        assert_eq!(
            response["result"]["_meta"][SERVED_SESSION_META_KEY]["refusal"],
            ServedRefusal::ExecutesNothing.token(),
            "the turn ended without executing and the record must say so, \
             since the stop reason no longer can"
        );
        assert_eq!(
            update["params"]["_meta"][SERVED_SESSION_META_KEY]["refusal"],
            ServedRefusal::ExecutesNothing.token()
        );
    }

    /// Deny-by-default. Everything the surface does not list is refused, and
    /// the refusal names which one it is.
    #[test]
    fn every_unexposed_method_is_watched_refusing() {
        let mut connection = initialized("Zed");
        // `session/new` first, so a refusal cannot be mistaken for "there was
        // no session anyway".
        let session = new_session(&mut connection);
        let session_id = session["result"]["sessionId"]
            .as_str()
            .expect("a session id")
            .to_owned();

        for (index, (method, params)) in [
            (
                "session/load",
                json!({ "sessionId": session_id, "cwd": "/tmp" }),
            ),
            ("session/resume", json!({ "sessionId": session_id })),
            ("session/delete", json!({ "sessionId": session_id })),
            (
                "session/set_mode",
                json!({ "sessionId": session_id, "modeId": "yolo" }),
            ),
            (
                "session/set_config_option",
                json!({ "sessionId": session_id, "configId": "a", "value": true }),
            ),
            ("authenticate", json!({ "methodId": "anything" })),
            ("logout", json!({})),
            (
                "fs/write_text_file",
                json!({ "path": "/tmp/x", "content": "x" }),
            ),
            ("terminal/create", json!({ "command": "sh" })),
            ("full_auto/start", json!({ "objective": "ship it" })),
            ("omega/pin_executor", json!({ "class": "engine_lane" })),
        ]
        .into_iter()
        .enumerate()
        {
            let lines = connection.handle_line(
                &json!({ "jsonrpc": "2.0", "id": 100 + index, "method": method, "params": params })
                    .to_string(),
            );
            assert_eq!(
                lines.len(),
                1,
                "{method} answered something other than one refusal"
            );
            let answer: Value = serde_json::from_str(&lines[0]).expect("JSON");
            assert!(
                answer.get("result").is_none(),
                "{method} was answered with a result: {answer}"
            );
            assert_eq!(answer["error"]["code"], METHOD_NOT_FOUND, "{method}");
            assert_eq!(
                answer["error"]["data"]["refusal"],
                ServedRefusal::NotExposed.token(),
                "{method} was refused without naming which refusal it was"
            );
        }

        // Nothing the attached host attempted minted a session, changed one, or
        // left any trace at all.
        assert_eq!(connection.sessions().len(), 1);
        assert_eq!(
            connection.sessions()[0].decision.chosen,
            ExecutorClass::NativeLoop
        );
    }

    /// Every refusal has a distinct token and a message that says what did not
    /// happen.
    #[test]
    fn every_refusal_states_what_did_not_happen() {
        let tokens: BTreeSet<&str> = ServedRefusal::all()
            .iter()
            .map(|refusal| refusal.token())
            .collect();
        assert_eq!(tokens.len(), ServedRefusal::all().len());
        for refusal in ServedRefusal::all() {
            assert!(
                refusal.message().len() > 60,
                "{} is stated in a phrase, which is not stated",
                refusal.token()
            );
        }
    }

    /// The surface offers no credential to present.
    #[test]
    fn the_served_surface_is_unauthenticated_and_says_so() {
        let mut connection = ServedConnection::new("test");
        let lines = connection.handle_line(
            &json!({
                "jsonrpc": "2.0", "id": 0, "method": "initialize",
                "params": { "protocolVersion": 1 },
            })
            .to_string(),
        );
        let answer: Value = serde_json::from_str(&lines[0]).expect("JSON");
        assert_eq!(answer["result"]["authMethods"], json!([]));
        assert_eq!(answer["result"]["agentCapabilities"]["loadSession"], false);
        assert!(AUTH_METHODS.is_empty());

        // A host that said nothing about itself is disclosed as having said
        // nothing, not as anonymous-but-fine.
        let session = new_session(&mut connection);
        let origin = &session["result"]["_meta"][SERVED_SESSION_META_KEY]["origin"];
        assert!(origin["host_name"].is_null());
        assert_eq!(origin["authenticated"], false);
    }

    /// Every message this surface writes reads back as the pinned ACP schema
    /// type it claims to be.
    ///
    /// The conformance link. The wire is built as JSON so the *exact* key set
    /// can be asserted; this is what stops that freedom from drifting away from
    /// the protocol Zed actually speaks.
    #[test]
    fn every_answer_deserialises_as_its_pinned_schema_type() {
        let mut connection = initialized("Zed");

        let initialize = connection.handle_line(
            &json!({ "jsonrpc": "2.0", "id": 0, "method": "initialize", "params": {} }).to_string(),
        );
        let initialize: Value = serde_json::from_str(&initialize[0]).expect("JSON");
        let parsed: InitializeResponse =
            serde_json::from_value(initialize["result"].clone()).expect("an InitializeResponse");
        assert!(parsed.auth_methods.is_empty());

        let session = new_session(&mut connection);
        let parsed: NewSessionResponse =
            serde_json::from_value(session["result"].clone()).expect("a NewSessionResponse");
        let session_id = parsed.session_id.0.to_string();

        let lines = connection.handle_line(
            &json!({
                "jsonrpc": "2.0", "id": 2, "method": "session/prompt",
                "params": { "sessionId": session_id, "prompt": [{ "type": "text", "text": "hi" }] },
            })
            .to_string(),
        );
        let update: Value = serde_json::from_str(&lines[0]).expect("JSON");
        let _: SessionNotification =
            serde_json::from_value(update["params"].clone()).expect("a SessionNotification");
        let response: Value = serde_json::from_str(&lines[1]).expect("JSON");
        let parsed: PromptResponse =
            serde_json::from_value(response["result"].clone()).expect("a PromptResponse");
        assert_eq!(parsed.stop_reason, StopReason::EndTurn);
    }

    /// The whole thing over a real loopback socket, end to end.
    #[test]
    fn a_real_client_attaches_over_loopback_and_is_disclosed_to() {
        let server = LoopbackAcpServer::bind(LoopbackHost::DEFAULT, 0).expect("binds loopback");
        let address = server.local_addr().expect("an address");
        assert!(address.ip().is_loopback());
        std::thread::spawn(move || server.serve());

        let stream = TcpStream::connect(address).expect("connects");
        let mut writer = stream.try_clone().expect("a write half");
        let mut reader = BufReader::new(stream);
        let send = |message: Value, writer: &mut TcpStream| {
            writeln!(writer, "{message}").expect("writes");
            writer.flush().expect("flushes");
        };
        let read = |reader: &mut BufReader<TcpStream>| {
            let mut line = String::new();
            reader.read_line(&mut line).expect("reads");
            serde_json::from_str::<Value>(&line).expect("JSON")
        };

        send(
            json!({
                "jsonrpc": "2.0", "id": 0, "method": "initialize",
                "params": {
                    "protocolVersion": 1,
                    "clientInfo": { "name": "conformance-client", "version": "0.1.0" },
                },
            }),
            &mut writer,
        );
        assert_eq!(read(&mut reader)["result"]["authMethods"], json!([]));

        send(
            json!({ "jsonrpc": "2.0", "id": 1, "method": "session/new", "params": { "cwd": "/tmp" } }),
            &mut writer,
        );
        let session = read(&mut reader);
        let session_id = session["result"]["sessionId"]
            .as_str()
            .expect("id")
            .to_owned();
        assert_eq!(
            session["result"]["_meta"][SERVED_SESSION_META_KEY]["origin"]["ingress"],
            "loopback_acp"
        );

        send(
            json!({
                "jsonrpc": "2.0", "id": 2, "method": "session/prompt",
                "params": { "sessionId": session_id, "prompt": [{ "type": "text", "text": "hi" }] },
            }),
            &mut writer,
        );
        let update = read(&mut reader);
        assert!(
            update["params"]["update"]["content"]["text"]
                .as_str()
                .expect("text")
                .contains("executor: Omega Agent")
        );
        let answered = read(&mut reader);
        assert_eq!(answered["result"]["stopReason"], "end_turn");
        assert_eq!(
            answered["result"]["_meta"][SERVED_SESSION_META_KEY]["refusal"],
            ServedRefusal::ExecutesNothing.token()
        );

        // And the mutation attempt, over the same real socket.
        send(
            json!({ "jsonrpc": "2.0", "id": 3, "method": "full_auto/start", "params": {} }),
            &mut writer,
        );
        assert_eq!(
            read(&mut reader)["error"]["data"]["refusal"],
            ServedRefusal::NotExposed.token()
        );
    }

    /// An **upstream** ACP client, over a real loopback socket, sees the
    /// disclosure.
    ///
    /// The client here is `agent_client_protocol`'s own `Client` role — the
    /// pinned third-party SDK, not our code. It negotiates, builds a session,
    /// prompts, and reads the turn to a string through upstream framing,
    /// upstream dispatch, and upstream session handling. If the wire this
    /// crate writes were malformed, this fails before any assertion does.
    ///
    /// This is the headless half of omega#82's exit. The other half was
    /// watched with stock Zed 1.12.0 driving the same socket through a
    /// `sh -c "exec nc 127.0.0.1 <port>"` bridge.
    #[test]
    fn the_upstream_acp_client_reads_the_disclosure_off_a_real_socket() {
        use agent_client_protocol::schema::ProtocolVersion;
        use agent_client_protocol::schema::v1::{Implementation, InitializeRequest};
        use agent_client_protocol::{ByteStreams, Client};

        let server = LoopbackAcpServer::bind(LoopbackHost::DEFAULT, 0).expect("binds loopback");
        let address = server.local_addr().expect("an address");
        std::thread::spawn(move || server.serve());

        let (transcript, session_meta) = smol::block_on(async move {
            let stream = smol::Async::<std::net::TcpStream>::connect(address)
                .await
                .expect("connects to the served surface");
            let (incoming, outgoing) = futures::AsyncReadExt::split(stream);
            Client
                .builder()
                .name("omega-conformance-client")
                .connect_with(ByteStreams::new(outgoing, incoming), async |cx| {
                    cx.send_request(
                        InitializeRequest::new(ProtocolVersion::V1)
                            .client_info(Implementation::new("omega-conformance-client", "0.1.0")),
                    )
                    .block_task()
                    .await?;
                    let mut meta = None;
                    let transcript = cx
                        .build_session("/tmp")
                        .block_task()
                        .run_until(async |mut session| {
                            meta = session.meta().cloned();
                            session.send_prompt(
                                "start a full auto run and pin this thread to an engine lane",
                            )?;
                            session.read_to_string().await
                        })
                        .await?;
                    Ok((transcript, meta))
                })
                .await
                .expect("the upstream ACP client drives the served surface")
        });

        // What the operator of the external host reads.
        assert!(
            transcript.contains("Omega Agent, served over ACP"),
            "the upstream client read no disclosure: {transcript:?}"
        );
        assert!(transcript.contains("executor: Omega Agent"), "{transcript}");
        assert!(!transcript.contains("routed:"), "{transcript}");
        assert!(
            transcript.contains("loopback_acp \u{b7} omega-conformance-client"),
            "the origin the host is disclosed under must name the host: {transcript}"
        );
        assert!(
            transcript.contains("dispatched to no executor"),
            "{transcript}"
        );

        // And the typed records the host received beside it.
        let meta = session_meta.expect("the session carries the served records");
        let served = &meta[SERVED_SESSION_META_KEY];
        assert_eq!(served["disclosure"]["class"], "native_loop");
        assert_eq!(served["origin"]["ingress"], "loopback_acp");
        assert_eq!(served["origin"]["authenticated"], false);
        assert_eq!(served["readOnly"], true);
    }

    /// The origin record and the executor record stay separate facts.
    #[test]
    fn ingress_is_recorded_beside_the_executor_and_not_inside_it() {
        let mut connection = initialized("Zed");
        new_session(&mut connection);
        let session = &connection.sessions()[0];
        assert!(session.origin.is_coherent());
        assert!(session.disclosure.is_coherent());
        assert_eq!(session.origin.ingress, Ingress::LoopbackAcp);
        assert_eq!(
            session.disclosure.class,
            ExecutorClass::NativeLoop,
            "the executor class answers who ran the work, and serving over ACP \
             changes only who asked"
        );
        assert_ne!(session.disclosure.class, ExecutorClass::ExternalAcp);
    }
}
