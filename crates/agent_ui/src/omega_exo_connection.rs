//! Driving Exo. `OMEGA-DELTA-0042`, omega#87, Tier A.
//!
//! The law lives in `crates/omega_exo_lane`, which is a leaf and can be checked
//! in a second. This is the half that needs a process and a thread: it runs the
//! `exo` binary, reads what it printed, and pushes the result into an
//! `AcpThread` so the existing agent panel renders it like any other lane.
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
//! 4. **Which agent.** `exo agent show` is read, and a turn is refused when the
//!    agent carries self-modification capability — runtime tool authoring, a
//!    tool module (which is how `guardian_action` is installed), or a
//!    read-write mount. Tier C is out of scope, and this is the enforcement of
//!    that rather than a promise about it.
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
use std::cell::RefCell;
use std::collections::HashMap;
use std::path::PathBuf;
use std::process::Stdio;
use std::rc::Rc;

use acp_thread::{AcpThread, AgentConnection};
use action_log::ActionLog;
use agent_client_protocol::schema::v1 as acp;
use anyhow::{Context as _, Result, anyhow, bail};
use gpui::{App, AppContext as _, Entity, SharedString, Task, WeakEntity};
use omega_exo_lane::{
    EXO_HARNESS_ID, EXO_PIN, ExoAgent, ExoCommand, ExoLaneIdentity, ExoModelBinding, ExoRoot,
    ExoTurn, LoopbackEndpoint, ObservedExoCheckout, admits_bytes,
};
use omega_harness::MeasuredDigest;
use project::{AgentId, Project};
use util::path_list::PathList;

/// How many events the lane reads back from Exo's durable log after a turn.
///
/// A bound rather than "everything": Exo's log is append-only and a long-lived
/// conversation accumulates without limit, and a lane that read all of it to
/// render one turn would get slower for the whole life of the conversation.
const TOOL_ACTIVITY_EVENT_LIMIT: u32 = 64;

/// Where the lane's configuration lives, under the Omega data directory.
const EXO_LANE_FILE: &str = "omega-exo-lane.json";

/// The schema that file carries.
const EXO_LANE_SCHEMA: &str = "openagents.omega.exo_lane.v1";

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
                log::warn!("OMEGA-DELTA-0042: the Exo lane file is not JSON ({error}); no Exo lane");
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
        Some(Self {
            binary: PathBuf::from(field("binary")?),
            checkout: PathBuf::from(field("checkout")?),
            root: ExoRoot::at(field("root")?),
            agent: field("agent")?,
            conversation: field("conversation")?,
        })
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
}

/// One Exo install, behind Omega Agent's router.
pub struct ExoHarnessConnection {
    driver: Rc<ExoDriver>,
    /// The threads this connection built, by session.
    sessions: RefCell<HashMap<acp::SessionId, WeakEntity<AcpThread>>>,
}

impl ExoHarnessConnection {
    /// A connection to the Exo the lane file names.
    #[must_use]
    pub fn new(config: ExoLaneConfig, frozen_digest: Option<String>) -> Self {
        Self {
            driver: Rc::new(ExoDriver {
                config,
                identity: RefCell::new(None),
                frozen_digest,
            }),
            sessions: RefCell::new(HashMap::new()),
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
            .stdin(Stdio::null())
            .output()
            .await
            .with_context(|| format!("running {} {}", self.config.binary.display(), argv.join(" ")))?;
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
    async fn check_pin(&self) -> Result<()> {
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
        EXO_PIN.admits(&observed).map_err(|mismatch| {
            anyhow!(
                "{mismatch}: the Exo lane is pinned to {} at {}",
                EXO_PIN.upstream,
                EXO_PIN.source_commit
            )
        })?;

        let bytes = smol::fs::read(&self.config.binary)
            .await
            .context("reading the exo binary")?;
        admits_bytes(self.frozen_digest.as_deref(), &MeasuredDigest::measure(&bytes))
            .map_err(|mismatch| anyhow!("{mismatch}"))?;
        Ok(())
    }

    /// Read the agent, refuse self-modification, and resolve the disclosure.
    async fn check_agent(&self) -> Result<ExoLaneIdentity> {
        let shown = self
            .run(&ExoCommand::ShowAgent {
                agent: self.config.agent.clone(),
            })
            .await?;
        let agent = ExoAgent::parse(&shown).map_err(|error| {
            anyhow!("{error}; the Omega lane refuses an Exo agent it cannot read")
        })?;
        agent
            .admits_lane_turn()
            .map_err(|refusal| anyhow!("{refusal}"))?;

        let bindings = ExoModelBinding::read_table(&self.run(&ExoCommand::ListModels).await?);
        let identity = ExoLaneIdentity::resolve(&agent, &bindings);
        *self.identity.borrow_mut() = Some(identity.clone());
        Ok(identity)
    }

    /// Everything one turn does outside GPUI: the three refusals, the send, and
    /// the read-back.
    async fn drive_turn(&self, prompt: String) -> Result<ExoTurn> {
        self.check_endpoint()?;
        self.check_pin().await?;
        self.check_agent().await?;
        let stdout = self
            .run(&ExoCommand::SendTurn {
                agent: self.config.agent.clone(),
                conversation: self.config.conversation.clone(),
                prompt,
            })
            .await?;
        let turn = ExoTurn::read(&stdout).map_err(|error| anyhow!("{error}"))?;
        // The durable log is read for its own sake even though the send output
        // already carried the tool lines: Exo's log is the record, the printed
        // lines are a rendering of it, and a lane that only ever read the
        // rendering could not tell a truncated turn from a complete one. Failure
        // to read it does not fail the turn, because the turn already ran.
        if let Err(error) = self
            .run(&ExoCommand::ReadEvents {
                agent: self.config.agent.clone(),
                conversation: self.config.conversation.clone(),
                limit: TOOL_ACTIVITY_EVENT_LIMIT,
            })
            .await
        {
            log::warn!("OMEGA-DELTA-0042: Exo's durable log could not be read back: {error:#}");
        }
        Ok(turn)
    }
}

/// The Exo lane the owner configured, if they configured one.
///
/// Called once, when the router is built. A machine with no lane file gets
/// `None` and the router registers no external executor — which is the ordinary
/// case, and is why the return type is an `Option` rather than a `Result` that
/// every caller would have to decide to ignore.
#[must_use]
pub fn connect_configured_lane(lane_path: &std::path::Path) -> Option<Rc<dyn AgentConnection>> {
    let config = ExoLaneConfig::load(lane_path)?;
    let frozen = frozen_exo_digest();
    log::info!(
        "OMEGA-DELTA-0042: Exo harness lane configured at {} ({} {}), pin {}",
        config.root.as_str(),
        config.agent,
        config.conversation,
        if frozen.is_some() { "frozen" } else { "unfrozen" }
    );
    Some(Rc::new(ExoHarnessConnection::new(config, frozen)))
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
        Ok(ledger) => ledger
            .pin(EXO_HARNESS_ID)
            .map(|pin| pin.digest.clone()),
        Err(error) => {
            log::warn!("OMEGA-DELTA-0042: the harness pin ledger could not be read ({error})");
            None
        }
    }
}

/// The prompt text a request carries, joined.
fn prompt_text(request: &acp::PromptRequest) -> String {
    request
        .prompt
        .iter()
        .filter_map(|block| match block {
            acp::ContentBlock::Text(text) => Some(text.text.as_str()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("\n")
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
        // The session is Exo's conversation. Tier A binds one Omega thread to
        // one Exo conversation rather than minting a new conversation per
        // thread, because minting one would mean Omega creating state inside
        // `.exo`, which is the thing this lane does not do.
        let session_id = acp::SessionId::new(format!(
            "exo/{}/{}",
            self.driver.config.agent, self.driver.config.conversation
        ));
        let action_log = cx.new(|_| ActionLog::new(project.clone()));
        let thread = cx.new(|cx| {
            AcpThread::new(
                None,
                Some(SharedString::from(format!(
                    "Exo · {}",
                    self.driver.config.conversation
                ))),
                Some(work_dirs),
                self.clone(),
                project,
                action_log,
                session_id.clone(),
                watch::Receiver::constant(
                    acp::PromptCapabilities::new().embedded_context(true),
                ),
                cx,
            )
        });
        self.sessions
            .borrow_mut()
            .insert(session_id, thread.downgrade());
        Task::ready(Ok(thread))
    }

    fn auth_methods(&self) -> &[acp::AuthMethod] {
        &[]
    }

    fn authenticate(&self, _method: acp::AuthMethodId, _cx: &mut App) -> Task<Result<()>> {
        Task::ready(Err(anyhow!(
            "Exo holds its own credentials; Omega never edits them"
        )))
    }

    /// One shot. No deltas, by Exo's limit at this pin — see
    /// `omega_exo_lane::turn`.
    fn prompt(&self, params: acp::PromptRequest, cx: &mut App) -> Task<Result<acp::PromptResponse>> {
        let Some(thread) = self.sessions.borrow().get(&params.session_id).cloned() else {
            return Task::ready(Err(anyhow!("no Exo thread for {}", params.session_id.0)));
        };
        let prompt = prompt_text(&params);
        // Cloned into the task rather than driven here: the turn must not run
        // on the thread that draws the window. See `run`.
        let driver = Rc::clone(&self.driver);
        cx.spawn(async move |cx| {
            let turn = driver.drive_turn(prompt).await?;
            thread.update(cx, |thread, cx| {
                for tool in &turn.tools {
                    let update = acp::SessionUpdate::AgentMessageChunk(acp::ContentChunk::new(
                        format!("`{}` → {}", tool.name, tool.output).into(),
                    ));
                    let _ = thread.handle_session_update(update, cx);
                }
                let _ = thread.handle_session_update(
                    acp::SessionUpdate::AgentMessageChunk(acp::ContentChunk::new(
                        turn.text.clone().into(),
                    )),
                    cx,
                );
            })?;
            Ok(acp::PromptResponse::new(acp::StopReason::EndTurn))
        })
    }

    /// Exo's turn is one blocking process. There is nothing to cancel that
    /// would leave Exo's durable log consistent, so the lane says so rather
    /// than pretending.
    fn cancel(&self, session_id: &acp::SessionId, _cx: &mut App) {
        log::info!(
            "OMEGA-DELTA-0042: cancel is not available on the Exo lane; \
             session {} runs one shot per turn",
            session_id.0
        );
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
        let config = ExoLaneConfig::load(std::path::Path::new(&lane_file))
            .expect("the lane file names an Exo install");

        // The turn runs a real process against a real model, so this test
        // waits on wall-clock I/O rather than on the deterministic scheduler.
        // That is the point of it: the same await that parks here is what keeps
        // the window drawing in production, and the workspace's clippy
        // configuration disallows the blocking call that would not park.
        cx.executor().allow_parking();
        crate::test_support::init_test(cx);
        let fs = fs::FakeFs::new(cx.executor());
        let project = Project::test(fs, [], cx).await;

        let connection: Rc<ExoHarnessConnection> =
            Rc::new(ExoHarnessConnection::new(config, None));
        let thread = cx
            .update(|cx| {
                connection
                    .clone()
                    .new_session(project, PathList::default(), cx)
            })
            .await
            .expect("a session on the Exo lane");

        let session_id = thread.read_with(cx, |thread, _| thread.session_id().clone());
        let marker = "OMEGA-EXO-TIER-A";
        let request = acp::PromptRequest::new(
            session_id,
            vec![acp::ContentBlock::Text(acp::TextContent::new(format!(
                "Reply with exactly the word {marker} and nothing else."
            )))],
        );
        let response = cx
            .update(|cx| AgentConnection::prompt(connection.as_ref(), request, cx))
            .await
            .expect("Exo ran the turn");
        assert_eq!(response.stop_reason, acp::StopReason::EndTurn);

        let rendered = thread.read_with(cx, |thread, cx| thread.to_markdown(cx));
        assert!(
            rendered.contains(marker),
            "the Exo reply did not reach the thread: {rendered}"
        );

        let disclosure = thread.read_with(cx, |thread, cx| thread.omega_executor_disclosure(cx));
        assert!(disclosure.is_coherent(), "{disclosure:?}");
        assert_eq!(disclosure.class, omega_exo_lane::EXO_EXECUTOR_CLASS);
        assert_eq!(disclosure.run_ref, None);
        let identity = connection.identity().expect("Exo told the lane who it is");
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

    /// The prompt reaches Exo as text and nothing else. A content block this
    /// build does not carry is dropped rather than rendered into the command
    /// line as a debug string.
    #[test]
    fn only_text_reaches_exos_command_line() {
        let request = acp::PromptRequest::new(
            acp::SessionId::new("exo/omega-lane/tier-a"),
            vec![
                acp::ContentBlock::Text(acp::TextContent::new(String::from("first"))),
                acp::ContentBlock::Text(acp::TextContent::new(String::from("second"))),
            ],
        );
        assert_eq!(prompt_text(&request), "first\nsecond");
    }
}
