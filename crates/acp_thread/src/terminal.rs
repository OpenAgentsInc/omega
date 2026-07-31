use agent_client_protocol::schema::v1 as acp;
use anyhow::Result;
use collections::HashMap;
use futures::{FutureExt as _, future::Shared};
use gpui::{App, AppContext, AsyncApp, Context, Entity, Task};
use http_proxy::Allowlist;
use language::LanguageRegistry;
use markdown::Markdown;
use project::Project;
use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap as StdHashMap,
    path::PathBuf,
    process::ExitStatus,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Instant,
};
use task::Shell;
use util::get_default_system_shell_preferring_bash;

use crate::tool_result_artifact::{
    TOOL_RESULT_PREVIEW_BYTE_BUDGET, ToolResultArtifact, ToolResultArtifactId,
    ToolResultArtifactStore, preview_tool_result,
};

/// Request to run a terminal command inside an OS-level sandbox.
///
/// Passed to [`super::AcpThread::create_terminal`]. The actual sandboxing
/// mechanism is platform-specific (macOS Seatbelt; Linux Bubblewrap; Windows
/// via Bubblewrap inside WSL), so callers describe the *intent* with plain data
/// here rather than constructing platform-specific types directly.
///
/// Default is the fully-sandboxed run (no network, project-only writes).
/// Setting `network` / `allow_fs_write` requests a relaxation; the caller is
/// responsible for having obtained user approval before reaching this point.
#[derive(Clone, Debug, Default)]
pub struct SandboxWrap {
    /// Directory subtrees the sandbox should allow writes to. Pass the
    /// project's worktree paths (and any per-command scratch directory)
    /// here — *not* the command's working directory, which is model-
    /// controlled and would let the model widen its own writable scope.
    pub writable_paths: Vec<PathBuf>,
    /// Additional write subtrees the user explicitly approved for this
    /// command (per-path write grants). Kept separate from `writable_paths`
    /// to make the trust boundary explicit: these originate from
    /// model-requested paths that passed a user-approval prompt. They are
    /// merged with `writable_paths` when generating the sandbox policy.
    pub extra_write_paths: Vec<PathBuf>,
    /// Outbound network access explicitly approved for this command.
    pub network: SandboxNetworkAccess,
    /// Additional paths that should remain readable but not writable, even when
    /// they fall under writable paths.
    pub protected_paths: Vec<PathBuf>,
    /// Allow unrestricted filesystem writes except for protected paths (ignores
    /// ordinary writable paths).
    pub allow_fs_write: bool,
    /// Whether the project (and therefore this terminal) is local. The
    /// enforcing proxy binds a loopback port on this host, so it can only
    /// confine local commands; a remote terminal can't reach it.
    pub is_local: bool,
    /// Windows/WSL only: `(release channel, version)` of the Linux `zed` to
    /// provision inside WSL as the sandbox helper (version `latest` for dev
    /// builds). Resolved by the agent (which can read the running app's release
    /// info) and forwarded to the sandbox. `None` on other platforms, or when
    /// the release can't be determined, in which case the WSL backend falls back
    /// to running bwrap without in-sandbox bind validation.
    pub wsl_zed_release: Option<(String, String)>,
}

#[derive(Clone, Debug, Default)]
pub enum SandboxNetworkAccess {
    /// Block all outbound network access.
    #[default]
    None,
    /// Allow only hosts in this allowlist, enforced by routing HTTP/HTTPS
    /// through an in-process proxy and confining the command to the proxy's
    /// loopback port.
    Restricted(Allowlist),
    /// Allow unrestricted outbound network access.
    All,
}

/// A structured, serializable reason the OS sandbox could not be created for a
/// command. Mirrors the Linux/WSL Bubblewrap failure modes; surfaced to the user
/// (and persisted in tool-call metadata) so the UI can
/// explain what went wrong.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum LinuxWslSandboxError {
    /// No usable `bwrap` binary was found on `PATH`.
    BwrapNotFound,
    /// The only `bwrap` found is setuid-root, which Zed refuses to run.
    SetuidRejected,
    /// `bwrap` is present but couldn't set up the sandbox (typically because
    /// unprivileged user namespaces are disabled).
    SandboxProbeFailed,
    /// Any other failure, with a human-readable description.
    Other(String),
}

impl From<sandbox::SandboxError> for LinuxWslSandboxError {
    fn from(error: sandbox::SandboxError) -> Self {
        match error {
            sandbox::SandboxError::BwrapNotFound => Self::BwrapNotFound,
            sandbox::SandboxError::BwrapSetuidRejected => Self::SetuidRejected,
            sandbox::SandboxError::SandboxProbeFailed => Self::SandboxProbeFailed,
            error => Self::Other(error.to_string()),
        }
    }
}

impl LinuxWslSandboxError {
    /// A short, user-facing explanation of why the sandbox couldn't be created,
    /// suitable for display in the agent panel.
    pub fn user_facing_message(&self) -> String {
        match self {
            LinuxWslSandboxError::BwrapNotFound => {
                "No usable `bwrap` binary was found on your PATH. Install Bubblewrap to let \
                 the agent sandbox terminal commands."
                    .to_string()
            }
            LinuxWslSandboxError::SetuidRejected => {
                "The only `bwrap` available is setuid-root, which Omega refuses to run. Install \
                 a non-setuid Bubblewrap to let the agent sandbox terminal commands."
                    .to_string()
            }
            LinuxWslSandboxError::SandboxProbeFailed => {
                "`bwrap` is installed but couldn't create a sandbox, likely because \
                 unprivileged user namespaces are disabled on this system."
                    .to_string()
            }
            LinuxWslSandboxError::Other(message) => message.clone(),
        }
    }

    /// The slug of the sandboxing docs section that best explains how to resolve
    /// this failure, for deep-linking from the UI. Pair with
    /// `client::zed_urls::sandboxing_docs`.
    pub fn docs_section(&self) -> &'static str {
        match self {
            // Both "no bwrap" and "only a setuid-root bwrap" are resolved by
            // installing a non-setuid Bubblewrap.
            LinuxWslSandboxError::BwrapNotFound | LinuxWslSandboxError::SetuidRejected => {
                "installing-bubblewrap"
            }
            // A failed probe on Linux is almost always disabled unprivileged
            // user namespaces, which the Ubuntu-specific section covers.
            LinuxWslSandboxError::SandboxProbeFailed => "installing-bubblewrap-ubuntu",
            // Catch-all (includes WSL/Windows messages): point at the platform
            // overview for the current OS.
            LinuxWslSandboxError::Other(_) => {
                if cfg!(target_os = "windows") {
                    "windows"
                } else {
                    "linux"
                }
            }
        }
    }
}

impl SandboxWrap {
    /// Whether the OS sandbox for this request can actually be created right now,
    /// returning a structured [`LinuxWslSandboxError`] when it can't.
    ///
    /// The sandbox implementation never runs a command unsandboxed on its own —
    /// it aborts if it can't create the sandbox. This lets a caller decide, up
    /// front, whether to run sandboxed, fall back to an unsandboxed run
    /// (fail-open), or refuse (fail-closed). It runs a brief probe subprocess on
    /// Linux, so call it off the main thread. On platforms whose sandbox can't
    /// fail to set up this way it always returns `Ok`.
    pub fn can_create_sandbox(&self) -> Result<(), LinuxWslSandboxError> {
        sandbox::Sandbox::can_create(&self.to_policy()).map_err(LinuxWslSandboxError::from)
    }

    /// Translate this request into the cross-platform [`sandbox::SandboxPolicy`].
    ///
    /// This is the enforcement-policy construction point, so it **captures** each
    /// grant as a [`sandbox::HostFilesystemLocation`] (pinning the inode / canonical
    /// path) rather than passing a re-resolvable path. A location that can't be
    /// captured (e.g. it doesn't exist) is dropped from the grant — fail-closed.
    ///
    /// This function has **no filesystem side effects**: it never creates paths.
    /// It is used both by the side-effect-free [`Self::can_create_sandbox`] probe
    /// and by real sandbox construction, and must behave identically. On Linux a
    /// writable grant that doesn't exist yet simply can't be captured (bwrap
    /// can't bind a missing path), so it's dropped here — the sanctioned way to
    /// get a grant to a new directory is the `create_directory` tool, which
    /// creates it (pinning the inode) before the grant is recorded. On macOS a
    /// missing leaf still canonicalizes, so such grants are captured directly.
    fn to_policy(&self) -> sandbox::SandboxPolicy {
        let protected_paths = self
            .protected_paths
            .iter()
            .filter_map(|path| sandbox::HostFilesystemLocation::new(path).ok())
            .collect();
        let fs = if self.allow_fs_write {
            sandbox::SandboxFsPolicy::Unrestricted { protected_paths }
        } else {
            let writable_paths = self
                .writable_paths
                .iter()
                .chain(self.extra_write_paths.iter())
                // Capture only — never create anything here (see the doc comment):
                // materializing an approved-but-missing grant is deferred to
                // `Sandbox::new` so it can never happen during the `can_create`
                // probe, before the user has approved the grant.
                .filter_map(|path| sandbox::HostFilesystemLocation::new(path).ok())
                .collect();
            sandbox::SandboxFsPolicy::Restricted {
                writable_paths,
                protected_paths,
            }
        };
        let network = match &self.network {
            SandboxNetworkAccess::None => sandbox::SandboxNetPolicy::Blocked,
            SandboxNetworkAccess::All => sandbox::SandboxNetPolicy::Unrestricted,
            SandboxNetworkAccess::Restricted(allowlist) => sandbox::SandboxNetPolicy::Restricted {
                allowed_domains: allowlist
                    .patterns()
                    .iter()
                    .map(|pattern| pattern.to_string())
                    .collect(),
            },
        };
        sandbox::SandboxPolicy { fs, network }
    }
}

/// Why the OS sandbox was *not* applied to a terminal command, even though
/// sandboxing is active for the thread. Persisted in tool-call metadata so the
/// UI can explain the situation after the fact.
///
/// This is deliberately platform-agnostic — every variant exists on every
/// platform — so the serialized form stored in the thread database never
/// depends on which OS wrote it. Today only Linux/WSL can fail to create a
/// sandbox (`ErrorLinuxWsl`), but the variant is named so macOS/Windows can
/// grow their own failure cases later without a migration.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum SandboxNotAppliedReason {
    /// The user disabled the sandbox for the rest of this thread, so the command
    /// ran without one. This happens either when the user approved a
    /// model-requested `unsandboxed: true` escape "for this thread", or when
    /// they chose to run unsandboxed for the thread after a sandbox-creation
    /// failure (in which case a preceding tool call's reason is
    /// [`SandboxNotAppliedReason::ErrorLinuxWsl`]).
    DisabledForThisThread,
    /// The Linux/WSL (Bubblewrap) sandbox could not be created for this command.
    ErrorLinuxWsl(LinuxWslSandboxError),
}

/// The live sandbox kept alive for its per-command resources (the network proxy
/// and, on macOS, the Seatbelt policy file) until the terminal exits.
type SandboxConfigHandle = sandbox::Sandbox;

/// Upper bound on preparing a WSL-sandboxed command. Deliberately generous:
/// the first invocation after the WSL utility VM has shut down (or after boot)
/// has to start the VM and the distro, which routinely takes 10-30 seconds on
/// slow disks or under antivirus scanning.
#[cfg(target_os = "windows")]
pub(crate) const WSL_SANDBOX_WRAP_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(60);

/// Wrap `(program, args)` for sandboxed execution, returning the wrapped
/// invocation (program, argv, env) plus the live [`sandbox::Sandbox`] that must
/// be kept alive for the command's duration. When `sandbox_wrap` is `None` the
/// command is returned unchanged.
///
/// The sandbox owns the network proxy (for restricted-network policies) and any
/// per-command policy file; the env it returns already routes through that
/// proxy when applicable.
pub(crate) async fn prepare_sandbox_wrap(
    program: String,
    args: Vec<String>,
    cwd: Option<PathBuf>,
    sandbox_wrap: Option<SandboxWrap>,
    env: HashMap<String, String>,
) -> anyhow::Result<(
    String,
    Vec<String>,
    HashMap<String, String>,
    Option<SandboxConfigHandle>,
)> {
    let Some(sandbox_wrap) = sandbox_wrap else {
        return Ok((program, args, env, None));
    };

    let mut sandbox =
        sandbox::Sandbox::new(sandbox_wrap.to_policy()).map_err(anyhow::Error::new)?;
    // Windows/WSL only: tell the sandbox which Linux `zed` to provision inside
    // WSL as its `--wsl-sandbox-helper`. A no-op (and a no-op setter) elsewhere.
    #[cfg(target_os = "windows")]
    if let Some((channel, version)) = sandbox_wrap.wsl_zed_release.clone() {
        sandbox.set_wsl_zed_release(channel, version);
    }
    let command = sandbox::CommandAndArgs {
        program,
        args,
        env: env.into_iter().collect::<StdHashMap<_, _>>(),
        cwd,
    };
    let wrapped = sandbox.wrap(&command).await.map_err(anyhow::Error::new)?;
    Ok((
        wrapped.program,
        wrapped.args,
        wrapped.env.into_iter().collect(),
        Some(sandbox),
    ))
}

pub struct Terminal {
    id: acp::TerminalId,
    command: Entity<Markdown>,
    working_dir: Option<PathBuf>,
    terminal: Entity<terminal::Terminal>,
    started_at: Instant,
    output: Option<TerminalOutput>,
    output_byte_limit: Option<usize>,
    _output_task: Shared<Task<acp::TerminalExitStatus>>,
    /// Flag indicating whether this terminal was stopped by explicit user action
    /// (e.g., clicking the Stop button). This is set before kill() is called
    /// so that code awaiting wait_for_exit() can check it deterministically.
    user_stopped: Arc<AtomicBool>,
    /// The live sandbox (Seatbelt policy file and/or network proxy) kept alive
    /// until the sandboxed command exits. `None` when the command isn't
    /// sandboxed or after it finishes. Dropping it tears down the proxy on a
    /// background thread (see `sandbox::Sandbox`'s `Drop`).
    _sandbox: Option<SandboxConfigHandle>,
    /// `OMEGA-DELTA-0103`. Every complete capture of this terminal's result.
    ///
    /// Held here rather than on the thread because the terminal is what the
    /// thread keeps: a finished `ToolCall` still owns this entity, so an
    /// artifact recorded during the turn is still addressable after it ends.
    /// Empty for a result that fits in the preview budget — that result is
    /// already carried whole by the event, and a second copy buys nothing.
    result_artifacts: ToolResultArtifactStore,
}

pub struct TerminalOutput {
    pub ended_at: Instant,
    pub exit_status: Option<ExitStatus>,
    pub content: String,
    pub original_content_len: usize,
    pub content_line_count: usize,
}

impl Terminal {
    pub fn new(
        id: acp::TerminalId,
        command_label: &str,
        working_dir: Option<PathBuf>,
        output_byte_limit: Option<usize>,
        terminal: Entity<terminal::Terminal>,
        language_registry: Arc<LanguageRegistry>,
        sandbox: Option<SandboxConfigHandle>,
        cx: &mut Context<Self>,
    ) -> Self {
        let command_task = terminal.read(cx).wait_for_completed_task(cx);
        // Tear the sandbox down on a GPUI background thread when this entity is
        // released, rather than relying on `Sandbox`'s `Drop` (which would spawn
        // a throwaway thread) on whatever thread releases us. `on_release` hands
        // us an `App`, so we can drive the teardown through the background
        // executor with `drop_on_current_thread`.
        cx.on_release(|this, cx| {
            if let Some(sandbox) = this._sandbox.take() {
                cx.background_executor()
                    .spawn(async move { sandbox.drop_on_current_thread() })
                    .detach();
            }
        })
        .detach();
        let result_artifacts = ToolResultArtifactStore::new(format!("terminal:{id}"));
        Self {
            id,
            result_artifacts,
            _sandbox: sandbox,
            command: cx.new(|cx| {
                Markdown::new(
                    format!("```\n{}\n```", command_label).into(),
                    Some(language_registry.clone()),
                    None,
                    cx,
                )
            }),
            working_dir,
            terminal,
            started_at: Instant::now(),
            output: None,
            output_byte_limit,
            user_stopped: Arc::new(AtomicBool::new(false)),
            _output_task: cx
                .spawn(async move |this, cx| {
                    let exit_status = command_task.await;

                    this.update(cx, |this, cx| {
                        let (content, original_content_len) = this.truncated_output(cx);
                        let content_line_count = this.terminal.read(cx).total_lines();

                        // `OMEGA-DELTA-0103`. The command has exited, so there
                        // is a complete result to version. Record it before
                        // anything reads the bounded one, or the marker the
                        // reader gets points at an address that resolves to
                        // nothing.
                        let complete = this.terminal.read(cx).get_content();
                        if complete.len() > TOOL_RESULT_PREVIEW_BYTE_BUDGET {
                            this.result_artifacts.record(&complete);
                        }

                        this.output = Some(TerminalOutput {
                            ended_at: Instant::now(),
                            exit_status,
                            content,
                            original_content_len,
                            content_line_count,
                        });
                        // Free the sandbox (and its network proxy) as soon as
                        // the command finishes, rather than holding it until
                        // this entity is released. The proxy's teardown joins a
                        // listener thread, so run it on the background executor
                        // to keep it off the foreground thread.
                        if let Some(sandbox) = this._sandbox.take() {
                            cx.background_executor()
                                .spawn(async move { sandbox.drop_on_current_thread() })
                                .detach();
                        }
                        cx.notify();
                    })
                    .ok();

                    let exit_status = exit_status.map(portable_pty::ExitStatus::from);

                    acp::TerminalExitStatus::new()
                        .exit_code(exit_status.as_ref().map(|e| e.exit_code()))
                        .signal(exit_status.and_then(|e| e.signal().map(ToOwned::to_owned)))
                })
                .shared(),
        }
    }

    pub fn id(&self) -> &acp::TerminalId {
        &self.id
    }

    pub fn wait_for_exit(&self) -> Shared<Task<acp::TerminalExitStatus>> {
        self._output_task.clone()
    }

    pub fn kill(&mut self, cx: &mut App) {
        self.terminal.update(cx, |terminal, _cx| {
            terminal.kill_active_task();
        });
    }

    /// Marks this terminal as stopped by user action and then kills it.
    /// This should be called when the user explicitly clicks a Stop button.
    pub fn stop_by_user(&mut self, cx: &mut App) {
        self.user_stopped.store(true, Ordering::SeqCst);
        self.kill(cx);
    }

    /// Returns whether this terminal was stopped by explicit user action.
    pub fn was_stopped_by_user(&self) -> bool {
        self.user_stopped.load(Ordering::SeqCst)
    }

    /// The event this terminal's result goes out on.
    ///
    /// `OMEGA-DELTA-0103`. What leaves here is a preview, not the result: the
    /// full text stays in [`Self::result_artifacts`] and the preview names the
    /// address to fetch it from. Every consumer downstream of this — the
    /// model's context above all — is bounded by that, rather than by whatever
    /// each of them remembers to do for itself.
    ///
    /// A result inside the budget is returned exactly as it always was.
    /// Whether this terminal has anything worth drawing a pane for.
    ///
    /// omega#112. A finished command that printed nothing was still given a
    /// full terminal element: a dark box with a blinking cursor that took focus
    /// when clicked. The owner found four of them in one transcript, under
    /// commands like `find ... 2>/dev/null` that legitimately match nothing.
    ///
    /// A terminal that is still running answers `true` — the live pane is the
    /// point, and a command that has not printed *yet* is not a command that
    /// printed nothing. Only a finished, empty one is hidden.
    pub fn has_output_to_render(&self) -> bool {
        match self.output.as_ref() {
            Some(output) => !output.content.trim().is_empty(),
            None => true,
        }
    }

    pub fn current_output(&self, cx: &App) -> acp::TerminalOutputResponse {
        if let Some(output) = self.output.as_ref() {
            let exit_status = output.exit_status.map(portable_pty::ExitStatus::from);
            // Totals come from the complete artifact when there is one, so a
            // result cut by `output_byte_limit` before it ever reached here is
            // reported through the same marker rather than absorbed silently.
            let (total_bytes, total_lines) = match self.result_artifacts.latest() {
                Some(artifact) => (artifact.byte_count(), artifact.line_count()),
                None => (output.original_content_len, output.content_line_count),
            };
            let preview = preview_tool_result(
                &output.content,
                total_bytes,
                total_lines,
                TOOL_RESULT_PREVIEW_BYTE_BUDGET,
                self.result_artifacts.latest().map(|a| a.id().clone()),
            );

            let truncated = preview.is_truncated();
            acp::TerminalOutputResponse::new(preview.text, truncated).exit_status(
                acp::TerminalExitStatus::new()
                    .exit_code(exit_status.as_ref().map(|e| e.exit_code()))
                    .signal(exit_status.and_then(|e| e.signal().map(ToOwned::to_owned))),
            )
        } else {
            // Still running: there is no complete result to version yet, so the
            // marker says that instead of naming an address that resolves to
            // nothing.
            let (current_content, original_len) = self.truncated_output(cx);
            let preview = preview_tool_result(
                &current_content,
                original_len,
                self.terminal.read(cx).total_lines(),
                TOOL_RESULT_PREVIEW_BYTE_BUDGET,
                None,
            );
            let truncated = preview.is_truncated();
            acp::TerminalOutputResponse::new(preview.text, truncated)
        }
    }

    /// `OMEGA-DELTA-0103`. The fetch path: every complete capture of this
    /// terminal's result, addressable by the id the preview's marker names.
    pub fn result_artifacts(&self) -> &ToolResultArtifactStore {
        &self.result_artifacts
    }

    /// The complete result at `id`, or `None` when nothing was recorded there.
    pub fn result_artifact(&self, id: &ToolResultArtifactId) -> Option<&ToolResultArtifact> {
        self.result_artifacts.get(id)
    }

    /// `OMEGA-DELTA-0103`. Lines the complete result has, when it is longer
    /// than the body a surface can render. `None` when the body is the whole
    /// result, which is the ordinary case and must stay silent.
    ///
    /// This is what `tool_output_ceiling_label` needs as its extra input: a
    /// ceiling that counts only the lines in front of it would state a total
    /// that is not the total, and a reader who lifted the ceiling and reached
    /// the last line would conclude they had the whole result.
    pub fn result_record_total_lines(&self, cx: &App) -> Option<usize> {
        let artifact = self.result_artifacts.latest()?;
        let rendered_lines = self.terminal.read(cx).total_lines();
        (artifact.line_count() > rendered_lines).then_some(artifact.line_count())
    }

    fn truncated_output(&self, cx: &App) -> (String, usize) {
        let terminal = self.terminal.read(cx);
        let mut content = terminal.get_content();

        let original_content_len = content.len();

        if let Some(limit) = self.output_byte_limit
            && content.len() > limit
        {
            let mut end_ix = limit.min(content.len());
            while !content.is_char_boundary(end_ix) {
                end_ix -= 1;
            }
            // Don't truncate mid-line, clear the remainder of the last line
            end_ix = content[..end_ix].rfind('\n').unwrap_or(end_ix);
            content.truncate(end_ix);
        }

        (content, original_content_len)
    }

    pub fn command(&self) -> &Entity<Markdown> {
        &self.command
    }

    pub fn update_command_label(&self, label: &str, cx: &mut App) {
        self.command.update(cx, |command, cx| {
            command.replace(format!("```\n{}\n```", label), cx);
        });
    }

    pub fn working_dir(&self) -> &Option<PathBuf> {
        &self.working_dir
    }

    pub fn started_at(&self) -> Instant {
        self.started_at
    }

    pub fn output(&self) -> Option<&TerminalOutput> {
        self.output.as_ref()
    }

    pub fn inner(&self) -> &Entity<terminal::Terminal> {
        &self.terminal
    }

    pub fn to_markdown(&self, cx: &App) -> String {
        format!(
            "Terminal:\n```\n{}\n```\n",
            self.terminal.read(cx).get_content()
        )
    }
}

pub async fn create_terminal_entity(
    command: String,
    args: &[String],
    env_vars: Vec<(String, String)>,
    cwd: Option<PathBuf>,
    project: &Entity<Project>,
    cx: &mut AsyncApp,
) -> Result<Entity<terminal::Terminal>> {
    let mut env = if let Some(dir) = &cwd {
        project
            .update(cx, |project, cx| {
                project.environment().update(cx, |env, cx| {
                    env.directory_environment(dir.clone().into(), cx)
                })
            })
            .await
            .unwrap_or_default()
    } else {
        Default::default()
    };

    disable_pagers_through_env(&mut env);
    env.extend(env_vars);

    // Use remote shell or default system shell, as appropriate
    let shell = project
        .update(cx, |project, cx| {
            project
                .remote_client()
                .and_then(|r| r.read(cx).default_system_shell())
                .map(Shell::Program)
        })
        .unwrap_or_else(|| Shell::Program(get_default_system_shell_preferring_bash()));
    let is_windows = project.read_with(cx, |project, cx| project.path_style(cx).is_windows());
    let (task_command, task_args) = task::ShellBuilder::new(&shell, is_windows)
        .redirect_stdin_to_dev_null()
        .build(Some(command.clone()), &args);

    project
        .update(cx, |project, cx| {
            project.create_terminal_task(
                task::SpawnInTerminal {
                    command: Some(task_command),
                    args: task_args,
                    cwd,
                    env,
                    ..Default::default()
                },
                cx,
            )
        })
        .await
}

// Disable pagers so agent/terminal commands don't hang behind interactive UIs
pub(crate) fn disable_pagers_through_env(env: &mut collections::HashMap<String, String>) {
    env.insert("PAGER".into(), "".into());
    // Override user core.pager (e.g. delta) which Git prefers over PAGER
    env.insert("GIT_PAGER".into(), "cat".into());
}

#[cfg(test)]
mod empty_terminal_pane_tests {
    use super::*;

    fn finished_with(content: &str) -> TerminalOutput {
        TerminalOutput {
            ended_at: Instant::now(),
            exit_status: None,
            content: content.to_owned(),
            original_content_len: content.len(),
            content_line_count: content.lines().count(),
        }
    }

    // omega#112. The three cases the owner's screenshot separates: a command
    // that printed something, a command that finished having printed nothing,
    // and a command still running.
    #[test]
    fn only_a_finished_and_empty_terminal_has_nothing_to_render() {
        assert!(
            has_output_to_render(Some(&finished_with("total 8\ndrwxr-xr-x"))),
            "a finished command that printed output must still draw its pane"
        );
        assert!(
            !has_output_to_render(Some(&finished_with(""))),
            "a finished command that printed nothing must not draw a pane; an \
             empty terminal element is a dark box with a cursor in it"
        );
        assert!(
            !has_output_to_render(Some(&finished_with("  \n\n \t "))),
            "whitespace is not output. `find ... 2>/dev/null` matching nothing \
             can still leave a newline behind, and a pane containing one blank \
             line is the same box"
        );
        assert!(
            has_output_to_render(None),
            "a running terminal must draw, or there is nowhere to watch the \
             command work. Not having printed *yet* is not having printed \
             nothing"
        );
    }

    /// The rule under test, extracted so it can be exercised without building a
    /// `Terminal` (which needs a window, a project and a spawned process).
    fn has_output_to_render(output: Option<&TerminalOutput>) -> bool {
        match output {
            Some(output) => !output.content.trim().is_empty(),
            None => true,
        }
    }
}
