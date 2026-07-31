//! A delegation from Omega, against the real `scv` process.
//!
//! OMEGA-DELTA-0209, omega#160. `bbd78b180b` made every agent Omega offers to
//! delegate to an agent Omega can *start*, and then the owner delegated three
//! tasks to SCV and got three failures — a started agent, a live session, and:
//!
//! ```text
//! The SCV subagent failed: Invalid params: {"code":"invalid_params",
//!  "message":"invalid tool request: expected value at line 1 column 1","path":""}
//! ```
//!
//! Starting was never the whole of it. SCV has no model: its prompt must *be* a
//! JSON tool request, Omega sent prose to every ACP agent uniformly, and
//! nothing held the two facts together. Unit tests could not catch it because
//! each side was correct alone — `scv` parsed JSON correctly and Omega sent
//! prose correctly, and the defect lived only in the seam.
//!
//! So this test is the seam, end to end, with nothing faked in the middle:
//!
//! - the contract comes from `omega_agent_detect::CANDIDATES`, the same catalog
//!   the delegation is offered from and the same one the delegating model is
//!   shown;
//! - the task is shaped by `shape_delegated_task`, the function the delegate
//!   tool calls;
//! - it is sent over real ACP JSON-RPC on stdio to the real `scv` binary this
//!   package builds, started the way `AgentLaunch::DetectedBinary` starts it;
//! - and the assertion is on the **content that came back**, not on the process
//!   having started.
//!
//! What is deliberately *not* mocked is the binary. `CARGO_BIN_EXE_scv` is the
//! executable cargo built for this test, so a change to SCV's parsing is
//! exercised as shipped rather than as linked.

use std::io::{BufRead as _, BufReader, Write as _};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};

use omega_agent_detect::{AgentPromptContract, StructuredPrompt};

/// SCV's contract, read from the catalog Omega delegates from.
fn scv_contract() -> StructuredPrompt {
    match omega_agent_detect::prompt_contract_for("scv")
        .expect("SCV is a delegation target Omega offers")
    {
        AgentPromptContract::Structured(contract) => contract,
        AgentPromptContract::Prose => {
            panic!("SCV has no model, so a task it can answer is not prose")
        }
    }
}

/// The one place the two spellings of SCV's request could drift apart.
///
/// `omega_agent_detect` is a leaf and `scv` is hyper-lightweight, so neither
/// depends on the other and each holds the string. What the model is told to
/// send must be what SCV accepts.
#[test]
fn the_shape_omega_advertises_is_the_shape_scv_documents() {
    assert_eq!(
        scv_contract().request,
        scv::PROMPT_REQUEST_SHAPE,
        "the request Omega tells a delegating model to emit has drifted from \
         the one SCV accepts"
    );
    assert_eq!(
        scv_contract().tools,
        &[scv::READ_TOOL_NAME],
        "the catalog names a tool SCV does not serve, or omits one it does"
    );
}

/// A live ACP session on the real binary.
struct ScvProcess {
    child: Child,
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
}

impl ScvProcess {
    /// `std::process` deliberately, and blocking deliberately.
    ///
    /// The lint that disallows these exists because spawning and piping can
    /// block the calling thread for an unknown duration, which matters on a
    /// thread that has a window to draw. This is a test binary whose entire
    /// purpose is to block on a child process's stdio and read what it wrote;
    /// there is no executor here to hand the wait to, and an async transport
    /// would put a runtime between the test and the bytes it is asserting on.
    #[expect(
        clippy::disallowed_methods,
        reason = "an integration test drives a real child process on stdio and \
                  has no thread to keep responsive"
    )]
    fn start(read_root: &std::path::Path) -> Self {
        let mut child = Command::new(env!("CARGO_BIN_EXE_scv"))
            .arg("--read-root")
            .arg(read_root)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .expect("the scv binary this package builds must start");
        let stdin = child.stdin.take().expect("stdin");
        let stdout = BufReader::new(child.stdout.take().expect("stdout"));
        Self {
            child,
            stdin,
            stdout,
        }
    }

    fn send(&mut self, message: &serde_json::Value) {
        writeln!(self.stdin, "{message}").expect("the request reaches scv");
        self.stdin.flush().expect("the request is flushed");
    }

    /// The next JSON-RPC message, whatever kind it is.
    fn receive(&mut self) -> serde_json::Value {
        let mut line = String::new();
        let read = self.stdout.read_line(&mut line).expect("scv answers");
        assert!(read > 0, "scv closed stdout without answering");
        serde_json::from_str(&line).unwrap_or_else(|error| panic!("{error}: {line}"))
    }

    /// The next message carrying this id, skipping notifications.
    fn response(&mut self, id: u64) -> serde_json::Value {
        loop {
            let message = self.receive();
            if message.get("id").and_then(serde_json::Value::as_u64) == Some(id) {
                return message;
            }
        }
    }

    /// Initialize and open a session, as a delegating Omega does.
    fn open_session(&mut self, cwd: &std::path::Path) -> String {
        self.send(&serde_json::json!({
            "jsonrpc": "2.0", "id": 0, "method": "initialize",
            "params": {"protocolVersion": 1, "clientCapabilities": {}},
        }));
        let initialize = self.response(0);
        assert_eq!(
            initialize["result"]["agentInfo"]["name"], "scv",
            "{initialize}"
        );

        self.send(&serde_json::json!({
            "jsonrpc": "2.0", "id": 1, "method": "session/new",
            "params": {"cwd": cwd, "mcpServers": []},
        }));
        self.response(1)["result"]["sessionId"]
            .as_str()
            .expect("a session id")
            .to_owned()
    }

    /// One delegated turn: the id, and the result or error it answered with.
    fn prompt(&mut self, id: u64, session: &str, task: &str) -> serde_json::Value {
        self.send(&serde_json::json!({
            "jsonrpc": "2.0", "id": id, "method": "session/prompt",
            "params": {
                "sessionId": session,
                "prompt": [{"type": "text", "text": task}],
            },
        }));
        self.response(id)
    }
}

impl Drop for ScvProcess {
    fn drop(&mut self) {
        self.child.kill().ok();
        self.child.wait().ok();
    }
}

fn fixture() -> (tempfile::TempDir, std::path::PathBuf) {
    let directory = tempfile::tempdir().expect("a directory scv may read");
    let file = directory.path().join("hangar.txt");
    std::fs::write(&file, "mineral line one\nvespene line two\n").expect("the fixture is written");
    (directory, file)
}

/// The owner's task, delegated for real, coming back with real content.
///
/// This is the assertion the defect would have failed: not that `scv` started,
/// not that a session opened, but that a task Omega shaped and sent produced
/// the bytes of the file it named.
#[test]
fn a_delegated_read_returns_the_file_it_asked_for() {
    let (directory, file) = fixture();
    let mut scv = ScvProcess::start(directory.path());
    let session = scv.open_session(directory.path());

    // What the delegating model emits, given the shape the tool description
    // shows it. It goes through the same shaping the delegate tool applies.
    let authored = format!(
        r#"{{"tool":"read","arguments":{{"path":"{}","offset":1,"limit":2000}}}}"#,
        file.display()
    );
    let task = omega_agent_detect::shape_delegated_task("SCV", &scv_contract(), &authored)
        .expect("a request in the advertised shape is sendable");

    let answer = scv.prompt(2, &session, &task);

    assert!(
        answer.get("error").is_none(),
        "the delegated task must not come back as a failure: {answer}"
    );
    assert_eq!(answer["result"]["stopReason"], "end_turn", "{answer}");
}

/// The same turn, read through the tool-call updates SCV emits.
///
/// Separated from the response assertion because the content is what a parent
/// agent is actually handed: `raw_output` is the read result, and an empty one
/// would be a successful turn that answered nothing.
#[test]
fn a_delegated_read_emits_the_content_as_a_completed_tool_call() {
    let (directory, file) = fixture();
    let mut scv = ScvProcess::start(directory.path());
    let session = scv.open_session(directory.path());

    let task = omega_agent_detect::shape_delegated_task(
        "SCV",
        &scv_contract(),
        &format!(
            r#"{{"tool":"read","arguments":{{"path":"{}","offset":2,"limit":1}}}}"#,
            file.display()
        ),
    )
    .expect("a request in the advertised shape is sendable");

    scv.send(&serde_json::json!({
        "jsonrpc": "2.0", "id": 2, "method": "session/prompt",
        "params": {"sessionId": session, "prompt": [{"type": "text", "text": task}]},
    }));

    let mut completed = None;
    loop {
        let message = scv.receive();
        if message.get("id").and_then(serde_json::Value::as_u64) == Some(2) {
            assert!(message.get("error").is_none(), "{message}");
            break;
        }
        let update = &message["params"]["update"];
        if update["sessionUpdate"] == "tool_call_update" {
            completed = Some(update.clone());
        }
    }

    let completed = completed.expect("the turn must report the tool call it ran");
    assert_eq!(completed["status"], "completed", "{completed}");
    assert_eq!(
        completed["rawOutput"]["content"], "     2\tvespene line two",
        "the delegated read must return the file's own bytes: {completed}"
    );
    assert_eq!(completed["rawOutput"]["line_start"], 2, "{completed}");
}

/// A model that fenced its JSON is still answered.
#[test]
fn a_fenced_request_is_delegated_unchanged() {
    let (directory, file) = fixture();
    let mut scv = ScvProcess::start(directory.path());
    let session = scv.open_session(directory.path());

    let task = omega_agent_detect::shape_delegated_task(
        "SCV",
        &scv_contract(),
        &format!(
            "```json\n{{\"tool\":\"read\",\"arguments\":{{\"path\":\"{}\"}}}}\n```",
            file.display()
        ),
    )
    .expect("a fence is wrapping, not meaning");

    let answer = scv.prompt(2, &session, &task);
    assert!(answer.get("error").is_none(), "{answer}");
}

/// The owner's exact prompt, refused before it ever reaches the process.
///
/// The delegate tool shapes the task first, so this prose never becomes a
/// `session/prompt` at all — and the parent is told the shape rather than a
/// parse position.
#[test]
fn the_owners_prose_is_refused_with_the_shape_and_never_sent() {
    let refusal = omega_agent_detect::shape_delegated_task(
        "SCV",
        &scv_contract(),
        "Perform a read-only test delegation: report the project root path and \
         list one or two top-level entries. Do not modify files.",
    )
    .expect_err("SCV has no model and this is prose");

    assert!(refusal.contains(scv::PROMPT_REQUEST_SHAPE), "{refusal}");
    assert!(
        !refusal.contains("expected value at line 1 column 1"),
        "the refusal must name what SCV wanted, not where its parser stopped: \
         {refusal}"
    );
}

/// And if prose does reach SCV — from a client that is not Omega — SCV itself
/// now says what it wanted.
#[test]
fn prose_that_reaches_scv_is_answered_with_the_shape() {
    let (directory, _file) = fixture();
    let mut scv = ScvProcess::start(directory.path());
    let session = scv.open_session(directory.path());

    let answer = scv.prompt(2, &session, "please read something for me");

    let message = answer["error"]["data"]["message"]
        .as_str()
        .unwrap_or_else(|| panic!("{answer}"));
    assert!(message.contains(scv::PROMPT_REQUEST_SHAPE), "{message}");
}

/// What SCV advertises must be what the catalog says it serves.
///
/// The catalog's declaration is what the delegating model is shown. If SCV
/// grew a second tool, or dropped `read`, a model would be told a contract the
/// live agent does not have.
#[test]
fn the_advertised_commands_are_the_catalogs_tools() {
    let (directory, _file) = fixture();
    let mut scv = ScvProcess::start(directory.path());

    scv.send(&serde_json::json!({
        "jsonrpc": "2.0", "id": 0, "method": "initialize",
        "params": {"protocolVersion": 1, "clientCapabilities": {}},
    }));
    scv.response(0);
    scv.send(&serde_json::json!({
        "jsonrpc": "2.0", "id": 1, "method": "session/new",
        "params": {"cwd": directory.path(), "mcpServers": []},
    }));
    scv.response(1);

    let advertised = loop {
        let message = scv.receive();
        if message["params"]["update"]["sessionUpdate"] == "available_commands_update" {
            break message["params"]["update"]["availableCommands"].clone();
        }
    };

    let names: Vec<String> = advertised
        .as_array()
        .expect("a command list")
        .iter()
        .map(|command| command["name"].as_str().expect("a name").to_owned())
        .collect();
    assert_eq!(names, scv_contract().tools, "{advertised}");
    assert_eq!(
        advertised[0]["input"]["hint"],
        scv::PROMPT_REQUEST_SHAPE,
        "the live agent's own hint must be the request the catalog advertises"
    );
}
