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
//! `Ok(None)` has exactly one meaning, and this module returns it in exactly
//! one place: **nothing drivable is installed**. That is the ordinary case for
//! a new person, and it is the case omega#106 requires to keep working — the
//! router registers no external executor, the native loop runs, and the
//! composer shows the existing visible fallback reason.
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
//! The cost is stated rather than argued away: `Agent::NativeAgent` *is* the
//! router, so while a chosen agent stays unreachable there is no picker entry
//! that reaches the native loop, and a persistently unreachable adapter leaves
//! the panel with no first-party path. The fix for that is an explicit "run on
//! Omega's own loop" action on the error — a choice the reader makes, not a
//! substitution made for them — and it belongs to the panel, not here.

use std::rc::Rc;
use std::time::Duration;

use acp_thread::AgentConnection;
use agent_servers::{AgentServer as _, AgentServerDelegate, CustomAgentServer};
use anyhow::{Context as _, Result, anyhow};
use gpui::{AsyncApp, Entity, WeakEntity};
use omega_agent_detect::DetectedAgent;
use project::{AgentId, Project, agent_server_store::AgentServerStore};

/// The detected agents Omega can actually host over ACP, in preference order.
///
/// Taken from `agent_servers` rather than spelled again, so an id that is
/// renamed there cannot leave this list pointing at an agent that no longer
/// exists.
pub const DRIVABLE_AGENT_IDS: &[&str] = &[agent_servers::CODEX_ID, agent_servers::CLAUDE_AGENT_ID];

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

/// Connect the installed coding agent as the router's external ACP executor.
///
/// Returns `Ok(None)` only when nothing drivable is installed. Every other
/// outcome is either the connection or an error that names the agent.
///
/// # Errors
///
/// Returns an error when an agent was chosen and could not be reached: the
/// store is gone, the agent never registers an ACP server, or the ACP server
/// fails to start.
pub async fn connect_detected_executor(
    detected: &[DetectedAgent],
    project: Entity<Project>,
    agent_server_store: WeakEntity<AgentServerStore>,
    cx: &mut AsyncApp,
) -> Result<Option<Rc<dyn AgentConnection>>> {
    let Some(choice) = choose_executor(detected) else {
        log::info!(
            "OMEGA-DELTA-0095: no coding agent Omega can drive is installed \
             (detected: {}); threads stay on the native loop",
            named(&detected.iter().collect::<Vec<_>>())
        );
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

    let store = agent_server_store.upgrade().with_context(|| {
        format!(
            "Omega's agent-server store is gone, so `{}` cannot be attached to \
             run {} (found at {})",
            agent.id,
            agent.name,
            agent.binary.display()
        )
    })?;

    await_registration(agent, &store, cx).await?;

    let server = CustomAgentServer::new(AgentId::new(agent.id));
    let delegate = AgentServerDelegate::new(store, None, None);
    let connect = cx.update(|cx| server.connect(delegate, project, cx));
    let connection = connect.await.with_context(|| {
        format!(
            "`{}`, the ACP adapter Omega runs {} through, did not start. That \
             adapter is resolved from the ACP registry and is not the {} at \
             {} detection found",
            agent.id,
            agent.name,
            agent.name,
            agent.binary.display()
        )
    })?;
    Ok(Some(connection))
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
