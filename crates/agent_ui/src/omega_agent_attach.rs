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
            "{} is installed at {}, but Omega's agent-server store is gone, so \
             `{}` cannot be attached",
            agent.name,
            agent.binary.display(),
            agent.id
        )
    })?;

    await_registration(agent, &store, cx).await?;

    let server = CustomAgentServer::new(AgentId::new(agent.id));
    let delegate = AgentServerDelegate::new(store, None, None);
    let connect = cx.update(|cx| server.connect(delegate, project, cx));
    let connection = connect.await.with_context(|| {
        format!(
            "{} is installed at {}, but its ACP server `{}` did not start",
            agent.name,
            agent.binary.display(),
            agent.id
        )
    })?;
    Ok(Some(connection))
}

/// Wait, boundedly, for the chosen agent's ACP server to register.
///
/// # Errors
///
/// Returns an error naming the agent when it has not registered within
/// [`REGISTRATION_ATTEMPTS`].
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
        "{} is installed at {}, but Omega registered no ACP server under `{}`. \
         The agent is present and Omega cannot drive it, so the turn is not \
         quietly moved to the native loop.",
        agent.name,
        agent.binary.display(),
        agent.id
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
