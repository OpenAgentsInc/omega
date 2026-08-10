//! The typed core of Omega's chat-first front door.
//!
//! Omega Agent is a router. It owns routing, disclosure, and receipts, and it
//! owns no execution. This crate holds the parts of the front door that are
//! decisions rather than pixels: where a fresh launch lands, what a thread
//! says about the executor that did the work, and which gestures are allowed
//! to start engine-lane authority.
//!
//! It is deliberately a leaf. It depends on nothing, so the laws below can be
//! checked in a second without building GPUI, and so the router packets
//! (omega#77, omega#78) can take this vocabulary without taking the UI crate.
//!
//! Product contract: `specs/omega/omega-agent.product-spec.md` revision 1 in
//! the openagents repository, admitted by the owner on 2026-07-25.

use std::sync::atomic::{AtomicBool, Ordering};

pub mod router;
mod send_during_turn;

pub use send_during_turn::{
    QueueItemState, Quiescence, SendCommand, SendDisposition, SendFallback, SteerCapability,
    SteerRefusal, disposition, may_promote,
};

pub use router::{
    EngineLane, EngineReadiness, EngineUnreachable, ExecutorCandidate, ExecutorOverride,
    ExecutorPin, ExecutorReadiness, ExecutorTarget, LaneState, RESERVED_RECORD_CHARACTERS,
    ROUTING_POLICY_VERSION, RouteDecision, RouteFallback, RouteInputs, RouteReason,
    RouteUnavailable, TaskKind, TaskRequirements, lane_ref_is_recordable, route, select_lane,
};

// -------------------------------------------------------------------------
// Optional integrations
// -------------------------------------------------------------------------

static EXO_ENABLED: AtomicBool = AtomicBool::new(false);

/// Enable Exo for this process from the parsed application command line.
///
/// There is deliberately no disable operation or settings-backed setter. Exo
/// is absent by default, and only the person launching this process may opt in
/// before application startup reaches integration discovery.
pub fn enable_exo_from_command_line() {
    EXO_ENABLED.store(true, Ordering::Release);
}

/// Whether this process was explicitly launched with Exo enabled.
#[must_use]
pub fn exo_enabled() -> bool {
    EXO_ENABLED.load(Ordering::Acquire)
}

// -------------------------------------------------------------------------
// Where a fresh launch lands
// -------------------------------------------------------------------------

/// The surface Omega shows when a window opens with nothing to restore.
///
/// Upstream Zed calls `Editor::new_file` here, so the first thing a new user
/// meets is an empty untitled buffer. Omega's front door is the agent instead:
/// the window opens on the New Agent Thread surface with the composer focused,
/// and typing starts a thread. `OMEGA-DELTA-0019`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LaunchSurface {
    /// The New Agent Thread surface, composer focused. Omega's front door.
    NewAgentThread,
    /// An empty untitled buffer. Upstream Zed's answer, kept only for the
    /// startup behaviours that explicitly ask for a file-first window.
    EmptyBuffer,
    /// Nothing: the launchpad startup behaviour opens no content at all, and
    /// overriding it would be Omega ignoring a setting the user set.
    Nothing,
}

/// The `restore_on_startup` values that reach the no-restorable-session path.
///
/// Mirrors `settings::RestoreOnStartupBehavior` without depending on it. The
/// mapping below is the whole reason this is a type and not a bool: the
/// front door replaces the *empty buffer*, and must not replace a deliberate
/// launchpad choice.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RestoreOnStartup {
    /// The user asked for the launchpad. Omega opens no content.
    Launchpad,
    /// Anything else. Upstream opens an empty buffer here; Omega opens the
    /// front door.
    Other,
}

/// Decide what a window with no restorable session opens on.
///
/// This is the single rule behind `OMEGA-DELTA-0019`, kept here so it can be
/// tested without a window.
#[must_use]
pub fn launch_surface(restore_on_startup: RestoreOnStartup) -> LaunchSurface {
    match restore_on_startup {
        RestoreOnStartup::Launchpad => LaunchSurface::Nothing,
        RestoreOnStartup::Other => LaunchSurface::NewAgentThread,
    }
}

/// The permanent choices at Omega's new-conversation boundary.
///
/// This is intentionally not an open string vocabulary. A conversation owns
/// one of these modes for its lifetime; changing mode means creating another
/// conversation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConversationMode {
    DirectAgent,
    MuseGlimmerLocal,
    OmegaAgent,
    Sarah,
}

impl ConversationMode {
    #[must_use]
    pub const fn all() -> &'static [Self] {
        &[
            Self::DirectAgent,
            Self::MuseGlimmerLocal,
            Self::OmegaAgent,
            Self::Sarah,
        ]
    }

    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::DirectAgent => "Direct Agent",
            Self::MuseGlimmerLocal => "Muse Glimmer (Local)",
            Self::OmegaAgent => "Omega Agent",
            Self::Sarah => "Sarah",
        }
    }

    #[must_use]
    pub const fn token(self) -> &'static str {
        match self {
            Self::DirectAgent => "direct-agent",
            Self::MuseGlimmerLocal => "muse-glimmer-local",
            Self::OmegaAgent => "omega-agent",
            Self::Sarah => "sarah",
        }
    }
}

/// The executor identity resolved before a conversation may be created.
///
/// The direct-agent identifier is the exact persisted ACP identifier. It is
/// never normalized to the native Omega identifier, because doing so would
/// turn an unavailable direct executor into a different conversation mode.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct DirectAgentId(String);

impl DirectAgentId {
    #[must_use]
    pub fn new(agent_id: impl Into<String>) -> Option<Self> {
        let agent_id = agent_id.into();
        (!agent_id.trim().is_empty()).then_some(Self(agent_id))
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum ConversationTarget {
    DirectAgent { agent_id: DirectAgentId },
    MuseGlimmerLocal,
    OmegaAgent,
    Sarah,
}

impl ConversationTarget {
    #[must_use]
    pub const fn mode(&self) -> ConversationMode {
        match self {
            Self::DirectAgent { .. } => ConversationMode::DirectAgent,
            Self::MuseGlimmerLocal => ConversationMode::MuseGlimmerLocal,
            Self::OmegaAgent => ConversationMode::OmegaAgent,
            Self::Sarah => ConversationMode::Sarah,
        }
    }

    #[must_use]
    pub fn executor_label(&self) -> &str {
        match self {
            Self::DirectAgent { agent_id } => agent_id.as_str(),
            Self::MuseGlimmerLocal => "Muse Glimmer local model",
            Self::OmegaAgent => "Omega router",
            Self::Sarah => "Sarah voice executor",
        }
    }

    /// Interpret an owner identity written under the current persistence
    /// contract as a conversation target.
    ///
    /// Legacy non-null rows may contain executor identities instead of owner
    /// identities. This function has no version marker with which to detect
    /// them, so callers must not treat its result as proof that a legacy row
    /// recorded a conversation owner.
    #[must_use]
    pub fn from_persisted_agent_id(
        agent_id: &str,
        omega_agent_id: &str,
        sarah_agent_id: Option<&str>,
    ) -> Option<Self> {
        if agent_id == omega_agent_id {
            Some(Self::OmegaAgent)
        } else if agent_id == "muse-glimmer-local" {
            Some(Self::MuseGlimmerLocal)
        } else if sarah_agent_id == Some(agent_id) {
            Some(Self::Sarah)
        } else {
            Some(Self::DirectAgent {
                agent_id: DirectAgentId::new(agent_id)?,
            })
        }
    }
}

/// Proof that a particular target completed real session creation.
///
/// Fields stay private so `Ready` cannot be assembled from binary discovery
/// or a successful connection alone. The UI validates this receipt against
/// both the selected target and the still-live prepared session before claim.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreparationReceipt {
    target: ConversationTarget,
    proof: PreparationProof,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum PreparationProof {
    SessionCreated { session_id: String },
    OmegaRouterConnected,
}

impl PreparationReceipt {
    #[must_use]
    pub fn after_session_created(
        target: ConversationTarget,
        session_id: impl Into<String>,
    ) -> Option<Self> {
        let session_id = session_id.into();
        (!session_id.trim().is_empty()).then_some(Self {
            target,
            proof: PreparationProof::SessionCreated { session_id },
        })
    }

    #[must_use]
    pub fn proves(&self, target: &ConversationTarget, session_id: &str) -> bool {
        self.target == *target
            && matches!(
                &self.proof,
                PreparationProof::SessionCreated {
                    session_id: proved_session_id,
                } if proved_session_id == session_id
            )
    }

    /// Proof that Omega's router and executor inventory are connected.
    ///
    /// Omega cannot create the physical executor session until the first
    /// request supplies its structured requirements. Direct Agent keeps using
    /// [`after_session_created`](Self::after_session_created), so this receipt
    /// cannot weaken its physical-session readiness gate.
    #[must_use]
    pub fn after_omega_router_connected() -> Self {
        Self {
            target: ConversationTarget::OmegaAgent,
            proof: PreparationProof::OmegaRouterConnected,
        }
    }

    #[must_use]
    pub fn proves_omega_router_connected(&self) -> bool {
        self.target == ConversationTarget::OmegaAgent
            && self.proof == PreparationProof::OmegaRouterConnected
    }
}

/// A real action that can satisfy a setup requirement.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModeSetupAction {
    OpenFolder,
    AddAcpAgent,
    PrepareMuseGlimmerLocal,
    PrepareOmegaAgent,
    PrepareDirectAgent,
    RevealPreparedConversation,
}

impl ModeSetupAction {
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::OpenFolder => "Choose Folder",
            Self::AddAcpAgent => "Add an ACP Agent",
            Self::PrepareMuseGlimmerLocal => "Start Muse Glimmer session",
            Self::PrepareOmegaAgent => "Start Omega session",
            Self::PrepareDirectAgent => "Start direct session",
            Self::RevealPreparedConversation => "Open setup",
        }
    }
}

/// Readiness observed at the new-conversation boundary.
///
/// `Ready` means a connection and an actual session both exist. Detection of
/// an executable, configuration file, or registered server is not readiness.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ModeReadiness {
    Ready {
        receipt: PreparationReceipt,
    },
    SetupRequired {
        reason: String,
        action: ModeSetupAction,
    },
    TemporarilyUnavailable {
        reason: String,
    },
    NotSupportedInBuild {
        reason: String,
    },
}

impl ModeReadiness {
    #[must_use]
    pub const fn label(&self) -> &'static str {
        match self {
            Self::Ready { .. } => "Ready",
            Self::SetupRequired { .. } => "Setup required",
            Self::TemporarilyUnavailable { .. } => "Temporarily unavailable",
            Self::NotSupportedInBuild { .. } => "Not supported in this build",
        }
    }

    #[must_use]
    pub fn reason(&self) -> Option<&str> {
        match self {
            Self::Ready { .. } => None,
            Self::SetupRequired { reason, .. }
            | Self::TemporarilyUnavailable { reason }
            | Self::NotSupportedInBuild { reason } => Some(reason),
        }
    }

    #[must_use]
    pub const fn setup_action(&self) -> Option<ModeSetupAction> {
        match self {
            Self::SetupRequired { action, .. } => Some(*action),
            Self::Ready { .. }
            | Self::TemporarilyUnavailable { .. }
            | Self::NotSupportedInBuild { .. } => None,
        }
    }
}

// -------------------------------------------------------------------------
// Executor disclosure
// -------------------------------------------------------------------------

/// The three admitted executor classes.
///
/// ProductSpec `OMEGA-AGENT-AC-04` fixes the set at exactly three. A fourth
/// class needs a new spec revision, which is why this is a closed enum and not
/// a string.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExecutorClass {
    /// The inherited native agent loop in `crates/agent`.
    NativeLoop,
    /// An external ACP agent reached through `crates/agent_servers`.
    ExternalAcp,
    /// An `omega-effectd` engine lane. Full Auto and Agent Computer.
    EngineLane,
}

impl ExecutorClass {
    /// The stable wire token for this class.
    ///
    /// Persisted and compared. Never shown to a user on its own — the user
    /// sees [`ExecutorDisclosure::label`], which is derived.
    #[must_use]
    pub const fn token(self) -> &'static str {
        match self {
            Self::NativeLoop => "native_loop",
            Self::ExternalAcp => "external_acp",
            Self::EngineLane => "engine_lane",
        }
    }

    /// Every admitted class, in declaration order.
    #[must_use]
    pub const fn all() -> &'static [Self] {
        &[Self::NativeLoop, Self::ExternalAcp, Self::EngineLane]
    }
}

/// What a thread says about the executor that did its work.
///
/// **This is a record, not a label.** The owner admitted the Omega Agent shape
/// on 2026-07-25 on the recorded condition that the first-party agent does not
/// sign with its own principal *and* that disclosure is stored as a typed
/// record that a label renders. That condition is the only reason the identity
/// choice stays cheap to reverse: moving to a signing principal later then
/// needs a signer, not a rewrite of every stored thread record.
///
/// So [`label`](Self::label) is a function of the fields, and there is no
/// field to put a rendered label in. omega#76 fixed this shape; omega#77
/// populates it from live threads and renders the line.
///
/// omega#82 tightened how that is checked. The old check dumped the struct and
/// failed if the word `label` appeared in it — a **denylist**, which passes for
/// a field called `line`, `text`, `summary`, or `rendered`. The check now
/// asserts the declared fields **exactly** against
/// [`EXECUTOR_DISCLOSURE_FIELDS`], so any new field at all is a deliberate edit
/// to a list that says why the shape is closed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExecutorDisclosure {
    /// Which of the three admitted classes ran the work.
    pub class: ExecutorClass,
    /// The executor's own identifier, as the executor reports it.
    pub agent_id: String,
    /// The model provider that served the turn, where the executor reports
    /// one.
    ///
    /// `None` is *not disclosed*, and is different from an empty string, which
    /// is a bug. `AcpConnection` does not implement
    /// `AgentConnection::model_selector`, so an external ACP agent such as
    /// `codex-acp` genuinely does not tell Omega which model served a turn.
    /// Before omega#77 this was a `String`, which left only two options for
    /// that case — fabricate a model, or fail [`is_coherent`](Self::is_coherent)
    /// on every external thread. Saying "not disclosed" is the honest third.
    pub provider: Option<String>,
    /// The model that served the turn, where the executor reports one. See
    /// [`provider`](Self::provider) for why this is optional.
    pub model: Option<String>,
    /// The engine run this thread is bound to, where one applies.
    ///
    /// `Some` only for [`ExecutorClass::EngineLane`]: a native or ACP turn has
    /// no run authority to reference.
    pub run_ref: Option<String>,
    /// Why Omega Agent routed the thread here, where it routed the thread.
    ///
    /// A typed part, added by omega#78 — not a caption. `None` means this
    /// thread was not routed by the router: a thread restored from before
    /// `OMEGA-DELTA-0029`, or one opened directly on an executor. Saying "not
    /// routed" is different from claiming a reason nobody recorded, and the
    /// difference matters most for the fallback reasons, which are the ones a
    /// user needs to see.
    ///
    /// This is the field that makes an engine-down fallback *visible*. A
    /// fallback the user cannot see is the same defect class as a handoff with
    /// no system note.
    pub route: Option<RouteReason>,
}

impl ExecutorDisclosure {
    /// Render the disclosure line for a thread.
    ///
    /// Derived on every call. Nothing stores the output.
    ///
    /// An undisclosed model is *said*, not skipped. A line that quietly
    /// dropped the model segment would read as a complete disclosure, and the
    /// reader would have no way to tell "Omega did not ask" from "the executor
    /// would not say".
    ///
    /// This is the **record's** line: it names the model by its wire pair,
    /// which is what a receipt, a copied system spec and a machine reader want.
    /// A person's chrome wants the model's own name, and asks
    /// [`label_with_model`](Self::label_with_model) for the same line with that
    /// phrase substituted. See `OMEGA-DELTA-0208`.
    #[must_use]
    pub fn label(&self) -> String {
        self.label_with_model(&self.model_phrase())
    }

    /// How the record itself names the model: the `provider/model` wire pair,
    /// or exactly which half is undisclosed.
    ///
    /// Separated from [`label`](Self::label) so that a caller substituting a
    /// human model name has a defined answer for the undisclosed cases too,
    /// rather than inventing a second vocabulary for them.
    #[must_use]
    pub fn model_phrase(&self) -> String {
        match (&self.provider, &self.model) {
            (Some(provider), Some(model)) => format!("{provider}/{model}"),
            (Some(provider), None) => format!("{provider}/model not disclosed"),
            (None, Some(model)) => format!("provider not disclosed/{model}"),
            (None, None) => "model not disclosed".to_owned(),
        }
    }

    /// The same line, naming the model with the phrase the caller supplies.
    ///
    /// `OMEGA-DELTA-0208`. The composer chrome drew this record's line *and* a
    /// second line holding the model's own name, so a person read
    /// `Omega Agent · openagents/kimi-k3` above `Kimi K3` — the same fact
    /// twice, once in a vocabulary the surface is not allowed to teach. The
    /// owner: "remove the `openagents/gpt-5.6-luna` … its duplicative with gpt
    /// 5.6 luna like the real name."
    ///
    /// The shape of the line — who ran it, then the model, then the run, then a
    /// fallback if there was one — is decided **here and only here**, so a
    /// surface choosing a different word for the model cannot also drift into a
    /// different line. The wire pair is not deleted; it is what
    /// [`label`](Self::label) still renders for receipts and machine readers.
    #[must_use]
    pub fn label_with_model(&self, model: &str) -> String {
        // omega#100. The class token is not shown to a person.
        //
        // `native_loop`, `external_acp` and `engine_lane` are wire tokens. They
        // are persisted and compared, and the doc on `token` already says they
        // are never shown to a user on their own — but this line showed one
        // anyway, first in the row. The owner read it and said so: "i have no
        // fucking clue what youre talking about so the user won't". The agent
        // id beside it already answers the question a person is actually
        // asking, which is who ran this. The token stays in the record, where
        // machines read it.
        let mut line = format!("{} · {model}", self.agent_id);
        if let Some(run_ref) = &self.run_ref {
            line.push_str(" · ");
            line.push_str(run_ref);
        }
        // omega#78, narrowed by omega#100. A fallback is still always said. An
        // ordinary route is not.
        //
        // This said "routed: unpinned" on every ordinary turn, which is the
        // steady state and therefore tells a reader nothing. Saying it
        // everywhere also spends the reader's attention on the case that does
        // not matter, which is how the case that does matter stops being read.
        // `is_fallback` already draws exactly this line: `UnpinnedDefault` and
        // `PinHonored` are the two non-events, and every other reason is an
        // executor the thread did not get.
        if let Some(route) = self.route
            && route.is_fallback()
        {
            line.push_str(" · routed: ");
            line.push_str(route.phrase());
        }
        line
    }

    /// Whether this record is internally consistent.
    ///
    /// A run reference on a native turn means something mislabelled a routed
    /// result, which `OMEGA-AGENT-AC-05` exists to prevent. An identifier that
    /// is present but empty means something built the record out of a missing
    /// value instead of leaving it absent, so it stays incoherent.
    ///
    /// omega#78 adds two clauses about the route. A record that says a pin
    /// could not be honoured while showing an engine lane is claiming both that
    /// the router fell back and that it did not; and a record that says the
    /// thread was unpinned while showing an engine lane is owner gate 8 broken,
    /// because nothing but a human pin may reach Full Auto authority.
    #[must_use]
    pub fn is_coherent(&self) -> bool {
        let present_and_named =
            |value: &Option<String>| value.as_ref().is_none_or(|value| !value.is_empty());
        let route_agrees_with_the_class = match self.route {
            Some(route) if route.is_fallback() || route == RouteReason::UnpinnedDefault => {
                self.class == ExecutorClass::NativeLoop
            }
            Some(_) | None => true,
        };
        !self.agent_id.is_empty()
            && present_and_named(&self.provider)
            && present_and_named(&self.model)
            && route_agrees_with_the_class
            && match self.class {
                ExecutorClass::EngineLane => self.run_ref.is_some(),
                ExecutorClass::NativeLoop | ExecutorClass::ExternalAcp => self.run_ref.is_none(),
            }
    }
}

/// Every field [`ExecutorDisclosure`] is allowed to have, in declaration order.
///
/// Asserted **exactly** against the struct's own source by
/// `the_disclosure_record_holds_no_rendered_label`. The point of an exact list
/// rather than a denylist is that a denylist only catches the names its author
/// thought of: a `label` field fails a `contains("label")` check, and `line`,
/// `text`, `summary`, `rendered`, and `caption` all sail through it. The
/// binding condition of the owner's 2026-07-25 identity decision is that
/// disclosure is a *record a label renders*, and a stored rendering under any
/// name breaks it, not just one spelled `label`.
pub const EXECUTOR_DISCLOSURE_FIELDS: &[&str] =
    &["class", "agent_id", "provider", "model", "run_ref", "route"];

// -------------------------------------------------------------------------
// Where a session was reached from
// -------------------------------------------------------------------------

/// How a session reached Omega Agent. omega#82.
///
/// **This is not an [`ExecutorClass`].** `ExecutorClass` answers *who ran the
/// work*; this answers *who asked*. Serving Omega Agent over ACP to an external
/// host changes the second and nothing about the first: the turn is still
/// executed by the native loop, an external ACP agent, or an engine lane, and
/// which one is a fact about the route decision rather than about the socket
/// the request arrived on.
///
/// Reusing [`ExecutorClass::ExternalAcp`] for a served session would be
/// actively wrong rather than merely imprecise. It would make a served
/// session's disclosure say *an external ACP agent did this work* when Omega
/// did, and it would make one token mean two opposite things depending on which
/// side of the socket the reader is standing on — Omega-as-client reaching out
/// to a foreign agent, and Omega-as-agent being reached by a foreign host. A
/// disclosure record whose meaning depends on the reader's vantage point is not
/// a disclosure record. A fourth `ExecutorClass` variant is wrong for the same
/// reason: there is no fourth *executor*, and `ServedOverAcp` would be an
/// ingress fact wearing an execution field's clothes.
///
/// So `OMEGA-AGENT-AC-04` stands unrevised at exactly three executor classes,
/// and ingress is modelled here instead. That also keeps the identity decision
/// cheap to reverse in the direction that matters: an origin record can be
/// added and removed without rewriting a single stored thread record, whereas
/// re-pointing `ExternalAcp` would retroactively change what every record
/// already written meant.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Ingress {
    /// The session was started inside Omega, by a person at the front door.
    InApp,
    /// The session was reached over the loopback ACP server by an external
    /// host. `OMEGA-DELTA-0041`.
    LoopbackAcp,
}

impl Ingress {
    /// Every admitted ingress, in declaration order.
    #[must_use]
    pub const fn all() -> &'static [Self] {
        &[Self::InApp, Self::LoopbackAcp]
    }

    /// The stable wire token for this ingress.
    #[must_use]
    pub const fn token(self) -> &'static str {
        match self {
            Self::InApp => "in_app",
            Self::LoopbackAcp => "loopback_acp",
        }
    }
}

/// What a session says about where it was reached from.
///
/// A record, on the same terms as [`ExecutorDisclosure`]: [`label`](Self::label)
/// is derived on every call and nothing stores the output, so there is no field
/// here to put a rendered sentence in either.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionOrigin {
    /// How the session was reached.
    pub ingress: Ingress,
    /// The name the external host gave for itself, where one did.
    ///
    /// `None` is *not disclosed*. An in-app session has no host to name, and a
    /// served session whose client sent no `clientInfo` genuinely did not say.
    pub host_name: Option<String>,
    /// The version the external host gave for itself, where one did.
    pub host_version: Option<String>,
    /// Whether the host authenticated.
    ///
    /// Always `false` for [`Ingress::LoopbackAcp`], and
    /// [`is_coherent`](Self::is_coherent) enforces it: the served surface
    /// declares an empty `authMethods` list, so a served session claiming an
    /// authenticated host is claiming a credential no one could have presented.
    pub authenticated: bool,
}

impl SessionOrigin {
    /// The origin of a session a person started inside Omega.
    #[must_use]
    pub const fn in_app() -> Self {
        Self {
            ingress: Ingress::InApp,
            host_name: None,
            host_version: None,
            authenticated: false,
        }
    }

    /// The origin of a session an external host reached over the loopback ACP
    /// server.
    #[must_use]
    pub const fn loopback_acp(host_name: Option<String>, host_version: Option<String>) -> Self {
        Self {
            ingress: Ingress::LoopbackAcp,
            host_name,
            host_version,
            authenticated: false,
        }
    }

    /// Render the origin line for a session.
    ///
    /// Derived on every call. Nothing stores the output. An undisclosed host is
    /// *said*, for the same reason [`ExecutorDisclosure::label`] says an
    /// undisclosed model: a line that quietly dropped the host would read as a
    /// complete origin.
    #[must_use]
    pub fn label(&self) -> String {
        let host = match (&self.host_name, &self.host_version) {
            (Some(name), Some(version)) => format!("{name} {version}"),
            (Some(name), None) => format!("{name}, version not disclosed"),
            (None, _) => "host not disclosed".to_owned(),
        };
        match self.ingress {
            Ingress::InApp => "in_app".to_owned(),
            Ingress::LoopbackAcp => {
                format!("loopback_acp · {host} · unauthenticated")
            }
        }
    }

    /// Whether this record is internally consistent.
    #[must_use]
    pub fn is_coherent(&self) -> bool {
        let present_and_named =
            |value: &Option<String>| value.as_ref().is_none_or(|value| !value.is_empty());
        let named_host_matches_the_ingress = match self.ingress {
            // An in-app session has no external host, so naming one means the
            // record was built from the wrong session.
            Ingress::InApp => self.host_name.is_none() && self.host_version.is_none(),
            Ingress::LoopbackAcp => true,
        };
        // The served surface offers no authentication method at all, so an
        // authenticated served session is a credential nobody could present.
        let unauthenticated_where_it_must_be =
            self.ingress != Ingress::LoopbackAcp || !self.authenticated;
        present_and_named(&self.host_name)
            && present_and_named(&self.host_version)
            && named_host_matches_the_ingress
            && unauthenticated_where_it_must_be
    }
}

/// Every field [`SessionOrigin`] is allowed to have, in declaration order.
///
/// Asserted exactly, for the reason [`EXECUTOR_DISCLOSURE_FIELDS`] gives.
pub const SESSION_ORIGIN_FIELDS: &[&str] =
    &["ingress", "host_name", "host_version", "authenticated"];

/// The `pub` field names a struct declares, read from source text.
///
/// A lexical scan rather than reflection, because Rust has none: the check that
/// matters is "what does this struct declare", and the source is where that is
/// written. Starts at `pub struct {name} {` and stops at the first line that is
/// exactly `}`, which is where rustfmt puts a struct's closing brace.
#[must_use]
pub fn declared_struct_fields(source: &str, struct_name: &str) -> Vec<String> {
    let opening = format!("pub struct {struct_name} {{");
    let mut fields = Vec::new();
    let mut inside = false;
    for line in source.lines() {
        if !inside {
            inside = line.trim_start().starts_with(&opening);
            continue;
        }
        if line == "}" {
            break;
        }
        let trimmed = line.trim_start();
        let Some(rest) = trimmed.strip_prefix("pub ") else {
            continue;
        };
        if let Some((name, _)) = rest.split_once(':') {
            fields.push(name.trim().to_owned());
        }
    }
    fields
}

// -------------------------------------------------------------------------
// Who may start Full Auto
// -------------------------------------------------------------------------

/// Every gesture that may start Full Auto authority.
///
/// Owner gate 8, as restated for the fold: *no model-initiated path can start
/// Full Auto authority; only an explicit human action can, wherever that
/// action lives.* Folding Full Auto into chat moves where the action lives. It
/// does not add a new way to reach it.
///
/// Every variant here is a pointer device or keyboard gesture on a control the
/// user can see. There is deliberately no variant for a tool call, a slash
/// command, a restored draft, an agent turn, or a composer mode flag — and
/// `origins_are_all_human_gestures` fails if one appears, because the set is
/// asserted against a written allowlist rather than merely being short today.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LaunchOrigin {
    /// The Full Auto entry in the agent panel's new-thread menu.
    NewThreadMenuItem,
    /// The `full_auto_panel::OpenLauncher` action, dispatched from a keymap or
    /// the command palette. Retained across the fold so no existing user
    /// keybinding stops working.
    OpenLauncherAction,
    /// The `+` button on the run monitor rail.
    RunMonitorNewRun,
    /// The "New run" button on a finished run's surface.
    RunSurfaceNewRun,
}

impl LaunchOrigin {
    /// Every admitted origin, in declaration order.
    #[must_use]
    pub const fn all() -> &'static [Self] {
        &[
            Self::NewThreadMenuItem,
            Self::OpenLauncherAction,
            Self::RunMonitorNewRun,
            Self::RunSurfaceNewRun,
        ]
    }

    /// The stable token for this origin, recorded with the run.
    #[must_use]
    pub const fn token(self) -> &'static str {
        match self {
            Self::NewThreadMenuItem => "new_thread_menu_item",
            Self::OpenLauncherAction => "open_launcher_action",
            Self::RunMonitorNewRun => "run_monitor_new_run",
            Self::RunSurfaceNewRun => "run_surface_new_run",
        }
    }
}

/// Reaching the launch surface is not starting a run.
///
/// Every [`LaunchOrigin`] opens the Full Auto launch surface with an unsent
/// draft. Starting the run is a second, separate human act on the "Start Full
/// Auto" button. Two gestures, both human, is the whole guard — and it is the
/// reason the fold does not need a mode flag on the composer.
#[must_use]
pub const fn origin_starts_a_run(_origin: LaunchOrigin) -> bool {
    false
}

// -------------------------------------------------------------------------
// Who may pin an executor
// -------------------------------------------------------------------------

/// Every gesture that may set a thread's executor pin.
///
/// A pin is the only way a thread reaches anything but the native loop, and
/// [`ExecutorClass::EngineLane`] *is* Full Auto authority. So owner gate 8 —
/// *no model-initiated path can start Full Auto authority; only an explicit
/// human action can* — reaches the pin as directly as it reaches the Start
/// button. omega#76 rejected a composer mode flag for Full Auto because a
/// boolean the send path reads can be set by a slash command, a restored
/// draft, or a model-authored insertion. A pin is the same construct wearing a
/// different name, and it gets the same treatment.
///
/// The mechanism is a *required argument*, not a convention. `pin_session` and
/// `pin_next_session` in `crates/agent_ui/src/omega_router.rs` take a
/// `PinGesture`, so there is no way to set a pin without naming the gesture
/// that set it, and `omega_deltas` asserts every call site passes a literal
/// variant rather than a value it was handed. A tool call, a slash command, a
/// restored draft, an agent turn, and a composer mode flag each have no
/// variant here, and `pin_gestures_are_all_human_gestures` fails if one
/// appears.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PinGesture {
    /// A click on an entry of the executor pin menu, on the thread's own
    /// disclosure line.
    ExecutorPinMenuItem,
    /// A click on the "unpin" entry of the same menu, clearing the pin.
    ExecutorPinCleared,
}

impl PinGesture {
    /// Every admitted gesture, in declaration order.
    #[must_use]
    pub const fn all() -> &'static [Self] {
        &[Self::ExecutorPinMenuItem, Self::ExecutorPinCleared]
    }

    /// The stable token this gesture is logged under.
    #[must_use]
    pub const fn token(self) -> &'static str {
        match self {
            Self::ExecutorPinMenuItem => "executor_pin_menu_item",
            Self::ExecutorPinCleared => "executor_pin_cleared",
        }
    }
}

// -------------------------------------------------------------------------
// The affordance ledger
// -------------------------------------------------------------------------

/// Where a Full Auto panel affordance lives after the fold.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FoldedHome {
    /// The same view, rendered by the agent panel instead of by a dock panel.
    /// Nothing about the control changed except its parent.
    AgentPanelFullAutoSurface,
    /// Withdrawn, with the reason. Read the reason before assuming it was an
    /// oversight.
    Withdrawn(&'static str),
}

/// One interactive affordance of the Full Auto panel, and its home after the
/// fold.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Affordance {
    /// The GPUI element id, exactly as written in
    /// `crates/full_auto_ui/src/`.
    pub element_id: &'static str,
    /// What the control does, in the user's terms.
    pub does: &'static str,
    /// Where it lives now.
    pub home: FoldedHome,
}

/// Every interactive affordance the Full Auto panel expressed before the fold.
///
/// The owner asked for the panel to be folded into the Omega chat UI. "Folded"
/// is not "reduced", so this table exists to make a loss impossible to commit
/// by accident: `every_full_auto_affordance_is_mapped` scans
/// `crates/full_auto_ui/src/` for element ids and fails if one is missing here
/// or listed here and gone from the source.
///
/// If you are adding a control to the Full Auto surface and this test failed:
/// add a row. That is the whole fix, and the row is the record of where your
/// control lives after the fold.
pub const FULL_AUTO_AFFORDANCES: &[Affordance] = &[
    Affordance {
        element_id: "full-auto-panel",
        does: "The launch and run surface root, and its focus target.",
        home: FoldedHome::AgentPanelFullAutoSurface,
    },
    Affordance {
        element_id: "full-auto-openagents-connect",
        does: "Connect or reconnect the OpenAgents Sync account.",
        home: FoldedHome::AgentPanelFullAutoSurface,
    },
    Affordance {
        element_id: "full-auto-openagents-disconnect",
        does: "Revoke the OpenAgents Sync account credentials.",
        home: FoldedHome::AgentPanelFullAutoSurface,
    },
    Affordance {
        element_id: "full-auto-provider-account",
        does: "One connected provider account: readiness, quota, lane, ref.",
        home: FoldedHome::AgentPanelFullAutoSurface,
    },
    Affordance {
        element_id: "full-auto-advanced-toggle",
        does: "Reveal title, done condition, and turn cap on the draft.",
        home: FoldedHome::AgentPanelFullAutoSurface,
    },
    Affordance {
        element_id: "full-auto-start",
        does: "Start the run. The only path to engine-lane run authority.",
        home: FoldedHome::AgentPanelFullAutoSurface,
    },
    Affordance {
        element_id: "full-auto-cancel",
        does: "Clear the draft without starting anything.",
        home: FoldedHome::AgentPanelFullAutoSurface,
    },
    Affordance {
        element_id: "full-auto-pause",
        does: "Pause a running run.",
        home: FoldedHome::AgentPanelFullAutoSurface,
    },
    Affordance {
        element_id: "full-auto-resume",
        does: "Resume a paused run.",
        home: FoldedHome::AgentPanelFullAutoSurface,
    },
    Affordance {
        element_id: "full-auto-handoff",
        does: "Hand a paused run to the other local lane.",
        home: FoldedHome::AgentPanelFullAutoSurface,
    },
    Affordance {
        element_id: "full-auto-retry",
        does: "Retry a stalled run whose recovery action is retry_now.",
        home: FoldedHome::AgentPanelFullAutoSurface,
    },
    Affordance {
        element_id: "full-auto-stop",
        does: "Stop a non-terminal run.",
        home: FoldedHome::AgentPanelFullAutoSurface,
    },
    Affordance {
        element_id: "full-auto-new",
        does: "Return to the launch surface from a run.",
        home: FoldedHome::AgentPanelFullAutoSurface,
    },
    Affordance {
        element_id: "full-auto-evidence-chain",
        does: "The host-verified evidence chain for the active run.",
        home: FoldedHome::AgentPanelFullAutoSurface,
    },
    Affordance {
        element_id: "full-auto-monitor",
        does: "The concurrent run monitor rail.",
        home: FoldedHome::AgentPanelFullAutoSurface,
    },
    Affordance {
        element_id: "full-auto-monitor-new",
        does: "Start a new Full Auto draft from the monitor rail.",
        home: FoldedHome::AgentPanelFullAutoSurface,
    },
    Affordance {
        element_id: "full-auto-run-row",
        does: "Open one run from the monitor rail.",
        home: FoldedHome::AgentPanelFullAutoSurface,
    },
];

/// What the fold costs, stated rather than discovered.
///
/// Every *control* survives the fold — the ledger above proves that
/// mechanically, because the same views render under a new parent. Two
/// capabilities of the panel were not controls, and they do not survive. They
/// are recorded here because the owner asked for a fold, not a reduction, and
/// a reduction that nobody wrote down is indistinguishable from a bug.
pub const FOLD_COSTS: &[&str] = &[
    "Independent dock placement. Full Auto had its own DockPosition::Right and \
     its own 520px default width, so it could sit opposite the agent panel. It \
     now inherits the agent panel's dock and size.",
    "Simultaneous full detail. A separate dock panel could show a run's full \
     detail while the agent panel showed a chat thread. One panel shows one \
     surface at a time, so watching a run in full while typing in a thread is \
     no longer possible. Active runs still render on the front door beneath \
     the composer, and the monitor rail still lists them, so noticing a run is \
     preserved; reading one in full alongside a thread is not.",
];

// -------------------------------------------------------------------------
// Checks
// -------------------------------------------------------------------------

/// Repository-root-relative path resolution.
///
/// `CARGO_MANIFEST_DIR` is `crates/omega_front_door`, so the root is two
/// levels up.
#[must_use]
pub fn repository_path(relative: &str) -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(relative)
}

/// Every `"full-auto-…"` string literal in a source file.
///
/// GPUI element ids are string literals, so this is a lexical scan. It is
/// deliberately generous: it will pick up a `"full-auto-…"` literal that is
/// not an element id, which fails the ledger check and forces a human to look.
/// The opposite error — missing a real control — is the one that would let a
/// capability disappear silently.
#[must_use]
pub fn full_auto_element_ids(source: &str) -> std::collections::BTreeSet<String> {
    let mut ids = std::collections::BTreeSet::new();
    let bytes = source.as_bytes();
    let needle = b"\"full-auto";
    let mut index = 0;
    while index + needle.len() <= bytes.len() {
        if &bytes[index..index + needle.len()] == needle {
            let start = index + 1;
            if let Some(offset) = source[start..].find('"') {
                ids.insert(source[start..start + offset].to_owned());
                index = start + offset + 1;
                continue;
            }
        }
        index += 1;
    }
    ids
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_new_conversation_mode_set_is_closed_and_ordered() {
        assert_eq!(
            ConversationMode::all(),
            &[
                ConversationMode::DirectAgent,
                ConversationMode::MuseGlimmerLocal,
                ConversationMode::OmegaAgent,
                ConversationMode::Sarah,
            ]
        );
        assert_eq!(
            ConversationMode::all()
                .iter()
                .map(|mode| mode.label())
                .collect::<Vec<_>>(),
            [
                "Direct Agent",
                "Muse Glimmer (Local)",
                "Omega Agent",
                "Sarah"
            ]
        );
    }

    // `is_mode_activation_key` and its pointer-equivalence test left with the
    // full-screen chooser (omega#165). The composer executor dropdown's
    // keyboard path is `agent::ToggleComposerExecutorMenu` plus the context
    // menu's own key handling, which needs no bespoke key predicate here.

    #[test]
    fn readiness_has_exactly_four_honest_states() {
        let Some(receipt) =
            PreparationReceipt::after_session_created(ConversationTarget::OmegaAgent, "session-1")
        else {
            panic!("a non-empty session id must produce a receipt");
        };
        let states = [
            ModeReadiness::Ready { receipt },
            ModeReadiness::SetupRequired {
                reason: "folder required".into(),
                action: ModeSetupAction::OpenFolder,
            },
            ModeReadiness::TemporarilyUnavailable {
                reason: "provider offline".into(),
            },
            ModeReadiness::NotSupportedInBuild {
                reason: "audio omitted".into(),
            },
        ];
        assert_eq!(
            states.iter().map(ModeReadiness::label).collect::<Vec<_>>(),
            [
                "Ready",
                "Setup required",
                "Temporarily unavailable",
                "Not supported in this build",
            ]
        );
        assert_eq!(states[1].setup_action(), Some(ModeSetupAction::OpenFolder));
        assert!(states[2].setup_action().is_none());
    }

    #[test]
    fn direct_targets_preserve_the_exact_persisted_agent_id() {
        let Some(target) =
            ConversationTarget::from_persisted_agent_id("codex.acp/nightly", "omega", None)
        else {
            panic!("a non-empty direct agent id must resolve");
        };
        let Some(agent_id) = DirectAgentId::new("codex.acp/nightly") else {
            panic!("a non-empty direct agent id must be valid");
        };
        assert_eq!(target, ConversationTarget::DirectAgent { agent_id });
        assert_eq!(target.mode(), ConversationMode::DirectAgent);
        assert_eq!(target.executor_label(), "codex.acp/nightly");
        assert_eq!(
            ConversationTarget::from_persisted_agent_id("omega", "omega", None),
            Some(ConversationTarget::OmegaAgent)
        );
        assert_eq!(
            ConversationTarget::from_persisted_agent_id("muse-glimmer-local", "omega", None),
            Some(ConversationTarget::MuseGlimmerLocal)
        );
        assert!(ConversationTarget::from_persisted_agent_id("", "omega", None).is_none());
    }

    #[test]
    fn readiness_receipts_are_bound_to_the_exact_target_and_session() {
        let target = ConversationTarget::OmegaAgent;
        let Some(receipt) = PreparationReceipt::after_session_created(target.clone(), "session-1")
        else {
            panic!("a non-empty session id must produce a receipt");
        };
        assert!(receipt.proves(&target, "session-1"));
        assert!(!receipt.proves(&ConversationTarget::Sarah, "session-1"));
        assert!(!receipt.proves(&target, "session-2"));
        assert!(PreparationReceipt::after_session_created(target, "").is_none());
    }

    /// The front door replaces the empty buffer and nothing else.
    #[test]
    fn a_fresh_launch_lands_on_the_new_agent_thread_surface() {
        assert_eq!(
            launch_surface(RestoreOnStartup::Other),
            LaunchSurface::NewAgentThread,
            "a window with nothing to restore must open on the front door, \
             not on upstream Zed's empty untitled buffer"
        );
    }

    /// Overriding a deliberate launchpad choice would be Omega ignoring a
    /// setting, which is a different bug from the one the front door fixes.
    #[test]
    fn the_launchpad_startup_behaviour_still_wins() {
        assert_eq!(
            launch_surface(RestoreOnStartup::Launchpad),
            LaunchSurface::Nothing
        );
        assert_ne!(
            launch_surface(RestoreOnStartup::Launchpad),
            LaunchSurface::EmptyBuffer,
            "the launchpad path never opened a buffer and must not start"
        );
    }

    /// The disclosure is a record. A label is derived from it.
    ///
    /// The failure this guards is subtle: storing the rendered line would make
    /// the owner's reversible identity decision irreversible, because moving to
    /// a signing principal would then need every stored thread rewritten.
    #[test]
    fn disclosure_renders_from_its_fields() {
        let disclosure = ExecutorDisclosure {
            class: ExecutorClass::EngineLane,
            agent_id: "codex-local".into(),
            provider: Some("google".into()),
            model: Some("gemini-3.6-flash".into()),
            run_ref: Some("run.abc".into()),
            route: None,
        };
        // omega#100. The class token is no longer rendered. It stays a field,
        // and machines still read it; the reader sees the agent id, which is
        // the answer to the question a person is asking.
        assert_eq!(
            disclosure.label(),
            "codex-local · google/gemini-3.6-flash · run.abc"
        );

        // Change one field and the rendered line follows, which is only true
        // because nothing cached it. `run_ref` carries the change here, since
        // the class no longer reaches the line.
        let mut moved = disclosure;
        moved.class = ExecutorClass::NativeLoop;
        moved.run_ref = None;
        assert_eq!(moved.label(), "codex-local · google/gemini-3.6-flash");
    }

    /// The class token is a wire token, and stays off the rendered line.
    ///
    /// omega#100. `ExecutorClass::token` already documents that it is "never
    /// shown to a user on its own", and the line was leading with one anyway:
    /// `native_loop · Omega Agent · google/gemini-3.6-flash`. The owner read it
    /// and said "i have no fucking clue what youre talking about so the user
    /// won't".
    ///
    /// The record keeps the class. Only the rendering drops it, so a reader is
    /// not asked to learn three internal words to find out who answered them.
    #[test]
    fn the_class_token_is_not_rendered_for_any_class() {
        for class in ExecutorClass::all() {
            let disclosure = ExecutorDisclosure {
                class: *class,
                agent_id: "codex-local".into(),
                provider: Some("google".into()),
                model: Some("gemini-3.6-flash".into()),
                run_ref: None,
                route: None,
            };
            let label = disclosure.label();
            assert!(
                !label.contains(class.token()),
                "OMEGA-DELTA-0021: the label for {class:?} still names its wire \
                 token: {label}"
            );
            assert!(
                label.contains("codex-local"),
                "the agent id must survive: {label}"
            );
        }
    }

    /// An executor that does not report a model is disclosed as not reporting
    /// one. omega#77.
    ///
    /// The failure this guards is a line that reads as complete while a part
    /// of it is missing: `codex-acp` alone gives the reader no way to tell a
    /// fully disclosed thread from a partly disclosed one.
    #[test]
    fn an_undisclosed_model_is_said_rather_than_skipped() {
        let disclosure = ExecutorDisclosure {
            class: ExecutorClass::ExternalAcp,
            agent_id: "codex-acp".into(),
            provider: None,
            model: None,
            run_ref: None,
            route: None,
        };
        assert!(disclosure.is_coherent());
        assert_eq!(disclosure.label(), "codex-acp · model not disclosed");

        let half_known = ExecutorDisclosure {
            provider: Some("openai".into()),
            ..disclosure
        };
        assert_eq!(half_known.label(), "codex-acp · openai/model not disclosed");
    }

    /// A struct with no field to hold a rendered label cannot accidentally
    /// grow one without this test's author noticing.
    ///
    /// omega#82 replaced a denylist with an **exact** field list. The old check
    /// dumped the struct and failed if the word `label` appeared; a field named
    /// `line`, `text`, `summary`, `rendered`, or `caption` passed it, and every
    /// one of those is the same defect. The list is the whole check now: a new
    /// field of any name fails until someone edits `EXECUTOR_DISCLOSURE_FIELDS`
    /// on purpose.
    #[test]
    fn the_disclosure_record_holds_no_rendered_label() {
        let source = std::fs::read_to_string(repository_path(
            "crates/omega_front_door/src/omega_front_door.rs",
        ))
        .expect("this crate's source is readable");
        let declared = declared_struct_fields(&source, "ExecutorDisclosure");
        assert_eq!(
            declared, EXECUTOR_DISCLOSURE_FIELDS,
            "ExecutorDisclosure's declared fields moved. The owner admitted \
             the non-signing identity choice on the condition that disclosure \
             stays a typed record a label renders, so a field holding a \
             rendered line — under any name, not only `label` — breaks it. If \
             this is a deliberate shape change, edit \
             EXECUTOR_DISCLOSURE_FIELDS and say why."
        );

        // The scan reaching nothing would make the assertion above vacuous in
        // one direction, so the list is also checked to be non-empty and to
        // name a field the struct genuinely round-trips.
        assert!(!declared.is_empty(), "the field scan reached no fields");
        let disclosure = ExecutorDisclosure {
            class: ExecutorClass::NativeLoop,
            agent_id: "a".into(),
            provider: Some("p".into()),
            model: Some("m".into()),
            run_ref: None,
            route: None,
        };
        let dumped = format!("{disclosure:?}");
        for field in EXECUTOR_DISCLOSURE_FIELDS {
            assert!(
                dumped.contains(field),
                "EXECUTOR_DISCLOSURE_FIELDS names {field}, which the struct \
                 does not carry: {dumped}"
            );
        }
    }

    /// The field scan finds real fields and stops at the struct it was asked
    /// about.
    ///
    /// A scan that silently matched nothing would make the exactness check
    /// above pass for a struct that had grown a rendered line.
    #[test]
    fn the_field_scan_reads_a_struct_and_stops_at_its_brace() {
        let source = "\
pub struct Wanted {
    /// doc
    pub first: String,
    pub second: Option<u8>,
    private: bool,
}

pub struct Other {
    pub third: String,
}
";
        assert_eq!(
            declared_struct_fields(source, "Wanted"),
            ["first".to_owned(), "second".to_owned()],
            "the scan must read public fields and must not walk into the \
             next struct"
        );
        assert!(declared_struct_fields(source, "Missing").is_empty());
    }

    /// omega#82. Ingress is a separate record from the executor class, and the
    /// executor set stays closed at three.
    #[test]
    fn reaching_over_acp_is_an_origin_and_not_an_executor() {
        // The whole design call, as an assertion: no executor class names a
        // socket direction, so a served session cannot claim an external agent
        // did work Omega did.
        for class in ExecutorClass::all() {
            assert!(
                !class.token().contains("served") && !class.token().contains("ingress"),
                "{} names an ingress fact in an execution field",
                class.token()
            );
        }
        assert_eq!(ExecutorClass::all().len(), 3);

        let origin = SessionOrigin::loopback_acp(Some("Zed".into()), Some("1.12.0".into()));
        assert!(origin.is_coherent());
        assert_eq!(
            origin.label(),
            "loopback_acp · Zed 1.12.0 · unauthenticated"
        );
        assert_eq!(origin.ingress.token(), "loopback_acp");

        let quiet = SessionOrigin::loopback_acp(None, None);
        assert!(quiet.is_coherent());
        assert_eq!(
            quiet.label(),
            "loopback_acp · host not disclosed · unauthenticated"
        );
    }

    /// omega#82. A served session cannot claim it authenticated, and an in-app
    /// session cannot claim an external host.
    #[test]
    fn a_served_origin_cannot_claim_a_credential_nobody_could_present() {
        let claiming = SessionOrigin {
            authenticated: true,
            ..SessionOrigin::loopback_acp(Some("Zed".into()), None)
        };
        assert!(
            !claiming.is_coherent(),
            "the served surface offers no auth method, so an authenticated \
             served session is a credential that could not have been presented"
        );

        let borrowed_host = SessionOrigin {
            host_name: Some("Zed".into()),
            ..SessionOrigin::in_app()
        };
        assert!(!borrowed_host.is_coherent());

        let blank_host = SessionOrigin::loopback_acp(Some(String::new()), None);
        assert!(
            !blank_host.is_coherent(),
            "absent means not disclosed; empty means something built the \
             record out of a missing value and lost the distinction"
        );
    }

    /// The origin record holds no rendered line either, on the same terms.
    #[test]
    fn the_origin_record_holds_no_rendered_label() {
        let source = std::fs::read_to_string(repository_path(
            "crates/omega_front_door/src/omega_front_door.rs",
        ))
        .expect("this crate's source is readable");
        assert_eq!(
            declared_struct_fields(&source, "SessionOrigin"),
            SESSION_ORIGIN_FIELDS,
            "SessionOrigin's declared fields moved. Ingress is disclosed the \
             same way execution is: a typed record a label renders."
        );
    }

    /// A run reference on a native turn is a routed result wearing the wrong
    /// name.
    #[test]
    fn only_an_engine_lane_carries_a_run_reference() {
        let base = ExecutorDisclosure {
            class: ExecutorClass::EngineLane,
            agent_id: "codex-local".into(),
            provider: Some("google".into()),
            model: Some("gemini-3.6-flash".into()),
            run_ref: Some("run.abc".into()),
            route: None,
        };
        assert!(base.is_coherent());

        let mut engine_without_run = base.clone();
        engine_without_run.run_ref = None;
        assert!(!engine_without_run.is_coherent());

        let mut native_with_run = base.clone();
        native_with_run.class = ExecutorClass::NativeLoop;
        assert!(!native_with_run.is_coherent());

        // A present-but-empty identifier stays incoherent after omega#77 made
        // these optional. Absent means "not disclosed"; empty means something
        // built the record out of a missing value and lost the distinction.
        let mut blank_model = base.clone();
        blank_model.model = Some(String::new());
        assert!(!blank_model.is_coherent());

        let mut blank_provider = base;
        blank_provider.provider = Some(String::new());
        assert!(!blank_provider.is_coherent());
    }

    /// omega#78. A thread routed by a fallback says so on its own line.
    ///
    /// This is the "prove it is disclosed rather than silent" half of the
    /// engine-down exit property. The record carries the typed reason; the line
    /// renders it; neither stores a sentence.
    #[test]
    fn an_engine_down_fallback_is_visible_on_the_disclosure_line() {
        let decision = route(
            &RouteInputs::native_only()
                .with_engine(EngineReadiness::Unreachable(EngineUnreachable::NotRunning))
                .pinned(ExecutorPin::on_lane("claude-local")),
        );
        assert_eq!(decision.chosen, ExecutorClass::NativeLoop);

        let disclosure = ExecutorDisclosure {
            class: decision.chosen,
            agent_id: "omega-agent".into(),
            provider: Some("anthropic".into()),
            model: Some("claude-opus-5".into()),
            run_ref: None,
            route: Some(decision.disclosed_route()),
        };
        assert!(disclosure.is_coherent());

        let line = disclosure.label();
        assert!(
            line.contains("engine unreachable, fell back to the native loop"),
            "the thread ran somewhere the user did not pin and the line does \
             not say so: {line}"
        );

        // The same record with the route dropped renders a line that reads as
        // an ordinary native thread. That silence is the defect.
        let silent = ExecutorDisclosure {
            route: None,
            ..disclosure
        };
        assert!(!silent.label().contains("fell back"));
    }

    /// omega#78. A record cannot say both "the pin was not honoured" and "an
    /// engine lane ran it", and cannot say an unpinned thread reached Full Auto
    /// authority.
    #[test]
    fn a_route_that_contradicts_the_executor_is_incoherent() {
        let engine_thread = ExecutorDisclosure {
            class: ExecutorClass::EngineLane,
            agent_id: "codex-local".into(),
            provider: None,
            model: None,
            run_ref: Some("run.abc".into()),
            route: Some(RouteReason::PinHonored),
        };
        assert!(engine_thread.is_coherent());

        for contradiction in [
            RouteReason::EngineUnreachable,
            RouteReason::EngineAtCapacity,
            RouteReason::EngineHasNoReadyLane,
            RouteReason::PinnedLaneUnavailable,
            RouteReason::ExternalAcpUnavailable,
            RouteReason::UnrecordableLane,
            // Owner gate 8: nothing unpinned reaches an engine lane.
            RouteReason::UnpinnedDefault,
        ] {
            let lying = ExecutorDisclosure {
                route: Some(contradiction),
                ..engine_thread.clone()
            };
            assert!(
                !lying.is_coherent(),
                "{} on an engine-lane thread passed coherence",
                contradiction.token()
            );
        }
    }

    /// `OMEGA-AGENT-AC-04` fixes the executor set at exactly three.
    #[test]
    fn the_admitted_executor_set_is_exactly_three() {
        let tokens: Vec<&str> = ExecutorClass::all().iter().map(|c| c.token()).collect();
        assert_eq!(tokens, ["native_loop", "external_acp", "engine_lane"]);
    }

    /// Owner gate 8. The allowlist is written out so that adding an origin is
    /// a deliberate edit to a test that says why the list is closed.
    #[test]
    fn origins_are_all_human_gestures() {
        let tokens: Vec<&str> = LaunchOrigin::all().iter().map(|o| o.token()).collect();
        assert_eq!(
            tokens,
            [
                "new_thread_menu_item",
                "open_launcher_action",
                "run_monitor_new_run",
                "run_surface_new_run",
            ],
            "every Full Auto launch origin must be a visible control a person \
             operates. A tool call, a slash command, a restored draft, an agent \
             turn, or a composer mode flag is not one. If you are adding an \
             origin, prove it is a human gesture before you edit this list."
        );
    }

    /// Owner gate 8, at the pin. An engine lane is Full Auto authority and a
    /// pin is the only door to one, so the set of gestures that may set a pin
    /// is closed for the same reason the set of launch origins is.
    #[test]
    fn pin_gestures_are_all_human_gestures() {
        let tokens: Vec<&str> = PinGesture::all().iter().map(|g| g.token()).collect();
        assert_eq!(
            tokens,
            ["executor_pin_menu_item", "executor_pin_cleared"],
            "every executor pin gesture must be a visible control a person \
             operates. A tool call, a slash command, a restored draft, an \
             agent turn, or a composer mode flag is not one — and a pin \
             reaches engine-lane authority, so admitting one of those here is \
             owner gate 8 broken through a door nobody flagged. If you are \
             adding a gesture, prove it is a human one before you edit this \
             list."
        );
    }

    /// Reaching the launch surface is not starting a run.
    #[test]
    fn no_origin_starts_a_run_by_itself() {
        for origin in LaunchOrigin::all() {
            assert!(
                !origin_starts_a_run(*origin),
                "{} reached the launch surface and started a run in one \
                 gesture; the Start button is the second human act",
                origin.token()
            );
        }
    }

    /// The fold carries every control, and the ledger proves it against the
    /// source rather than against its author's memory.
    #[test]
    fn every_full_auto_affordance_is_mapped() {
        let directory = repository_path("crates/full_auto_ui/src");
        let mut found = std::collections::BTreeSet::new();
        for entry in std::fs::read_dir(&directory).expect("full_auto_ui sources are readable") {
            let path = entry.expect("directory entry").path();
            if path.extension().and_then(|e| e.to_str()) != Some("rs") {
                continue;
            }
            let source = std::fs::read_to_string(&path).expect("source is readable");
            found.extend(full_auto_element_ids(&source));
        }

        assert!(
            !found.is_empty(),
            "the element-id scan found nothing in {}; the ledger check would \
             be vacuous",
            directory.display()
        );

        let mapped: std::collections::BTreeSet<String> = FULL_AUTO_AFFORDANCES
            .iter()
            .map(|a| a.element_id.to_owned())
            .collect();

        let unmapped: Vec<&String> = found.difference(&mapped).collect();
        assert!(
            unmapped.is_empty(),
            "Full Auto controls with no entry in FULL_AUTO_AFFORDANCES: \
             {unmapped:?}. The owner asked for Full Auto to be folded into the \
             chat UI, not reduced. Add a row saying where your control lives \
             after the fold."
        );

        let stale: Vec<&String> = mapped.difference(&found).collect();
        assert!(
            stale.is_empty(),
            "FULL_AUTO_AFFORDANCES maps controls that no longer exist: \
             {stale:?}. A ledger that outlives its source stops being evidence."
        );
    }

    /// A scan that reaches nothing passes every check it is asked to make.
    #[test]
    fn the_element_id_scan_reaches_real_ids() {
        let ids = full_auto_element_ids(
            r#"Button::new("full-auto-start", "Start").id(("full-auto-run-row", index))"#,
        );
        assert_eq!(ids.len(), 2);
        assert!(ids.contains("full-auto-start"));
        assert!(ids.contains("full-auto-run-row"));

        // An unterminated literal must not swallow the rest of the file.
        assert!(full_auto_element_ids("\"full-auto-start").is_empty());
    }

    /// The costs are stated, not discovered.
    #[test]
    fn the_fold_costs_are_written_down() {
        assert_eq!(
            FOLD_COSTS.len(),
            2,
            "FOLD_COSTS is the honest half of the ledger. If the fold now costs \
             more or less, say so here."
        );
        for cost in FOLD_COSTS {
            assert!(cost.len() > 80, "a cost stated in a phrase is not stated");
        }
    }
}
