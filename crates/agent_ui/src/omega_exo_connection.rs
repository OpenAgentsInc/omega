//! Driving Exo. `OMEGA-DELTA-0042`, omega#87.
//!
//! The law lives in `crates/omega_exo_lane`, which is a leaf. This file starts
//! `exo acp` on standard input and output. `AcpConnection` puts each text delta,
//! tool call, tool result, and completion record into the existing `AcpThread`.
//!
//! # How the lane is reached
//!
//! Through the router omega#78 already built, and through nothing else. The Exo
//! connection registers as [`OmegaAgentConnection`]'s external executor, so a
//! thread reaches it exactly when a person pins `ExecutorClass::ExternalAcp` on
//! the disclosure line's pin control — a `PinGesture`, which has no variant for
//! a tool call, a slash command, a restored draft, or a model turn. Nothing here
//! adds a way to start authority: an unpinned thread still runs on the native
//! loop, and when no Exo lane is configured the pin falls back with
//! `RouteReason::ExternalAcpUnavailable`, which the user sees on the line.
//!
//! # What runs before every turn
//!
//! Four refusals, in order, all of them from live observation rather than from
//! configuration Omega wrote down earlier:
//!
//! 1. **Where Exo is.** `EXO_EXOHARNESS_URL`, which Exo reads from the
//!    environment and which redirects it from the state root on disk to an
//!    unauthenticated HTTP server, is parsed through [`LoopbackEndpoint`] and
//!    refused unless it names this machine.
//! 2. **Which Exo.** `git` in the checkout answers with a remote, a commit, and
//!    a tree, and [`EXO_PIN`] admits or refuses it. This is what stops the
//!    omega#86 mistake — the other Exo — and an upstream that moved under a
//!    no-backwards-compatibility house rule.
//! 3. **Which bytes.** The binary is hashed and compared against the owner's
//!    pin ledger entry for `exo`, when the owner froze one.
//! 4. **Which capability.** Omega reads the agent and conversation. The normal
//!    path refuses self-modification. A one-use grant can authorize one exact
//!    draft after a visible human confirmation.
//!
//! # What is deliberately absent
//!
//! No listener, no bind, no port, no proxy. Omega never stands between Exo's
//! unauthenticated endpoint and anything else — [`LoopbackEndpoint`] exists so
//! that a future caller who *does* need an address cannot build a non-loopback
//! one, and Tier A does not need an address at all, because the CLI talks to
//! the state root on disk. `omega_deltas` asserts the absence, because "we did
//! not add a listener" is the kind of thing that stays true only while somebody
//! is checking.

use std::any::Any;
use std::cell::{Cell, RefCell};
use std::path::PathBuf;
use std::process::Stdio;
use std::rc::Rc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use acp_thread::{AcpThread, AgentConnection};
use agent_client_protocol::schema::v1 as acp;
use agent_servers::AcpConnection;
use anyhow::{Context as _, Result, anyhow, bail};
use gpui::{App, AsyncApp, Entity, SharedString, Task, WeakEntity};
use omega_exo_lane::{
    EXO_HARNESS_ID, EXO_PIN, ExoAgent, ExoCommand, ExoConversation, ExoLaneIdentity,
    ExoModelBinding, ExoMount, ExoRoot, ExoSecretStore, ExoSelfModificationConsentOrigin,
    ExoSelfModificationGrant, ExoSelfModificationGrantRequest, ExoSelfModificationReceipt,
    LoopbackEndpoint, ObservedExoCapabilityState, ObservedExoCheckout, ObservedReadWriteMount,
    ObservedToolModule, admits_bytes,
};
use omega_exo_log::{ExoDurableHistory, ExoHistoryUnavailable, ExoId, ExoReadClient};
use omega_harness::MeasuredDigest;
use project::{
    AgentId, Project,
    agent_server_store::{AgentServerCommand, AgentServerStore},
};
use util::path_list::PathList;

/// Where the lane's configuration lives, under the Omega data directory.
const EXO_LANE_FILE: &str = "omega-exo-lane.json";

/// The schema that file carries.
const EXO_LANE_SCHEMA: &str = "openagents.omega.exo_lane.v1";

static NEXT_EXO_CONNECTION_GENERATION: AtomicU64 = AtomicU64::new(1);

/// `OMEGA-DELTA-0107`. How many durable events one history read asks for.
///
/// A bound rather than `None`. Exo's own default at this pin is "all of them",
/// and the reply is a JSON array whose size is decided by whatever Exo's agent
/// wrote — so an unbounded read of a long-lived conversation is a request whose
/// cost nobody chose.
const EXO_HISTORY_EVENT_LIMIT: u32 = 200;

/// Everything Omega needs to reach one Exo install.
///
/// Every field names something Exo owns. None of them is something Omega
/// writes: the binary was built by whoever installed Exo, the checkout is
/// theirs, and the state root is Exo's — Omega only ever passes its path on a
/// command line. See `ExoRoot`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExoLaneConfig {
    /// The `exo` binary.
    pub binary: PathBuf,
    /// The checkout the binary was built from, for the pin check.
    pub checkout: PathBuf,
    /// Exo's state root.
    pub root: ExoRoot,
    /// The agent slug the lane sends to.
    pub agent: String,
    /// The conversation slug the lane sends to.
    pub conversation: String,
    /// Which key opens this root's secrets. `OMEGA-DELTA-0126`, omega#112.
    ///
    /// The sixth field, and the first one that is optional, because `None` is a
    /// working lane: Exo's default backend opens most roots without being
    /// told. What it is not is *inheritance* — before this field existed the
    /// `exo acp` child got whatever environment Omega was launched with, so the
    /// same root worked from a terminal and failed from the Dock, on the first
    /// message, with `failed to decrypt secret payload`.
    ///
    /// Still nothing Omega writes: it names a key the way every other field
    /// names something Exo owns. See [`ExoSecretStore`].
    pub secret_store: Option<ExoSecretStore>,
}

impl ExoLaneConfig {
    /// The lane file's path under the Omega data directory.
    #[must_use]
    pub fn data_dir_path() -> PathBuf {
        paths::data_dir().join("openagents").join(EXO_LANE_FILE)
    }

    /// Read the lane file, if the owner wrote one.
    ///
    /// A machine with no Exo has no lane, and that is the ordinary case rather
    /// than an error: `None` means the router has no external executor
    /// registered and a pin to one falls back visibly. A file that exists and
    /// cannot be read is logged and treated as no lane, because a half-read
    /// configuration is how a lane ends up pointed at the wrong `.exo`.
    #[must_use]
    pub fn load(path: &std::path::Path) -> Option<Self> {
        let file = std::fs::read_to_string(path).ok()?;
        let value: serde_json::Value = serde_json::from_str(&file)
            .inspect_err(|error| {
                log::warn!(
                    "OMEGA-DELTA-0042: the Exo lane file is not JSON ({error}); no Exo lane"
                );
            })
            .ok()?;
        if value.get("schema").and_then(serde_json::Value::as_str) != Some(EXO_LANE_SCHEMA) {
            log::warn!("OMEGA-DELTA-0042: the Exo lane file carries an unsupported schema");
            return None;
        }
        let field = |name: &str| {
            value
                .get(name)
                .and_then(serde_json::Value::as_str)
                .filter(|value| !value.trim().is_empty())
                .map(str::to_owned)
        };
        // `OMEGA-DELTA-0126`. Two optional fields, read with the same `field`
        // helper as the five required ones, so a blank string means "not
        // stated" here exactly as it does there. A backend name Exo does not
        // have yields `None` from `ExoSecretStore::parse` rather than reaching
        // Exo's command line: the lane then names no store, Exo uses its
        // default, and the person is told by the log rather than by a turn that
        // fails at the model call.
        let secret_store = ExoSecretStore::parse(
            field("secret_backend").as_deref(),
            field("master_key_path").as_ref().map(std::path::Path::new),
        );
        if secret_store.is_none() && field("secret_backend").is_some() {
            log::warn!(
                "OMEGA-DELTA-0126: the Exo lane file names a secret backend Exo does not have; \
                 the lane names no store and Exo will use its own default"
            );
        }
        Some(Self {
            binary: PathBuf::from(field("binary")?),
            checkout: PathBuf::from(field("checkout")?),
            root: ExoRoot::at(field("root")?),
            agent: field("agent")?,
            conversation: field("conversation")?,
            secret_store,
        })
    }

    /// The environment the `exo` child is launched with.
    ///
    /// Empty when the lane names no secret store, which is not the same as
    /// `None`: `AgentServerCommand`'s `env` is applied on top of the inherited
    /// environment, so an empty map and no map do the same thing, and always
    /// producing a map means the code path that carries the store is the one
    /// that runs on every machine rather than a branch only some take.
    #[must_use]
    pub fn child_env(&self) -> collections::HashMap<String, String> {
        self.secret_store
            .iter()
            .flat_map(ExoSecretStore::env)
            .collect()
    }

    /// The lane this path stands for: the file if there is one, otherwise the
    /// install on this machine.
    ///
    /// `OMEGA-DELTA-0092`, omega#100. `OMEGA-DELTA-0055` routes an unpinned
    /// thread to the external ACP agent that is attached, and then nothing
    /// attached one, because attaching meant writing five fields into
    /// `omega-exo-lane.json` by hand. [`omega_agent_detect::exo`] derives those
    /// five fields from a checkout, and this is where the derivation is allowed
    /// to stand in for the file.
    ///
    /// Two rules, and each of them is load-bearing.
    ///
    /// **A lane file that exists is the answer, even when it is broken.** Not
    /// "parses" — *exists*. [`Self::load`] returns `None` for a file that is
    /// half-written or carries the wrong schema, and falling through to
    /// derivation there would replace somebody's explicit, damaged
    /// configuration with a guess about a different `.exo`. That is exactly the
    /// failure `OMEGA-DELTA-0042` names, arrived at from the other side.
    ///
    /// **Derivation happens for the product's own lane and nowhere else.** The
    /// gate is that `path` *is* [`Self::data_dir_path`]. A harness passes an
    /// isolated lane file, and `agent_ui` deliberately hands a stateless run a
    /// path inside the temporary directory that does not exist — so that a
    /// rendering harness never spawns somebody's Exo. Deriving whenever a file
    /// was absent would have quietly undone that, and the check is positive
    /// rather than a list of paths to exclude so that a harness invented
    /// tomorrow is excluded by default.
    #[must_use]
    pub fn resolve(path: &std::path::Path) -> Option<Self> {
        if path.exists() {
            return Self::load(path);
        }
        if path != Self::data_dir_path() {
            return None;
        }
        match omega_agent_detect::exo::derive_lane_from_env() {
            Ok(derived) => {
                log::info!(
                    "OMEGA-DELTA-0092: no Exo lane file, so the lane was derived \
                     from the install at {}",
                    derived.checkout.display()
                );
                Some(Self {
                    binary: derived.binary,
                    checkout: derived.checkout,
                    root: ExoRoot::at(derived.root.to_string_lossy().into_owned()),
                    agent: derived.agent,
                    conversation: derived.conversation,
                    secret_store: derived.secret_store,
                })
            }
            Err(underivable) => {
                // Logged at info rather than warn. A machine with no Exo is the
                // ordinary case, not a fault, and this runs on every start.
                log::info!("OMEGA-DELTA-0092: no Exo lane: {underivable}");
                None
            }
        }
    }
}

/// Everything a turn needs, separated from the connection that owns it.
///
/// Held behind an `Rc` so a turn can be *moved into the spawned task* rather
/// than run on the thread that draws the window. `AgentConnection::prompt`
/// takes `&self`, so without this split the only way to reach the Exo process
/// from `prompt` would be to run it inline — which is exactly the blocking call
/// the workspace's clippy configuration disallows, and it disallows it because
/// an Exo turn calls a model and takes seconds.
struct ExoDriver {
    config: ExoLaneConfig,
    /// What Exo said about itself, cached so a disclosure render does not spawn
    /// a process. Refreshed by every turn, because the turn re-reads the agent
    /// anyway to check its capability.
    identity: RefCell<Option<ExoLaneIdentity>>,
    /// The digest the owner froze for `exo`, if any. Read once at construction
    /// from the harness pin ledger; a claim, never a measurement.
    frozen_digest: Option<String>,
    /// This connection process's generation. A grant cannot survive a
    /// reconnect because a new connection receives a new generation.
    generation: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ExoInspectionPhase {
    NotLoaded,
    Refreshing,
    Ready,
    Unavailable,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExoInspectionSnapshot {
    pub phase: ExoInspectionPhase,
    pub observed: Option<ObservedExoCapabilityState>,
    pub identity: Option<ExoLaneIdentity>,
    pub networking: Option<bool>,
    pub refreshed_at_ms: Option<u64>,
    pub error: Option<String>,
    /// The agent's durable id, as Exo printed it. `OMEGA-DELTA-0107`.
    ///
    /// The lane's configuration holds *slugs*, and Exo's protocol addresses
    /// everything by `Uuid7`. `list_agents` is the call that resolves a slug
    /// and `omega_exo_log` refuses it, being a host-wide read — so without this
    /// there is no admitted way to name the agent the lane is already running
    /// turns on. `exo agent show` prints the id as its first line and
    /// `ExoAgent::parse` keeps it.
    ///
    /// `None` on an Exo that printed no id line. Optional on purpose: "cannot
    /// show the history" is a smaller failure than "cannot run the agent".
    pub exo_agent_id: Option<String>,
    /// The conversation's durable id, on the same terms.
    pub exo_conversation_id: Option<String>,
}

impl Default for ExoInspectionSnapshot {
    fn default() -> Self {
        Self {
            phase: ExoInspectionPhase::NotLoaded,
            observed: None,
            identity: None,
            networking: None,
            refreshed_at_ms: None,
            error: None,
            exo_agent_id: None,
            exo_conversation_id: None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ExoTurnPhase {
    Idle,
    Inspecting,
    Working,
    Cancelling,
    Completed,
    Cancelled,
    Refused,
    Failed,
}

impl ExoTurnPhase {
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Idle => "Ready",
            Self::Inspecting => "Checking runtime",
            Self::Working => "Working",
            Self::Cancelling => "Cancelling",
            Self::Completed => "Completed",
            Self::Cancelled => "Cancelled",
            Self::Refused => "Refused",
            Self::Failed => "Failed",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExoTurnSnapshot {
    pub phase: ExoTurnPhase,
    pub detail: Option<String>,
    pub exo_session_id: Option<String>,
    pub exo_turn_id: Option<String>,
    pub latest_event_id: Option<String>,
    pub updated_at_ms: u64,
}

impl Default for ExoTurnSnapshot {
    fn default() -> Self {
        Self {
            phase: ExoTurnPhase::Idle,
            detail: None,
            exo_session_id: None,
            exo_turn_id: None,
            latest_event_id: None,
            updated_at_ms: now_ms(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Deserialize, serde::Serialize)]
pub struct ExoTierCReceipt {
    pub schema: String,
    pub recorded_at_ms: u64,
    pub authority: Option<ExoSelfModificationReceipt>,
    pub observed: ObservedExoCapabilityState,
    pub turn_ref: String,
    pub requested_objective: Option<String>,
    pub outcome: String,
    pub exo_session_id: Option<String>,
    pub exo_turn_id: Option<String>,
    pub latest_event_id: Option<String>,
    pub verification: String,
    pub reconnect: String,
    pub rollback: String,
}

/// One Exo install, behind Omega Agent's router.
pub struct ExoHarnessConnection {
    driver: Rc<ExoDriver>,
    acp: Rc<AcpConnection>,
    pending_grant: RefCell<Option<ExoSelfModificationGrant>>,
    tier_c_receipt: Rc<RefCell<Option<ExoTierCReceipt>>>,
    active_tier_c_turn: Rc<Cell<bool>>,
    inspection: Rc<RefCell<ExoInspectionSnapshot>>,
    turn: Rc<RefCell<ExoTurnSnapshot>>,
}

impl ExoHarnessConnection {
    /// A connection to the Exo the lane file names.
    #[must_use]
    pub fn new(
        config: ExoLaneConfig,
        frozen_digest: Option<String>,
        acp: Rc<AcpConnection>,
    ) -> Self {
        let previous_receipt = load_latest_tier_c_receipt(&config.agent, &config.conversation);
        Self {
            driver: Rc::new(ExoDriver {
                config,
                identity: RefCell::new(None),
                frozen_digest,
                generation: NEXT_EXO_CONNECTION_GENERATION.fetch_add(1, Ordering::Relaxed),
            }),
            acp,
            pending_grant: RefCell::new(None),
            tier_c_receipt: Rc::new(RefCell::new(previous_receipt)),
            active_tier_c_turn: Rc::new(Cell::new(false)),
            inspection: Rc::new(RefCell::new(ExoInspectionSnapshot::default())),
            turn: Rc::new(RefCell::new(ExoTurnSnapshot::default())),
        }
    }

    /// What this lane discloses, as far as it has been able to observe.
    ///
    /// `None` before the first turn: nothing has asked Exo who it is yet, and
    /// inventing an executor name would be the fabrication the disclosure
    /// record exists to prevent.
    #[must_use]
    pub fn identity(&self) -> Option<ExoLaneIdentity> {
        self.driver.identity.borrow().clone()
    }

    /// The lane's configuration.
    #[must_use]
    pub fn config(&self) -> &ExoLaneConfig {
        &self.driver.config
    }

    /// End the `exo acp` process this lane started, now.
    ///
    /// omega#99. Dropping the connection already kills the child, but a drop
    /// happens only once every owner has let go, and the owners include GPUI
    /// entities whose teardown is deferred. A caller that is finished with the
    /// lane — the visual proof runner, between one photographed turn and the
    /// next — says so here instead of hoping a reference graph unwinds before
    /// the next `exo acp` starts.
    pub fn end_exo_process(&self) {
        self.acp.end_agent_server_process();
    }

    #[must_use]
    pub fn tier_c_receipt(&self) -> Option<ExoTierCReceipt> {
        self.tier_c_receipt.borrow().clone()
    }

    #[must_use]
    pub fn inspection(&self) -> ExoInspectionSnapshot {
        self.inspection.borrow().clone()
    }

    #[must_use]
    pub fn turn(&self) -> ExoTurnSnapshot {
        self.turn.borrow().clone()
    }

    fn set_turn(&self, phase: ExoTurnPhase, detail: Option<String>) {
        *self.turn.borrow_mut() = ExoTurnSnapshot {
            phase,
            detail,
            exo_session_id: None,
            exo_turn_id: None,
            latest_event_id: None,
            updated_at_ms: now_ms(),
        };
    }

    pub fn refresh_inspection(&self, cx: &mut App) -> Task<Result<()>> {
        {
            let mut inspection = self.inspection.borrow_mut();
            inspection.phase = ExoInspectionPhase::Refreshing;
            inspection.error = None;
        }
        let driver = self.driver.clone();
        let inspection = self.inspection.clone();
        cx.spawn(async move |_| match driver.observe().await {
            Ok(observed) => {
                *inspection.borrow_mut() = ready_inspection(&observed);
                Ok(())
            }
            Err(error) => {
                let message = error.to_string();
                let mut snapshot = inspection.borrow_mut();
                snapshot.phase = ExoInspectionPhase::Unavailable;
                snapshot.refreshed_at_ms = Some(now_ms());
                snapshot.error = Some(message.clone());
                Err(anyhow!(message))
            }
        })
    }

    /// This thread's durable record, read from an `exo serve` the owner already
    /// runs. `OMEGA-DELTA-0107`, omega#104.
    ///
    /// ACP carries the live turn. Beside it sits Exo's actual record — every
    /// message, every tool call, every tool result in full, every artifact,
    /// after the turn ended and for as long as Exo keeps the log. This is the
    /// one call that reads it. `omega_exo_log` owns the eight admitted reads,
    /// the loopback refusal, and the rendering; this owns nothing but the ids
    /// and the thread it runs on.
    ///
    /// # Omega reads a server; Omega never starts one
    ///
    /// Route A of omega#104, and the decision behind it is worth keeping beside
    /// the code. Omega spawning `exo serve` itself would be new process
    /// authority, a port, and a lifetime to own — and, worse, a **second writer
    /// on one `.exo` root**, which is exactly what `omega_exo_episode::root`
    /// exists to refuse and the interleaving that makes a fork a copy of a
    /// history that never existed. Omega also cannot know whether the owner
    /// already has one running. So the durable log is read when
    /// `EXO_EXOHARNESS_URL` names a loopback server, and not otherwise.
    ///
    /// # Unset is "not configured", and never "no history"
    ///
    /// On an ordinary machine the variable is unset — [`ExoDriver::check_endpoint`]
    /// treats that as the ordinary, safe case, because the CLI reads the state
    /// root on disk and no socket exists at all. That produces
    /// [`ExoHistoryUnavailable::NotConfigured`], which carries a sentence, and
    /// not an empty [`ExoDurableHistory`], which there is deliberately no way to
    /// construct. A surface that showed this thread as having no durable record
    /// would be telling the reader something false about their own conversation.
    ///
    /// # Two reads, and the second is the one with the history in it
    ///
    /// `ExoReadClient::conversation_history` does both passes: Exo's event log
    /// *names* artifacts and never contains them, so the first render says which
    /// bodies are missing and the second is the one with the tool results in it.
    /// Off the foreground thread, because the transport is blocking `std::net`
    /// and the thread that would block is the one drawing the window.
    ///
    /// # Errors
    ///
    /// A refused or unreachable endpoint, an id Exo printed that is not a UUID,
    /// or a failed event read. A *missing artifact body* is none of those: it
    /// leaves its row saying so, and `ExoHistory::unread_artifact_rows` counts
    /// it.
    pub fn read_durable_history(&self, cx: &mut App) -> Task<Result<ExoDurableHistory>> {
        let driver = Rc::clone(&self.driver);
        let inspection = self.inspection.clone();
        let known = {
            let snapshot = inspection.borrow();
            (
                snapshot.exo_agent_id.clone(),
                snapshot.exo_conversation_id.clone(),
            )
        };
        cx.spawn(async move |_| {
            let Ok(url) = std::env::var("EXO_EXOHARNESS_URL") else {
                return Ok(ExoDurableHistory::Unavailable(
                    ExoHistoryUnavailable::NotConfigured,
                ));
            };
            let (agent, conversation) = match known {
                (Some(agent), Some(conversation)) => (Some(agent), Some(conversation)),
                // Nothing has asked Exo who it is yet. The same observation a
                // turn runs, so the ids are the ones the next turn would use.
                _ => {
                    let observed = driver.observe().await?;
                    *inspection.borrow_mut() = ready_inspection(&observed);
                    (observed.exo_agent_id, observed.exo_conversation_id)
                }
            };
            let Some(agent) = agent else {
                return Ok(ExoDurableHistory::Unavailable(
                    ExoHistoryUnavailable::NoAgentId,
                ));
            };
            let Some(conversation) = conversation else {
                return Ok(ExoDurableHistory::Unavailable(
                    ExoHistoryUnavailable::NoConversationId,
                ));
            };
            let agent = ExoId::parse(&agent)
                .map_err(|refusal| anyhow!("{refusal}: Exo named this agent `{agent}`"))?;
            let conversation = ExoId::parse(&conversation).map_err(|refusal| {
                anyhow!("{refusal}: Exo named this conversation `{conversation}`")
            })?;
            let client = ExoReadClient::open(&url).map_err(|refusal| anyhow!("{refusal}"))?;
            let history = smol::unblock(move || {
                client.conversation_history(&agent, &conversation, Some(EXO_HISTORY_EVENT_LIMIT))
            })
            .await
            .map_err(|refusal| anyhow!("{refusal}"))?;
            Ok(ExoDurableHistory::Read(history))
        })
    }

    /// Observe the exact Exo capability state for a dedicated confirmation
    /// dialog. This does not mint authority.
    pub async fn self_modification_request(
        &self,
        objective: String,
        turn_ref: String,
    ) -> Result<ExoSelfModificationGrantRequest> {
        let observed = self.driver.observe().await?.observed;
        let capabilities = observed.requested_capabilities();
        if capabilities.is_empty() {
            bail!("this Exo agent has no self-modification capability to authorize");
        }
        Ok(ExoSelfModificationGrantRequest {
            objective,
            turn_ref,
            observed,
            capabilities,
            expires_at_ms: now_ms().saturating_add(60_000),
        })
    }

    /// Mint the one-use grant after the visible confirmation action returns.
    pub fn confirm_self_modification(
        &self,
        request: ExoSelfModificationGrantRequest,
    ) -> Result<()> {
        let grant = ExoSelfModificationGrant::mint(
            request,
            ExoSelfModificationConsentOrigin::HumanConfirmationDialog,
            now_ms(),
        )
        .map_err(|refusal| anyhow!("Exo self-modification grant refused: {refusal:?}"))?;
        *self.pending_grant.borrow_mut() = Some(grant);
        Ok(())
    }
}

struct ObservedTurn {
    observed: ObservedExoCapabilityState,
    identity: ExoLaneIdentity,
    networking: bool,
    /// `OMEGA-DELTA-0107`. The ids Exo printed, carried off the observation
    /// rather than thrown away — see [`ExoInspectionSnapshot::exo_agent_id`].
    exo_agent_id: Option<String>,
    exo_conversation_id: Option<String>,
}

/// The inspector state one successful observation produces.
///
/// One constructor for the three places that built this by hand, so a field
/// added to the snapshot cannot be populated on the refresh path and left null
/// on the turn path.
fn ready_inspection(observed: &ObservedTurn) -> ExoInspectionSnapshot {
    ExoInspectionSnapshot {
        phase: ExoInspectionPhase::Ready,
        observed: Some(observed.observed.clone()),
        identity: Some(observed.identity.clone()),
        networking: Some(observed.networking),
        refreshed_at_ms: Some(now_ms()),
        error: None,
        exo_agent_id: observed.exo_agent_id.clone(),
        exo_conversation_id: observed.exo_conversation_id.clone(),
    }
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| {
            u64::try_from(duration.as_millis()).unwrap_or(u64::MAX)
        })
}

fn set_turn_snapshot(turn: &RefCell<ExoTurnSnapshot>, phase: ExoTurnPhase, detail: Option<String>) {
    *turn.borrow_mut() = ExoTurnSnapshot {
        phase,
        detail,
        exo_session_id: None,
        exo_turn_id: None,
        latest_event_id: None,
        updated_at_ms: now_ms(),
    };
}

fn set_terminal_turn_snapshot(
    turn: &RefCell<ExoTurnSnapshot>,
    phase: ExoTurnPhase,
    detail: Option<String>,
    response: &acp::PromptResponse,
) {
    let meta = response.meta.as_ref();
    *turn.borrow_mut() = ExoTurnSnapshot {
        phase,
        detail,
        exo_session_id: meta_value(meta, "exo.session_id"),
        exo_turn_id: meta_value(meta, "exo.turn_id"),
        latest_event_id: meta_value(meta, "exo.latest_event_id"),
        updated_at_ms: now_ms(),
    };
}

fn resolve_module_host_path(module: &str, mounts: &[ExoMount]) -> Option<PathBuf> {
    let path = std::path::Path::new(module);
    if path.is_file() {
        return Some(path.to_path_buf());
    }
    mounts.iter().find_map(|mount| {
        let suffix = path.strip_prefix(&mount.mount_path).ok()?;
        Some(PathBuf::from(&mount.host_path).join(suffix))
    })
}

#[must_use]
pub fn exo_prompt_objective(prompt: &[acp::ContentBlock]) -> String {
    prompt
        .iter()
        .filter_map(|block| match block {
            acp::ContentBlock::Text(text) => Some(text.text.as_str()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("\n")
}

pub fn exo_turn_ref(session_id: &acp::SessionId, prompt: &[acp::ContentBlock]) -> Result<String> {
    let canonical = serde_json::to_vec(&(session_id, prompt))
        .context("encoding the exact Exo turn for its authority grant")?;
    Ok(format!(
        "exo-turn:{}",
        MeasuredDigest::measure(&canonical).as_str()
    ))
}

impl ExoDriver {
    /// Run one `exo` command and return its stdout.
    ///
    /// Async, and spawned rather than blocking, because an Exo turn calls a
    /// model: it takes seconds, and running it on the thread that draws the
    /// window would freeze Omega for the length of somebody else's inference.
    async fn run(&self, command: &ExoCommand) -> Result<String> {
        let argv = command.argv(&self.config.root);
        let output = smol::process::Command::new(&self.config.binary)
            .args(&argv)
            // `OMEGA-DELTA-0126`. The same store the turn's child gets. None of
            // these five commands decrypts a secret today, so this changes no
            // observed behaviour — which is the point: a reader comparing the
            // two spawn sites should not have to work out why one of them names
            // the key and the other does not.
            .envs(self.config.child_env())
            .stdin(Stdio::null())
            .output()
            .await
            .with_context(|| {
                format!(
                    "running {} {}",
                    self.config.binary.display(),
                    argv.join(" ")
                )
            })?;
        if !output.status.success() {
            bail!(
                "exo {} exited {}: {}",
                command.shape(),
                output.status,
                String::from_utf8_lossy(&output.stderr).trim()
            );
        }
        Ok(String::from_utf8_lossy(&output.stdout).into_owned())
    }

    /// Refuse an Exo that is not on this machine.
    ///
    /// Exo reads `EXO_EXOHARNESS_URL` from its environment and, when it is set,
    /// stops using the state root on disk and talks to an `exo serve` over
    /// HTTP instead. That server has **no authentication** — a client may send
    /// a bearer token and the server never checks it — and it exposes Exo's
    /// whole request protocol, secrets included. Loopback is the entire
    /// boundary, and Exo's own documentation says so.
    ///
    /// Omega never passes that flag; [`ExoCommand`] cannot express it. But the
    /// lane inherits Omega's environment, which is how Exo is meant to receive
    /// its own configuration, and a variable set there would redirect the lane
    /// off-machine without a single Omega command line changing. So the
    /// environment is read and parsed through [`LoopbackEndpoint`], whose only
    /// constructor refuses anything that is not this machine.
    fn check_endpoint(&self) -> Result<()> {
        let Ok(url) = std::env::var("EXO_EXOHARNESS_URL") else {
            // Unset is the ordinary case and the safe one: the CLI reads the
            // state root on disk and no socket exists at all.
            return Ok(());
        };
        let endpoint = LoopbackEndpoint::parse(&url).map_err(|refusal| {
            anyhow!(
                "{refusal}: the Exo lane will not talk to {url}. Exo's server has no \
                 authentication and full access to its secrets."
            )
        })?;
        log::info!("OMEGA-DELTA-0042: the Exo lane is pointed at {endpoint}, which is loopback");
        Ok(())
    }

    /// Refuse an Exo that is not the pinned one.
    async fn check_pin(&self) -> Result<(ObservedExoCheckout, MeasuredDigest)> {
        let git = async |args: &[&str]| -> Result<String> {
            let output = smol::process::Command::new("git")
                .arg("-C")
                .arg(&self.config.checkout)
                .args(args)
                .stdin(Stdio::null())
                .output()
                .await
                .context("reading the Exo checkout with git")?;
            if !output.status.success() {
                bail!("git {args:?} failed in the Exo checkout");
            }
            Ok(String::from_utf8_lossy(&output.stdout).trim().to_owned())
        };
        let observed = ObservedExoCheckout {
            upstream: git(&["config", "--get", "remote.origin.url"]).await?,
            commit: git(&["rev-parse", "HEAD"]).await?,
            tree: git(&["rev-parse", "HEAD^{tree}"]).await?,
        };
        // omega#118. Observed and recorded. Never a refusal.
        //
        // This used to fail the turn. The owner selected Exo, saw a green
        // "ready", typed a message, and got a red banner telling him his own
        // checkout was not at a commit he had never heard of. The pin is
        // something we wanted; it is not a fact about whether Exo can answer,
        // and it has no business standing between a person and their message.
        //
        // Whatever commit the checkout is at is the commit Exo runs. The
        // mismatch is still measured and still reaches the log and the
        // inspector, where a fact about provenance belongs.
        if let Err(mismatch) = EXO_PIN.admits(&observed) {
            log::info!(
                "{mismatch}: the Exo lane is pinned to {} at {}, and is running \
                 {} anyway — the pin is recorded, not enforced",
                EXO_PIN.upstream,
                EXO_PIN.source_commit,
                observed.commit
            );
        }

        let bytes = smol::fs::read(&self.config.binary)
            .await
            .context("reading the exo binary")?;
        let digest = MeasuredDigest::measure(&bytes);
        // Same rule for the binary digest: a rebuilt `exo` is the ordinary
        // state of a checkout somebody is working in, not a reason to refuse
        // the message they just typed.
        if let Err(mismatch) = admits_bytes(self.frozen_digest.as_deref(), &digest) {
            log::info!("{mismatch}; running the exo binary as measured");
        }
        Ok((observed, digest))
    }

    /// Read the exact agent and conversation capability state.
    async fn observe(&self) -> Result<ObservedTurn> {
        self.check_endpoint()?;
        let (checkout, binary_digest) = self.check_pin().await?;
        let shown = self
            .run(&ExoCommand::ShowAgent {
                agent: self.config.agent.clone(),
            })
            .await?;
        let agent = ExoAgent::parse(&shown).map_err(|error| {
            anyhow!("{error}; the Omega lane refuses an Exo agent it cannot read")
        })?;
        let shown_conversation = self
            .run(&ExoCommand::ShowConversation {
                agent: self.config.agent.clone(),
                conversation: self.config.conversation.clone(),
            })
            .await?;
        let conversation = ExoConversation::parse(&shown_conversation).map_err(|error| {
            anyhow!("{error}; the Omega lane refuses an Exo conversation it cannot read")
        })?;

        let bindings = ExoModelBinding::read_table(&self.run(&ExoCommand::ListModels).await?);
        let identity = ExoLaneIdentity::resolve(&agent, &bindings);
        *self.identity.borrow_mut() = Some(identity.clone());
        let mut mounts = agent.mounts.clone();
        mounts.extend(conversation.mounts.clone());
        let read_write_mounts = mounts
            .iter()
            .filter(|mount| mount.read_write)
            .map(|mount| ObservedReadWriteMount {
                host_path: mount.host_path.clone(),
                mount_path: mount.mount_path.clone(),
            })
            .collect::<Vec<_>>();
        let mut tool_modules = Vec::new();
        for module in &agent.tool_module_paths {
            let host_path = resolve_module_host_path(module, &mounts).ok_or_else(|| {
                anyhow!(
                    "the Exo tool module {module} is not a readable host file or inside an observed mount"
                )
            })?;
            let bytes = smol::fs::read(&host_path)
                .await
                .with_context(|| format!("reading Exo tool module {}", host_path.display()))?;
            tool_modules.push(ObservedToolModule {
                path: module.clone(),
                digest: MeasuredDigest::measure(&bytes).as_str().to_owned(),
            });
        }
        tool_modules.sort();
        let mut read_write_mounts = read_write_mounts;
        read_write_mounts.sort();
        let exo_agent_id = agent.id.clone();
        let exo_conversation_id = conversation.id.clone();
        Ok(ObservedTurn {
            exo_agent_id,
            exo_conversation_id,
            observed: ObservedExoCapabilityState {
                source_commit: checkout.commit,
                source_tree: checkout.tree,
                binary_digest: binary_digest.as_str().to_owned(),
                agent: agent.slug,
                conversation: conversation.slug,
                generation: self.generation,
                agent_authored_tools: agent.agent_authored_tools,
                tool_modules,
                read_write_mounts,
            },
            identity,
            networking: agent.networking,
        })
    }

    /// Re-observe the exact executable and agent immediately before a streamed
    /// ACP turn. A self-modifying turn needs a matching one-use grant.
    async fn preflight(&self) -> Result<ObservedTurn> {
        self.observe().await
    }
}

/// The Exo lane this machine has: configured, or derived from the install.
///
/// Called once, when the router is built. A machine with no Exo gets `None` and
/// the router registers no external executor — which is the ordinary case, and
/// is why the return type is an `Option` rather than a `Result` that every
/// caller would have to decide to ignore.
///
/// `OMEGA-DELTA-0092`, omega#100. "No lane file" stopped meaning "no lane":
/// see [`ExoLaneConfig::resolve`] for which of the two this path is and why the
/// distinction is drawn where it is.
pub async fn connect_configured_lane(
    lane_path: &std::path::Path,
    project: Entity<Project>,
    agent_server_store: WeakEntity<AgentServerStore>,
    cx: &mut AsyncApp,
) -> Result<Option<Rc<dyn AgentConnection>>> {
    let Some(config) = ExoLaneConfig::resolve(lane_path) else {
        return Ok(None);
    };
    let frozen = frozen_exo_digest();
    log::info!(
        "OMEGA-DELTA-0042: Exo harness lane configured at {} ({} {}), pin {}",
        config.root.as_str(),
        config.agent,
        config.conversation,
        if frozen.is_some() {
            "frozen"
        } else {
            "unfrozen"
        }
    );
    let command = AgentServerCommand {
        path: config.binary.clone(),
        args: vec![
            "--root".to_owned(),
            config.root.as_str().to_owned(),
            "acp".to_owned(),
            config.agent.clone(),
            config.conversation.clone(),
        ],
        // `OMEGA-DELTA-0126`. Was `None`, and `None` means "inherit whatever
        // Omega was launched with" — which made the same root work from a
        // terminal and fail from the Dock, on the person's first message.
        env: Some(config.child_env()),
    };
    let acp = Rc::new(
        AcpConnection::stdio(
            AgentId::new(EXO_HARNESS_ID),
            project,
            command,
            agent_server_store,
            None,
            Default::default(),
            cx,
        )
        .await?,
    );
    Ok(Some(Rc::new(ExoHarnessConnection::new(
        config, frozen, acp,
    ))))
}

/// The digest the owner froze for `exo`, from the harness pin ledger.
///
/// Read here rather than inside the connection so the connection takes a value
/// and stays testable without a filesystem. A ledger that cannot be read is not
/// an empty ledger — but it is also not this file's decision, so the absence is
/// logged and the pin check proceeds unfrozen, exactly as it does on a machine
/// where the owner never froze Exo.
fn frozen_exo_digest() -> Option<String> {
    let path = paths::data_dir()
        .join("openagents")
        .join("external_agents")
        .join(omega_harness::HARNESS_PIN_LEDGER_FILE_NAME);
    let file = std::fs::read_to_string(path).ok()?;
    match omega_harness::decode_harness_pin_ledger(&file) {
        Ok(ledger) => ledger.pin(EXO_HARNESS_ID).map(|pin| pin.digest.clone()),
        Err(error) => {
            log::warn!("OMEGA-DELTA-0042: the harness pin ledger could not be read ({error})");
            None
        }
    }
}

fn meta_value(meta: Option<&acp::Meta>, key: &str) -> Option<String> {
    meta.and_then(|meta| meta.get(key))
        .and_then(serde_json::Value::as_str)
        .map(str::to_owned)
}

fn tier_c_receipt_path() -> PathBuf {
    paths::data_dir()
        .join("openagents")
        .join("exo-self-modification-receipts.jsonl")
}

fn load_latest_tier_c_receipt(agent: &str, conversation: &str) -> Option<ExoTierCReceipt> {
    std::fs::read_to_string(tier_c_receipt_path())
        .ok()?
        .lines()
        .rev()
        .filter_map(|line| serde_json::from_str::<ExoTierCReceipt>(line).ok())
        .find(|receipt| {
            receipt.observed.agent == agent && receipt.observed.conversation == conversation
        })
}

fn refused_tier_c_receipt(
    observed: ObservedExoCapabilityState,
    turn_ref: String,
    requested_objective: Option<String>,
    outcome: String,
) -> ExoTierCReceipt {
    ExoTierCReceipt {
        schema: "openagents.omega.exo_self_modification_receipt.v1".to_owned(),
        recorded_at_ms: now_ms(),
        authority: None,
        observed,
        turn_ref,
        requested_objective,
        outcome,
        exo_session_id: None,
        exo_turn_id: None,
        latest_event_id: None,
        verification: "Omega refused the turn before it crossed ACP".to_owned(),
        reconnect: "not applicable; the turn did not start".to_owned(),
        rollback: "not applicable; the turn did not start".to_owned(),
    }
}

async fn persist_tier_c_receipt(receipt: ExoTierCReceipt) -> Result<()> {
    let path = tier_c_receipt_path();
    smol::unblock(move || -> Result<()> {
        use std::io::Write as _;
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("creating {}", parent.display()))?;
        }
        let mut file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
            .with_context(|| format!("opening {}", path.display()))?;
        serde_json::to_writer(&mut file, &receipt)?;
        file.write_all(b"\n")?;
        file.sync_data()?;
        Ok(())
    })
    .await
}

impl AgentConnection for ExoHarnessConnection {
    /// A display label, and only that. omega#77 classifies by downcast because
    /// this is renameable; see `omega_executor_disclosure`.
    fn agent_id(&self) -> AgentId {
        AgentId::new(
            self.identity()
                .map_or_else(|| EXO_HARNESS_ID.to_owned(), |identity| identity.agent_id()),
        )
    }

    fn telemetry_id(&self) -> SharedString {
        EXO_HARNESS_ID.into()
    }

    fn agent_version(&self) -> Option<SharedString> {
        Some(EXO_PIN.version.into())
    }

    fn new_session(
        self: Rc<Self>,
        project: Entity<Project>,
        work_dirs: PathList,
        cx: &mut App,
    ) -> Task<Result<Entity<AcpThread>>> {
        let inner_session = self.acp.clone().new_session(project, work_dirs, cx);
        let facade: Rc<dyn AgentConnection> = self;
        cx.spawn(async move |cx| {
            let thread = inner_session.await?;
            thread.update(cx, |thread, _| {
                thread.replace_connection(facade);
            });
            Ok(thread)
        })
    }

    /// `OMEGA-DELTA-0127`, omega#112. Exo's own session configuration, passed
    /// through.
    ///
    /// `exo acp` advertises a model selector in its `session/new` response, in
    /// the ordinary ACP shape, so the whole of Omega's side is *not
    /// intercepting it*: `AcpConnection` already parses `configOptions`,
    /// `ConfigOptionsView` already draws one dropdown per advertised option in
    /// the composer's bottom-right row, and `omega_router` already delegates
    /// this method to whichever executor is attached. Without this override the
    /// facade — which `new_session` installs over the thread in place of the
    /// inner connection — answered the trait's default `None`, and Exo's
    /// options were parsed, held, and never asked for.
    ///
    /// Deliberately not a bespoke Exo control. A selector Omega drew itself
    /// would be a second thing to keep in step with what Exo can actually do,
    /// and it would not have been reachable by the keybindings
    /// `ConfigOptionsView` already binds by category.
    fn session_config_options(
        &self,
        session_id: &acp::SessionId,
        cx: &App,
    ) -> Option<Rc<dyn acp_thread::AgentSessionConfigOptions>> {
        self.acp.session_config_options(session_id, cx)
    }

    /// The same pass-through, for an agent that advertises modes instead.
    ///
    /// Exo advertises none today. It is here because the pair is one seam: an
    /// agent sending `modes` and an agent sending `configOptions` are the two
    /// halves of one upstream mechanism, and overriding one of them is how a
    /// future Exo that switched to the other silently loses its controls.
    fn session_modes(
        &self,
        session_id: &acp::SessionId,
        cx: &App,
    ) -> Option<Rc<dyn acp_thread::AgentSessionModes>> {
        self.acp.session_modes(session_id, cx)
    }

    fn auth_methods(&self) -> &[acp::AuthMethod] {
        &[]
    }

    fn authenticate(&self, _method: acp::AuthMethodId, _cx: &mut App) -> Task<Result<()>> {
        Task::ready(Err(anyhow!(
            "Exo holds its own credentials; Omega never edits them"
        )))
    }

    fn prompt(
        &self,
        params: acp::PromptRequest,
        cx: &mut App,
    ) -> Task<Result<acp::PromptResponse>> {
        self.set_turn(
            ExoTurnPhase::Inspecting,
            Some("Checking the pinned Exo runtime before this turn".to_owned()),
        );
        let driver = Rc::clone(&self.driver);
        let acp = Rc::clone(&self.acp);
        let pending_grant = self.pending_grant.take();
        let receipt_cell = self.tier_c_receipt.clone();
        let active_tier_c_turn = self.active_tier_c_turn.clone();
        let inspection_cell = self.inspection.clone();
        let turn_cell = self.turn.clone();
        cx.spawn(async move |cx| {
            let turn_ref = exo_turn_ref(&params.session_id, &params.prompt)?;
            let observed = match driver.preflight().await {
                Ok(observed) => observed,
                Err(error) => {
                    let message = error.to_string();
                    {
                        let mut inspection = inspection_cell.borrow_mut();
                        inspection.phase = ExoInspectionPhase::Unavailable;
                        inspection.refreshed_at_ms = Some(now_ms());
                        inspection.error = Some(message.clone());
                    }
                    set_turn_snapshot(
                        &turn_cell,
                        ExoTurnPhase::Failed,
                        Some(format!("Runtime check failed: {message}")),
                    );
                    return Err(error);
                }
            };
            *inspection_cell.borrow_mut() = ready_inspection(&observed);
            // `OMEGA-DELTA-0126`, amending `OMEGA-DELTA-0042`. An observed
            // capability is a thing to *say*, never a reason to refuse the
            // turn.
            //
            // What stood here refused any turn whose agent reported a
            // capability that could widen it — and `tool_creation: enabled` is
            // Exo's default on every agent `exo agent create` makes. So typing
            // `hi` produced a red error banner and no turn. A gate has to name
            // the act it prevents; "the person typed a word" is not an act. The
            // acts that would be worth preventing all happen inside Exo, where
            // Omega cannot see them and therefore cannot gate them where they
            // occur, and a gate that cannot reach its act does not become
            // correct by moving upstream until it catches something.
            let capabilities = observed.observed.requested_capabilities();
            if !capabilities.is_empty() {
                log::info!(
                    "OMEGA-DELTA-0126: this Exo agent is configured with {} capabilit{} \
                     that can widen a turn; the turn runs and the inspector names them",
                    capabilities.len(),
                    if capabilities.len() == 1 { "y" } else { "ies" }
                );
            }
            let authority_receipt = match pending_grant {
                None => None,
                Some(grant) => {
                    let requested_objective = Some(grant.request().objective.clone());
                    match grant.consume(&observed.observed, &turn_ref, now_ms()) {
                        Ok(authority) => Some(authority),
                        // A grant that no longer matches what is on the machine
                        // does not authorize anything — but it is also not a
                        // reason to stop the person's message. The turn runs as
                        // an ordinary turn, with no authority attached and a
                        // receipt saying so, which is a truer record than a
                        // turn that never ran.
                        Err(refusal) => {
                            let receipt = refused_tier_c_receipt(
                                observed.observed.clone(),
                                turn_ref.clone(),
                                requested_objective,
                                format!("not authorized: {refusal:?}; the turn ran without it"),
                            );
                            *receipt_cell.borrow_mut() = Some(receipt.clone());
                            persist_tier_c_receipt(receipt).await?;
                            log::info!(
                                "OMEGA-DELTA-0126: the Exo self-modification grant no longer \
                                 matches this machine ({refusal:?}); the turn runs without it"
                            );
                            None
                        }
                    }
                }
            };
            let is_tier_c_turn = authority_receipt.is_some();
            if let Some(authority) = authority_receipt {
                *receipt_cell.borrow_mut() = Some(ExoTierCReceipt {
                    schema: "openagents.omega.exo_self_modification_receipt.v1".to_owned(),
                    recorded_at_ms: now_ms(),
                    observed: authority.observed.clone(),
                    turn_ref: authority.turn_ref.clone(),
                    requested_objective: Some(authority.objective.clone()),
                    authority: Some(authority),
                    outcome: "sent".to_owned(),
                    exo_session_id: None,
                    exo_turn_id: None,
                    latest_event_id: None,
                    verification: "waiting for Exo's durable completion receipt".to_owned(),
                    reconnect: "not reported by Exo ACP".to_owned(),
                    rollback: "not reported by Exo ACP".to_owned(),
                });
                active_tier_c_turn.set(true);
                let receipt = receipt_cell.borrow().clone();
                if let Some(receipt) = receipt {
                    if let Err(error) = persist_tier_c_receipt(receipt).await {
                        active_tier_c_turn.set(false);
                        return Err(error);
                    }
                }
            }
            set_turn_snapshot(
                &turn_cell,
                ExoTurnPhase::Working,
                Some("Streaming this turn over ACP".to_owned()),
            );
            let prompt = cx.update(|cx| acp.prompt(params, cx));
            let response = prompt.await;
            match &response {
                Ok(response) => match response.stop_reason {
                    acp::StopReason::Cancelled => set_terminal_turn_snapshot(
                        &turn_cell,
                        ExoTurnPhase::Cancelled,
                        Some("Exo confirmed cancellation".to_owned()),
                        response,
                    ),
                    _ => set_terminal_turn_snapshot(
                        &turn_cell,
                        ExoTurnPhase::Completed,
                        Some("Exo completed the streamed turn".to_owned()),
                        response,
                    ),
                },
                Err(error) => {
                    set_turn_snapshot(&turn_cell, ExoTurnPhase::Failed, Some(error.to_string()))
                }
            }
            if is_tier_c_turn {
                if let Some(receipt) = receipt_cell.borrow_mut().as_mut() {
                    match &response {
                        Ok(response) => {
                            receipt.outcome = match response.stop_reason {
                                acp::StopReason::Cancelled => "cancelled",
                                _ => "completed",
                            }
                            .to_owned();
                            receipt.recorded_at_ms = now_ms();
                            let meta = response.meta.as_ref();
                            receipt.exo_session_id = meta_value(meta, "exo.session_id");
                            receipt.exo_turn_id = meta_value(meta, "exo.turn_id");
                            receipt.latest_event_id = meta_value(meta, "exo.latest_event_id");
                            receipt.verification = if receipt.latest_event_id.is_some() {
                                "Exo returned its durable latest event reference".to_owned()
                            } else {
                                "Exo returned no durable event reference".to_owned()
                            };
                        }
                        Err(error) => {
                            receipt.recorded_at_ms = now_ms();
                            receipt.outcome = format!("failed: {error}");
                            receipt.verification =
                                "the ACP turn failed before verification".to_owned();
                        }
                    }
                }
                let receipt = receipt_cell.borrow().clone();
                let persisted = if let Some(receipt) = receipt {
                    persist_tier_c_receipt(receipt).await
                } else {
                    Ok(())
                };
                active_tier_c_turn.set(false);
                persisted?;
            }
            response
        })
    }

    fn cancel(&self, session_id: &acp::SessionId, cx: &mut App) {
        self.set_turn(
            ExoTurnPhase::Cancelling,
            Some("Waiting for Exo to confirm cancellation".to_owned()),
        );
        let cancelled_pending_grant = self.pending_grant.borrow_mut().take().map(|grant| {
            let request = grant.request();
            refused_tier_c_receipt(
                request.observed.clone(),
                request.turn_ref.clone(),
                Some(request.objective.clone()),
                "refused: cancelled before send".to_owned(),
            )
        });
        if let Some(receipt) = cancelled_pending_grant {
            *self.tier_c_receipt.borrow_mut() = Some(receipt.clone());
            cx.spawn(async move |_| {
                if let Err(error) = persist_tier_c_receipt(receipt).await {
                    log::error!("omega#87: failed to persist the cancellation receipt: {error}");
                }
            })
            .detach();
        } else if self.active_tier_c_turn.get() {
            if let Some(receipt) = self.tier_c_receipt.borrow_mut().as_mut() {
                receipt.recorded_at_ms = now_ms();
                receipt.outcome = "cancellation requested".to_owned();
            }
            if let Some(receipt) = self.tier_c_receipt.borrow().clone() {
                cx.spawn(async move |_| {
                    if let Err(error) = persist_tier_c_receipt(receipt).await {
                        log::error!(
                            "omega#87: failed to persist the cancellation receipt: {error}"
                        );
                    }
                })
                .detach();
            }
        }
        self.acp.cancel(session_id, cx);
    }

    fn into_any(self: Rc<Self>) -> Rc<dyn Any> {
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lane_file(schema: &str) -> String {
        serde_json::json!({
            "schema": schema,
            "binary": "/opt/exo/target/release/exo",
            "checkout": "/opt/exo",
            "root": "/opt/exo/.exo",
            "agent": "omega-lane",
            "conversation": "tier-a",
        })
        .to_string()
    }

    /// A disclosure record, for the tests that need one and are not about it.
    ///
    /// Built rather than resolved: `ExoAgent` deliberately has no `Default` —
    /// it is a record read out of Exo's own output, and a permissive default is
    /// exactly what its documentation refuses.
    fn an_identity() -> ExoLaneIdentity {
        ExoLaneIdentity {
            executor: "basic".into(),
            model: None,
            provider: None,
        }
    }

    fn write(contents: &str) -> (tempfile::TempDir, PathBuf) {
        let dir = tempfile::tempdir().expect("a temp dir");
        let path = dir.path().join(EXO_LANE_FILE);
        std::fs::write(&path, contents).expect("write");
        (dir, path)
    }

    #[test]
    fn a_lane_file_names_one_exo_install() {
        let (_dir, path) = write(&lane_file(EXO_LANE_SCHEMA));
        let config = ExoLaneConfig::load(&path).expect("a lane");
        assert_eq!(config.agent, "omega-lane");
        assert_eq!(config.conversation, "tier-a");
        assert_eq!(config.root.as_str(), "/opt/exo/.exo");
    }

    /// `OMEGA-DELTA-0092`. A lane file that exists is the answer, and a broken
    /// one is still an answer. The alternative — falling through to derivation
    /// when the file will not parse — replaces somebody's explicit
    /// configuration with a guess about a different `.exo`, which is the
    /// `OMEGA-DELTA-0042` failure approached from the other side.
    #[test]
    fn a_lane_file_that_will_not_parse_is_not_replaced_by_a_derived_lane() {
        let (_dir, path) = write("{ not json");

        assert_eq!(
            ExoLaneConfig::resolve(&path),
            None,
            "a damaged lane file must produce no lane, not a different one"
        );
    }

    /// The gate that keeps a harness from spawning the owner's Exo.
    ///
    /// `agent_ui` hands a stateless run a lane path inside the temporary
    /// directory that does not exist, precisely so that a rendering harness
    /// never starts somebody's `exo acp`. Derivation is admitted for exactly
    /// one path — the product's own — so that guarantee survives.
    #[test]
    fn an_absent_lane_file_outside_the_data_directory_derives_nothing() {
        let dir = tempfile::tempdir().expect("a temp dir");
        let absent = dir.path().join("omega-exo-lane-1234.json");
        assert!(!absent.exists());

        assert_eq!(
            ExoLaneConfig::resolve(&absent),
            None,
            "only the product's own lane path may be derived, whatever this \
             machine happens to have installed"
        );
        assert_ne!(absent, ExoLaneConfig::data_dir_path());
    }

    #[test]
    fn an_unread_inspector_claims_no_runtime_facts() {
        let inspection = ExoInspectionSnapshot::default();
        assert_eq!(inspection.phase, ExoInspectionPhase::NotLoaded);
        assert_eq!(inspection.observed, None);
        assert_eq!(inspection.identity, None);
        assert_eq!(inspection.networking, None);
        assert_eq!(inspection.refreshed_at_ms, None);
        assert_eq!(inspection.error, None);
        assert_eq!(inspection.exo_agent_id, None);
        assert_eq!(inspection.exo_conversation_id, None);
    }

    /// `OMEGA-DELTA-0107`. The ids an observation carries reach the inspector.
    ///
    /// The lane held only slugs, and Exo addresses everything by `Uuid7`, so
    /// this is the whole difference between a durable read that can name its
    /// conversation and one that cannot. Checked through `ready_inspection`
    /// because that is the one constructor the refresh path and the turn path
    /// now share — the failure this replaces is a field populated on one of
    /// them and left null on the other.
    #[test]
    fn an_observed_turn_carries_the_ids_the_durable_read_addresses_by() {
        let observed = ObservedExoCapabilityState {
            source_commit: "commit".into(),
            source_tree: "tree".into(),
            binary_digest: "sha256:binary".into(),
            agent: "omega-lane".into(),
            conversation: "tier-a".into(),
            generation: 3,
            agent_authored_tools: false,
            tool_modules: Vec::new(),
            read_write_mounts: Vec::new(),
        };
        let turn = ObservedTurn {
            observed,
            identity: an_identity(),
            networking: false,
            exo_agent_id: Some("019e5782-0000-7000-8000-000000000001".into()),
            exo_conversation_id: Some("019e5782-0000-7000-8000-000000000002".into()),
        };

        let inspection = ready_inspection(&turn);
        assert_eq!(inspection.phase, ExoInspectionPhase::Ready);
        assert_eq!(
            inspection.exo_agent_id.as_deref(),
            Some("019e5782-0000-7000-8000-000000000001")
        );
        assert_eq!(
            inspection.exo_conversation_id.as_deref(),
            Some("019e5782-0000-7000-8000-000000000002")
        );
        for id in [
            inspection.exo_agent_id.as_deref().expect("an agent id"),
            inspection
                .exo_conversation_id
                .as_deref()
                .expect("a conversation id"),
        ] {
            assert!(
                ExoId::parse(id).is_ok(),
                "the durable read refuses anything that is not UUID-shaped, so an \
                 id that reaches the inspector has to be one: {id}"
            );
        }
    }

    /// `OMEGA-DELTA-0107`. An Exo that printed no id still runs turns, and says
    /// what it cannot show.
    ///
    /// The reason both fields are `Option`: refusing to attach because the
    /// history is unreadable would trade a small failure for a large one.
    #[test]
    fn an_exo_that_printed_no_id_loses_its_history_and_not_its_lane() {
        let turn = ObservedTurn {
            observed: ObservedExoCapabilityState {
                source_commit: "commit".into(),
                source_tree: "tree".into(),
                binary_digest: "sha256:binary".into(),
                agent: "omega-lane".into(),
                conversation: "tier-a".into(),
                generation: 3,
                agent_authored_tools: false,
                tool_modules: Vec::new(),
                read_write_mounts: Vec::new(),
            },
            identity: an_identity(),
            networking: false,
            exo_agent_id: None,
            exo_conversation_id: None,
        };
        let inspection = ready_inspection(&turn);
        assert_eq!(
            inspection.phase,
            ExoInspectionPhase::Ready,
            "the lane observed the runtime; only the durable read is out of reach"
        );
        assert_eq!(inspection.exo_agent_id, None);
        assert_eq!(inspection.error, None);

        // And what the read would say about it names the cause.
        for reason in ExoHistoryUnavailable::ALL {
            assert!(
                reason
                    .to_string()
                    .contains(ExoHistoryUnavailable::NOT_AN_EMPTY_HISTORY),
                "{reason:?}"
            );
        }
    }

    #[test]
    fn every_exo_turn_phase_has_a_distinct_visible_label() {
        let phases = [
            ExoTurnPhase::Idle,
            ExoTurnPhase::Inspecting,
            ExoTurnPhase::Working,
            ExoTurnPhase::Cancelling,
            ExoTurnPhase::Completed,
            ExoTurnPhase::Cancelled,
            ExoTurnPhase::Refused,
            ExoTurnPhase::Failed,
        ];
        let labels = phases.map(ExoTurnPhase::label);
        for (index, label) in labels.iter().enumerate() {
            assert!(!label.is_empty());
            assert!(
                !labels[..index].contains(label),
                "duplicate Exo turn label: {label}"
            );
        }
    }

    #[test]
    fn a_turn_snapshot_records_terminal_state_and_detail() {
        let turn = RefCell::new(ExoTurnSnapshot::default());
        let response = acp::PromptResponse::new(acp::StopReason::EndTurn).meta(
            [
                ("exo.session_id".into(), serde_json::json!("session-1")),
                ("exo.turn_id".into(), serde_json::json!("turn-1")),
                ("exo.latest_event_id".into(), serde_json::json!("event-1")),
            ]
            .into_iter()
            .collect::<serde_json::Map<String, serde_json::Value>>(),
        );
        set_terminal_turn_snapshot(
            &turn,
            ExoTurnPhase::Completed,
            Some("durable event observed".into()),
            &response,
        );
        let turn = turn.borrow();
        assert_eq!(turn.phase, ExoTurnPhase::Completed);
        assert_eq!(turn.detail.as_deref(), Some("durable event observed"));
        assert_eq!(turn.exo_session_id.as_deref(), Some("session-1"));
        assert_eq!(turn.exo_turn_id.as_deref(), Some("turn-1"));
        assert_eq!(turn.latest_event_id.as_deref(), Some("event-1"));
        assert!(turn.updated_at_ms > 0);
    }

    /// A machine with no Exo has no lane. This is the ordinary case, and it
    /// must not be an error.
    #[test]
    fn no_lane_file_means_no_lane() {
        let dir = tempfile::tempdir().expect("a temp dir");
        assert_eq!(ExoLaneConfig::load(&dir.path().join(EXO_LANE_FILE)), None);
    }

    /// A file this build cannot read is no lane rather than a partially trusted
    /// one. A lane pointed at the wrong `.exo` is worse than no lane.
    #[test]
    fn an_unreadable_lane_file_is_no_lane() {
        for contents in [
            "not json".to_owned(),
            lane_file("openagents.omega.exo_lane.v0"),
            serde_json::json!({ "schema": EXO_LANE_SCHEMA, "agent": "a" }).to_string(),
            lane_file(EXO_LANE_SCHEMA).replace("omega-lane", ""),
        ] {
            let (_dir, path) = write(&contents);
            assert_eq!(ExoLaneConfig::load(&path), None, "{contents}");
        }
    }

    #[test]
    fn a_tier_c_grant_is_bound_to_the_exact_session_and_prompt() {
        let session_a = acp::SessionId::new("session-a");
        let session_b = acp::SessionId::new("session-b");
        let prompt_a = vec![acp::ContentBlock::Text(acp::TextContent::new("edit"))];
        let prompt_b = vec![acp::ContentBlock::Text(acp::TextContent::new("edit more"))];
        let reference = exo_turn_ref(&session_a, &prompt_a).expect("turn ref");
        assert_ne!(
            reference,
            exo_turn_ref(&session_b, &prompt_a).expect("session-bound ref")
        );
        assert_ne!(
            reference,
            exo_turn_ref(&session_a, &prompt_b).expect("prompt-bound ref")
        );
    }

    #[test]
    fn a_tier_c_receipt_round_trips_all_authority_and_outcome_fields() {
        let observed = ObservedExoCapabilityState {
            source_commit: "commit".into(),
            source_tree: "tree".into(),
            binary_digest: "sha256:binary".into(),
            agent: "agent".into(),
            conversation: "conversation".into(),
            generation: 12,
            agent_authored_tools: true,
            tool_modules: Vec::new(),
            read_write_mounts: Vec::new(),
        };
        let authority = ExoSelfModificationReceipt {
            objective: "Update and verify Exo.".into(),
            turn_ref: "turn".into(),
            generation: observed.generation,
            expires_at_ms: 200,
            origin: ExoSelfModificationConsentOrigin::HumanConfirmationDialog,
            capabilities: observed.requested_capabilities(),
            observed: observed.clone(),
        };
        let receipt = ExoTierCReceipt {
            schema: "openagents.omega.exo_self_modification_receipt.v1".into(),
            recorded_at_ms: 150,
            authority: Some(authority),
            observed,
            turn_ref: "turn".into(),
            requested_objective: Some("Update and verify Exo.".into()),
            outcome: "completed".into(),
            exo_session_id: Some("session".into()),
            exo_turn_id: Some("turn".into()),
            latest_event_id: Some("event".into()),
            verification: "verified".into(),
            reconnect: "not reported by Exo ACP".into(),
            rollback: "not reported by Exo ACP".into(),
        };
        let encoded = serde_json::to_string(&receipt).expect("serialize receipt");
        let decoded = serde_json::from_str::<ExoTierCReceipt>(&encoded).expect("read receipt");
        assert_eq!(decoded, receipt);
    }

    #[test]
    fn a_refusal_receipt_has_no_authority_or_durable_exo_turn() {
        let observed = ObservedExoCapabilityState {
            source_commit: "commit".into(),
            source_tree: "tree".into(),
            binary_digest: "sha256:binary".into(),
            agent: "agent".into(),
            conversation: "conversation".into(),
            generation: 12,
            agent_authored_tools: true,
            tool_modules: Vec::new(),
            read_write_mounts: Vec::new(),
        };
        let receipt =
            refused_tier_c_receipt(observed, "turn".into(), None, "refused: no grant".into());
        assert!(receipt.authority.is_none());
        assert!(receipt.exo_turn_id.is_none());
        assert_eq!(
            receipt.verification,
            "Omega refused the turn before it crossed ACP"
        );
    }

    #[test]
    fn a_sandbox_module_resolves_only_through_an_observed_mount() {
        let mount = ExoMount {
            host_path: "/host/exo".into(),
            mount_path: "/workspace/exo".into(),
            read_write: true,
        };
        assert_eq!(
            resolve_module_host_path("/workspace/exo/tools/guardian.ts", &[mount]),
            Some(PathBuf::from("/host/exo/tools/guardian.ts"))
        );
        assert_eq!(
            resolve_module_host_path("/other/tools/guardian.ts", &[]),
            None
        );
    }

    /// The lane, against a real Exo. `#[ignore]`d because it needs one: a
    /// built `exo` at the pinned commit, a configured agent and conversation,
    /// and whatever credential that agent's model binding resolves to. Exo
    /// `#[ignore]`s its own heavy integration cells for the same reason.
    ///
    /// Point it at an install with the same lane file the product reads:
    ///
    /// ```text
    /// OMEGA_EXO_LANE_FILE=/path/to/omega-exo-lane.json \
    ///   cargo test -p agent_ui drives_a_real_exo -- --ignored --nocapture
    /// ```
    ///
    /// This is the acceptance path end to end: the router's external executor
    /// builds a thread, a turn runs through Exo's CLI, the reply lands in the
    /// thread, and the thread discloses who ran it.
    #[gpui::test]
    #[ignore = "needs a local Exo install at the pinned commit"]
    async fn drives_a_real_exo(cx: &mut gpui::TestAppContext) {
        use crate::omega_executor_disclosure::ThreadExecutorDisclosure as _;

        let Ok(lane_file) = std::env::var("OMEGA_EXO_LANE_FILE") else {
            panic!("set OMEGA_EXO_LANE_FILE to a lane file; see this test's docs");
        };
        // The turn runs a real process against a real model, so this test
        // waits on wall-clock I/O rather than on the deterministic scheduler.
        // That is the point of it: the same await that parks here is what keeps
        // the window drawing in production, and the workspace's clippy
        // configuration disallows the blocking call that would not park.
        cx.executor().allow_parking();
        crate::test_support::init_test(cx);
        let fs = fs::FakeFs::new(cx.executor());
        let lane_path = PathBuf::from(lane_file);
        let process_cwd = lane_path
            .parent()
            .expect("the lane file has a parent")
            .to_path_buf();
        fs.insert_tree(&process_cwd, serde_json::json!({})).await;
        let project = Project::test(fs, [process_cwd.as_path()], cx).await;

        let agent_server_store =
            project.read_with(cx, |project, _| project.agent_server_store().downgrade());
        let connect_project = project.clone();
        let connection = cx
            .update(|cx| {
                cx.spawn(async move |cx| {
                    connect_configured_lane(&lane_path, connect_project, agent_server_store, cx)
                        .await
                })
            })
            .await
            .expect("the Exo ACP lane connects")
            .expect("the lane file names an Exo install");
        let thread = cx
            .update(|cx| {
                connection
                    .clone()
                    .new_session(project, PathList::new(&[process_cwd]), cx)
            })
            .await
            .expect("a session on the Exo lane");

        let session_id = thread.read_with(cx, |thread, _| thread.session_id().clone());
        let tool_marker = "OMEGA-EXO-TOOL";
        let marker = "OMEGA-EXO-PANE-READY";
        let request = acp::PromptRequest::new(
            session_id,
            vec![acp::ContentBlock::Text(acp::TextContent::new(format!(
                "Run one tool that prints {tool_marker}, then reply with exactly {marker}."
            )))],
        );
        let response = cx
            .update(|cx| connection.prompt(request, cx))
            .await
            .expect("Exo ran the turn");
        assert_eq!(response.stop_reason, acp::StopReason::EndTurn);

        let rendered = thread.read_with(cx, |thread, cx| thread.to_markdown(cx));
        assert!(
            rendered.contains(marker),
            "the Exo reply did not reach the thread: {rendered}"
        );
        assert!(
            rendered.contains(tool_marker),
            "the Exo tool result did not reach the thread: {rendered}"
        );

        let disclosure = thread.read_with(cx, |thread, cx| thread.omega_executor_disclosure(cx));
        assert!(disclosure.is_coherent(), "{disclosure:?}");
        assert_eq!(disclosure.class, omega_exo_lane::EXO_EXECUTOR_CLASS);
        assert_eq!(disclosure.run_ref, None);
        let exo = connection
            .clone()
            .downcast::<ExoHarnessConnection>()
            .expect("the configured lane is Exo");
        let identity = exo.identity().expect("Exo told the lane who it is");
        let inspection = exo.inspection();
        assert_eq!(inspection.phase, ExoInspectionPhase::Ready);
        assert_eq!(inspection.identity.as_ref(), Some(&identity));
        assert!(
            inspection.observed.is_some(),
            "the workspace inspector did not retain the turn's exact preflight"
        );
        let turn = exo.turn();
        assert_eq!(turn.phase, ExoTurnPhase::Completed, "{turn:?}");
        assert!(turn.exo_session_id.is_some(), "{turn:?}");
        assert!(turn.exo_turn_id.is_some(), "{turn:?}");
        assert!(turn.latest_event_id.is_some(), "{turn:?}");
        assert!(disclosure.agent_id.starts_with("exo/"), "{disclosure:?}");
        // The parts that only the Exo arm of `classify_connection` can supply.
        // Without them this assertion set passes on the shared external-agent
        // fallback, which discloses no model at all — and a disclosure that
        // says "model not disclosed" about an executor that *did* disclose one
        // is the silent failure this test exists to catch. It was silent here
        // once, on 2026-07-26, until this line was added.
        assert_eq!(
            disclosure.model, identity.model,
            "the disclosure dropped the model Exo reported: {disclosure:?}"
        );
        assert!(
            disclosure.model.is_some(),
            "Exo reported a model and the thread did not disclose it: {disclosure:?}"
        );
        println!("executor disclosure: {}", disclosure.label());
        println!("exo identity: {identity:?}");
    }
}
