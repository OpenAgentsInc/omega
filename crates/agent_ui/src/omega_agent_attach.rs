//! Attaching the coding agent that is actually on this machine. omega#106.
//!
//! `OMEGA-DELTA-0095`. Everything omega#106 needs existed except one call.
//! `omega_agent_detect` finds Codex and Claude on `PATH`, `agent_servers`
//! already hosts `codex-acp` and `claude-acp` over ACP, and
//! `OMEGA-DELTA-0055` already routes an unpinned thread to the attached
//! external agent. Nothing attached one, so a machine with Codex installed
//! still ran every turn on the native loop and the executor disclosure still
//! named `native_loop`.
//!
//! This module is that call, and only that call. It makes no routing decision
//! — [`crate::omega_router`] does — and it holds no state.
//!
//! # Presence decides, configuration does not
//!
//! `AllAgentServersSettings` records what is **configured**. Omega ships
//! `codex-acp` in its own defaults, so that map is non-empty on every machine
//! including one that has never had Codex installed. Attaching from it would
//! attach Codex everywhere, and the failure would arrive as a thread that
//! reports Codex and runs something else — which is exactly the defect class
//! the disclosure line exists to prevent.
//!
//! So the gate is [`omega_agent_detect`]: an executable file on `PATH`. The
//! settings map is then only the means of *hosting* what detection found.
//!
//! # One agent, chosen by a rule that does not move
//!
//! The router has one external-ACP slot, and several agents can be present.
//! The chosen one is the first entry of `omega_agent_detect::CANDIDATES` that
//! is both present and drivable, which is Codex first — the owner's stated
//! preference on omega#100 — and Claude next. That order is the *candidate*
//! order, not `PATH` order, so the shell Omega was launched from cannot change
//! which agent runs.
//!
//! `omega_agent_detect::preferred` is deliberately **not** used here. It
//! answers a stricter question — "is Codex here?" — and returns `None` when
//! Codex is absent even though Claude is present, because omega#100 asked for
//! Codex specifically and did not want a substitution made silently. omega#106
//! asks the wider question and answers it out loud: acceptance 3 is a machine
//! with Codex absent and Claude present, running the turn on Claude, with the
//! disclosure line saying so. The disclosure carries the agent id of the
//! connection that ran, so `codex-acp` and `claude-acp` are distinguishable in
//! the window without a second record to drift from it.
//!
//! GitHub Copilot and Cursor are detected by `omega_agent_detect` and are not
//! drivable here: Omega hosts no ACP server for either, so choosing one would
//! produce a failure at connect time rather than a thread. They are passed
//! over, by name, in the log line.
//!
//! # A chosen agent that cannot start is an error, never a fallback
//!
//! Once an agent has been chosen, every way of not reaching it returns `Err`
//! naming the agent and the file detection found. It never degrades to
//! `Ok(None)`. A silent degrade would put a thread on the native loop on a
//! machine the person believes is running Codex, and the whole point of
//! omega#77's disclosure is that this cannot happen quietly.
//!
//! `Ok(None)` means **no external executor is to be attached**, and this module
//! returns it from exactly one place. `OMEGA-DELTA-0095` admitted one way to
//! reach it — nothing drivable is installed, the ordinary case for a new person
//! and the case omega#106 requires to keep working — and `OMEGA-DELTA-0114`
//! admits a second, a person choosing Omega's own loop. The single return is
//! kept, because the rule being counted is not "one reason" but **no failure
//! reaches it**. Either way the router registers no external executor, the
//! native loop runs, and the composer shows the existing visible fallback
//! reason.
//!
//! # Why the failure was kept, when degrading looked kinder
//!
//! omega#106's close-out reopened the rule above. The case against it: an
//! unreachable *registry* is not an absent *agent*. The agent is installed and
//! the download is not available, and refusing to open a thread over Omega's
//! own supply chain reads as the app failing to open. That is a real
//! distinction and it is drawable — but it is not decidable here, which is the
//! only place it would have to be decided.
//!
//! Three things settled it.
//!
//! **The failure is the retry.** `ConversationView` subscribes to the
//! agent-server store. The ACP registry finishing its load rebuilds that store
//! and emits `AgentServersUpdated`, and a view sitting in `ServerState::
//! LoadError` resets and connects again. So a registry that is a few seconds
//! late costs a few seconds of a named error and then heals into Codex on its
//! own. A degrade would connect *successfully* with the native loop, reach
//! `Connected` with no thread error, and never be re-driven — stranding the
//! session on the native loop after the agent became reachable. That is a
//! thread running one executor while the reader believes another, reached from
//! the opposite direction. `a_failed_attach_is_retried_when_the_adapter_
//! registers` in `crates/omega_deltas` holds that seam, because the argument
//! for failing lives in a file this one does not own.
//!
//! **Nothing here can tell late from gone.** At the moment the bound expires,
//! a registry three seconds behind and a registry permanently unreachable are
//! the same observation. A policy fixed at that instant is wrong in one
//! direction or the other; an error defers the question to the only thing that
//! can answer it, which is time, and the retry above closes the loop.
//!
//! **The offline case buys less than it looks.** With a warm registry cache
//! the adapter registers and the attach succeeds. With a cold one — a first
//! launch that has never had network — the native loop has no configured
//! provider either, so a degrade would hand back a composer that fails one
//! layer later with a worse sentence. What the reader needs there is to be
//! told what is missing, not to be moved somewhere without being asked.
//!
//! What was wrong was the sentence, not the rule; see [`await_registration`].
//!
//! # The first-party path back, and why it is a button
//!
//! `OMEGA-DELTA-0114`, omega#106. The cost above was stated rather than argued
//! away, and then paid: `Agent::NativeAgent` *is* the router, so while a chosen
//! agent stays unreachable there is no picker entry that reaches the native
//! loop, and a persistently unreachable adapter left the panel with no
//! first-party path at all.
//!
//! The fix is [`run_on_omegas_own_loop`], and the whole of its design is that a
//! **person** calls it. A degrade decided here would be the substitution this
//! module refuses; the same destination reached by someone reading a sentence
//! and pressing a button is a choice, and a thread that runs on the native loop
//! because its reader asked for the native loop discloses exactly what happened.
//! That is the distinction the hard failure was kept to protect, so the escape
//! hatch must not quietly become the policy: the choice is per-process, it is
//! offered only once an attach has actually failed, and a restart returns to
//! the adapter.
//!
//! # A wait a person can tell from a hang
//!
//! `OMEGA-DELTA-0114`. What the attach spends its time on is not the `codex`
//! binary. It is `npm exec --yes` resolving an npx package — plus Zed's Node
//! runtime, if this machine has never fetched one — and that had no overall
//! bound anywhere: not in the resolve, not in the ACP handshake (which races
//! only against the child process exiting), only whatever the underlying HTTP
//! connect timeout happened to be. It also said nothing.
//! `LocalRegistryNpxAgent` does not implement `set_loading_status_tx`, so the
//! whole download rendered as the generic pulsing `Loading…` the panel shows
//! for any unfinished connect.
//!
//! A silent unbounded wait is indistinguishable from a hang, and this one sits
//! between a new person and their first composer. Both halves are fixed here
//! rather than in the npx agent, because what the reader needs named is not
//! "an npm package" but *which adapter, for which agent, and how long so far*
//! — and this is the only place that knows all three:
//!
//! - **Bounded** by [`ADAPTER_START_TIMEOUT`], so a wedged resolve becomes a
//!   sentence instead of a spinner that never ends.
//! - **Named**, through the loading-status channel the router takes off its
//!   delegate ([`agent_servers::AgentServerDelegate::take_loading_status`]).
//! - **Ticking**, once a second. The elapsed count is the part that does the
//!   work: a label that changes is a wait, a label that does not is a hang, and
//!   a person can tell those apart without knowing what npx is.
//!
//! It recurs, too — `npm exec --yes` runs on every connect, not once — so this
//! is not a first-launch-only sentence and is not written as one.

use std::path::PathBuf;
use std::rc::Rc;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use acp_thread::AgentConnection;
use agent_servers::{AgentServer as _, AgentServerDelegate, CustomAgentServer};
use anyhow::{Context as _, Result, anyhow};
use futures::FutureExt as _;
use gpui::{AsyncApp, Entity, WeakEntity};
use omega_agent_detect::DetectedAgent;
use project::{AgentId, Project, agent_server_store::AgentServerStore};

/// The detected agents Omega can actually host over ACP, in preference order.
///
/// Taken from `agent_servers` rather than spelled again, so an id that is
/// renamed there cannot leave this list pointing at an agent that no longer
/// exists.
pub const DRIVABLE_AGENT_IDS: &[&str] = &[
    agent_servers::CODEX_ID,
    agent_servers::CLAUDE_AGENT_ID,
    agent_servers::GROK_ID,
];

/// How long a chosen agent is given to appear in the agent-server store.
///
/// The store's external agents are rebuilt when the ACP registry loads, and on
/// a fresh `--user-data-dir` that load is a network fetch that has not
/// finished when the first thread is created. Without this the very case
/// omega#106's first acceptance names — a fresh data directory on a machine
/// with Codex — would fail on a race rather than on anything about Codex.
///
/// It is a bound, not a wait: a machine where the agent never registers spends
/// this once and then gets the error that names it. A machine where nothing is
/// detected never reaches here at all, so the no-agent case costs nothing.
///
/// Five seconds is deliberately far short of the registry's own 30-second
/// fetch timeout, and that is not an oversight. Sitting here for the full
/// fetch would show a spinner where an explanation belongs; expiring early
/// shows the explanation, and `ConversationView` re-drives the connect when
/// the registry does land. Erring short is therefore erring towards telling
/// the reader something. Raising this to "cover" the fetch would trade a
/// self-healing few seconds of prose for half a minute of nothing.
pub const REGISTRATION_ATTEMPTS: usize = 50;

/// The interval between the attempts [`REGISTRATION_ATTEMPTS`] bounds.
pub const REGISTRATION_INTERVAL: Duration = Duration::from_millis(100);

/// How long the chosen agent's ACP adapter is given to start.
///
/// `OMEGA-DELTA-0114`. This covers `npm exec --yes` resolving the adapter
/// package, Zed's Node runtime being fetched if this machine has none, the
/// process launching, and the ACP `initialize` handshake completing. None of
/// those had any overall bound: the handshake races only against the child
/// exiting, so an adapter that starts and then never answers held the panel
/// open with a pulsing label for as long as the machine stayed on.
///
/// Three minutes is deliberately generous, and the generosity is the point.
/// A cold resolve is tens of megabytes over whatever link the reader has, and
/// a bound tight enough to catch a hang quickly would cut a working first
/// launch on a slow connection — turning "this is slow" into "this is broken",
/// which is the worse of the two errors. What makes the wait tolerable is
/// [`starting_adapter`]'s ticking elapsed count, not a short deadline. The
/// bound exists so that a resolve which is *not* progressing ends in a sentence
/// rather than in a spinner nobody can outlast.
pub const ADAPTER_START_TIMEOUT: Duration = Duration::from_secs(180);

/// How often the adapter's start is re-announced while it is still running.
///
/// One second, because the number in the label is the only evidence a reader
/// has that anything is still happening. Slower and a live wait starts to look
/// frozen; faster and the label flickers without saying anything new.
pub const PROGRESS_TICK: Duration = Duration::from_secs(1);

/// The ACP adapter a chosen agent could not be run through, if one failed.
///
/// Carried rather than discarded so a surface can offer the reader a way
/// forward that names the thing that failed. The agent's own binary is here
/// too, and is here to be *exonerated*: a surface that mentions it must say it
/// is fine, never imply it is the cause.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UnreachableAdapter {
    /// The ACP adapter id, e.g. `codex-acp`. This is the npm package Omega
    /// resolves, and it is what actually failed.
    pub adapter_id: &'static str,
    /// The agent that adapter would have driven, e.g. `Codex`.
    pub agent_name: &'static str,
    /// The binary detection found on `PATH`. Present and working.
    pub binary: PathBuf,
}

/// The last adapter failure, or `None` if the last attach reached its agent.
///
/// Process-global because the reader it is for is looking at a panel that has
/// no other way to learn why its connect failed: the error crosses that seam as
/// a `LoadError::Other(String)`, and re-deriving the cause by reading that
/// string back would be a parser over prose.
static UNREACHABLE_ADAPTER: Mutex<Option<UnreachableAdapter>> = Mutex::new(None);

/// A person's standing choice to run on Omega's own loop.
///
/// See the module docs. Per-process and not persisted: the adapter is the
/// intended executor, this is a way past a failure, and a restart is the
/// cheapest honest expiry for a decision made about a network that was down a
/// minute ago.
static OMEGAS_OWN_LOOP_CHOSEN: AtomicBool = AtomicBool::new(false);

/// The adapter the last attach could not reach, if it could not reach one.
#[must_use]
pub fn unreachable_adapter() -> Option<UnreachableAdapter> {
    UNREACHABLE_ADAPTER
        .lock()
        .expect("the unreachable-adapter record is never held across a panic")
        .clone()
}

/// Record that this attach could not reach `agent`'s adapter.
fn record_unreachable(agent: &DetectedAgent) {
    *UNREACHABLE_ADAPTER
        .lock()
        .expect("the unreachable-adapter record is never held across a panic") =
        Some(UnreachableAdapter {
            adapter_id: agent.id,
            agent_name: agent.name,
            binary: agent.binary.clone(),
        });
}

/// Forget any recorded failure, because an attach reached its agent.
fn clear_unreachable() {
    *UNREACHABLE_ADAPTER
        .lock()
        .expect("the unreachable-adapter record is never held across a panic") = None;
}

/// Run subsequent threads on Omega's own loop instead of the detected agent.
///
/// `OMEGA-DELTA-0114`. **Only a person may call this.** It is the one thing in
/// this module that reaches the native loop with a drivable agent installed,
/// and everything above about silent fallbacks applies to it in full: called
/// from a timeout, a retry limit, or any other piece of code deciding on the
/// reader's behalf, it becomes precisely the substitution the hard failure
/// exists to refuse. Called from a button the reader pressed after reading what
/// failed, it is a choice, and the disclosure line then reports the native loop
/// because the native loop is what they asked for.
pub fn run_on_omegas_own_loop() {
    log::info!(
        "OMEGA-DELTA-0114: a person chose Omega's own loop over the ACP \
         adapter for the rest of this session"
    );
    OMEGAS_OWN_LOOP_CHOSEN.store(true, Ordering::SeqCst);
}

/// Whether a person has chosen Omega's own loop for this session.
#[must_use]
pub fn omegas_own_loop_chosen() -> bool {
    OMEGAS_OWN_LOOP_CHOSEN.load(Ordering::SeqCst)
}

/// What the panel says while the chosen agent's ACP adapter is starting.
///
/// The elapsed seconds are not decoration. The failure this replaces was a
/// pulsing `Loading…` over an unbounded npm resolve, and a reader's whole
/// problem there was that a slow install and a wedged process render
/// identically. A number that goes up is the difference.
///
/// Kept short on purpose: this draws as a single centered `Label` in a panel
/// that is often a narrow sidebar, so a sentence explaining npx would be a
/// sentence nobody can read. The explanation belongs in the failure callout,
/// which wraps.
#[must_use]
pub fn starting_adapter(agent: &DetectedAgent, elapsed: Duration) -> String {
    format!("Starting {} (npm) — {}s", agent.id, elapsed.as_secs())
}

/// Which detected agent will run the turn, and which were passed over.
///
/// The passed-over set is carried rather than dropped so the log line can name
/// the agents that were present and not chosen. "Codex ran it" is a much
/// weaker statement on its own than "Codex ran it, and Claude was also here".
#[derive(Debug, PartialEq, Eq)]
pub struct ExecutorChoice<'a> {
    /// The agent to attach.
    pub chosen: &'a DetectedAgent,
    /// Everything else detection found, present and not chosen, in detection
    /// order.
    pub passed_over: Vec<&'a DetectedAgent>,
}

/// One exact ACP executor that is connected and ready to create sessions.
#[derive(Clone)]
pub struct AttachedExecutor {
    pub agent: DetectedAgent,
    pub connection: Rc<dyn AgentConnection>,
}

/// Why one detected executor is not in the router's live inventory.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExecutorAttachmentFailure {
    pub agent: DetectedAgent,
    pub reason: String,
}

/// The complete result of attaching the detected executors Omega can drive.
///
/// A failed adapter does not erase the adapters that did connect. It remains
/// in `unavailable`, by exact id and with its bounded failure, so routing can
/// record the readiness snapshot it actually saw instead of silently treating
/// the executor as if it had never been installed.
#[derive(Default)]
pub struct AttachedExecutorInventory {
    pub ready: Vec<AttachedExecutor>,
    pub unavailable: Vec<ExecutorAttachmentFailure>,
}

/// Every detected executor Omega can drive, in the router's stable priority
/// order and with duplicate ids removed.
#[must_use]
pub fn drivable_agents(detected: &[DetectedAgent]) -> Vec<DetectedAgent> {
    DRIVABLE_AGENT_IDS
        .iter()
        .filter_map(|id| detected.iter().find(|agent| agent.id == *id).cloned())
        .collect()
}

/// The agent a thread's turns should execute on, given what is installed.
///
/// `None` means nothing drivable is installed. It does not mean "an error
/// happened"; see the module docs for why those two are kept apart.
#[must_use]
pub fn choose_executor(detected: &[DetectedAgent]) -> Option<ExecutorChoice<'_>> {
    let index = detected
        .iter()
        .position(|agent| DRIVABLE_AGENT_IDS.contains(&agent.id))?;
    Some(ExecutorChoice {
        chosen: &detected[index],
        passed_over: detected
            .iter()
            .enumerate()
            .filter(|(position, _)| *position != index)
            .map(|(_, agent)| agent)
            .collect(),
    })
}

/// The agent an attach should reach, given what is installed and whether a
/// person has asked for Omega's own loop instead.
///
/// `OMEGA-DELTA-0114`. Split out from [`connect_detected_executor`] so the one
/// rule that can send a machine with Codex installed to the native loop is a
/// function with arguments rather than a branch over process-global state. The
/// flag is a parameter here for the same reason `PATH` is a parameter to
/// `omega_agent_detect::detect_on_path`: a rule that can only be exercised by
/// mutating a global is a rule whose test order matters.
#[must_use]
pub fn executor_to_attach(
    detected: &[DetectedAgent],
    omegas_own_loop_chosen: bool,
) -> Option<ExecutorChoice<'_>> {
    choose_executor(detected).filter(|_| !omegas_own_loop_chosen)
}

/// Name a set of agents for a log line.
fn named(agents: &[&DetectedAgent]) -> String {
    if agents.is_empty() {
        return "nothing".to_owned();
    }
    agents
        .iter()
        .map(|agent| format!("{} ({})", agent.name, agent.binary.display()))
        .collect::<Vec<_>>()
        .join(", ")
}

/// Say something on the panel's loading-status channel, if there is one.
///
/// There is not one on every path — the Exo lane and the tests both connect
/// without a panel behind them — so this is an `Option` rather than a required
/// argument, and a missing channel is silence rather than a failure.
struct Progress(Option<watch::Sender<Option<String>>>);

impl Progress {
    /// Put `message` under the pulsing label where the composer will be.
    fn say(&mut self, message: String) {
        if let Some(channel) = self.0.as_mut() {
            // `Err` here is only "nobody is listening", which is the ordinary
            // state of a connect whose view has gone away.
            let _ = channel.send(Some(message));
        }
    }

    /// Hand the label back to whatever the panel says by default.
    fn clear(&mut self) {
        if let Some(channel) = self.0.as_mut() {
            let _ = channel.send(None);
        }
    }
}

/// Connect the installed coding agent as the router's external ACP executor.
///
/// `loading_status` is the panel's channel, taken off the router's delegate;
/// see [`agent_servers::AgentServerDelegate::take_loading_status`]. Passing
/// `None` connects silently, which is right for a caller with no panel behind
/// it and wrong for the one a person is waiting on.
///
/// Returns `Ok(None)` when no external executor is to be attached. There are
/// exactly two ways to reach it and neither is a fallback from a failure:
/// nothing drivable is installed, or a person chose Omega's own loop
/// ([`run_on_omegas_own_loop`]). Every other outcome is either the connection
/// or an error that names the adapter.
///
/// # Errors
///
/// Returns an error when an agent was chosen and its adapter could not be
/// reached: the store is gone, the adapter never registers, it does not start
/// within [`ADAPTER_START_TIMEOUT`], or it fails outright.
pub async fn connect_detected_executor(
    detected: &[DetectedAgent],
    project: Entity<Project>,
    agent_server_store: WeakEntity<AgentServerStore>,
    loading_status: Option<watch::Sender<Option<String>>>,
    cx: &mut AsyncApp,
) -> Result<Option<Rc<dyn AgentConnection>>> {
    let attaching = executor_to_attach(detected, omegas_own_loop_chosen());
    if attaching.is_none() {
        match choose_executor(detected) {
            None => log::info!(
                "OMEGA-DELTA-0095: no coding agent Omega can drive is installed \
                 (detected: {}); threads stay on the native loop",
                named(&detected.iter().collect::<Vec<_>>())
            ),
            // OMEGA-DELTA-0114. Not a degrade: a person read what failed and
            // asked for this. Logged with the agent it passes over, so a
            // machine with Codex installed running the native loop is never a
            // mystery to whoever reads the log next.
            Some(choice) => log::info!(
                "OMEGA-DELTA-0114: a person chose Omega's own loop, so {} ({}) \
                 at {} is not attached",
                choice.chosen.name,
                choice.chosen.id,
                choice.chosen.binary.display()
            ),
        }
    }
    // One `Ok(None)`, two admitted reasons to reach it, and no path from a
    // failure to either. See the module docs.
    let Some(choice) = attaching else {
        return Ok(None);
    };
    let agent = choice.chosen;
    log::info!(
        "OMEGA-DELTA-0095: attaching {} ({}) at {} as the external ACP \
         executor; passed over {}",
        agent.name,
        agent.id,
        agent.binary.display(),
        named(&choice.passed_over)
    );

    // `OMEGA-DELTA-0117`. The adapter may already be started, because the last
    // connection warmed the executors it was not. Nothing about the rest of
    // this changes: a miss here is the ordinary attach below, with its channel,
    // its bound, and its failure.
    if let Some(warm) = crate::omega_executor_warmth::take_warm(agent.id, &project, cx) {
        clear_unreachable();
        return Ok(Some(warm));
    }

    let mut progress = Progress(loading_status);
    let attached = attach(agent, project, agent_server_store, &mut progress, cx).await;
    progress.clear();
    match attached {
        Ok(connection) => {
            clear_unreachable();
            Ok(Some(connection))
        }
        Err(error) => {
            // Recorded before the error leaves, because it crosses into the
            // panel as a `LoadError::Other(String)` and a surface that wants to
            // offer a way forward must not have to read the prose back.
            record_unreachable(agent);
            Err(error)
        }
    }
}

/// Connect every installed coding-agent adapter the router may select.
///
/// Starts cold adapters concurrently. Three independent bounded npm starts
/// must cost at most the slowest bound, not the sum of all three bounds. Warm
/// connections are claimed first and participate in the same exact-id result.
///
/// The returned vectors are in [`DRIVABLE_AGENT_IDS`] order regardless of the
/// order in which adapter handshakes finish. That ordering is part of the
/// router input and therefore cannot depend on scheduler timing.
pub async fn connect_detected_executors(
    detected: &[DetectedAgent],
    project: Entity<Project>,
    agent_server_store: WeakEntity<AgentServerStore>,
    loading_status: Option<watch::Sender<Option<String>>>,
    cx: &mut AsyncApp,
) -> AttachedExecutorInventory {
    let agents = drivable_agents(detected);
    if agents.is_empty() || omegas_own_loop_chosen() {
        return AttachedExecutorInventory::default();
    }

    let mut progress = Progress(loading_status);
    progress.say(format!(
        "Connecting {} installed coding agent{}",
        agents.len(),
        if agents.len() == 1 { "" } else { "s" }
    ));

    let mut ready = Vec::new();
    let mut starts = Vec::new();
    for agent in agents {
        if let Some(connection) = crate::omega_executor_warmth::take_warm(agent.id, &project, cx) {
            ready.push(AttachedExecutor { agent, connection });
            continue;
        }

        let project = project.clone();
        let agent_server_store = agent_server_store.clone();
        let task = cx.spawn({
            let agent = agent.clone();
            async move |cx| start_adapter_silently(&agent, project, agent_server_store, cx).await
        });
        starts.push((agent, task));
    }

    let mut unavailable = Vec::new();
    for (agent, start) in starts {
        match start.await {
            Ok(connection) => ready.push(AttachedExecutor { agent, connection }),
            Err(error) => unavailable.push(ExecutorAttachmentFailure {
                agent,
                reason: format!("{error:#}"),
            }),
        }
    }
    progress.clear();

    ready.sort_by_key(|attached| {
        DRIVABLE_AGENT_IDS
            .iter()
            .position(|id| *id == attached.agent.id)
            .unwrap_or(usize::MAX)
    });
    unavailable.sort_by_key(|failure| {
        DRIVABLE_AGENT_IDS
            .iter()
            .position(|id| *id == failure.agent.id)
            .unwrap_or(usize::MAX)
    });

    AttachedExecutorInventory { ready, unavailable }
}

/// Start `agent`'s adapter with nothing said and nothing recorded.
///
/// `OMEGA-DELTA-0117`. The one door a preload may start an adapter through, so
/// that there is still exactly one place an adapter is started and it is the
/// place [`ADAPTER_START_TIMEOUT`] and the tick live. What differs from a
/// person's own attach is only what is *not* here: no loading-status channel,
/// because that channel has one holder and stealing it would leave a person's
/// own start silent again; and no [`record_unreachable`], because nobody asked
/// for this adapter and a failure nobody saw must not offer them a way past it.
///
/// # Errors
///
/// Exactly what [`attach`] returns. The caller is expected to discard them.
pub(crate) async fn start_adapter_silently(
    agent: &DetectedAgent,
    project: Entity<Project>,
    agent_server_store: WeakEntity<AgentServerStore>,
    cx: &mut AsyncApp,
) -> Result<Rc<dyn AgentConnection>> {
    attach(agent, project, agent_server_store, &mut Progress(None), cx).await
}

/// Reach `agent`'s ACP adapter, saying what is happening while it happens.
///
/// # Errors
///
/// Every way of not reaching the adapter, each naming the adapter rather than
/// the agent's own installation.
async fn attach(
    agent: &DetectedAgent,
    project: Entity<Project>,
    agent_server_store: WeakEntity<AgentServerStore>,
    progress: &mut Progress,
    cx: &mut AsyncApp,
) -> Result<Rc<dyn AgentConnection>> {
    let store = agent_server_store.upgrade().with_context(|| {
        format!(
            "Omega's agent-server store is gone, so `{}`, the ACP adapter it \
             runs {} through, cannot be attached",
            agent.id, agent.name
        )
    })?;

    progress.say(format!("Finding {} in the ACP registry", agent.id));
    await_registration(agent, &store, cx).await?;

    let server = CustomAgentServer::new(AgentId::new(agent.id));
    // The adapter gets a channel of its own rather than the panel's, because
    // `watch::Sender` is not `Clone` and closes on drop, so the panel's channel
    // has exactly one holder. Handing it to the adapter would mean an npx
    // adapter that says nothing renders as silence again; keeping it here and
    // giving the adapter nothing would throw away the archive path's
    // `Installing {version}…`. Forwarding gets both: whatever the adapter says
    // wins, and Omega only fills the silence.
    let (adapter_status, adapter_said) = watch::channel::<Option<String>>(None);
    let delegate = AgentServerDelegate::new(store, None, Some(adapter_status));
    let connect = cx.update(|cx| server.connect(delegate, project, cx));
    start_adapter(agent, connect, adapter_said, progress, cx).await
}

/// Await the adapter's start, bounded, announcing how long it has taken.
///
/// `OMEGA-DELTA-0114`. `adapter_said` carries whatever the adapter reported for
/// itself, and it wins: the archive path emits `Installing {version}…`, which
/// is more specific than anything this loop knows. The two npx adapters
/// omega#106 attaches report nothing at all (`LocalRegistryNpxAgent` does not
/// implement `set_loading_status_tx`), and they are why this exists — without
/// it their entire download renders as one unchanging word.
///
/// # Errors
///
/// The adapter's own failure, or [`ADAPTER_START_TIMEOUT`] elapsing with the
/// adapter neither started nor failed. The two are worded separately and
/// neither is wrapped in the other: the panel receives this as
/// `LoadError::Other(err.to_string())`, and `anyhow`'s `to_string` renders only
/// the outermost context — so a timeout carrying a generic outer sentence would
/// arrive at the reader as the generic sentence with the timeout invisible.
async fn start_adapter(
    agent: &DetectedAgent,
    connect: gpui::Task<Result<Rc<dyn AgentConnection>>>,
    mut adapter_said: watch::Receiver<Option<String>>,
    progress: &mut Progress,
    cx: &mut AsyncApp,
) -> Result<Rc<dyn AgentConnection>> {
    let mut connect = Box::pin(connect).fuse();
    let mut waited = Duration::ZERO;
    loop {
        let mut tick = cx.background_executor().timer(PROGRESS_TICK).fuse();
        futures::select_biased! {
            started = connect => return started.with_context(|| format!(
                "`{}`, the ACP adapter Omega runs {} through, did not start. \
                 That adapter is resolved from the ACP registry and is not the \
                 {} at {} detection found",
                agent.id,
                agent.name,
                agent.name,
                agent.binary.display()
            )),
            () = tick => {
                waited += PROGRESS_TICK;
                if waited >= ADAPTER_START_TIMEOUT {
                    return Err(anyhow!(
                        "`{}`, the ACP adapter Omega runs {} through, did not \
                         start within {} seconds. It is an npm package Omega \
                         resolves from the ACP registry every time it connects, \
                         and it is a separate download from the {} at {}, which \
                         is fine. A network that cannot reach the npm registry \
                         is the usual cause. Omega retries when the registry \
                         reloads, and you can run this thread on Omega's own \
                         loop instead.",
                        agent.id,
                        agent.name,
                        ADAPTER_START_TIMEOUT.as_secs(),
                        agent.name,
                        agent.binary.display()
                    ));
                }
                let said = adapter_said.borrow().clone();
                progress.say(said.unwrap_or_else(|| starting_adapter(agent, waited)));
            }
        }
    }
}

/// Wait, boundedly, for the chosen agent's ACP server to register.
///
/// # The failure names Omega's supply chain, not the reader's installation
///
/// Detection proved that `{agent.binary}` exists. It is *not* what runs the
/// turn: `codex-acp` and `claude-acp` are separate adapters, resolved from the
/// ACP registry, and this wait is for one of those to appear in the store.
/// When it does not, the reader's own Codex is working and Omega's fetch of
/// its adapter is not.
///
/// So the sentence must not open with their binary and its path and then
/// report a failure. That reads as "your Codex is broken" and sends them to
/// debug the one part of this that is fine. It is the honest-attribution rule
/// — say what happened, do not attribute it to something that did not do it —
/// applied to a failure instead of to a turn.
///
/// It also says that Omega retries, because that is true and because it is
/// what makes waiting the right thing for the reader to do. The retry is
/// `ConversationView::handle_agent_servers_updated`; see the module docs for
/// why this stays an error at all.
///
/// # Errors
///
/// Returns an error naming the adapter, and the agent it would have driven,
/// when it has not registered within [`REGISTRATION_ATTEMPTS`].
async fn await_registration(
    agent: &DetectedAgent,
    store: &Entity<AgentServerStore>,
    cx: &mut AsyncApp,
) -> Result<()> {
    let id = AgentId::new(agent.id);
    for _ in 0..REGISTRATION_ATTEMPTS {
        if cx.update(|cx| store.read(cx).external_agents().any(|name| *name == id)) {
            return Ok(());
        }
        cx.background_executor().timer(REGISTRATION_INTERVAL).await;
    }
    Err(anyhow!(
        "Omega could not resolve `{}`, the ACP adapter it runs {} through, \
         from the ACP registry. Your {} at {} is fine — the adapter is a \
         separate download, and a first launch needs to reach the network \
         once. Omega retries the moment the registry loads. Until then the \
         turn is not quietly moved to the native loop.",
        agent.id,
        agent.name,
        agent.name,
        agent.binary.display()
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn detected(id: &'static str, name: &'static str) -> DetectedAgent {
        DetectedAgent {
            id,
            name,
            binary: PathBuf::from("/usr/local/bin").join(name.to_lowercase()),
        }
    }

    fn codex() -> DetectedAgent {
        detected("codex-acp", "Codex")
    }

    fn claude() -> DetectedAgent {
        detected("claude-acp", "Claude")
    }

    fn copilot() -> DetectedAgent {
        detected("github-copilot-cli", "Copilot")
    }

    /// omega#106 acceptance 1 and 2, at the choice layer.
    #[test]
    fn codex_runs_the_turn_when_it_is_installed() {
        let installed = vec![codex(), claude()];

        let choice = choose_executor(&installed).expect("Codex is drivable and installed");

        assert_eq!(choice.chosen.id, "codex-acp");
        assert_eq!(
            choice.passed_over.iter().map(|a| a.id).collect::<Vec<_>>(),
            vec!["claude-acp"],
            "an agent that was present and not chosen is named, not dropped"
        );
    }

    /// omega#106 acceptance 3. The case `omega_agent_detect::preferred`
    /// deliberately answers `None` for, and the reason this module does not
    /// call it.
    #[test]
    fn claude_runs_the_turn_when_codex_is_absent() {
        let installed = vec![claude()];

        let choice = choose_executor(&installed).expect("Claude is drivable and installed");

        assert_eq!(choice.chosen.id, "claude-acp");
        assert!(
            omega_agent_detect::preferred(&installed).is_none(),
            "the stricter Codex-only question still answers None here, which is \
             why this module asks its own"
        );
        assert!(choice.passed_over.is_empty());
    }

    /// omega#106 acceptance 4 and deliverable 3, at the choice layer.
    #[test]
    fn nothing_installed_attaches_nothing() {
        assert!(
            choose_executor(&[]).is_none(),
            "a machine with no coding agent keeps the native loop"
        );
    }

    /// An agent Omega cannot host is not chosen, because choosing it would
    /// fail at connect time instead of leaving the native loop running.
    #[test]
    fn an_agent_omega_cannot_host_is_not_chosen() {
        assert!(
            choose_executor(&[copilot()]).is_none(),
            "GitHub Copilot is detected and has no ACP server here"
        );

        let installed = vec![copilot(), claude()];
        let choice = choose_executor(&installed).expect("Claude is drivable");
        assert_eq!(choice.chosen.id, "claude-acp");
        assert_eq!(
            choice.passed_over.iter().map(|a| a.id).collect::<Vec<_>>(),
            vec!["github-copilot-cli"],
        );
    }

    /// The order is the candidate order, so the shell that launched Omega
    /// cannot decide which agent runs a turn.
    #[test]
    fn the_choice_does_not_depend_on_the_order_detection_was_asked_in() {
        let installed = vec![codex(), claude()];
        let reversed = vec![claude(), codex()];

        assert_eq!(
            choose_executor(&installed).map(|choice| choice.chosen.id),
            Some("codex-acp")
        );
        assert_eq!(
            choose_executor(&reversed).map(|choice| choice.chosen.id),
            Some("claude-acp"),
            "this function takes detection's order as given; the Codex-first \
             rule lives in omega_agent_detect::CANDIDATES, and detect_on_path \
             is what guarantees the slice arrives in it"
        );
    }

    #[test]
    fn the_multi_attach_inventory_has_stable_exact_ids() {
        let detected = vec![claude(), copilot(), codex(), claude()];

        assert_eq!(
            drivable_agents(&detected)
                .iter()
                .map(|agent| agent.id)
                .collect::<Vec<_>>(),
            vec!["codex-acp", "claude-acp"],
            "scheduler order, PATH order, and duplicate observations must not change the exact router inventory"
        );
    }

    #[test]
    fn every_drivable_detected_agent_is_kept_for_routing() {
        let installed = vec![detected("grok", "Grok"), claude(), codex(), copilot()];

        assert_eq!(
            drivable_agents(&installed)
                .iter()
                .map(|agent| agent.id)
                .collect::<Vec<_>>(),
            vec!["codex-acp", "claude-acp", "grok"],
        );
    }

    /// The drivable ids are adapters, not the binaries detection found.
    ///
    /// This is the fact every failure sentence in this module turns on, and it
    /// is easy to lose because the two are named after the same product. If an
    /// id ever became the detected binary's own name, the sentences would
    /// start telling the truth by accident and stop when it changed back.
    #[test]
    fn the_drivable_ids_are_adapters_and_not_the_detected_binaries() {
        for agent in [codex(), claude()] {
            let binary = agent
                .binary
                .file_name()
                .expect("the fixture binary has a name")
                .to_string_lossy()
                .into_owned();
            assert_ne!(
                agent.id, binary,
                "`{}` is the adapter Omega resolves from the ACP registry and \
                 `{binary}` is what detection found on PATH. They are separate \
                 downloads, which is why a failure to reach one says nothing \
                 about the other.",
                agent.id
            );
            assert!(
                agent.id.ends_with("-acp"),
                "`{}` no longer reads as an ACP adapter id",
                agent.id
            );
        }
    }

    /// OMEGA-DELTA-0114. A person's choice of Omega's own loop is honoured,
    /// and it is the only thing that reaches the native loop with a drivable
    /// agent installed.
    #[test]
    fn a_person_who_chose_omegas_own_loop_gets_it() {
        let installed = vec![codex(), claude()];

        assert_eq!(
            executor_to_attach(&installed, false).map(|choice| choice.chosen.id),
            Some("codex-acp"),
            "the default is unchanged: the installed agent runs the turn"
        );
        assert!(
            executor_to_attach(&installed, true).is_none(),
            "a person who read the failure and asked for Omega's own loop is \
             given it, and nothing else in this module can produce this answer \
             with a drivable agent present"
        );
    }

    /// OMEGA-DELTA-0114. The choice is a request, not a state a failure can
    /// enter on the reader's behalf.
    ///
    /// Nothing here can stop a future caller reaching for
    /// `run_on_omegas_own_loop` from inside a timeout handler. What it can do
    /// is keep the fact checkable: the flag starts false, and the only thing
    /// in this crate that sets it is the button in `conversation_view`.
    #[test]
    fn omegas_own_loop_is_not_chosen_by_default() {
        assert!(
            !omegas_own_loop_chosen(),
            "a fresh process attaches the installed agent; the native loop is \
             somewhere a person asks to go"
        );
        assert!(
            unreachable_adapter().is_none(),
            "nothing has failed, so nothing offers a way out of a failure"
        );
    }

    /// OMEGA-DELTA-0114. The label a person watches while npm resolves names
    /// the adapter and counts, and blames nobody.
    #[test]
    fn the_starting_label_counts_and_names_the_adapter() {
        let agent = codex();

        let first = starting_adapter(&agent, Duration::from_secs(1));
        let later = starting_adapter(&agent, Duration::from_secs(47));

        assert!(
            first.contains("codex-acp"),
            "the reader must be able to tell which download this is: {first}"
        );
        assert!(
            first.contains("npm"),
            "an unexplained wait is what this replaces: {first}"
        );
        assert_ne!(
            first, later,
            "the elapsed count is the whole mechanism. A label that does not \
             change is exactly what a hang looks like, which is the defect \
             this replaces"
        );
        assert!(later.contains("47"), "{later}");
        assert!(
            !first.contains(&agent.binary.display().to_string()),
            "nothing is wrong with the reader's Codex, so the wait must not \
             put its path in front of them: {first}"
        );
    }

    /// OMEGA-DELTA-0114. The bound is a bound, and it is far enough out that a
    /// slow link is not mistaken for a broken one.
    #[test]
    fn the_adapter_start_is_bounded_well_past_a_slow_download() {
        assert!(
            ADAPTER_START_TIMEOUT > Duration::from_secs(60),
            "a cold npx resolve also fetches Zed's Node runtime. A bound tight \
             enough to catch a hang quickly turns a working first launch on a \
             slow connection into a failure, which is the worse error"
        );
        assert!(
            ADAPTER_START_TIMEOUT < Duration::from_secs(600),
            "unbounded is what this replaces; a bound nobody outlasts is the \
             same defect with a number attached"
        );
        assert!(
            PROGRESS_TICK <= Duration::from_secs(1),
            "the tick is the reader's only evidence that anything is still \
             happening"
        );
        assert!(
            ADAPTER_START_TIMEOUT
                .as_secs()
                .is_multiple_of(PROGRESS_TICK.as_secs()),
            "the bound is counted in ticks, so a tick that does not divide it \
             would expire late by up to one tick without saying so"
        );
    }

    /// Every id this module will attach is one `agent_servers` can host.
    #[test]
    fn every_drivable_id_is_a_candidate_agent_servers_hosts() {
        for id in DRIVABLE_AGENT_IDS {
            assert!(
                omega_agent_detect::CANDIDATES
                    .iter()
                    .any(|candidate| candidate.id == *id),
                "`{id}` is drivable here but is not something detection looks \
                 for, so it could never be chosen"
            );
        }
    }
}
