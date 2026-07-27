//! `ExternalAcpSubagentHandle`, against a real Codex or Claude binary.
//!
//! omega#102's second gap. Everything else about per-spawn executors is decided
//! by pure functions and checked without a process: the resolution law, the
//! refusals, the disclosure records, the shape of the result. The handle is not
//! like that. It opens a real ACP session against a real agent server, sends a
//! turn, reads a stream it does not control, maps a stop reason, and tears the
//! child process down when it drops. None of that has an opinion until it runs.
//!
//! **A window is not required for any of it.** An external subagent is a child
//! process on stdio; the parent reading its result is another agent, not a
//! person. So this is `#[gpui::test]` behind the `e2e` feature, next to the
//! model-backed tests that already live in this crate:
//!
//! ```sh
//! cargo test -p agent --features e2e --lib external_acp_subagent -- --nocapture
//! ```
//!
//! What it is *not* is a rendered subagent card in the agent panel. That is a
//! window, it is `crates/agent_ui`, and it is named as unproven in
//! `OMEGA-DELTA-0101` rather than implied by a green test here.
//!
//! The agent is registered as a **custom** command rather than a registry
//! entry, so the test needs no ACP registry fetch and no settings the machine
//! happens to have — it launches the same published adapter the registry
//! resolves to, through the same `CustomAgentServer` → `connect` →
//! `new_session` path production uses.

use std::rc::Rc;

use agent_servers::{AgentServer, AgentServerDelegate, CustomAgentServer};
use gpui::{Entity, TestAppContext};
use project::{
    Project,
    agent_server_store::{
        AgentId, AgentServerCommand, AllAgentServersSettings, CustomAgentServerSettings,
    },
};
use settings::Settings as _;
use util::path_list::PathList;

use crate::{ExternalAcpSubagentHandle, SubagentHandle};

/// The published adapters, pinned to the versions the shipped ACP registry
/// resolves `codex-acp` and `claude-acp` to.
const CODEX_PACKAGE: &str = "@agentclientprotocol/codex-acp@1.1.7";
const CLAUDE_PACKAGE: &str = "@agentclientprotocol/claude-agent-acp@0.62.0";

fn custom(package: &str) -> CustomAgentServerSettings {
    CustomAgentServerSettings::Custom {
        command: AgentServerCommand {
            path: "npx".into(),
            args: vec!["-y".to_owned(), package.to_owned()],
            env: None,
        },
        default_mode: None,
        default_config_options: Default::default(),
        favorite_config_option_values: Default::default(),
    }
}

/// Register both adapters, once for the whole test.
///
/// Once, because `init_test` installs globals: calling it per subagent would
/// reset the settings the previous one is running under, and the fan-out needs
/// all three alive at the same time.
async fn init(cx: &mut TestAppContext) {
    agent_servers::e2e_tests::init_test(cx).await;
    cx.update(|cx| {
        let mut settings = AllAgentServersSettings::default();
        settings.insert("codex-acp".to_owned(), custom(CODEX_PACKAGE));
        settings.insert("claude-acp".to_owned(), custom(CLAUDE_PACKAGE));
        AllAgentServersSettings::override_global(settings, cx);
    });
}

/// Open a real external ACP session and wrap it as a subagent handle.
async fn external_subagent(
    agent_id: &str,
    agent_name: &str,
    cx: &mut TestAppContext,
) -> Rc<dyn SubagentHandle> {
    let tempdir = tempfile::tempdir().expect("a working directory for the agent");
    let cwd = tempdir.path().to_path_buf();

    let project = Project::example([cwd.as_path()], &mut cx.to_async()).await;
    let store = project.read_with(cx, |project, _| project.agent_server_store().clone());
    let delegate = AgentServerDelegate::new(store, None, None);
    let server = CustomAgentServer::new(AgentId::new(agent_id.to_owned()));

    let connection = cx
        .update(|cx| server.connect(delegate, project.clone(), cx))
        .await
        .expect("the external agent server must start");

    let acp_thread: Entity<acp_thread::AcpThread> = cx
        .update(|cx| {
            connection
                .clone()
                .new_session(project.clone(), PathList::new(&[cwd.as_path()]), cx)
        })
        .await
        .expect("the external agent must open a session");

    let session_id = acp_thread.read_with(cx, |thread, _| thread.session_id().clone());

    // The directory has to outlive the agent that is working in it.
    std::mem::forget(tempdir);

    Rc::new(ExternalAcpSubagentHandle::new(
        session_id,
        acp_thread,
        agent_id.to_owned(),
        agent_name.to_owned(),
        connection,
    ))
}

/// One external subagent, driven end to end against the real binary.
///
/// Live: session creation, the prompt, the stream, the stop reason, the final
/// message, and the disclosure — asked of the handle after the turn, so it is
/// the record of something that actually happened.
#[gpui::test]
#[cfg_attr(not(feature = "e2e"), ignore)]
async fn a_codex_subagent_runs_a_real_turn_and_discloses_itself(cx: &mut TestAppContext) {
    init(cx).await;
    let subagent = external_subagent("codex-acp", "Codex", cx).await;

    let answer = subagent
        .send(
            "Reply with exactly the word OMEGA102 and nothing else. Use no tools.".to_owned(),
            &cx.to_async(),
        )
        .await
        .expect("the Codex subagent must complete its turn");

    assert!(
        answer.contains("OMEGA102"),
        "the final message must be what the agent said, not a placeholder: {answer:?}"
    );
    assert!(
        !answer.contains("finished its turn without producing a final message"),
        "the sentinel means the stream was read wrong, not that the agent was \
         quiet: {answer:?}"
    );

    let (disclosure, entries) =
        cx.update(|cx| (subagent.executor_disclosure(cx), subagent.num_entries(cx)));
    assert_eq!(
        disclosure.class,
        omega_front_door::ExecutorClass::ExternalAcp
    );
    assert_eq!(disclosure.agent_id, "codex-acp");
    // Not disclosed, because the agent does not say. Live, this is the value
    // that would be fabricated if anything guessed.
    assert_eq!(disclosure.provider, None);
    assert_eq!(disclosure.model, None);
    assert!(disclosure.is_coherent(), "{disclosure:?}");
    assert!(
        entries >= 2,
        "the session must hold the prompt and the answer, not just one of them"
    );

    // Teardown. Dropping the handle drops the connection, which drops the child
    // process — the reason `_connection` is held for the life of the subagent.
    drop(subagent);
    cx.run_until_parked();
}

/// Two Codex and one Claude, in one fan-out, concurrently.
///
/// Acceptance 1, 2 and 3 against real binaries: the three turns are in flight
/// together, the three results come back apart, and each carries the record of
/// the executor that produced it.
#[gpui::test]
#[cfg_attr(not(feature = "e2e"), ignore)]
async fn a_mixed_fan_out_runs_concurrently_against_real_agents(cx: &mut TestAppContext) {
    init(cx).await;
    let first = external_subagent("codex-acp", "Codex", cx).await;
    let second = external_subagent("codex-acp", "Codex", cx).await;
    let third = external_subagent("claude-acp", "Claude", cx).await;

    let async_cx = cx.to_async();
    let sends = futures::future::join3(
        first.send(
            "Reply with exactly OMEGA102-A and nothing else. Use no tools.".to_owned(),
            &async_cx,
        ),
        second.send(
            "Reply with exactly OMEGA102-B and nothing else. Use no tools.".to_owned(),
            &async_cx,
        ),
        third.send(
            "Reply with exactly OMEGA102-C and nothing else. Use no tools.".to_owned(),
            &async_cx,
        ),
    );
    let (a, b, c) = sends.await;

    let a = a.expect("the first Codex subagent must complete");
    let b = b.expect("the second Codex subagent must complete");
    let c = c.expect("the Claude subagent must complete");

    assert!(a.contains("OMEGA102-A"), "{a:?}");
    assert!(b.contains("OMEGA102-B"), "{b:?}");
    assert!(c.contains("OMEGA102-C"), "{c:?}");

    // Three sessions, kept apart.
    assert_ne!(first.id(), second.id());
    assert_ne!(first.id(), third.id());

    // And each result is attributable to what produced it.
    let (first_disclosure, third_disclosure) =
        cx.update(|cx| (first.executor_disclosure(cx), third.executor_disclosure(cx)));
    assert_eq!(first_disclosure.agent_id, "codex-acp");
    assert_eq!(third_disclosure.agent_id, "claude-acp");
    assert_ne!(first_disclosure, third_disclosure);

    drop((first, second, third));
    cx.run_until_parked();
}
