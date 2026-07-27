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

use std::path::Path;
use std::rc::Rc;
use std::sync::Arc;
use std::task::Poll;
use std::time::Duration;

use acp_thread::AgentConnection as _;
use agent_client_protocol::schema::v1 as acp;
use agent_servers::{AgentServer, AgentServerDelegate, CustomAgentServer};
use fs::FakeFs;
use gpui::{AppContext as _, Entity, TestAppContext};
use project::{
    Project,
    agent_server_store::{
        AgentId, AgentServerCommand, AllAgentServersSettings, CustomAgentServerSettings,
    },
};
use settings::Settings as _;
use util::path_list::PathList;

use crate::{
    AgentTool as _, ExternalAcpSubagentHandle, ExternalSubagentSessions, NativeAgent,
    NativeAgentConnection, NativeThreadEnvironment, ReadSubagentTranscriptTool,
    ReadSubagentTranscriptToolInput, SpawnAgentTool, SpawnAgentToolInput, SpawnAgentToolOutput,
    SubagentHandle, Templates, ThreadStore, ToolCallEventStream, ToolInput,
};

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

// ---------------------------------------------------------------------------
// omega#109. The same thing, driven through the tool.
//
// Everything above proves the *handle*. It opens the session itself, with the
// same three calls `create_external_acp_subagent` makes, which is why it proved
// the handle and not the route into it: a test that re-states a sequence cannot
// fail when the caller of that sequence is wrong, or absent, or refuses first.
// `NativeThreadEnvironment::create_external_acp_subagent` and `spawn_agent`'s
// branch into it had therefore never executed at all.
//
// So these run `SpawnAgentTool` — the real tool, the one the model calls —
// against a real `NativeThreadEnvironment` built on a real parent thread, and
// let it decide, connect, prompt and report on its own. Nothing below names
// `ExternalAcpSubagentHandle` or `CustomAgentServer`; if the tool stops routing
// an executor to an external agent, there is no second path here to hide it.
// ---------------------------------------------------------------------------

/// A real parent thread, registered the way production registers one.
///
/// `NativeAgentConnection::new_session` is the path the panel uses, and it is
/// what calls `register_session` — which is what installs the tool set and
/// builds the `NativeThreadEnvironment` that production hands to `spawn_agent`.
/// Constructing a `Thread` directly would skip both.
async fn parent_thread(cx: &mut TestAppContext, cwd: &Path) -> ParentThread {
    let project = Project::example([cwd], &mut cx.to_async()).await;
    let thread_store = cx.new(|cx| ThreadStore::new(cx));
    let fs = FakeFs::new(cx.executor());
    let agent = cx.update(|cx| NativeAgent::new(thread_store, Templates::new(), fs, cx));

    let acp_thread = cx
        .update(|cx| {
            Rc::new(NativeAgentConnection(agent.clone())).new_session(
                project.clone(),
                PathList::new(&[cwd]),
                cx,
            )
        })
        .await
        .expect("the parent session must open");
    let session_id = acp_thread.read_with(cx, |thread, _| thread.session_id().clone());
    let thread = agent.read_with(cx, |agent, _| {
        agent
            .sessions
            .get(&session_id)
            .expect("the parent session must be registered")
            .thread
            .clone()
    });

    // The same three weak handles `register_session` builds the production
    // environment from, in the same order.
    let environment = Rc::new(NativeThreadEnvironment {
        agent: agent.downgrade(),
        thread: thread.downgrade(),
        acp_thread: acp_thread.downgrade(),
    });

    ParentThread {
        environment,
        _project: project,
        _agent: agent,
        _thread: thread,
        _acp_thread: acp_thread,
        _session_id: session_id,
    }
}

/// The parent, held for the length of a test.
///
/// Every field but the environment is here only to be alive. The environment
/// holds weak references — it is the production one, and production keeps these
/// alive elsewhere — so dropping them early would make the tool report "Parent
/// thread no longer exists" instead of spawning anything. They are dropped at
/// the end of the test rather than leaked, because GPUI's test harness fails a
/// test that exits holding entities and it is right to.
struct ParentThread {
    environment: Rc<NativeThreadEnvironment>,
    _project: Entity<Project>,
    _agent: Entity<NativeAgent>,
    _thread: Entity<crate::Thread>,
    _acp_thread: Entity<acp_thread::AcpThread>,
    _session_id: acp::SessionId,
}

/// Ask `spawn_agent` for one subagent on `executor`, and run it.
fn spawn_through_the_tool(
    environment: &Rc<NativeThreadEnvironment>,
    executor: &str,
    message: &str,
    cx: &mut TestAppContext,
) -> gpui::Task<Result<SpawnAgentToolOutput, SpawnAgentToolOutput>> {
    let input = SpawnAgentToolInput {
        label: format!("{executor} subagent"),
        message: message.to_owned(),
        session_id: None,
        executor: Some(executor.to_owned()),
    };
    // One event stream per tool call, which is what a real turn builds.
    let (event_stream, _events) = ToolCallEventStream::test();
    let tool = Arc::new(SpawnAgentTool::new(
        environment.clone() as Rc<dyn crate::ThreadEnvironment>
    ));
    // The receiver is dropped: `subagent_spawned` and the meta updates are sent
    // on an unbounded channel, so a dropped receiver does not block the tool.
    cx.update(|cx| tool.run(ToolInput::resolved(input), event_stream, cx))
}

fn succeeded(
    result: Result<SpawnAgentToolOutput, SpawnAgentToolOutput>,
) -> (String, String, acp::SessionId) {
    match result {
        Ok(SpawnAgentToolOutput::Success {
            session_id,
            output,
            executor,
            ..
        }) => {
            let executor = executor.expect(
                "a successful spawn must report what ran it; `None` here is the \
                 undisclosed case, and an external subagent is never that",
            );
            assert_eq!(
                executor.class, "external_acp",
                "an external subagent must not report as Omega's own loop"
            );
            (output, executor.agent_id, session_id)
        }
        Ok(SpawnAgentToolOutput::Error { .. }) => unreachable!(),
        Err(SpawnAgentToolOutput::Error {
            error, executor, ..
        }) => panic!(
            "the spawn failed: {error}\nreported executor: {executor:?}\n\
             A refusal here means the tool never reached the agent — check that \
             `codex` and `claude` are on this machine's PATH, which is what \
             `resolve_requested_executor` reads."
        ),
        Err(SpawnAgentToolOutput::Success { .. }) => unreachable!(),
    }
}

/// Acceptance 1 and 2. One turn, three subagents, through the tool.
///
/// Two Codex and one Claude, spawned concurrently by three `spawn_agent` calls
/// — which is what one assistant turn issuing three tool calls does, since
/// `Thread` drives its tool calls on a `FuturesUnordered` and each gets its own
/// event stream. Each comes back with its own answer and its own executor
/// record, so the parent can say which agent said which thing.
///
/// This is the check that fails if the tool's route into
/// `create_external_acp_subagent` is removed: `SubagentExecutor::ExternalAcp`
/// would fall to the inherited branch, the subagent would run on the parent's
/// own model, and `executor.class` would read `native_loop` — on a parent with
/// no model configured, it would not run at all.
#[gpui::test]
#[cfg_attr(not(feature = "e2e"), ignore)]
async fn one_turn_spawns_two_codex_and_one_claude_through_the_tool(cx: &mut TestAppContext) {
    init(cx).await;
    let tempdir = tempfile::tempdir().expect("a working directory for the agents");
    let parent = parent_thread(cx, tempdir.path()).await;

    let first = spawn_through_the_tool(
        &parent.environment,
        "codex-acp",
        "Reply with exactly OMEGA109-A and nothing else. Use no tools.",
        cx,
    );
    let second = spawn_through_the_tool(
        &parent.environment,
        "codex-acp",
        "Reply with exactly OMEGA109-B and nothing else. Use no tools.",
        cx,
    );
    let third = spawn_through_the_tool(
        &parent.environment,
        "claude-acp",
        "Reply with exactly OMEGA109-C and nothing else. Use no tools.",
        cx,
    );

    let (a, b, c) = futures::future::join3(first, second, third).await;
    let (a_output, a_executor, a_session) = succeeded(a);
    let (b_output, b_executor, b_session) = succeeded(b);
    let (c_output, c_executor, c_session) = succeeded(c);

    assert!(a_output.contains("OMEGA109-A"), "{a_output:?}");
    assert!(b_output.contains("OMEGA109-B"), "{b_output:?}");
    assert!(c_output.contains("OMEGA109-C"), "{c_output:?}");

    // Acceptance 2. Each result names the executor that produced it, and the
    // Claude one is not the Codex one.
    assert_eq!(a_executor, "codex-acp");
    assert_eq!(b_executor, "codex-acp");
    assert_eq!(c_executor, "claude-acp");

    // Three sessions, kept apart.
    assert_ne!(a_session, b_session);
    assert_ne!(a_session, c_session);
    assert_ne!(b_session, c_session);

    std::mem::forget(tempdir);
    cx.run_until_parked();
}

/// What the panel has to be able to do, at the seam where it has to do it.
///
/// The subagent card resolves its thread from the **native** connection's
/// session map, and an external subagent is not in it — there is no native
/// `Thread` behind it and `NativeAgent::sessions` never learns of it. All the
/// panel receives is `AcpThreadEvent::SubagentSpawned(session_id)`, so unless a
/// session id can be turned back into the running `AcpThread`, the card has
/// nothing to render and the executor has no name.
///
/// So this asks the question the panel asks, in the order the panel asks it:
/// while the turn is still running, is there a thread under this id? The timing
/// is the point. The subagent's lifetime belongs to the tool call — the handle
/// drops the connection, and therefore the child process, when the call ends —
/// so a reader that only looks afterwards finds nothing however correct its
/// lookup is. The panel looks on `SubagentSpawned`, which is emitted the moment
/// the session opens, and takes a strong reference then. Everything it shows
/// afterwards it shows because it did.
#[gpui::test]
#[cfg_attr(not(feature = "e2e"), ignore)]
async fn an_external_subagent_is_findable_by_session_id_while_it_runs(cx: &mut TestAppContext) {
    init(cx).await;
    let tempdir = tempfile::tempdir().expect("a working directory for the agent");
    let parent = parent_thread(cx, tempdir.path()).await;

    let mut running = spawn_through_the_tool(
        &parent.environment,
        "codex-acp",
        "Reply with exactly OMEGA109-E and nothing else. Use no tools.",
        cx,
    );

    // The strong reference a `ThreadView` would hold. Taken during the turn,
    // exactly as `load_subagent_session` takes it.
    let mut observed: Option<Entity<acp_thread::AcpThread>> = None;
    let outcome = loop {
        if observed.is_none() {
            observed = cx.update(|cx| {
                let session_id = cx
                    .try_global::<ExternalSubagentSessions>()?
                    .live_session_ids()
                    .into_iter()
                    .next()?;
                crate::external_subagent_thread(&session_id, cx)
            });
        }
        if let Poll::Ready(outcome) = futures::poll!(&mut running) {
            break outcome;
        }
        cx.executor().timer(Duration::from_millis(25)).await;
    };

    let (_output, executor, session_id) = succeeded(outcome);
    assert_eq!(executor, "codex-acp");
    // The answer itself is not asserted here on purpose. What this test is
    // about is whether the session could be found while it was open, and a live
    // agent that answers "selected model is at capacity" has still opened a
    // session and still run a turn. The fan-out above is where the content has
    // to be right; making this one depend on it too would only add a second
    // place for the same upstream outage to fail.

    let observed = observed.expect(
        "the subagent ran and finished without ever being findable by its \
         session id. This is the panel's whole problem: it is handed an id and \
         has no way to reach the thread, so the card renders no transcript and \
         names no executor.",
    );

    // It is the right thread, and it still holds what the subagent did — which
    // is what the card shows after the turn, when the agent server is gone.
    cx.update(|cx| {
        let thread = observed.read(cx);
        assert_eq!(*thread.session_id(), session_id);
        assert!(
            thread.entries().len() >= 2,
            "the resolved thread must hold the prompt and the answer, not an \
             empty shell: {} entries",
            thread.entries().len()
        );
    });

    std::mem::forget(tempdir);
    cx.run_until_parked();
}

/// Acceptance 4. Reading an external subagent's transcript.
///
/// It refuses, and the refusal is the sentence already written for this case —
/// not a generic "not found", which would send the parent looking for a typo in
/// an ID that is correct. Omega genuinely cannot read it: the transcript lives
/// in the agent server's own process, and the parent has the final message.
///
/// Driven through `read_subagent_transcript` rather than through the
/// environment directly, because the sentence has to survive the tool's own
/// error arm to reach the model.
#[gpui::test]
#[cfg_attr(not(feature = "e2e"), ignore)]
async fn reading_an_external_subagent_transcript_refuses_with_its_own_reason(
    cx: &mut TestAppContext,
) {
    init(cx).await;
    let tempdir = tempfile::tempdir().expect("a working directory for the agent");
    let parent = parent_thread(cx, tempdir.path()).await;

    let spawned = spawn_through_the_tool(
        &parent.environment,
        "codex-acp",
        "Reply with exactly OMEGA109-D and nothing else. Use no tools.",
        cx,
    )
    .await;
    let (_output, _executor, session_id) = succeeded(spawned);

    let (event_stream, _events) = ToolCallEventStream::test();
    let tool = Arc::new(ReadSubagentTranscriptTool::new(
        parent.environment.clone() as Rc<dyn crate::ThreadEnvironment>
    ));
    let result = cx
        .update(|cx| {
            tool.run(
                ToolInput::resolved(ReadSubagentTranscriptToolInput {
                    session_id: session_id.clone(),
                    offset: None,
                    limit: None,
                    detail: Default::default(),
                }),
                event_stream,
                cx,
            )
        })
        .await;

    let reason = match result {
        Err(crate::ReadSubagentTranscriptToolOutput::Refused {
            session_id: refused,
            reason,
        }) => {
            assert_eq!(
                refused, session_id,
                "the refusal must name the session asked for"
            );
            reason
        }
        Ok(crate::ReadSubagentTranscriptToolOutput::Transcript { rendered, .. }) => panic!(
            "Omega returned a transcript for a session that belongs to another \
             agent's process. Either the session became readable, in which case \
             this test should assert the transcript, or something invented one: \
             {rendered}"
        ),
        other => panic!("unexpected outcome: {other:?}"),
    };

    // The exact sentence, not a paraphrase of it.
    assert_eq!(reason, crate::no_transcript_available(&session_id));
    assert!(
        reason.contains("its transcript belongs to that agent and Omega cannot read it"),
        "{reason}"
    );

    std::mem::forget(tempdir);
    cx.run_until_parked();
}
