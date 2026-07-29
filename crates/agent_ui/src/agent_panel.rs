use std::{
    cell::Cell,
    fmt,
    path::PathBuf,
    rc::Rc,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

use acp_thread::{AcpThread, AcpThreadEvent, MentionUri, ThreadStatus, line_range_suffix};
use agent::{ContextServerRegistry, SharedThread, ThreadStore};
use agent_client_protocol::schema::v1 as acp;
use agent_servers::AgentServer;
use agent_settings::UserAgentsMd;
use collections::HashSet;
use db::kvp::{Dismissable, KeyValueStore};
use itertools::Itertools;
use project::{AgentId, ProjectItem};
use serde::{Deserialize, Serialize};

use zed_actions::{
    DecreaseBufferFontSize, IncreaseBufferFontSize, ResetBufferFontSize,
    agent::{
        AddSelectionToThread, ConflictContent, LogoutAgent, OpenSettings, ReauthenticateAgent,
        ResetAgentZoom, ResetOnboarding, ResolveConflictedFilesWithAgent,
        ResolveConflictsWithAgent, ReviewBranchDiff, SelectAgent,
    },
    agent_computer::OpenPanel as OpenAgentComputerPanel,
    assistant::{
        FocusAgent, ManageSkills, OpenGlobalAgentsMdRules, OpenProjectAgentsMdRules, Toggle,
        ToggleFocus,
    },
    full_auto_panel::{OpenLauncher, ToggleFocus as ToggleFullAutoFocus},
    git_panel::ToggleFocus as ToggleGitFocus,
    workroom::OpenPanel as OpenSarahWorkroomPanel,
};

use crate::ExpandMessageEditor;
use crate::ManageProfiles;
use crate::agent_connection_store::AgentConnectionStore;
use crate::completion_provider::{AgentContextSelection, AgentContextSource};
use crate::terminal_thread_metadata_store::{
    TerminalThreadMetadata, TerminalThreadMetadataStore, compose_terminal_thread_title,
    terminal_title_without_prefix,
};
use crate::thread_metadata_store::{
    ThreadId, ThreadMetadataStore, ThreadMetadataStoreEvent, WorktreePaths,
};
use crate::thread_outline::OutlineActionOutcome;
use crate::{
    Agent, AgentInitialContent, AgentThreadSource, ExternalSourcePrompt, NewExternalAgentThread,
    NewNativeAgentThreadFromSummary,
};
use crate::{
    AgentDiffPane, ConversationView, CopyThreadToClipboard, Follow, LoadThreadFromClipboard,
    NewTerminalThread, NewThread, OpenActiveThreadAsMarkdown, OpenAgentDiff, ResetFastModeWarnings,
    ResetTrialEndUpsell, ResetTrialUpsell, ShowAllSidebarThreadMetadata, ShowThreadMetadata,
    ToggleNewThreadMenu, ToggleOptionsMenu, ToggleThreadsSidebar,
    conversation_view::{
        AcpThreadViewEvent, RootThreadUpdated, ThreadView, reset_fast_mode_warnings,
    },
    ui::{AgentNotification, AgentNotificationEvent},
};
use crate::{
    omega_executor_selector, omega_nostr_activity, omega_sidebar, omega_threads_sidebar,
    thread_identity::{
        BranchIdentity, GitIdentitySummary, IdentityPhase, ThreadIdentityCandidate,
        ThreadIdentityObservation,
    },
    workbench_shell,
};
use agent_settings::AgentSettings;
use ai_onboarding::AgentPanelOnboarding;
use anyhow::{Context as _, Result, anyhow};
#[cfg(feature = "audio")]
use audio::{Audio, Sound};
use chrono::{DateTime, Utc};
use collections::HashMap;
use editor::{Editor, MultiBuffer};
use extension_host::ExtensionStore;
use feature_flags::{CreateThreadToolFeatureFlag, FeatureFlagAppExt as _};
use futures::AsyncReadExt as _;
use http_client::{AsyncBody, HttpClientWithUrl};

use fs::Fs;
use full_auto_ui::FullAutoPanel;
use futures::FutureExt as _;
use git_ui::git_panel::{
    Close as CloseGitPanel, GitPanel, GitPanelRepositoryScope, Toggle as ToggleGitPanel,
};
use gpui::{
    Action, Anchor, Animation, AnimationExt, AnyElement, App, AsyncWindowContext, ClipboardItem,
    Entity, EventEmitter, ExternalPaths, FocusHandle, Focusable, Hsla, ImageSource, KeyContext,
    ObjectFit, Pixels, PlatformDisplay, RenderImage, Subscription, Task, TaskExt, WeakEntity,
    WindowHandle, img, prelude::*, pulsating_between,
};
use language::LanguageRegistry;
use language_model::LanguageModelRegistry;
use notifications::status_toast::StatusToast;
use project::{Project, ProjectPath, Worktree, WorktreeId, git_store::RepositoryId};
use project_panel::ProjectPanel;
use settings::TerminalDockPosition;
use settings::{NotifyWhenAgentWaiting, Settings, update_settings_file};

use search::{BufferSearchBar, buffer_search::Deploy as DeployBufferSearch};
use sha2::{Digest as _, Sha256};
use terminal::{Event as TerminalEvent, terminal_settings::TerminalSettings};
use terminal_view::{TerminalView, terminal_panel::TerminalPanel};
use text::{OffsetRangeExt, Point};
use theme_settings::ThemeSettings;
use ui::{
    ContextMenu, ContextMenuEntry, GradientFade, IconButton, KeyBinding, ListItem, ListItemSpacing,
    PopoverMenu, PopoverMenuHandle, ProjectEmptyState, Tab, Tooltip, prelude::*,
    utils::WithRemSize,
};
use util::{ResultExt as _, paths::PathStyle, rel_path::RelPath};
use workspace::{
    CollaboratorId, DraggedSelection, DraggedTab, MultiWorkspace, PathList, SerializedPathList,
    SplitDirection, ToggleWorkspaceSidebar, ToggleZoom, ToolbarItemView, Workspace, WorkspaceId,
    dock::{DockPosition, Panel, PanelEvent},
    item::{ItemEvent, ItemHandle},
};

const AGENT_PANEL_KEY: &str = "agent_panel";
const MIN_PANEL_WIDTH: Pixels = px(300.);
const LAST_USED_AGENT_KEY: &str = "agent_panel__last_used_external_agent";
const LAST_CREATED_ENTRY_KIND_KEY: &str = "agent_panel__last_created_entry_kind";
const TERMINAL_AGENT_TELEMETRY_ID: &str = "terminal";
const TERMINAL_INIT_COMMAND_STARTUP_TIMEOUT: Duration = Duration::from_secs(5);
const KNOWN_TERMINAL_AGENT_COMMANDS: &[&str] = &[
    "agent", // Unfortunately, both Cursor cli + grok
    "agy",
    "aider",
    "amp",
    "claude",
    "codex",
    "copilot",
    "crush",
    "devin",
    "droid",
    "gemini",
    "goose",
    "grok",
    "openhands",
    "opencode",
    "pi",
    "qwen",
];

fn is_known_terminal_agent_command(command: &str) -> bool {
    KNOWN_TERMINAL_AGENT_COMMANDS.contains(&command)
}

fn terminal_program_to_report(
    last_observed_program: &mut Option<String>,
    current_program: Option<String>,
) -> Option<String> {
    let current_program =
        current_program.filter(|program| is_known_terminal_agent_command(program));
    let program_to_report =
        if current_program.is_some() && current_program != *last_observed_program {
            current_program.clone()
        } else {
            None
        };
    *last_observed_program = current_program;
    program_to_report
}

/// Maximum number of idle threads kept in the agent panel's retained list.
/// Set as a GPUI global to override; otherwise defaults to 5.
pub struct MaxIdleRetainedThreads(pub usize);
impl gpui::Global for MaxIdleRetainedThreads {}

impl MaxIdleRetainedThreads {
    pub fn global(cx: &App) -> usize {
        cx.try_global::<Self>().map_or(5, |g| g.0)
    }
}

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub struct TerminalId(uuid::Uuid);

impl TerminalId {
    pub(crate) fn new() -> Self {
        Self(uuid::Uuid::new_v4())
    }

    pub(crate) fn to_key_string(self) -> String {
        self.0.hyphenated().to_string()
    }

    pub(crate) fn from_key_string(key: &str) -> anyhow::Result<Self> {
        Ok(Self(uuid::Uuid::parse_str(key)?))
    }
}

impl fmt::Display for TerminalId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(formatter)
    }
}

#[derive(Clone, Debug)]
pub struct AgentPanelTerminalInfo {
    pub id: TerminalId,
    pub title: SharedString,
    pub created_at: DateTime<Utc>,
    pub has_notification: bool,
    pub custom_title: Option<SharedString>,
    pub working_directory: Option<PathBuf>,
}

#[derive(Serialize, Deserialize)]
struct LastUsedAgent {
    agent: Agent,
}

#[derive(Serialize, Deserialize)]
struct LastCreatedEntryKind {
    entry_kind: AgentPanelEntryKind,
}

struct SourcePanelInitialization {
    agent: Agent,
    initial_content: Option<AgentInitialContent>,
}

/// Reads the most recently used agent across all workspaces. Used as a fallback
/// when opening a workspace that has no per-workspace agent preference yet.
/// Read the published Agent Chat manifest and adapt it to the channel registry.
///
/// The current web API publishes one deployment manifest, not a registry.
/// `omega_public_channels` owns the compatibility adapter, so the generic
/// controller never imports the deployment relay or group constants.
async fn fetch_public_channel_registry(
    http_client: Arc<HttpClientWithUrl>,
) -> Result<crate::omega_public_channels::ChannelRegistry> {
    const MAX_REGISTRY_BYTES: u64 = 256 * 1024;

    let response = http_client
        .get(
            omega_nostr_activity::MANIFEST_URL,
            AsyncBody::default(),
            true,
        )
        .await
        .context("fetching the public chat manifest")?;
    anyhow::ensure!(
        response.status().is_success(),
        "the public chat manifest answered {}",
        response.status().as_u16()
    );
    let mut body = Vec::new();
    response
        .into_body()
        .take(MAX_REGISTRY_BYTES + 1)
        .read_to_end(&mut body)
        .await
        .context("reading the public chat manifest")?;
    anyhow::ensure!(
        body.len() as u64 <= MAX_REGISTRY_BYTES,
        "the public chat manifest exceeded the size limit"
    );
    crate::omega_public_channels::ChannelRegistry::from_agent_chat_manifest(
        &String::from_utf8_lossy(&body),
    )
}

fn read_global_last_used_agent(kvp: &KeyValueStore) -> Option<Agent> {
    kvp.read_kvp(LAST_USED_AGENT_KEY)
        .log_err()
        .flatten()
        .and_then(|json| serde_json::from_str::<LastUsedAgent>(&json).log_err())
        .map(|entry| entry.agent)
}

async fn write_global_last_used_agent(kvp: KeyValueStore, agent: Agent) {
    if let Some(json) = serde_json::to_string(&LastUsedAgent { agent }).log_err() {
        kvp.write_kvp(LAST_USED_AGENT_KEY.to_string(), json)
            .await
            .log_err();
    }
}

fn read_global_last_created_entry_kind(kvp: &KeyValueStore) -> Option<AgentPanelEntryKind> {
    kvp.read_kvp(LAST_CREATED_ENTRY_KIND_KEY)
        .log_err()
        .flatten()
        .and_then(|json| serde_json::from_str::<LastCreatedEntryKind>(&json).log_err())
        .map(|entry| entry.entry_kind)
}

fn project_agents_md_path(
    project: &Entity<Project>,
    require_existing_file: bool,
    cx: &App,
) -> Option<PathBuf> {
    let rel_path = util::rel_path::RelPath::from_unix_str("AGENTS.md").ok()?;
    project
        .read(cx)
        .visible_worktrees(cx)
        .next()
        .and_then(|worktree| {
            let worktree = worktree.read(cx);

            if require_existing_file {
                let entry = worktree.entry_for_path(rel_path)?;
                if !entry.is_file() {
                    return None;
                }
            }

            Some(worktree.absolutize(rel_path))
        })
}

fn open_global_rules(workspace: &mut Workspace, window: &mut Window, cx: &mut Context<Workspace>) {
    // OMEGA-DELTA-0125. `open_abs_path` opens an item in the centre pane, which
    // `OMEGA-DELTA-0053` does not draw once zero base is sealed. The owner
    // clicked this entry and reported that nothing happened; the file opened,
    // took the composer's focus, and landed somewhere with no pixels. The
    // reader declines outside the seal, so a full editor keeps the pane.
    if crate::omega_file_peek::open_file(workspace, paths::agents_file().clone(), window, cx) {
        return;
    }
    workspace
        .open_abs_path(
            paths::agents_file().clone(),
            workspace::OpenOptions {
                focus: Some(true),
                ..Default::default()
            },
            window,
            cx,
        )
        .detach_and_log_err(cx);
}

fn open_project_rules(workspace: &mut Workspace, window: &mut Window, cx: &mut Context<Workspace>) {
    if let Some(path) = project_agents_md_path(workspace.project(), false, cx) {
        // OMEGA-DELTA-0125. As above. This is the entry the owner named —
        // "Open Project Rules (AGENTS.md)".
        //
        // The menu offers it only when the file exists (`project_agents_md_path`
        // with `require_existing_file`), and this resolves without that check,
        // so a file deleted between the menu being built and the entry being
        // clicked lands in the reader's "No file at …" state rather than in
        // silence. That gap is small and it is the one worth drawing for: a
        // repair whose own failure mode is a dead click has not repaired
        // anything.
        if crate::omega_file_peek::open_file(workspace, path.clone(), window, cx) {
            return;
        }
        workspace
            .open_abs_path(
                path,
                workspace::OpenOptions {
                    focus: Some(true),
                    ..Default::default()
                },
                window,
                cx,
            )
            .detach_and_log_err(cx);
    }
}

async fn write_global_last_created_entry_kind(kvp: KeyValueStore, entry_kind: AgentPanelEntryKind) {
    if let Some(json) = serde_json::to_string(&LastCreatedEntryKind { entry_kind }).log_err() {
        kvp.write_kvp(LAST_CREATED_ENTRY_KIND_KEY.to_string(), json)
            .await
            .log_err();
    }
}

fn read_serialized_panel(
    workspace_id: workspace::WorkspaceId,
    kvp: &KeyValueStore,
) -> Option<SerializedAgentPanel> {
    let scope = kvp.scoped(AGENT_PANEL_KEY);
    let key = i64::from(workspace_id).to_string();
    scope
        .read(&key)
        .log_err()
        .flatten()
        .and_then(|json| serde_json::from_str::<SerializedAgentPanel>(&json).log_err())
}

async fn save_serialized_panel(
    workspace_id: workspace::WorkspaceId,
    panel: SerializedAgentPanel,
    kvp: KeyValueStore,
) -> Result<()> {
    let scope = kvp.scoped(AGENT_PANEL_KEY);
    let key = i64::from(workspace_id).to_string();
    scope.write(key, serde_json::to_string(&panel)?).await?;
    Ok(())
}

/// Migration: reads the original single-panel format stored under the
/// `"agent_panel"` KVP key before per-workspace keying was introduced.
fn read_legacy_serialized_panel(kvp: &KeyValueStore) -> Option<SerializedAgentPanel> {
    kvp.read_kvp(AGENT_PANEL_KEY)
        .log_err()
        .flatten()
        .and_then(|json| serde_json::from_str::<SerializedAgentPanel>(&json).log_err())
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ThreadTitleRegenerationResult {
    NotOpen,
    Started,
    NoModel,
    AlreadyGenerating,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
enum AgentPanelEntryKind {
    #[default]
    Thread,
    Terminal,
}

#[derive(Serialize, Deserialize, Debug)]
struct SerializedAgentPanel {
    selected_agent: Option<Agent>,
    #[serde(default)]
    last_created_entry_kind: AgentPanelEntryKind,
    #[serde(default)]
    last_active_thread: Option<SerializedActiveThread>,
    #[serde(default)]
    last_active_terminal_id: Option<String>,
    #[serde(default)]
    new_draft_thread_id: Option<ThreadId>,
}

#[derive(Serialize, Deserialize, Debug)]
struct SerializedActiveThread {
    /// For drafts this is `None`; use `thread_id` to address them instead.
    session_id: Option<String>,
    /// Optional for back-compat with older serialized payloads that only carried `session_id`.
    #[serde(default)]
    thread_id: Option<ThreadId>,
    agent_type: Agent,
    title: Option<String>,
    work_dirs: Option<SerializedPathList>,
}

pub fn init(cx: &mut App) {
    cx.observe_new(
        |workspace: &mut Workspace, _window, _cx: &mut Context<Workspace>| {
            workspace
                .register_action(|workspace, _: &NewThread, window, cx| {
                    if let Some(panel) = workspace.panel::<AgentPanel>(cx) {
                        panel.update(cx, |panel, cx| {
                            panel.new_thread_with_workspace(Some(workspace), window, cx)
                        });
                        workspace.focus_panel::<AgentPanel>(window, cx);
                    }
                })
                .register_action(|workspace, _: &NewTerminalThread, window, cx| {
                    if let Some(panel) = workspace.panel::<AgentPanel>(cx) {
                        panel.update(cx, |panel, cx| {
                            panel.new_terminal(
                                Some(workspace),
                                AgentThreadSource::AgentPanel,
                                window,
                                cx,
                            )
                        });
                        workspace.focus_panel::<AgentPanel>(window, cx);
                    }
                })
                // `OMEGA-DELTA-0020`. Both Full Auto actions used to be
                // answered by `full_auto_ui::init` against a dock panel that
                // no longer exists. They are answered here now, so a keymap or
                // command-palette invocation that worked before still works.
                .register_action(|workspace, _: &OpenLauncher, window, cx| {
                    if let Some(panel) = workspace.panel::<AgentPanel>(cx) {
                        panel.update(cx, |panel, cx| panel.open_full_auto(true, window, cx));
                        workspace.focus_panel::<AgentPanel>(window, cx);
                    }
                })
                .register_action(|workspace, _: &ToggleFullAutoFocus, window, cx| {
                    if let Some(panel) = workspace.panel::<AgentPanel>(cx) {
                        panel.update(cx, |panel, cx| panel.toggle_full_auto(window, cx));
                        workspace.focus_panel::<AgentPanel>(window, cx);
                    }
                })
                .register_action(
                    |workspace, action: &NewNativeAgentThreadFromSummary, window, cx| {
                        if let Some(panel) = workspace.panel::<AgentPanel>(cx) {
                            panel.update(cx, |panel, cx| {
                                panel.new_native_agent_thread_from_summary(action, window, cx)
                            });
                            workspace.focus_panel::<AgentPanel>(window, cx);
                        }
                    },
                )
                .register_action(|workspace, _: &ExpandMessageEditor, window, cx| {
                    if let Some(panel) = workspace.panel::<AgentPanel>(cx) {
                        workspace.focus_panel::<AgentPanel>(window, cx);
                        panel.update(cx, |panel, cx| panel.expand_message_editor(window, cx));
                    }
                })
                .register_action(|workspace, action: &NewExternalAgentThread, window, cx| {
                    if let Some(panel) = workspace.panel::<AgentPanel>(cx) {
                        workspace.focus_panel::<AgentPanel>(window, cx);
                        panel.update(cx, |panel, cx| {
                            panel.new_external_agent_thread(action, window, cx);
                        });
                    }
                })
                .register_action(|workspace, action: &SelectAgent, window, cx| {
                    if let Some(panel) = workspace.panel::<AgentPanel>(cx) {
                        panel.update(cx, |panel, cx| {
                            let agent = AgentId::new(action.agent.clone()).into();
                            panel.select_agent(agent, window, cx);
                        });
                    }
                })
                .register_action(|workspace, action: &ManageSkills, window, cx| {
                    if let Some(panel) = workspace.panel::<AgentPanel>(cx) {
                        workspace.focus_panel::<AgentPanel>(window, cx);
                        panel.update(cx, |panel, cx| panel.manage_skills(action, window, cx));
                    }
                })
                .register_action(|workspace, _: &OpenGlobalAgentsMdRules, window, cx| {
                    open_global_rules(workspace, window, cx);
                })
                .register_action(|workspace, _: &OpenProjectAgentsMdRules, window, cx| {
                    open_project_rules(workspace, window, cx);
                })
                .register_action(|workspace, _: &Follow, window, cx| {
                    workspace.follow(CollaboratorId::Agent, window, cx);
                })
                .register_action(|workspace, _: &OpenAgentDiff, window, cx| {
                    let thread = workspace
                        .panel::<AgentPanel>(cx)
                        .and_then(|panel| panel.read(cx).active_conversation_view().cloned())
                        .and_then(|conversation| {
                            conversation
                                .read(cx)
                                .root_thread_view()
                                .map(|r| r.read(cx).thread.clone())
                        });

                    if let Some(thread) = thread {
                        AgentDiffPane::deploy_in_workspace(thread, workspace, window, cx);
                    }
                })
                .register_action(|workspace, _: &ToggleOptionsMenu, window, cx| {
                    if let Some(panel) = workspace.panel::<AgentPanel>(cx) {
                        workspace.focus_panel::<AgentPanel>(window, cx);
                        panel.update(cx, |panel, cx| {
                            panel.toggle_options_menu(&ToggleOptionsMenu, window, cx);
                        });
                    }
                })
                .register_action(|workspace, _: &ToggleNewThreadMenu, window, cx| {
                    if let Some(panel) = workspace.panel::<AgentPanel>(cx) {
                        workspace.focus_panel::<AgentPanel>(window, cx);
                        panel.update(cx, |panel, cx| {
                            panel.toggle_new_thread_menu(&ToggleNewThreadMenu, window, cx);
                        });
                    }
                })
                .register_action(|_workspace, _: &ResetOnboarding, window, cx| {
                    window.dispatch_action(workspace::RestoreBanner.boxed_clone(), cx);
                    window.refresh();
                })
                .register_action(|workspace, _: &ResetTrialUpsell, _window, cx| {
                    if let Some(panel) = workspace.panel::<AgentPanel>(cx) {
                        panel.update(cx, |panel, _| {
                            panel
                                .new_user_onboarding_upsell_dismissed
                                .store(false, Ordering::Release);
                        });
                    }
                    OnboardingUpsell::set_dismissed(false, cx);
                })
                .register_action(|_workspace, _: &ResetTrialEndUpsell, _window, cx| {
                    TrialEndUpsell::set_dismissed(false, cx);
                })
                .register_action(|_workspace, _: &ResetFastModeWarnings, _window, cx| {
                    reset_fast_mode_warnings(cx);
                })
                .register_action(|workspace, _: &ResetAgentZoom, window, cx| {
                    if let Some(panel) = workspace.panel::<AgentPanel>(cx) {
                        panel.update(cx, |panel, cx| {
                            panel.reset_agent_zoom(window, cx);
                        });
                    }
                })
                .register_action(|workspace, _: &CopyThreadToClipboard, window, cx| {
                    if let Some(panel) = workspace.panel::<AgentPanel>(cx) {
                        panel.update(cx, |panel, cx| {
                            panel.copy_thread_to_clipboard(window, cx);
                        });
                    }
                })
                .register_action(|workspace, _: &LoadThreadFromClipboard, window, cx| {
                    if let Some(panel) = workspace.panel::<AgentPanel>(cx) {
                        workspace.focus_panel::<AgentPanel>(window, cx);
                        panel.update(cx, |panel, cx| {
                            panel.load_thread_from_clipboard(window, cx);
                        });
                    }
                })
                .register_action(|workspace, _: &ShowThreadMetadata, window, cx| {
                    if let Some(panel) = workspace.panel::<AgentPanel>(cx) {
                        panel.update(cx, |panel, cx| {
                            panel.show_thread_metadata(&ShowThreadMetadata, window, cx);
                        });
                    }
                })
                .register_action(|workspace, _: &ShowAllSidebarThreadMetadata, window, cx| {
                    if let Some(panel) = workspace.panel::<AgentPanel>(cx) {
                        panel.update(cx, |panel, cx| {
                            panel.show_all_sidebar_thread_metadata(
                                &ShowAllSidebarThreadMetadata,
                                window,
                                cx,
                            );
                        });
                    }
                })
                .register_action(|workspace, action: &ReviewBranchDiff, window, cx| {
                    let Some(panel) = workspace.panel::<AgentPanel>(cx) else {
                        return;
                    };

                    let mention_uri = MentionUri::GitDiff {
                        base_ref: action.base_ref.to_string(),
                    };
                    let diff_uri = mention_uri.to_uri().to_string();

                    let content_blocks = vec![
                        acp::ContentBlock::Text(acp::TextContent::new(
                            "Please review this branch diff carefully. Point out any issues, \
                             potential bugs, or improvement opportunities you find.\n\n"
                                .to_string(),
                        )),
                        acp::ContentBlock::Resource(acp::EmbeddedResource::new(
                            acp::EmbeddedResourceResource::TextResourceContents(
                                acp::TextResourceContents::new(
                                    action.diff_text.to_string(),
                                    diff_uri,
                                ),
                            ),
                        )),
                    ];

                    workspace.focus_panel::<AgentPanel>(window, cx);

                    panel.update(cx, |panel, cx| {
                        panel.external_thread(
                            None,
                            None,
                            None,
                            None,
                            Some(AgentInitialContent::ContentBlock {
                                blocks: content_blocks,
                                auto_submit: true,
                            }),
                            true,
                            AgentThreadSource::GitPanel,
                            window,
                            cx,
                        );
                    });
                })
                .register_action(
                    |workspace, action: &ResolveConflictsWithAgent, window, cx| {
                        let Some(panel) = workspace.panel::<AgentPanel>(cx) else {
                            return;
                        };

                        let content_blocks = build_conflict_resolution_prompt(&action.conflicts);

                        workspace.focus_panel::<AgentPanel>(window, cx);

                        panel.update(cx, |panel, cx| {
                            panel.external_thread(
                                None,
                                None,
                                None,
                                None,
                                Some(AgentInitialContent::ContentBlock {
                                    blocks: content_blocks,
                                    auto_submit: true,
                                }),
                                true,
                                AgentThreadSource::GitPanel,
                                window,
                                cx,
                            );
                        });
                    },
                )
                .register_action(
                    |workspace, action: &ResolveConflictedFilesWithAgent, window, cx| {
                        let Some(panel) = workspace.panel::<AgentPanel>(cx) else {
                            return;
                        };

                        let content_blocks =
                            build_conflicted_files_resolution_prompt(&action.conflicted_file_paths);

                        workspace.focus_panel::<AgentPanel>(window, cx);

                        panel.update(cx, |panel, cx| {
                            panel.external_thread(
                                None,
                                None,
                                None,
                                None,
                                Some(AgentInitialContent::ContentBlock {
                                    blocks: content_blocks,
                                    auto_submit: true,
                                }),
                                true,
                                AgentThreadSource::GitPanel,
                                window,
                                cx,
                            );
                        });
                    },
                )
                .register_action(
                    |workspace: &mut Workspace, _: &AddSelectionToThread, window, cx| {
                        let active_editor = workspace
                            .active_item(cx)
                            .and_then(|item| item.act_as::<Editor>(cx));
                        let has_editor_selection = active_editor.is_some_and(|editor| {
                            editor.update(cx, |editor, cx| {
                                editor.has_non_empty_selection(&editor.display_snapshot(cx))
                            })
                        });

                        let has_terminal_selection = workspace
                            .active_item(cx)
                            .and_then(|item| item.act_as::<TerminalView>(cx))
                            .is_some_and(|terminal_view| {
                                terminal_view
                                    .read(cx)
                                    .terminal()
                                    .read(cx)
                                    .last_content
                                    .selection_text
                                    .as_ref()
                                    .is_some_and(|text| !text.is_empty())
                            });

                        let has_terminal_panel_selection =
                            workspace.panel::<TerminalPanel>(cx).is_some_and(|panel| {
                                let position = match TerminalSettings::get_global(cx).dock {
                                    TerminalDockPosition::Left => DockPosition::Left,
                                    TerminalDockPosition::Bottom => DockPosition::Bottom,
                                    TerminalDockPosition::Right => DockPosition::Right,
                                };
                                let dock_is_open =
                                    workspace.dock_at_position(position).read(cx).is_open();
                                dock_is_open && !panel.read(cx).terminal_selections(cx).is_empty()
                            });

                        if !has_editor_selection
                            && !has_terminal_selection
                            && !has_terminal_panel_selection
                        {
                            return;
                        }

                        let Some(agent_panel) = workspace.panel::<AgentPanel>(cx) else {
                            return;
                        };

                        let source = AgentContextSource::from_focused(workspace, window, cx);
                        let source = source.or_else(|| {
                            let cached = agent_panel.read(cx).last_context_source.clone()?;
                            cached.exists(workspace, cx).then_some(cached)
                        });
                        let source =
                            source.or_else(|| AgentContextSource::from_active(workspace, cx));

                        let Some(source) = source else {
                            return;
                        };

                        let Some(selection) = source.read_selection(workspace, true, cx) else {
                            return;
                        };

                        if !agent_panel.focus_handle(cx).contains_focused(window, cx) {
                            workspace.toggle_panel_focus::<AgentPanel>(window, cx);
                        }

                        agent_panel.update(cx, |panel, cx| {
                            panel.last_context_source = Some(source);
                            cx.defer_in(window, move |panel, window, cx| {
                                if let Some(conversation_view) = panel.active_conversation_view() {
                                    conversation_view.update(cx, |conversation_view, cx| {
                                        conversation_view.insert_selection(selection, window, cx);
                                    });
                                } else if let Some(terminal_id) = panel.active_terminal_id()
                                    && let Some(agent_terminal) = panel.terminals.get(&terminal_id)
                                {
                                    // Resolve mentions against the cwd: live cwd, else spawn dir.
                                    let working_directory = agent_terminal
                                        .view
                                        .read(cx)
                                        .terminal()
                                        .read(cx)
                                        .working_directory()
                                        .or_else(|| agent_terminal.working_directory.clone());
                                    let text = format_selection_for_terminal(
                                        &selection,
                                        &panel.project,
                                        working_directory.as_deref(),
                                        cx,
                                    );
                                    if !text.is_empty() {
                                        let view = agent_terminal.view.clone();
                                        view.update(cx, |view, cx| {
                                            view.terminal().update(cx, |terminal, _| {
                                                terminal.paste(&text);
                                            });
                                            window.focus(&view.focus_handle(cx), cx);
                                        });
                                    }
                                }
                            });
                        });
                    },
                )
                .register_action(|workspace, _: &menu::Cancel, _window, cx| {
                    if let Some(panel) = workspace.panel::<AgentPanel>(cx) {
                        let dismissed =
                            panel.update(cx, |panel, cx| panel.dismiss_all_notifications(cx));
                        if dismissed {
                            return;
                        }
                    }
                    cx.propagate();
                });
        },
    )
    .detach();
}

fn format_selection_for_terminal(
    selection: &AgentContextSelection,
    project: &Entity<Project>,
    working_directory: Option<&std::path::Path>,
    cx: &App,
) -> String {
    match selection {
        AgentContextSelection::Editor(ranges) => {
            let path_style = project.read(cx).path_style(cx);
            let mut parts: Vec<String> = Vec::new();
            for (buffer, range) in ranges {
                let buffer = buffer.read(cx);
                let Some(project_path) = buffer.project_path(cx) else {
                    continue;
                };
                let snapshot = buffer.snapshot();
                let point_range = range.to_point(&snapshot);
                let line_range = point_range.start.row..=point_range.end.row;
                let path = mention_path_for_terminal(
                    project,
                    &project_path,
                    working_directory,
                    path_style,
                    cx,
                );
                parts.push(format!("{path}{}", line_range_suffix(&line_range)));
            }
            if parts.is_empty() {
                String::new()
            } else {
                // Trailing space so the mention doesn't fuse with the next input.
                format!("{} ", parts.join(" "))
            }
        }
        AgentContextSelection::Terminal(texts) => texts.join("\n"),
    }
}

/// Path for a terminal mention: relative to the terminal cwd if possible, else absolute.
fn mention_path_for_terminal(
    project: &Entity<Project>,
    project_path: &ProjectPath,
    working_directory: Option<&std::path::Path>,
    path_style: util::paths::PathStyle,
    cx: &App,
) -> String {
    let abs_path = project.read(cx).absolute_path(project_path, cx);
    match (abs_path, working_directory) {
        (Some(abs_path), Some(working_directory)) => path_style
            .strip_prefix(&abs_path, working_directory)
            .map(|relative| relative.display(path_style).into_owned())
            .unwrap_or_else(|| abs_path.to_string_lossy().into_owned()),
        (Some(abs_path), None) => abs_path.to_string_lossy().into_owned(),
        (None, _) => project_path.path.display(path_style).into_owned(),
    }
}

fn conflict_resource_block(conflict: &ConflictContent) -> acp::ContentBlock {
    let mention_uri = MentionUri::MergeConflict {
        file_path: conflict.file_path.clone(),
    };
    acp::ContentBlock::Resource(acp::EmbeddedResource::new(
        acp::EmbeddedResourceResource::TextResourceContents(acp::TextResourceContents::new(
            conflict.conflict_text.clone(),
            mention_uri.to_uri().to_string(),
        )),
    ))
}

fn build_conflict_resolution_prompt(conflicts: &[ConflictContent]) -> Vec<acp::ContentBlock> {
    if conflicts.is_empty() {
        return Vec::new();
    }

    let mut blocks = Vec::new();

    if conflicts.len() == 1 {
        let conflict = &conflicts[0];

        blocks.push(acp::ContentBlock::Text(acp::TextContent::new(
            "Please resolve the following merge conflict in ",
        )));
        let mention = MentionUri::File {
            abs_path: PathBuf::from(conflict.file_path.clone()),
        };
        blocks.push(acp::ContentBlock::ResourceLink(acp::ResourceLink::new(
            mention.name(),
            mention.to_uri(),
        )));

        blocks.push(acp::ContentBlock::Text(acp::TextContent::new(
            indoc::formatdoc!(
                "\nThe conflict is between branch `{ours}` (ours) and `{theirs}` (theirs).

                Analyze both versions carefully and resolve the conflict by editing \
                the file directly. Choose the resolution that best preserves the intent \
                of both changes, or combine them if appropriate.

                ",
                ours = conflict.ours_branch_name,
                theirs = conflict.theirs_branch_name,
            ),
        )));
    } else {
        let n = conflicts.len();
        let unique_files: HashSet<&str> = conflicts.iter().map(|c| c.file_path.as_str()).collect();
        let ours = &conflicts[0].ours_branch_name;
        let theirs = &conflicts[0].theirs_branch_name;
        blocks.push(acp::ContentBlock::Text(acp::TextContent::new(
            indoc::formatdoc!(
                "Please resolve all {n} merge conflicts below.

                The conflicts are between branch `{ours}` (ours) and `{theirs}` (theirs).

                For each conflict, analyze both versions carefully and resolve them \
                by editing the file{suffix} directly. Choose resolutions that best preserve \
                the intent of both changes, or combine them if appropriate.

                ",
                suffix = if unique_files.len() > 1 { "s" } else { "" },
            ),
        )));
    }

    for conflict in conflicts {
        blocks.push(conflict_resource_block(conflict));
    }

    blocks
}

fn build_conflicted_files_resolution_prompt(
    conflicted_file_paths: &[String],
) -> Vec<acp::ContentBlock> {
    if conflicted_file_paths.is_empty() {
        return Vec::new();
    }

    let instruction = indoc::indoc!(
        "The following files have unresolved merge conflicts. Please open each \
         file, find the conflict markers (`<<<<<<<` / `=======` / `>>>>>>>`), \
         and resolve every conflict by editing the files directly.

         Choose resolutions that best preserve the intent of both changes, \
         or combine them if appropriate.

         Files with conflicts:
         ",
    );

    let mut content = vec![acp::ContentBlock::Text(acp::TextContent::new(instruction))];
    for path in conflicted_file_paths {
        let mention = MentionUri::File {
            abs_path: PathBuf::from(path),
        };
        content.push(acp::ContentBlock::ResourceLink(acp::ResourceLink::new(
            mention.name(),
            mention.to_uri(),
        )));
        content.push(acp::ContentBlock::Text(acp::TextContent::new("\n")));
    }
    content
}

fn format_timestamp_human(dt: &DateTime<Utc>) -> String {
    let now = Utc::now();
    let duration = now.signed_duration_since(*dt);

    let relative = if duration.num_seconds() < 0 {
        "in the future".to_string()
    } else if duration.num_seconds() < 60 {
        let seconds = duration.num_seconds();
        format!("{seconds} seconds ago")
    } else if duration.num_minutes() < 60 {
        let minutes = duration.num_minutes();
        format!("{minutes} minutes ago")
    } else if duration.num_hours() < 24 {
        let hours = duration.num_hours();
        format!("{hours} hours ago")
    } else {
        let days = duration.num_days();
        format!("{days} days ago")
    };

    format!("{} ({})", dt.to_rfc3339(), relative)
}

/// Used for `dev: show thread metadata` action
fn thread_metadata_to_debug_json(
    metadata: &crate::thread_metadata_store::ThreadMetadata,
) -> serde_json::Value {
    serde_json::json!({
        "thread_id": metadata.thread_id,
        "session_id": metadata.session_id.as_ref().map(|s| s.0.to_string()),
        "agent_id": metadata.agent_id.0.to_string(),
        "title": metadata.title.as_ref().map(|t| t.to_string()),
        "title_override": metadata.title_override.as_ref().map(|t| t.to_string()),
        "updated_at": format_timestamp_human(&metadata.updated_at),
        "created_at": metadata.created_at.as_ref().map(format_timestamp_human),
        "interacted_at": metadata.interacted_at.as_ref().map(format_timestamp_human),
        "worktree_paths": format!("{:?}", metadata.worktree_paths),
        "archived": metadata.archived,
    })
}

/// Optional parameters for `AgentPanel::create_thread_with_options`. All
/// fields default to the panel's current selection so the agent tool only
/// needs to override what it actually cares about.
#[derive(Default)]
pub struct CreateThreadOptions {
    /// Title to assign to the new thread up front.
    pub title: Option<SharedString>,
    /// Initial content to populate in the thread (optionally auto-submitted).
    pub initial_content: Option<AgentInitialContent>,
    /// Agent to use. Defaults to the panel's selected agent.
    pub agent: Option<Agent>,
    /// Model override, as `provider/model-id`. Only applied when the thread
    /// uses the native Omega Agent executor.
    pub model: Option<String>,
    /// Working directories to attach to the new thread (e.g., the path of a
    /// freshly-created sibling worktree). When `None`, the thread inherits
    /// the project's default path list.
    pub work_dirs: Option<PathList>,
}

pub(crate) struct AgentThread {
    conversation_view: Entity<ConversationView>,
}

struct AgentTerminal {
    view: Entity<TerminalView>,
    title_editor: Option<Entity<Editor>>,
    title_editor_initial_title: Option<String>,
    title_editor_subscription: Option<Subscription>,
    last_known_title: String,
    last_known_terminal_title: String,
    last_observed_program: Option<String>,
    working_directory: Option<PathBuf>,
    created_at: DateTime<Utc>,
    has_notification: bool,
    search_bar: Option<Entity<BufferSearchBar>>,
    notification_windows: Vec<WindowHandle<AgentNotification>>,
    notification_subscriptions: Vec<Subscription>,
    _subscriptions: Vec<Subscription>,
}

impl AgentTerminal {
    fn terminal_title_for_view(view: &TerminalView, cx: &App) -> SharedString {
        let terminal = view.terminal().read(cx);
        if terminal.breadcrumb_text.is_empty() {
            let title = terminal.title(true);
            if title == "Terminal" {
                SharedString::from("")
            } else {
                title.into()
            }
        } else {
            terminal.breadcrumb_text.clone().into()
        }
    }

    fn current_terminal_title(&self, cx: &App) -> SharedString {
        let view = self.view.read(cx);
        Self::terminal_title_for_view(view, cx)
    }

    fn terminal_title(&self, cx: &App) -> SharedString {
        let title = self.current_terminal_title(cx);
        if title.is_empty() && !self.last_known_terminal_title.is_empty() {
            SharedString::from(self.last_known_terminal_title.clone())
        } else {
            title
        }
    }

    fn title(&self, cx: &App) -> SharedString {
        let terminal_title = self.terminal_title(cx);
        let custom_title = self.custom_title(cx);
        compose_terminal_thread_title(
            terminal_title.as_ref(),
            custom_title.as_ref().map(|title| title.as_ref()),
        )
    }

    fn editable_title(&self, cx: &App) -> SharedString {
        if let Some(custom_title) = self.custom_title(cx) {
            custom_title
        } else {
            let terminal_title = self.terminal_title(cx);
            SharedString::from(terminal_title_without_prefix(terminal_title.as_ref()).to_string())
        }
    }

    fn refresh_title(&mut self, cx: &mut App) -> bool {
        let terminal_title = self.current_terminal_title(cx);
        if !terminal_title.is_empty() {
            self.last_known_terminal_title = terminal_title.to_string();
        }

        let title = self.title(cx);
        let changed = self.last_known_title != title.as_ref();
        if changed {
            self.last_known_title = title.to_string();
        }
        changed
    }

    fn refresh_metadata(&mut self, cx: &mut App) -> bool {
        let title_changed = self.refresh_title(cx);
        let current_working_directory = self.view.read(cx).terminal().read(cx).working_directory();
        let working_directory_changed = current_working_directory
            .as_ref()
            .is_some_and(|current| self.working_directory.as_ref() != Some(current));
        if working_directory_changed {
            self.working_directory = current_working_directory;
        }
        title_changed || working_directory_changed
    }

    fn custom_title(&self, cx: &App) -> Option<SharedString> {
        self.view.read(cx).custom_title().map(SharedString::from)
    }

    fn report_started_terminal_program(
        &mut self,
        terminal_id: TerminalId,
        source: AgentThreadSource,
        cx: &App,
    ) {
        let current_program = self
            .view
            .read(cx)
            .terminal()
            .read(cx)
            .foreground_process_command_name();

        if let Some(program) =
            terminal_program_to_report(&mut self.last_observed_program, current_program)
        {
            telemetry::event!(
                "Agent Terminal Program Started",
                agent = TERMINAL_AGENT_TELEMETRY_ID,
                terminal_id = terminal_id.to_key_string(),
                program = program,
                source = source.as_str(),
                side = crate::agent_sidebar_side(cx),
                thread_location = "current_worktree",
            );
        }
    }
}

enum BaseView {
    Uninitialized,
    AgentThread {
        conversation_view: Entity<ConversationView>,
    },
    Terminal {
        terminal_id: TerminalId,
    },
}

impl From<AgentThread> for BaseView {
    fn from(thread: AgentThread) -> Self {
        BaseView::AgentThread {
            conversation_view: thread.conversation_view,
        }
    }
}

enum VisibleSurface<'a> {
    Uninitialized,
    AgentThread(&'a Entity<ConversationView>),
    Terminal(&'a Entity<TerminalView>),
}

enum WhichFontSize {
    AgentFont,
    None,
}

impl BaseView {
    pub fn which_font_size_used(&self) -> WhichFontSize {
        match self {
            BaseView::AgentThread { .. } => WhichFontSize::AgentFont,
            BaseView::Terminal { .. } | BaseView::Uninitialized => WhichFontSize::None,
        }
    }
}

enum DevicePairingSurface {
    Ready {
        bootstrap: omega_effectd::PairingBootstrap,
        image: Arc<RenderImage>,
        image_size: Pixels,
    },
    Unavailable(SharedString),
}

const PAIRING_QR_MODULE_SIZE: u32 = 3;

fn render_pairing_qr(pairing_qr: &omega_effectd::PairingQr) -> Result<(Arc<RenderImage>, Pixels)> {
    let module_count = pairing_qr
        .width
        .checked_mul(pairing_qr.width)
        .context("pairing QR dimensions overflow")?;
    anyhow::ensure!(
        pairing_qr.modules.len() == module_count,
        "pairing QR has invalid module dimensions"
    );
    anyhow::ensure!(pairing_qr.width > 0, "pairing QR is empty");

    let module_width =
        u32::try_from(pairing_qr.width).context("pairing QR is too wide to render")?;
    let image_width = module_width
        .checked_mul(PAIRING_QR_MODULE_SIZE)
        .context("pairing QR image dimensions overflow")?;
    let buffer = image::ImageBuffer::from_fn(image_width, image_width, |image_x, image_y| {
        let module_x = (image_x / PAIRING_QR_MODULE_SIZE) as usize;
        let module_y = (image_y / PAIRING_QR_MODULE_SIZE) as usize;
        let module_index = module_y * pairing_qr.width + module_x;
        if pairing_qr.modules.get(module_index) == Some(&true) {
            image::Rgba([0, 0, 0, 255])
        } else {
            image::Rgba([255, 255, 255, 255])
        }
    });
    Ok((
        Arc::new(RenderImage::new(vec![image::Frame::new(buffer)])),
        px(image_width as f32),
    ))
}

fn render_ready_device_pairing_content(
    magic_dns_name: &str,
    port: u16,
    image: Arc<RenderImage>,
    image_size: Pixels,
) -> AnyElement {
    v_flex()
        .items_center()
        .gap_2()
        .child(
            div().p_2().bg(Hsla::white()).child(
                img(ImageSource::Render(image))
                    .size(image_size)
                    .object_fit(ObjectFit::Fill),
            ),
        )
        .child(
            Label::new(format!("{magic_dns_name}:{port}"))
                .size(LabelSize::XSmall)
                .color(Color::Muted),
        )
        .child(
            Label::new("Scan in OpenAgents Mobile. This code works once for 5 minutes.")
                .size(LabelSize::XSmall)
                .color(Color::Muted),
        )
        .into_any_element()
}

#[derive(Clone)]
struct ThreadIdentityOperationError {
    source_binding: Option<omega_workbench_state::RepositoryBinding>,
    attempted_binding: omega_workbench_state::RepositoryBinding,
    binding_generation: u64,
    request_id: u64,
    inconsistent: bool,
    message: SharedString,
}

pub struct AgentPanel {
    workspace: WeakEntity<Workspace>,
    /// Workspace id is used as a database key
    workspace_id: Option<WorkspaceId>,
    project: Entity<Project>,
    fs: Arc<dyn Fs>,
    language_registry: Arc<LanguageRegistry>,
    thread_store: Entity<ThreadStore>,
    connection_store: Entity<AgentConnectionStore>,
    context_server_registry: Entity<ContextServerRegistry>,
    focus_handle: FocusHandle,
    base_view: BaseView,
    last_created_entry_kind: AgentPanelEntryKind,
    draft_thread: Option<Entity<ConversationView>>,
    retained_threads: HashMap<ThreadId, Entity<ConversationView>>,
    terminals: HashMap<TerminalId, AgentTerminal>,
    pending_terminal_spawn: Option<TerminalId>,
    new_thread_menu_handle: PopoverMenuHandle<ContextMenu>,
    agent_panel_menu_handle: PopoverMenuHandle<ContextMenu>,
    thread_repository_menu_handle: PopoverMenuHandle<ContextMenu>,
    thread_worktree_menu_handle: PopoverMenuHandle<ContextMenu>,
    thread_branch_menu_handle: PopoverMenuHandle<git_ui::branch_picker::BranchList>,
    _extension_subscription: Option<Subscription>,
    _project_subscription: Subscription,
    _git_store_subscription: Subscription,
    thread_identity_observation_revision: u64,
    thread_identity_operation_request: u64,
    thread_identity_operation_requests: HashMap<String, u64>,
    thread_identity_pending_operations: HashMap<String, u64>,
    thread_identity_operation_errors: HashMap<String, ThreadIdentityOperationError>,
    zoomed: bool,
    pending_serialization: Option<Task<Result<()>>>,
    new_user_onboarding: Entity<AgentPanelOnboarding>,
    new_user_onboarding_upsell_dismissed: AtomicBool,
    selected_agent: Agent,
    _thread_view_subscription: Option<Subscription>,
    _active_thread_focus_subscription: Option<Subscription>,
    _base_view_observation: Option<Subscription>,
    _draft_editor_observation: Option<Subscription>,
    _active_draft_reclaim_observation: Option<Subscription>,
    _thread_metadata_store_subscription: Subscription,
    last_context_source: Option<AgentContextSource>,

    is_active: bool,

    /// The Full Auto launch and run surface, folded in from its retired dock
    /// panel. `OMEGA-DELTA-0020`.
    ///
    /// Created on first use and then retained, because the dock panel it
    /// replaces kept its draft text and selected run across a visit to a chat
    /// thread. Dropping it on every hide would be a capability regression
    /// wearing the costume of a cleanup.
    full_auto: Option<Entity<FullAutoPanel>>,
    /// Whether the Full Auto surface is the one currently on screen.
    ///
    /// Separate from `full_auto` for the reason above: hidden and absent are
    /// different states.
    showing_full_auto: bool,
    /// `OMEGA-DELTA-0118`, widened by `OMEGA-DELTA-0130`. What zero base's
    /// sidebar is showing, and which of its sections are collapsed.
    ///
    /// Values rather than a retained view: the sidebar holds nothing a person
    /// would lose by collapsing it. The thread rows are a pure function of the
    /// metadata store, recomputed on each render, so a thread renamed or
    /// archived while it is open cannot leave a stale row behind.
    ///
    /// This is the *preference*, not the layout. A window too narrow to hold
    /// both the sidebar and a composer draws a rail without touching this, so
    /// widening the window restores what the person asked for.
    sidebar: omega_sidebar::SidebarState,
    /// Versioned public-channel destinations and their independent snapshots.
    public_channels: crate::omega_public_channels::PublicChannelController,
    /// A bounded in-place registry load failure.
    public_channels_error: Option<SharedString>,
    /// Held so the manifest load dies with the panel.
    _public_channels_load: Option<Task<()>>,
    /// Relay-qualified channel views retain verified rows across selection
    /// changes. Only the selected view keeps its relay session active.
    public_channel_views:
        HashMap<String, Entity<crate::omega_public_channel_view::PublicChannelView>>,
    /// Snapshot events keep sidebar lifecycle, cursor, and unread state in
    /// agreement with each retained channel view.
    _public_channel_view_subscriptions: HashMap<String, Subscription>,
    /// The sentence the last refused reopen produced, if the sidebar is showing
    /// one.
    ///
    /// Rendered inside the sidebar rather than as a toast. The person is
    /// already looking at the list they clicked, and `OMEGA-DELTA-0053` records
    /// that a zero-base window is exactly where a notification is least likely
    /// to be where somebody is looking.
    threads_sidebar_refusal: Option<SharedString>,
    device_pairing_surface: Option<DevicePairingSurface>,
    /// `OMEGA-DELTA-0035`. The poll that feeds the router the engine's framed
    /// `get_capacity` answer.
    ///
    /// Held so it dies with the panel. It writes nothing to the engine and
    /// keeps no run state: `omega-effectd` remains the sole run authority and
    /// this only lets the router know whether an engine-lane pin is honourable
    /// before it decides.
    _engine_capacity_poll: Option<Task<()>>,
    thread_outline: Entity<crate::thread_outline::ThreadOutline>,
    #[cfg(any(test, feature = "test-support"))]
    thread_outline_navigation_target: Option<(acp_thread::ThreadEntryId, usize)>,
    workbench_shell: workbench_shell::WorkbenchShell,
    workbench_shell_enabled: bool,
    workbench_files_panel: Option<Entity<ProjectPanel>>,
    workbench_files_panel_handed_off: bool,
    _workbench_files_panel_observation: Option<Subscription>,
    _workbench_files_panel_event_subscription: Option<Subscription>,
    workbench_git_panel: Option<Entity<GitPanel>>,
    workbench_git_panel_handed_off: bool,
    _workbench_git_panel_observation: Option<Subscription>,
    _workbench_git_panel_event_subscription: Option<Subscription>,
    workbench_terminal_panel: Option<Entity<TerminalPanel>>,
    workbench_terminal_panel_handed_off: bool,
    workbench_terminal_handlers_installed: bool,
    _workbench_terminal_panel_observation: Option<Subscription>,
    _workbench_terminal_panel_event_subscription: Option<Subscription>,
    workbench_terminal_surface: Option<Entity<workbench_shell::NativeTerminalSurface>>,
    #[cfg(any(test, feature = "test-support"))]
    workbench_identity_phase_override: Option<IdentityPhase>,
    #[cfg(any(test, feature = "test-support"))]
    workbench_identity_observation_override: Option<ThreadIdentityObservation>,
    #[cfg(any(test, feature = "test-support"))]
    workbench_git_lifecycle_override: Option<workbench_shell::NativeGitLifecycle>,
    #[cfg(any(test, feature = "test-support"))]
    workbench_terminal_owner_state_override: Option<workbench_shell::NativeTerminalOwnerState>,
    #[cfg(any(test, feature = "test-support"))]
    workbench_plan_lifecycle_override: Option<workbench_shell::NativePlanLifecycle>,
    #[cfg(any(test, feature = "test-support"))]
    workbench_plan_navigation_target: Option<usize>,
    #[cfg(any(test, feature = "test-support"))]
    workbench_terminal_badge_override: Option<workbench_shell::SurfaceBadge>,
}

impl AgentPanel {
    fn serialize(&mut self, cx: &mut App) {
        let Some(workspace_id) = self.workspace_id else {
            return;
        };

        let selected_agent = self.selected_agent.clone();
        let last_created_entry_kind = self.last_created_entry_kind;
        let last_active_terminal_id = self
            .active_terminal_id()
            .map(|terminal_id| terminal_id.to_key_string());

        let last_active_thread = if last_active_terminal_id.is_some() {
            None
        } else {
            let is_draft_active = self.active_thread_is_draft(cx);
            let active_thread_id = self.active_thread_id(cx);
            let active_thread_agent = self
                .active_conversation_view()
                .map(|cv| cv.read(cx).agent_key().clone())
                .unwrap_or_else(|| self.selected_agent.clone());
            self.active_agent_thread(cx)
                .map(|thread| {
                    let thread = thread.read(cx);

                    let title = thread.title();
                    let work_dirs = thread.work_dirs().cloned();
                    SerializedActiveThread {
                        session_id: (!is_draft_active).then(|| thread.session_id().0.to_string()),
                        thread_id: active_thread_id,
                        agent_type: active_thread_agent.clone(),
                        title: title.map(|t| t.to_string()),
                        work_dirs: work_dirs.map(|dirs| dirs.serialize()),
                    }
                })
                .or_else(|| {
                    // The active view may be in `Loading` or `LoadError` — for
                    // example, while a restored thread is waiting for a custom
                    // agent to finish registering. Without this fallback, a
                    // stray `serialize()` triggered during that window would
                    // write `session_id=None` and wipe the restored session
                    if is_draft_active {
                        return None;
                    }
                    let conversation_view = self.active_conversation_view()?;
                    let session_id = conversation_view.read(cx).root_session_id.clone()?;
                    let metadata = ThreadMetadataStore::try_global(cx)
                        .and_then(|store| store.read(cx).entry_by_session(&session_id).cloned());
                    Some(SerializedActiveThread {
                        session_id: Some(session_id.0.to_string()),
                        thread_id: active_thread_id,
                        agent_type: active_thread_agent.clone(),
                        title: metadata
                            .as_ref()
                            .and_then(|m| m.title.as_ref())
                            .map(|t| t.to_string()),
                        work_dirs: metadata.map(|m| m.folder_paths().serialize()),
                    })
                })
        };

        let new_draft_thread_id = self
            .draft_thread
            .as_ref()
            .map(|draft| draft.read(cx).thread_id);

        let kvp = KeyValueStore::global(cx);
        self.pending_serialization = Some(cx.background_spawn(async move {
            save_serialized_panel(
                workspace_id,
                SerializedAgentPanel {
                    selected_agent: Some(selected_agent),
                    last_created_entry_kind,
                    last_active_thread,
                    last_active_terminal_id,
                    new_draft_thread_id,
                },
                kvp,
            )
            .await?;
            anyhow::Ok(())
        }));
    }

    pub fn load(
        workspace: WeakEntity<Workspace>,
        mut cx: AsyncWindowContext,
    ) -> Task<Result<Entity<Self>>> {
        let kvp = cx.update(|_window, cx| KeyValueStore::global(cx)).ok();
        cx.spawn(async move |cx| {
            let workspace_id = workspace
                .read_with(cx, |workspace, _| workspace.database_id())
                .ok()
                .flatten();

            let (serialized_panel, global_last_used_agent, global_last_created_entry_kind) = cx
                .background_spawn(async move {
                    match kvp {
                        Some(kvp) => {
                            let panel = workspace_id
                                .and_then(|id| read_serialized_panel(id, &kvp))
                                .or_else(|| read_legacy_serialized_panel(&kvp));
                            let global_agent = read_global_last_used_agent(&kvp);
                            let global_entry_kind = read_global_last_created_entry_kind(&kvp);
                            (panel, global_agent, global_entry_kind)
                        }
                        None => (None, None, None),
                    }
                })
                .await;

            let has_open_project = workspace
                .read_with(cx, |workspace, cx| !workspace.root_paths(cx).is_empty())
                .unwrap_or(false);
            let terminal_id_to_restore = if has_open_project {
                serialized_panel
                    .as_ref()
                    .and_then(|panel| panel.last_active_terminal_id.as_deref())
                    .and_then(|terminal_id| {
                        match TerminalId::from_key_string(terminal_id) {
                            Ok(terminal_id) => Some(terminal_id),
                            Err(error) => {
                                log::warn!("failed to parse last active terminal id: {error}");
                                None
                            }
                        }
                    })
            } else {
                None
            };
            let terminal_to_restore = if let Some(terminal_id) = terminal_id_to_restore {
                match cx.update(|_window, cx| {
                    TerminalThreadMetadataStore::try_global(cx).map(|store| {
                        let reload_task = store.read(cx).reload_task();
                        (store, reload_task)
                    })
                }) {
                    Ok(Some((store, reload_task))) => {
                        reload_task.await;
                        match store
                            .read_with(cx, |store, _cx| store.entry(terminal_id).cloned())
                        {
                            Some(metadata) => Some(metadata),
                            None => {
                                log::info!(
                                    "last active terminal is missing, skipping restoration"
                                );
                                None
                            }
                        }
                    }
                    Ok(None) => {
                        log::warn!("failed to restore active terminal: metadata store missing");
                        None
                    }
                    Err(err) => {
                        log::warn!("failed to access terminal metadata store: {err}");
                        None
                    }
                }
            } else {
                None
            };

            let thread_to_restore = if has_open_project && terminal_to_restore.is_none() {
                if let Some(info) = serialized_panel
                    .as_ref()
                    .and_then(|panel| panel.last_active_thread.as_ref())
                {
                    match cx.update(|_window, cx| {
                        ThreadMetadataStore::try_global(cx).map(|store| {
                            let reload_task = store.read(cx).reload_task();
                            (store, reload_task)
                        })
                    }) {
                        Ok(Some((store, reload_task))) => {
                            reload_task.await;
                            let thread_id = store.read_with(cx, |store, _cx| {
                                let primary = info.thread_id.and_then(|tid| store.entry(tid));
                                let fallback = info.session_id.as_ref().and_then(|sid| {
                                    store.entry_by_session(&acp::SessionId::new(sid.clone()))
                                });
                                primary
                                    .or(fallback)
                                    .filter(|entry| !entry.archived)
                                    .map(|entry| entry.thread_id)
                            });
                            match thread_id {
                                Some(thread_id) => Some((info, thread_id)),
                                None => {
                                    log::info!(
                                        "last active thread is archived or missing, skipping restoration"
                                    );
                                    None
                                }
                            }
                        }
                        Ok(None) => {
                            log::warn!("failed to restore active thread: metadata store missing");
                            None
                        }
                        Err(err) => {
                            log::warn!("failed to access thread metadata store: {err}");
                            None
                        }
                    }
                } else {
                    None
                }
            } else {
                None
            };

            let panel = workspace.update_in(cx, |workspace, window, cx| {
                let panel = cx.new(|cx| Self::new(workspace, window, cx));

                panel.update(cx, |panel, cx| {
                    let is_via_collab = panel.project.read(cx).is_via_collab();
                    // Collab workspaces only support NativeAgent; clamp any
                    // non-native choice so `set_active` can't bypass the
                    // collab guard in `external_thread`.
                    let clamp = |agent: Agent| {
                        if is_via_collab && !agent.is_native() {
                            Agent::NativeAgent
                        } else {
                            agent
                        }
                    };
                    let global_fallback =
                        global_last_used_agent.filter(|agent| !is_via_collab || agent.is_native());

                    if let Some(serialized_panel) = &serialized_panel {
                        panel.last_created_entry_kind = serialized_panel.last_created_entry_kind;
                    } else if let Some(entry_kind) = global_last_created_entry_kind {
                        panel.last_created_entry_kind = entry_kind;
                    }

                    // The thread being restored may have been bound to an
                    // agent different from the panel's last selected one
                    // (e.g. a draft created while a different agent was
                    // active). When restoring a thread, prefer its agent
                    // so the draft survives reload bound to the right
                    // backend; otherwise fall back to the serialized
                    // selection, then the global last-used agent.
                    let initial_agent = match &thread_to_restore {
                        Some((info, _)) => Some(clamp(info.agent_type.clone())),
                        None => serialized_panel
                            .as_ref()
                            .and_then(|p| p.selected_agent.clone())
                            .map(clamp)
                            .or(global_fallback),
                    };
                    if let Some(agent) = initial_agent {
                        panel.selected_agent = agent;
                    }

                    if let Some(metadata) = terminal_to_restore {
                        panel.restore_terminal_for_panel_load(
                            metadata,
                            false,
                            AgentThreadSource::AgentPanel,
                            Some(workspace),
                            window,
                            cx,
                        );
                    } else if let Some((info, thread_id)) = thread_to_restore {
                        let agent = panel.selected_agent.clone();
                        panel.load_agent_thread(
                            agent,
                            thread_id,
                            info.work_dirs.as_ref().map(PathList::deserialize),
                            info.title.clone().map(Into::into),
                            false,
                            AgentThreadSource::AgentPanel,
                            window,
                            cx,
                        );
                    }
                    if let Some(new_draft_thread_id) = serialized_panel
                        .as_ref()
                        .and_then(|p| p.new_draft_thread_id)
                    {
                        panel.restore_new_draft(new_draft_thread_id, window, cx);
                    }
                    cx.notify();
                });

                panel
            })?;

            Ok(panel)
        })
    }

    pub(crate) fn new(workspace: &Workspace, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let fs = workspace.app_state().fs.clone();
        let project = workspace.project();
        let language_registry = project.read(cx).languages().clone();
        let workspace_id = workspace.database_id();
        let workbench_files_panel = workspace.panel::<ProjectPanel>(cx);
        let workbench_files_panel_observation = workbench_files_panel
            .as_ref()
            .map(|panel| cx.observe(panel, |_this, _panel, cx| cx.notify()));
        let workbench_files_panel_event_subscription =
            workbench_files_panel.as_ref().map(|panel| {
                cx.subscribe_in(
                    panel,
                    window,
                    |this, _panel, event: &PanelEvent, window, cx| {
                        this.handle_workbench_files_panel_event(event, window, cx);
                    },
                )
            });
        let workbench_git_panel = workspace.panel::<GitPanel>(cx);
        let workbench_git_panel_observation = workbench_git_panel.as_ref().map(|panel| {
            cx.observe_in(panel, window, |this, _panel, window, cx| {
                this.synchronize_git_surface_lifecycle_for_panel(cx);
                this.sync_workbench_shell(window, cx);
                cx.notify();
            })
        });
        let workbench_git_panel_event_subscription = workbench_git_panel.as_ref().map(|panel| {
            cx.subscribe_in(
                panel,
                window,
                |this, _panel, event: &PanelEvent, window, cx| {
                    this.handle_workbench_git_panel_event(event, window, cx);
                },
            )
        });
        let workbench_terminal_panel = workspace.panel::<TerminalPanel>(cx);
        let workbench_terminal_panel_observation = workbench_terminal_panel.as_ref().map(|panel| {
            cx.observe_in(panel, window, |this, _panel, window, cx| {
                this.sync_workbench_shell(window, cx);
                cx.notify();
            })
        });
        let workbench_terminal_panel_event_subscription =
            workbench_terminal_panel.as_ref().map(|panel| {
                cx.subscribe_in(
                    panel,
                    window,
                    |this, _panel, event: &PanelEvent, window, cx| {
                        this.handle_workbench_terminal_panel_event(event, window, cx);
                    },
                )
            });
        let workspace = workspace.weak_handle();

        let context_server_registry =
            cx.new(|cx| ContextServerRegistry::new(project.read(cx).context_server_store(), cx));

        let thread_store = ThreadStore::global(cx);

        let base_view = BaseView::Uninitialized;

        let weak_panel = cx.entity().downgrade();
        let onboarding = cx.new(|cx| {
            AgentPanelOnboarding::new(
                move |_window, cx| {
                    weak_panel
                        .update(cx, |panel, cx| {
                            panel.dismiss_ai_onboarding(cx);
                        })
                        .ok();
                },
                cx,
            )
        });

        // Subscribe to extension events to sync agent servers when extensions change
        let extension_subscription = ExtensionStore::try_global(cx).map(|store| {
            cx.subscribe(&store, |this, _source, event, cx| match event {
                extension_host::Event::ExtensionUninstalled(id) => {
                    this.migrate_agent_server_from_extensions(id.clone(), cx);
                }
                _ => {}
            })
        });

        let connection_store = cx.new(|cx| AgentConnectionStore::new(project.clone(), cx));
        let _project_subscription =
            cx.subscribe(&project, |this, _project, event, cx| match event {
                project::Event::WorktreeAdded(_)
                | project::Event::WorktreeRemoved(_)
                | project::Event::WorktreeOrderChanged
                | project::Event::WorktreePathsChanged { .. } => {
                    this.thread_identity_observation_revision =
                        this.thread_identity_observation_revision.saturating_add(1);
                    this.ensure_native_agent_connection(cx);
                    this.update_thread_work_dirs(cx);
                    this.persist_all_terminal_metadata(cx);
                    cx.notify();
                }
                project::Event::DisconnectedFromHost
                | project::Event::DisconnectedFromRemote { .. }
                | project::Event::HostReshared
                | project::Event::Reshared
                | project::Event::Rejoined => {
                    this.thread_identity_observation_revision =
                        this.thread_identity_observation_revision.saturating_add(1);
                    cx.notify();
                }
                _ => {}
            });
        let git_store = project.read_with(cx, |project, _cx| project.git_store().clone());
        let _git_store_subscription = cx.subscribe(&git_store, |this, _git_store, _event, cx| {
            this.thread_identity_observation_revision =
                this.thread_identity_observation_revision.saturating_add(1);
            cx.notify();
        });

        let _thread_metadata_store_subscription = cx.subscribe(
            &ThreadMetadataStore::global(cx),
            |this, _store, event, cx| {
                let ThreadMetadataStoreEvent::ThreadArchived(thread_id) = event;
                if this.retained_threads.remove(thread_id).is_some() {
                    cx.notify();
                }
            },
        );

        cx.on_release(|this, cx| {
            this.dismiss_all_terminal_notifications(cx);
        })
        .detach();

        let workbench_shell = workbench_shell::WorkbenchShell::new(cx);
        let thread_outline = cx.new(crate::thread_outline::ThreadOutline::new);
        {
            let panel = cx.entity().downgrade();
            thread_outline.update(cx, |outline, _cx| {
                outline.set_navigation_handler(Rc::new(move |item, window, cx| {
                    panel
                        .update(cx, |panel, cx| {
                            panel.navigate_to_outline_entry(&item, window, cx)
                        })
                        .unwrap_or(false)
                }));
            });
        }
        {
            let panel = cx.entity().downgrade();
            thread_outline.update(cx, |outline, _cx| {
                outline.set_artifact_action_handler(Rc::new(move |item, window, cx| {
                    panel
                        .update(cx, |panel, cx| {
                            panel.activate_outline_artifact(item, window, cx)
                        })
                        .unwrap_or_else(|_| {
                            OutlineActionOutcome::Unavailable(
                                "The artifact panel is no longer available".into(),
                            )
                        })
                }));
            });
        }
        {
            let panel = cx.entity().downgrade();
            thread_outline.update(cx, |outline, _cx| {
                outline.set_artifact_source_navigation_handler(Rc::new(
                    move |item, source_event, entry_index, window, cx| {
                        panel
                            .update(cx, |panel, cx| {
                                panel.navigate_to_outline_artifact_source(
                                    item,
                                    source_event,
                                    entry_index,
                                    window,
                                    cx,
                                )
                            })
                            .unwrap_or(false)
                    },
                ));
            });
        }
        let panel = Self {
            workspace_id,
            base_view,
            last_created_entry_kind: AgentPanelEntryKind::Thread,
            workspace,
            project: project.clone(),
            fs: fs.clone(),
            language_registry,
            connection_store,
            focus_handle: cx.focus_handle(),
            context_server_registry,
            draft_thread: None,
            retained_threads: HashMap::default(),
            terminals: HashMap::default(),
            pending_terminal_spawn: None,
            new_thread_menu_handle: PopoverMenuHandle::default(),
            agent_panel_menu_handle: PopoverMenuHandle::default(),
            thread_repository_menu_handle: PopoverMenuHandle::default(),
            thread_worktree_menu_handle: PopoverMenuHandle::default(),
            thread_branch_menu_handle: PopoverMenuHandle::default(),

            _extension_subscription: extension_subscription,
            _project_subscription,
            _git_store_subscription,
            thread_identity_observation_revision: 0,
            thread_identity_operation_request: 0,
            thread_identity_operation_requests: HashMap::default(),
            thread_identity_pending_operations: HashMap::default(),
            thread_identity_operation_errors: HashMap::default(),
            zoomed: false,
            pending_serialization: None,
            new_user_onboarding: onboarding,
            thread_store,
            selected_agent: Agent::default(),
            _thread_view_subscription: None,
            _active_thread_focus_subscription: None,
            new_user_onboarding_upsell_dismissed: AtomicBool::new(OnboardingUpsell::dismissed(cx)),
            _base_view_observation: None,
            _draft_editor_observation: None,
            _active_draft_reclaim_observation: None,
            _thread_metadata_store_subscription,
            last_context_source: None,
            is_active: false,
            full_auto: None,
            showing_full_auto: false,
            // OMEGA-DELTA-0130. Default open, per "default open on the
            // zerobase chat page" — `from_stored` answers that for a machine
            // that has never stored a state, and answers with what the person
            // last chose for one that has.
            sidebar: omega_sidebar::SidebarState::from_stored(
                KeyValueStore::global(cx)
                    .read_kvp(omega_sidebar::STATE_KEY)
                    .log_err()
                    .flatten()
                    .as_deref(),
            ),
            public_channels: crate::omega_public_channels::PublicChannelController::empty(),
            public_channels_error: None,
            _public_channels_load: None,
            public_channel_views: HashMap::default(),
            _public_channel_view_subscriptions: HashMap::default(),
            threads_sidebar_refusal: None,
            device_pairing_surface: None,
            _engine_capacity_poll: None,
            thread_outline,
            #[cfg(any(test, feature = "test-support"))]
            thread_outline_navigation_target: None,
            workbench_shell,
            workbench_shell_enabled: omega_zero_base::is_active(),
            workbench_files_panel,
            workbench_files_panel_handed_off: false,
            _workbench_files_panel_observation: workbench_files_panel_observation,
            _workbench_files_panel_event_subscription: workbench_files_panel_event_subscription,
            workbench_git_panel,
            workbench_git_panel_handed_off: false,
            _workbench_git_panel_observation: workbench_git_panel_observation,
            _workbench_git_panel_event_subscription: workbench_git_panel_event_subscription,
            workbench_terminal_panel,
            workbench_terminal_panel_handed_off: false,
            workbench_terminal_handlers_installed: false,
            _workbench_terminal_panel_observation: workbench_terminal_panel_observation,
            _workbench_terminal_panel_event_subscription:
                workbench_terminal_panel_event_subscription,
            workbench_terminal_surface: None,
            #[cfg(any(test, feature = "test-support"))]
            workbench_identity_phase_override: None,
            #[cfg(any(test, feature = "test-support"))]
            workbench_identity_observation_override: None,
            #[cfg(any(test, feature = "test-support"))]
            workbench_git_lifecycle_override: None,
            #[cfg(any(test, feature = "test-support"))]
            workbench_terminal_owner_state_override: None,
            #[cfg(any(test, feature = "test-support"))]
            workbench_plan_lifecycle_override: None,
            #[cfg(any(test, feature = "test-support"))]
            workbench_plan_navigation_target: None,
            #[cfg(any(test, feature = "test-support"))]
            workbench_terminal_badge_override: None,
        };

        let mut panel = panel;
        panel.ensure_native_agent_connection(cx);
        panel.observe_engine_capacity(cx);
        panel.load_public_channels(cx);
        panel
    }

    /// `OMEGA-DELTA-0035`. Keep the router's view of the engine current.
    ///
    /// omega#78 built `observe_capacity` and nothing called it, so the router
    /// decided every engine-lane pin against `NotRunning` regardless of what
    /// `omega-effectd` was actually doing. This is the feed: the same framed
    /// `get_capacity` call the Full Auto roster makes, on the same three-second
    /// cadence, handed to the router as a snapshot.
    ///
    /// The direction is one-way by construction. `observe_capacity` takes an
    /// answer and stores it; there is no path from here back into the engine,
    /// and a decision already recorded is never re-derived when a later answer
    /// arrives — `a_later_engine_answer_does_not_rewrite_a_recorded_decision`
    /// asserts that at the dispatch layer, not only in the journal.
    ///
    /// An engine that cannot be reached is reported as unreachable rather than
    /// left at its last good answer. A router that kept believing a stale
    /// "available" would route a pin onto a lane that had gone away, and the
    /// user would see a pin honoured that was not.
    fn observe_engine_capacity(&mut self, cx: &mut Context<Self>) {
        let executor = cx.background_executor().clone();
        let supervisor = omega_effectd::shared_supervisor(cx).ok();
        self._engine_capacity_poll = Some(cx.spawn(async move |this, cx| {
            loop {
                let observed = match &supervisor {
                    Some(supervisor) => {
                        let mut guard = supervisor.lock().await;
                        guard.get_capacity().await.ok()
                    }
                    None => None,
                };
                // Reached through the panel entity so the loop ends with the
                // panel rather than outliving the window it belongs to.
                let alive = this
                    .update(cx, |_panel, _cx| {
                        let Some(router) = crate::omega_router::active_router() else {
                            return;
                        };
                        match &observed {
                            Some(capacity) => router.observe_capacity(Ok(capacity)),
                            None => router.observe_capacity(Err(
                                omega_front_door::EngineUnreachable::NotRunning,
                            )),
                        }
                    })
                    .is_ok();
                if !alive {
                    break;
                }
                executor.timer(std::time::Duration::from_secs(3)).await;
            }
        }));
    }

    pub fn toggle_focus(
        workspace: &mut Workspace,
        _: &ToggleFocus,
        window: &mut Window,
        cx: &mut Context<Workspace>,
    ) {
        if workspace
            .panel::<Self>(cx)
            .is_some_and(|panel| panel.read(cx).enabled(cx))
        {
            workspace.toggle_panel_focus::<Self>(window, cx);
        }
    }

    pub fn focus(
        workspace: &mut Workspace,
        _: &FocusAgent,
        window: &mut Window,
        cx: &mut Context<Workspace>,
    ) {
        if workspace
            .panel::<Self>(cx)
            .is_some_and(|panel| panel.read(cx).enabled(cx))
        {
            workspace.focus_panel::<Self>(window, cx);
        }
    }

    pub fn toggle(
        workspace: &mut Workspace,
        _: &Toggle,
        window: &mut Window,
        cx: &mut Context<Workspace>,
    ) {
        if workspace
            .panel::<Self>(cx)
            .is_some_and(|panel| panel.read(cx).enabled(cx))
        {
            if !workspace.toggle_panel_focus::<Self>(window, cx) {
                workspace.close_panel::<Self>(window, cx);
            }
        }
    }

    pub fn thread_store(&self) -> &Entity<ThreadStore> {
        &self.thread_store
    }

    pub fn connection_store(&self) -> &Entity<AgentConnectionStore> {
        &self.connection_store
    }

    /// The agent a **new** thread here is built on.
    ///
    /// `OMEGA-DELTA-0131`, omega#121. The panel's inherited agent selection is
    /// kept per workspace and globally as the last-used agent.
    ///
    /// The owner selected Exo, the selector said Exo, he typed `who are you`,
    /// and Codex answered. Every other surface was telling the truth: the
    /// thread was titled "New Codex Thread", the composer read "Message Codex",
    /// and the reply said "I'm Codex". His panel had been on `Agent::Codex`
    /// since an earlier session, so the conversation held Codex's *own* server
    /// — and Omega's router is the only thing that reads the executor
    /// selection. Choosing an executor tore that connection down and rebuilt
    /// the same one, three times in six seconds, logging nothing but the
    /// choice.
    ///
    /// So in zero base a new thread is built on Omega's router and nothing
    /// else. `OMEGA-DELTA-0150` additionally keeps the router's unpinned
    /// decision on Omega's native loop.
    ///
    /// **The clamp is here, on the accessor, and not on the stored field.**
    /// That is the correction to the first version of this fix, which clamped
    /// every write. `OMEGA-DELTA-0118` promises a thread reopens under *the
    /// executor that recorded it*, and the panel restores the last thread's own
    /// agent at launch to keep that promise — so clamping the writes rewrote a
    /// Codex thread's agent to the router on the way back in, the router had no
    /// route record for a session it had never opened, and the owner's next
    /// launch said `Failed to Launch — no thread found with ID`. A reopened
    /// thread keeps the agent it was recorded under. What is pinned is what a
    /// *new* one starts on.
    ///
    /// Beside the collaboration rule below for the same reason: both are cases
    /// where the stored selection is not what a new thread may use.
    pub fn selected_agent(&self, cx: &App) -> Agent {
        if self.project.read(cx).is_via_collab() {
            Agent::NativeAgent
        } else if omega_zero_base::is_active() && !matches!(self.selected_agent, Agent::NativeAgent)
        {
            log::info!(
                "OMEGA-DELTA-0150: a new thread in zero base is built on Omega's \
                 native router rather than on {} directly",
                self.selected_agent.label()
            );
            Agent::NativeAgent
        } else {
            self.selected_agent.clone()
        }
    }

    pub fn open_thread(
        &mut self,
        session_id: acp::SessionId,
        work_dirs: Option<PathList>,
        title: Option<SharedString>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // Share links / clipboard imports enter with only a session id. If
        // this machine already has a metadata row for the session, route
        // through the normal thread-id path.
        let existing_thread_id = ThreadMetadataStore::try_global(cx).and_then(|store| {
            store
                .read(cx)
                .entry_by_session(&session_id)
                .map(|m| m.thread_id)
        });
        if let Some(thread_id) = existing_thread_id {
            self.load_agent_thread(
                crate::Agent::NativeAgent,
                thread_id,
                work_dirs,
                title,
                true,
                AgentThreadSource::AgentPanel,
                window,
                cx,
            );
        } else {
            self.external_thread_by_session(
                crate::Agent::NativeAgent,
                session_id,
                work_dirs,
                title,
                true,
                AgentThreadSource::AgentPanel,
                window,
                cx,
            );
        }
    }

    fn external_thread_by_session(
        &mut self,
        agent: Agent,
        session_id: acp::SessionId,
        work_dirs: Option<PathList>,
        title: Option<SharedString>,
        focus: bool,
        source: AgentThreadSource,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let thread = self.create_agent_thread_with_server_for_external_session(
            agent, None, session_id, work_dirs, title, None, source, window, cx,
        );
        self.set_base_view(thread.into(), focus, window, cx);
    }

    pub(crate) fn context_server_registry(&self) -> &Entity<ContextServerRegistry> {
        &self.context_server_registry
    }

    pub fn is_visible(workspace: &Entity<Workspace>, cx: &App) -> bool {
        let workspace_read = workspace.read(cx);

        workspace_read
            .panel::<AgentPanel>(cx)
            .map(|panel| {
                let panel_id = Entity::entity_id(&panel);

                workspace_read.all_docks().iter().any(|dock| {
                    dock.read(cx)
                        .visible_panel()
                        .is_some_and(|visible_panel| visible_panel.panel_id() == panel_id)
                })
            })
            .unwrap_or(false)
    }

    /// Clear the active view, retaining any running thread in the background.
    pub fn clear_base_view(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let old_view = std::mem::replace(&mut self.base_view, BaseView::Uninitialized);
        self.retain_running_thread(old_view, cx);
        self.activate_draft(false, AgentThreadSource::AgentPanel, window, cx);
        self.serialize(cx);
        cx.emit(AgentPanelEvent::ActiveViewChanged);
        cx.notify();
    }

    /// `OMEGA-DELTA-0034`. A new thread does not require an open project.
    ///
    /// Upstream refused this (`agent_ui: Require an open project for agent
    /// panel`, "a bit brute force, but it works"). Omega's front door is the
    /// agent, and a fresh install *is* the no-project case, so refusing here
    /// refuses the front door to every new user. `new_thread_with_workspace`
    /// still asks `should_create_terminal_for_new_entry`, which requires an
    /// open project — a terminal genuinely needs a working directory, so with
    /// no project this falls through to a thread.
    pub fn new_thread(&mut self, _action: &NewThread, window: &mut Window, cx: &mut Context<Self>) {
        self.new_thread_with_workspace(None, window, cx);
    }

    fn new_thread_with_workspace(
        &mut self,
        workspace: Option<&Workspace>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.should_create_terminal_for_new_entry(cx) {
            self.new_terminal(workspace, AgentThreadSource::AgentPanel, window, cx);
        } else {
            self.activate_new_thread(true, AgentThreadSource::AgentPanel, window, cx);
        }
    }

    /// `OMEGA-DELTA-0034`. The front door's own entry point, project or no
    /// project.
    ///
    /// `AgentPanel::open_front_door` calls this on a window with nothing to
    /// restore, and a window with nothing to restore is by definition a window
    /// with no project. The guard that used to stand here is why omega#76's
    /// exit — *typing starts a real thread* — did not hold on a fresh install:
    /// landing worked, and the composer the user was supposed to type into was
    /// never built.
    pub fn activate_new_thread(
        &mut self,
        focus: bool,
        source: AgentThreadSource,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.set_last_created_entry_kind_from_user_action(AgentPanelEntryKind::Thread, cx);

        // If the user is viewing a *parked* draft and the ephemeral
        // new-draft slot is occupied, pressing `+` should just focus the
        // ephemeral draft — not park it and create yet another empty one.
        // This matches the mental model of `+` as "go to my new-thread
        // slot". The parked draft will be put back into `retained_threads`
        // by `set_base_view`'s `retain_running_thread` call.
        if let Some(draft) = self.draft_thread.clone()
            && self.active_thread_is_draft(cx)
            && !self.active_view_is_new_draft(cx)
            && *draft.read(cx).agent_key() == self.selected_agent
        {
            self.set_base_view(
                BaseView::AgentThread {
                    conversation_view: draft,
                },
                focus,
                window,
                cx,
            );
            return;
        }

        if let Some(draft) = self.draft_thread.clone() {
            if self.draft_has_content(&draft, cx) {
                let draft_id = draft.read(cx).thread_id;
                self.draft_thread = None;
                self._draft_editor_observation = None;
                self.retained_threads.insert(draft_id, draft);
            } else if *draft.read(cx).agent_key() != self.selected_agent {
                let old_draft_id = draft.read(cx).thread_id;
                ThreadMetadataStore::global(cx).update(cx, |store, cx| {
                    store.delete(old_draft_id, cx);
                });
                self.draft_thread = None;
                self._draft_editor_observation = None;
            }
        }
        self.activate_draft(focus, source, window, cx);
    }

    fn draft_has_content(&self, draft: &Entity<ConversationView>, cx: &App) -> bool {
        let cv = draft.read(cx);
        if let Some(thread_view) = cv.active_thread() {
            let text = thread_view.read(cx).message_editor.read(cx).text(cx);
            if !text.trim().is_empty() {
                return true;
            }
        }
        if let Some(acp_thread) = cv.root_thread(cx) {
            let thread = acp_thread.read(cx);
            if !thread.is_draft_thread() {
                return true;
            }
            if thread
                .draft_prompt()
                .is_some_and(|blocks| !blocks.is_empty())
            {
                return true;
            }
        }
        false
    }

    /// Reattaches the panel's new-draft slot to the persisted `thread_id`,
    /// seeding the editor with any prompt text from the draft-prompt kvp
    /// store.
    ///
    /// If the active view already holds this thread — because the user's
    /// last-active thread was the new-draft itself — we reuse that
    /// ConversationView instead of building a second one.
    fn restore_new_draft(
        &mut self,
        thread_id: ThreadId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.has_open_project(cx) {
            return;
        }

        let active_matching = match &self.base_view {
            BaseView::AgentThread { conversation_view }
                if conversation_view.read(cx).thread_id == thread_id =>
            {
                Some(conversation_view.clone())
            }
            _ => None,
        };
        if let Some(conversation_view) = active_matching {
            self.observe_draft_editor(&conversation_view, cx);
            self.draft_thread = Some(conversation_view);
            return;
        }

        let Some(metadata) = ThreadMetadataStore::try_global(cx)
            .and_then(|store| store.read(cx).entry(thread_id).cloned())
            .filter(|m| m.is_draft())
        else {
            return;
        };

        let agent = if self.project.read(cx).is_via_collab() {
            Agent::NativeAgent
        } else {
            Agent::from(metadata.agent_id.clone())
        };
        let initial_content = crate::draft_prompt_store::read(thread_id, cx).map(|blocks| {
            AgentInitialContent::ContentBlock {
                blocks,
                auto_submit: false,
            }
        });
        let thread = self.create_agent_thread_with_server(
            agent,
            None,
            Some(thread_id),
            Some(metadata.folder_paths().clone()),
            metadata.title.clone(),
            initial_content,
            None,
            AgentThreadSource::AgentPanel,
            window,
            cx,
        );
        self.observe_draft_editor(&thread.conversation_view, cx);
        self.draft_thread = Some(thread.conversation_view);
    }

    pub fn new_external_agent_thread(
        &mut self,
        action: &NewExternalAgentThread,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.has_open_project(cx) {
            return;
        }

        self.selected_agent = action.agent.clone().into();
        self.activate_new_thread(true, AgentThreadSource::AgentPanel, window, cx);
    }

    fn set_selected_agent_and_persist(&mut self, agent: Agent, cx: &mut Context<Self>) {
        if self.selected_agent != agent {
            self.selected_agent = agent.clone();
            self.serialize(cx);
        }

        cx.background_spawn({
            let kvp = KeyValueStore::global(cx);
            async move {
                write_global_last_used_agent(kvp, agent).await;
            }
        })
        .detach();
    }

    /// Sets the panel's selected agent without opening the panel or focusing
    /// it, so the agent is launched the next time the panel is opened (or
    /// right away, if the panel is already showing the empty new-thread
    /// draft).
    pub fn select_agent(&mut self, agent: Agent, window: &mut Window, cx: &mut Context<Self>) {
        if self.project.read(cx).is_via_collab() && !agent.is_native() {
            return;
        }

        let showing_new_draft = matches!(
            (&self.base_view, &self.draft_thread),
            (BaseView::AgentThread { conversation_view }, Some(draft))
                if conversation_view.entity_id() == draft.entity_id()
        );

        if matches!(self.base_view, BaseView::AgentThread { .. }) && showing_new_draft {
            self.set_selected_agent_and_persist(agent, cx);
            self.activate_draft(false, AgentThreadSource::AgentPanel, window, cx);
            cx.notify();
        }
    }

    pub fn new_terminal(
        &mut self,
        workspace: Option<&Workspace>,
        source: AgentThreadSource,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.supports_terminal(cx) {
            return;
        }
        self.set_last_created_entry_kind_from_user_action(AgentPanelEntryKind::Terminal, cx);
        let working_directory = self.terminal_working_directory(workspace, cx);
        self.spawn_terminal(
            TerminalId::new(),
            working_directory,
            None,
            None,
            None,
            true,
            true,
            true,
            source,
            window,
            cx,
        );
    }

    fn terminal_working_directory(
        &self,
        workspace: Option<&Workspace>,
        cx: &App,
    ) -> Option<PathBuf> {
        workspace
            .map(|workspace| terminal_view::default_working_directory(workspace, cx))
            .unwrap_or_else(|| self.default_terminal_working_directory(cx))
    }

    pub fn supports_terminal(&self, cx: &App) -> bool {
        self.has_open_project(cx) && self.project.read(cx).supports_terminal(cx)
    }

    /// `OMEGA-DELTA-0034`. A terminal entry still requires an open project.
    ///
    /// This asks `supports_terminal` rather than `project.supports_terminal`
    /// because the latter is `true` for *any* local project, worktree or not.
    /// Before the front-door guards moved, `new_thread`'s own
    /// `has_open_project` check was what stopped that: a fresh window whose
    /// persisted `last_created_entry_kind` was `Terminal` would otherwise open
    /// a terminal with no working directory instead of the composer. That is
    /// what those guards were protecting, and it is protected here now, where
    /// the requirement actually is.
    pub fn should_create_terminal_for_new_entry(&self, cx: &App) -> bool {
        self.last_created_entry_kind == AgentPanelEntryKind::Terminal && self.supports_terminal(cx)
    }

    fn set_last_created_entry_kind_from_user_action(
        &mut self,
        entry_kind: AgentPanelEntryKind,
        cx: &mut Context<Self>,
    ) {
        if self.last_created_entry_kind != entry_kind {
            self.last_created_entry_kind = entry_kind;
            self.serialize(cx);
        }

        cx.background_spawn({
            let kvp = KeyValueStore::global(cx);
            async move {
                write_global_last_created_entry_kind(kvp, entry_kind).await;
            }
        })
        .detach();
    }

    fn spawn_terminal(
        &mut self,
        terminal_id: TerminalId,
        working_directory: Option<PathBuf>,
        custom_title: Option<SharedString>,
        initial_title: Option<SharedString>,
        created_at: Option<DateTime<Utc>>,
        select: bool,
        focus: bool,
        run_init_command: bool,
        source: AgentThreadSource,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let terminal_working_directory = working_directory.clone();
        let init_command = Self::terminal_init_command(run_init_command, cx);
        let terminal_task = self.project.update(cx, |project, cx| {
            project.create_terminal_shell(working_directory, cx)
        });
        let workspace = self.workspace.clone();
        let workspace_id = self.workspace_id;
        let project = self.project.downgrade();

        cx.spawn_in(window, async move |this, cx| {
            let terminal = match terminal_task.await {
                Ok(terminal) => terminal,
                Err(error) => {
                    log::error!("failed to spawn agent panel terminal: {error:#}");
                    workspace
                        .update(cx, |workspace, cx| workspace.show_error(error, cx))
                        .log_err();
                    this.update(cx, |this, cx| {
                        if this.pending_terminal_spawn == Some(terminal_id) {
                            this.pending_terminal_spawn = None;
                            cx.notify();
                        }
                    })
                    .log_err();
                    return anyhow::Ok(());
                }
            };
            this.update_in(cx, |this, window, cx| {
                let terminal_for_init_command = terminal.clone();
                let terminal_view = cx.new(|cx| {
                    let mut view =
                        TerminalView::new(terminal, workspace, workspace_id, project, window, cx);
                    view.set_show_workspace_actions(false, cx);
                    view
                });
                this.insert_terminal(
                    terminal_id,
                    terminal_view,
                    terminal_working_directory,
                    custom_title,
                    initial_title,
                    created_at,
                    select,
                    focus,
                    source,
                    window,
                    cx,
                );
                Self::write_terminal_init_command(&terminal_for_init_command, init_command, cx);
            })?;
            anyhow::Ok(())
        })
        .detach_and_log_err(cx);
    }

    fn terminal_init_command(run_init_command: bool, cx: &App) -> Option<String> {
        run_init_command
            .then(|| AgentSettings::get_global(cx).terminal_init_command.clone())
            .flatten()
            .filter(|command| !command.trim().is_empty())
    }

    fn write_terminal_init_command(
        terminal: &Entity<terminal::Terminal>,
        init_command: Option<String>,
        cx: &mut Context<Self>,
    ) {
        let Some(command) = init_command else {
            return;
        };

        if !terminal.read(cx).is_pty() {
            terminal.update(cx, |terminal, _| {
                terminal.write_init_command(Self::terminal_init_command_input(command))
            });
            return;
        }

        let startup = terminal.update(cx, |terminal, _| {
            terminal.start_init_command_startup_handshake()
        });

        let terminal = terminal.downgrade();
        cx.spawn(async move |_this, cx| {
            // Fall back to the timeout so the init command is still delivered if
            // the shell never echoes the marker.
            let timeout = cx
                .background_executor()
                .timer(TERMINAL_INIT_COMMAND_STARTUP_TIMEOUT);
            futures::select_biased! {
                _ = startup.fuse() => {}
                _ = timeout.fuse() => {}
            }

            let input = Self::terminal_init_command_input(command);
            if let Err(error) = terminal.update(cx, move |terminal, cx| {
                if !terminal.write_init_command_after_startup(input, cx) {
                    log::debug!(
                        "skipping terminal init command because the terminal is no longer eligible"
                    );
                }
            }) {
                log::debug!("skipping terminal init command because the terminal closed: {error}");
            }
            anyhow::Ok(())
        })
        .detach_and_log_err(cx);
    }

    fn terminal_init_command_input(command: String) -> Vec<u8> {
        let mut input = command.into_bytes();
        // CR, not "\r\n": "\r\n" puts PowerShell into continuation
        // mode (same convention as the activation-script writes in
        // `TerminalBuilder::new`).
        input.push(b'\x0d');
        input
    }

    fn insert_terminal(
        &mut self,
        terminal_id: TerminalId,
        terminal_view: Entity<TerminalView>,
        working_directory: Option<PathBuf>,
        custom_title: Option<SharedString>,
        initial_title: Option<SharedString>,
        created_at: Option<DateTime<Utc>>,
        select: bool,
        focus: bool,
        source: AgentThreadSource,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(custom_title) = custom_title {
            terminal_view.update(cx, |terminal_view, cx| {
                terminal_view.set_custom_title(Some(custom_title.to_string()), cx);
            });
        }
        let terminal_entity = terminal_view.read(cx).terminal().clone();
        let view_subscription = cx.subscribe(
            &terminal_view,
            move |this, _terminal_view, event: &ItemEvent, cx| match event {
                ItemEvent::UpdateTab | ItemEvent::UpdateBreadcrumbs => {
                    this.refresh_terminal_metadata(terminal_id, cx);
                }
                ItemEvent::CloseItem | ItemEvent::Edit => {}
            },
        );
        // Listen on the underlying `Terminal` entity for shell-driven metadata
        // changes and bell.
        let terminal_subscription = cx.subscribe_in(
            &terminal_entity,
            window,
            move |this, _terminal, event: &TerminalEvent, window, cx| match event {
                TerminalEvent::TitleChanged
                | TerminalEvent::Wakeup
                | TerminalEvent::BreadcrumbsChanged => {
                    this.refresh_terminal_metadata(terminal_id, cx);
                    this.report_terminal_program(terminal_id, source, cx);
                }
                TerminalEvent::Bell => this.mark_terminal_notification(terminal_id, window, cx),
                TerminalEvent::CloseTerminal => {
                    this.request_close_terminal_from_terminal_event(terminal_id, cx);
                }
                TerminalEvent::BlinkChanged(_)
                | TerminalEvent::SelectionsChanged
                | TerminalEvent::NewNavigationTarget(_)
                | TerminalEvent::Open(_) => {}
            },
        );

        let last_known_terminal_title = initial_title
            .map(|title| title.to_string())
            .unwrap_or_default();
        let mut terminal = AgentTerminal {
            view: terminal_view,
            title_editor: None,
            title_editor_initial_title: None,
            title_editor_subscription: None,
            last_known_title: last_known_terminal_title.clone(),
            last_known_terminal_title,
            last_observed_program: None,
            working_directory,
            created_at: created_at.unwrap_or_else(Utc::now),
            has_notification: false,
            search_bar: None,
            notification_windows: Vec::new(),
            notification_subscriptions: Vec::new(),
            _subscriptions: vec![view_subscription, terminal_subscription],
        };
        if self.pending_terminal_spawn == Some(terminal_id) {
            self.pending_terminal_spawn = None;
        }
        terminal.refresh_metadata(cx);
        terminal.report_started_terminal_program(terminal_id, source, cx);
        self.terminals.insert(terminal_id, terminal);
        self.persist_terminal_metadata(terminal_id, cx);
        self.emit_terminal_thread_started(terminal_id, source, cx);
        if select {
            self.set_base_view(BaseView::Terminal { terminal_id }, focus, window, cx);
        }
        cx.emit(AgentPanelEvent::EntryChanged);
        cx.notify();
    }

    pub fn activate_terminal(
        &mut self,
        terminal_id: TerminalId,
        focus: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(terminal) = self.terminals.get_mut(&terminal_id) else {
            return;
        };
        let had_notification = terminal.has_notification;
        terminal.has_notification = false;
        if had_notification {
            self.dismiss_terminal_notifications(terminal_id, cx);
        }
        self.set_base_view(BaseView::Terminal { terminal_id }, focus, window, cx);
        if had_notification {
            cx.emit(AgentPanelEvent::EntryChanged);
            cx.notify();
        }
    }

    pub fn close_terminal(
        &mut self,
        terminal_id: TerminalId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.close_terminal_internal(terminal_id, true, window, cx);
    }

    pub fn close_terminal_without_activating_draft(
        &mut self,
        terminal_id: TerminalId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.close_terminal_internal(terminal_id, false, window, cx);
    }

    fn close_terminal_internal(
        &mut self,
        terminal_id: TerminalId,
        activate_draft_after_close: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let was_active = self.active_terminal_id() == Some(terminal_id);

        if self.pending_terminal_spawn == Some(terminal_id) {
            self.pending_terminal_spawn = None;
        }
        self.dismiss_terminal_notifications(terminal_id, cx);
        if self.terminals.remove(&terminal_id).is_none() {
            return;
        }
        if let Some(store) = TerminalThreadMetadataStore::try_global(cx) {
            store.update(cx, |store, cx| {
                store.delete(terminal_id, cx);
            });
        }
        if was_active {
            self.base_view = BaseView::Uninitialized;
            self.refresh_base_view_subscriptions(window, cx);
            if activate_draft_after_close {
                self.activate_draft(false, AgentThreadSource::AgentPanel, window, cx);
            }
        }

        cx.emit(AgentPanelEvent::EntryChanged);
        cx.notify();
    }

    fn request_close_terminal_from_terminal_event(
        &mut self,
        terminal_id: TerminalId,
        cx: &mut Context<Self>,
    ) {
        if let Some(metadata) = self.terminal_metadata(terminal_id, cx) {
            cx.emit(AgentPanelEvent::TerminalCloseRequested { metadata });
        }
    }

    fn emit_terminal_thread_started(
        &self,
        terminal_id: TerminalId,
        source: AgentThreadSource,
        cx: &App,
    ) {
        telemetry::event!(
            "Agent Thread Started",
            agent = TERMINAL_AGENT_TELEMETRY_ID,
            terminal_id = terminal_id.to_key_string(),
            source = source.as_str(),
            side = crate::agent_sidebar_side(cx),
            thread_location = "current_worktree",
        );
    }

    fn refresh_terminal_metadata(&mut self, terminal_id: TerminalId, cx: &mut Context<Self>) {
        if let Some(terminal) = self.terminals.get_mut(&terminal_id)
            && terminal.refresh_metadata(cx)
        {
            self.persist_terminal_metadata(terminal_id, cx);
            cx.emit(AgentPanelEvent::EntryChanged);
            cx.notify();
        }
    }

    fn report_terminal_program(
        &mut self,
        terminal_id: TerminalId,
        source: AgentThreadSource,
        cx: &mut Context<Self>,
    ) {
        if let Some(terminal) = self.terminals.get_mut(&terminal_id) {
            terminal.report_started_terminal_program(terminal_id, source, cx);
        }
    }

    fn persist_all_terminal_metadata(&self, cx: &mut Context<Self>) {
        let terminal_ids = self.terminals.keys().copied().collect::<Vec<_>>();
        for terminal_id in terminal_ids {
            self.persist_terminal_metadata(terminal_id, cx);
        }
    }

    fn persist_terminal_metadata(&self, terminal_id: TerminalId, cx: &mut Context<Self>) {
        let Some(store) = TerminalThreadMetadataStore::try_global(cx) else {
            return;
        };
        let Some(metadata) = self.terminal_metadata(terminal_id, cx) else {
            return;
        };
        store.update(cx, |store, cx| {
            store.save(metadata, cx);
        });
    }

    fn terminal_metadata(
        &self,
        terminal_id: TerminalId,
        cx: &App,
    ) -> Option<TerminalThreadMetadata> {
        let terminal = self.terminals.get(&terminal_id)?;
        let project = self.project.read(cx);
        Some(TerminalThreadMetadata {
            terminal_id,
            title: terminal.terminal_title(cx),
            custom_title: terminal.custom_title(cx),
            created_at: terminal.created_at,
            worktree_paths: project.worktree_paths(cx),
            remote_connection: project.remote_connection_options(cx),
            working_directory: terminal.working_directory.clone(),
        })
    }

    pub fn restore_terminal(
        &mut self,
        metadata: TerminalThreadMetadata,
        focus: bool,
        source: AgentThreadSource,
        workspace: Option<&Workspace>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.has_terminal(metadata.terminal_id) {
            self.activate_terminal(metadata.terminal_id, focus, window, cx);
            return;
        }

        if !self.supports_terminal(cx) {
            return;
        }

        self.pending_terminal_spawn = Some(metadata.terminal_id);
        let working_directory = self.terminal_restore_working_directory(&metadata, workspace, cx);
        let initial_title = Self::terminal_restore_initial_title(&metadata);
        self.spawn_terminal(
            metadata.terminal_id,
            working_directory,
            metadata.custom_title.clone(),
            initial_title,
            Some(metadata.created_at),
            true,
            focus,
            true,
            source,
            window,
            cx,
        );
    }

    fn restore_terminal_for_panel_load(
        &mut self,
        metadata: TerminalThreadMetadata,
        focus: bool,
        source: AgentThreadSource,
        workspace: Option<&Workspace>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        #[cfg(test)]
        self.restore_test_terminal(metadata, focus, source, workspace, window, cx)
            .log_err();

        #[cfg(not(test))]
        self.restore_terminal(metadata, focus, source, workspace, window, cx);
    }

    fn terminal_restore_working_directory(
        &self,
        metadata: &TerminalThreadMetadata,
        workspace: Option<&Workspace>,
        cx: &App,
    ) -> Option<PathBuf> {
        if let Some(working_directory) = metadata.working_directory.clone() {
            return Some(working_directory);
        }

        if let Some(workspace) = workspace {
            return terminal_view::default_working_directory(workspace, cx);
        }

        self.default_terminal_working_directory(cx)
    }

    fn terminal_restore_initial_title(metadata: &TerminalThreadMetadata) -> Option<SharedString> {
        (!metadata.title.is_empty()).then(|| metadata.title.clone())
    }

    fn edit_terminal_title(
        &mut self,
        terminal_id: TerminalId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(terminal) = self.terminals.get_mut(&terminal_id) else {
            return;
        };

        if let Some(title_editor) = terminal.title_editor.as_ref() {
            title_editor.focus_handle(cx).focus(window, cx);
            return;
        }

        let title = terminal.editable_title(cx).to_string();
        let title_editor_initial_title = title.clone();
        let title_editor = cx.new(|cx| {
            let mut editor = Editor::single_line(window, cx);
            editor.set_text(title, window, cx);
            editor
        });
        let title_editor_subscription = cx.subscribe_in(
            &title_editor,
            window,
            move |this, title_editor, event: &editor::EditorEvent, window, cx| {
                this.handle_terminal_title_editor_event(
                    terminal_id,
                    title_editor,
                    event,
                    window,
                    cx,
                );
            },
        );
        title_editor.update(cx, |editor, cx| {
            editor.select_all(&editor::actions::SelectAll, window, cx);
            editor.focus_handle(cx).focus(window, cx);
        });
        terminal.title_editor = Some(title_editor);
        terminal.title_editor_initial_title = Some(title_editor_initial_title);
        terminal.title_editor_subscription = Some(title_editor_subscription);
        cx.notify();
    }

    fn stop_editing_terminal_title(
        &mut self,
        terminal_id: TerminalId,
        focus_terminal: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(terminal) = self.terminals.get_mut(&terminal_id) else {
            return;
        };
        let terminal_view = terminal.view.clone();
        terminal.title_editor = None;
        terminal.title_editor_initial_title = None;
        terminal.title_editor_subscription = None;
        let title_changed = terminal.refresh_title(cx);

        if focus_terminal {
            terminal_view.focus_handle(cx).focus(window, cx);
        }
        if title_changed {
            cx.emit(AgentPanelEvent::EntryChanged);
        }
        cx.notify();
    }

    fn handle_terminal_title_editor_event(
        &mut self,
        terminal_id: TerminalId,
        title_editor: &Entity<Editor>,
        event: &editor::EditorEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match event {
            editor::EditorEvent::BufferEdited => {
                if !title_editor.read(cx).is_focused(window) {
                    return;
                }
                let Some((terminal_view, initial_title, terminal_title)) =
                    self.terminals.get(&terminal_id).and_then(|terminal| {
                        terminal
                            .title_editor
                            .as_ref()
                            .is_some_and(|current_editor| current_editor == title_editor)
                            .then(|| {
                                (
                                    terminal.view.clone(),
                                    terminal.title_editor_initial_title.clone(),
                                    terminal.terminal_title(cx),
                                )
                            })
                    })
                else {
                    return;
                };
                let new_title = title_editor.read(cx).text(cx);
                if initial_title.as_deref() == Some(new_title.as_str()) {
                    return;
                }
                let label = if new_title.trim().is_empty()
                    || new_title == terminal_title_without_prefix(terminal_title.as_ref())
                {
                    None
                } else {
                    Some(new_title)
                };

                cx.defer(move |cx| {
                    terminal_view.update(cx, |terminal_view, cx| {
                        terminal_view.set_custom_title(label, cx);
                    });
                });
            }
            editor::EditorEvent::Blurred => {
                if self
                    .terminals
                    .get(&terminal_id)
                    .and_then(|terminal| terminal.title_editor.as_ref())
                    .is_some_and(|current_editor| current_editor == title_editor)
                {
                    self.stop_editing_terminal_title(terminal_id, false, window, cx);
                }
            }
            _ => {}
        }
    }

    fn mark_terminal_notification(
        &mut self,
        terminal_id: TerminalId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.active_terminal_visible(terminal_id, window, cx) {
            return;
        }
        let newly_notified = {
            let Some(terminal) = self.terminals.get_mut(&terminal_id) else {
                return;
            };
            if terminal.has_notification {
                false
            } else {
                terminal.has_notification = true;
                true
            }
        };
        if newly_notified {
            cx.emit(AgentPanelEvent::EntryChanged);
            cx.notify();
            #[cfg(feature = "audio")]
            self.play_terminal_notification_sound(
                self.terminal_status_visible(terminal_id, window, cx),
                cx,
            );
            self.show_terminal_notification(terminal_id, window, cx);
        }
    }

    fn show_terminal_notification(
        &mut self,
        terminal_id: TerminalId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(terminal) = self.terminals.get(&terminal_id) else {
            return;
        };
        if !terminal.notification_windows.is_empty() {
            return;
        }
        let title = terminal.title(cx);
        if self.terminal_status_visible(terminal_id, window, cx) {
            return;
        }
        let settings = AgentSettings::get_global(cx);
        match settings.notify_when_agent_waiting {
            NotifyWhenAgentWaiting::PrimaryScreen => {
                window.request_attention();
                if let Some(primary) = cx.primary_display() {
                    self.pop_up_terminal_notification(terminal_id, &title, primary, window, cx);
                }
            }
            NotifyWhenAgentWaiting::AllScreens => {
                window.request_attention();
                for screen in cx.displays() {
                    self.pop_up_terminal_notification(terminal_id, &title, screen, window, cx);
                }
            }
            NotifyWhenAgentWaiting::Never => {}
        }
    }

    fn pop_up_terminal_notification(
        &mut self,
        terminal_id: TerminalId,
        title: &SharedString,
        screen: Rc<dyn PlatformDisplay>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let options = AgentNotification::window_options(screen, cx);
        let project_name = self.workspace.upgrade().and_then(|workspace| {
            workspace
                .read(cx)
                .project()
                .read(cx)
                .visible_worktrees(cx)
                .next()
                .map(|worktree| worktree.read(cx).root_name_str().to_string())
        });
        let title = title.clone();
        let Ok(screen_window) = cx.open_window(options, |_window, cx| {
            cx.new(|_cx| AgentNotification::new(title, None, IconName::Terminal, project_name))
        }) else {
            return;
        };
        let Ok(pop_up) = screen_window.entity(cx) else {
            return;
        };

        let event_subscription = cx.subscribe_in(&pop_up, window, {
            move |this, _, event: &AgentNotificationEvent, window, cx| match event {
                AgentNotificationEvent::Accepted => {
                    let Some(handle) = window.window_handle().downcast::<MultiWorkspace>() else {
                        log::error!("root view should be a MultiWorkspace");
                        return;
                    };
                    cx.activate(true);

                    let workspace = this.workspace.clone();
                    cx.defer(move |cx| {
                        handle
                            .update(cx, |multi_workspace, window, cx| {
                                window.activate_window();

                                let Some(workspace) = workspace.upgrade() else {
                                    return;
                                };
                                multi_workspace.activate(workspace.clone(), None, window, cx);

                                workspace.update(cx, |workspace, cx| {
                                    workspace.reveal_panel::<AgentPanel>(window, cx);
                                    if let Some(panel) = workspace.panel::<AgentPanel>(cx) {
                                        panel.update(cx, |panel, cx| {
                                            panel.activate_terminal(terminal_id, true, window, cx);
                                        });
                                    }
                                    workspace.focus_panel::<AgentPanel>(window, cx);
                                });
                            })
                            .log_err();
                    });

                    this.dismiss_terminal_notifications(terminal_id, cx);
                }
                AgentNotificationEvent::Dismissed => {
                    this.dismiss_terminal_notifications(terminal_id, cx);
                }
            }
        });

        let pop_up_weak = pop_up.downgrade();
        let window_activation_subscription = cx.observe_window_activation(window, {
            let pop_up_weak = pop_up_weak.clone();
            move |this, window, cx| {
                this.dismiss_terminal_pop_up_if_visible(terminal_id, &pop_up_weak, window, cx);
            }
        });

        let multi_workspace_subscription = {
            let pop_up_weak = pop_up_weak.clone();
            window.root::<MultiWorkspace>().flatten().map(|mw| {
                cx.observe_in(&mw, window, move |this, _, window, cx| {
                    this.dismiss_terminal_pop_up_if_visible(terminal_id, &pop_up_weak, window, cx);
                })
            })
        };

        let this_panel = cx.entity();
        let agent_panel_subscription = cx.subscribe_in(&this_panel, window, {
            move |this, _, event: &AgentPanelEvent, window, cx| match event {
                AgentPanelEvent::ActiveViewChanged | AgentPanelEvent::ActiveViewFocused => {
                    this.dismiss_terminal_pop_up_if_visible(terminal_id, &pop_up_weak, window, cx);
                }
                AgentPanelEvent::EntryChanged
                | AgentPanelEvent::TerminalCloseRequested { .. }
                | AgentPanelEvent::ThreadInteracted { .. } => {}
            }
        });

        let Some(terminal) = self.terminals.get_mut(&terminal_id) else {
            screen_window
                .update(cx, |_, window, _| window.remove_window())
                .ok();
            return;
        };
        terminal.notification_windows.push(screen_window);
        terminal.notification_subscriptions.push(event_subscription);
        terminal
            .notification_subscriptions
            .push(window_activation_subscription);
        terminal
            .notification_subscriptions
            .push(agent_panel_subscription);
        if let Some(subscription) = multi_workspace_subscription {
            terminal.notification_subscriptions.push(subscription);
        }
    }

    fn dismiss_terminal_notifications(&mut self, terminal_id: TerminalId, cx: &mut App) {
        let Some(terminal) = self.terminals.get_mut(&terminal_id) else {
            return;
        };
        let windows = std::mem::take(&mut terminal.notification_windows);
        terminal.notification_subscriptions.clear();
        for window in windows {
            window
                .update(cx, |_, window, _| {
                    window.remove_window();
                })
                .ok();
        }
    }

    fn dismiss_all_terminal_notifications(&mut self, cx: &mut App) {
        let terminal_ids = self.terminals.keys().copied().collect::<Vec<_>>();
        for terminal_id in terminal_ids {
            self.dismiss_terminal_notifications(terminal_id, cx);
        }
    }

    pub fn dismiss_all_notifications(&mut self, cx: &mut Context<Self>) -> bool {
        let mut dismissed = false;
        for conversation_view in self.conversation_views() {
            dismissed |= conversation_view.update(cx, |view, cx| view.dismiss_notifications(cx));
        }
        let had_terminal_notifications = self
            .terminals
            .values()
            .any(|t| !t.notification_windows.is_empty());
        if had_terminal_notifications {
            self.dismiss_all_terminal_notifications(cx);
            dismissed = true;
        }
        dismissed
    }

    fn active_terminal_visible(&self, terminal_id: TerminalId, window: &Window, cx: &App) -> bool {
        if !window.is_window_active() {
            return false;
        }
        if !self.terminal_surface_visible(terminal_id) {
            return false;
        }
        let Some(workspace) = self.workspace.upgrade() else {
            return false;
        };
        if let Some(multi_workspace) = window.root::<MultiWorkspace>().flatten() {
            let multi_workspace = multi_workspace.read(cx);
            if multi_workspace.workspace() != &workspace {
                return false;
            }
        }
        AgentPanel::is_visible(&workspace, cx)
    }

    fn terminal_surface_visible(&self, terminal_id: TerminalId) -> bool {
        self.active_terminal_id() == Some(terminal_id)
            && matches!(self.visible_surface(), VisibleSurface::Terminal(_))
    }

    fn terminal_status_visible(&self, terminal_id: TerminalId, window: &Window, cx: &App) -> bool {
        if !window.is_window_active() {
            return false;
        }

        if let Some(multi_workspace) = window.root::<MultiWorkspace>().flatten() {
            let multi_workspace = multi_workspace.read(cx);
            if multi_workspace.sidebar_open() && multi_workspace.is_threads_list_view_active(cx) {
                return true;
            }

            let Some(workspace) = self.workspace.upgrade() else {
                return false;
            };

            return multi_workspace.workspace() == &workspace
                && self.terminal_surface_visible(terminal_id)
                && AgentPanel::is_visible(&workspace, cx);
        }

        self.workspace.upgrade().is_some_and(|workspace| {
            self.terminal_surface_visible(terminal_id) && AgentPanel::is_visible(&workspace, cx)
        })
    }

    fn dismiss_terminal_pop_up_if_visible(
        &mut self,
        terminal_id: TerminalId,
        pop_up: &WeakEntity<AgentNotification>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.terminal_status_visible(terminal_id, window, cx) {
            return;
        }
        if self.active_terminal_visible(terminal_id, window, cx)
            && let Some(terminal) = self.terminals.get_mut(&terminal_id)
            && terminal.has_notification
        {
            terminal.has_notification = false;
            cx.emit(AgentPanelEvent::EntryChanged);
            cx.notify();
        }
        if let Some(pop_up) = pop_up.upgrade() {
            pop_up.update(cx, |notification, cx| {
                notification.dismiss(cx);
            });
        }
    }

    #[cfg(feature = "audio")]
    fn play_terminal_notification_sound(&self, visible: bool, cx: &mut App) {
        let settings = AgentSettings::get_global(cx);
        if settings.play_sound_when_agent_done.should_play(visible) {
            Audio::play_sound(Sound::AgentDone, cx);
        }
    }

    fn default_terminal_working_directory(&self, cx: &App) -> Option<PathBuf> {
        // Reuse the workspace-based helper so behavior matches the regular
        // terminal panel (e.g. `WorkingDirectory::FirstProjectDirectory` falling
        // back to a file's parent directory when the worktree root is a file).
        self.workspace
            .upgrade()
            .and_then(|workspace| terminal_view::default_working_directory(workspace.read(cx), cx))
    }

    fn has_open_project(&self, cx: &App) -> bool {
        self.project.read(cx).visible_worktrees(cx).next().is_some()
    }

    /// Open Omega's front door on a window that has nothing else to show.
    ///
    /// `OMEGA-DELTA-0019`. Upstream Zed answers an empty window with
    /// `Editor::new_file`, so the first thing a new user meets is an untitled
    /// buffer. Omega answers it with the agent.
    ///
    /// The wait is a bounded poll rather than a subscription because the agent
    /// panel is added to the dock by an async task in `crates/zed`, this runs
    /// from `Workspace::new_local`'s init callback, and `Workspace` emits no
    /// "panel added" event to subscribe to. It gives up after a second instead
    /// of holding a task for the window's lifetime: a window with no agent
    /// panel a second after opening has AI disabled, and the front door is not
    /// the right thing to force on it.
    pub fn open_front_door(window: &mut Window, cx: &mut Context<Workspace>) {
        cx.spawn_in(window, async move |workspace, cx| {
            for _ in 0..40 {
                let opened = workspace.update_in(cx, |workspace, window, cx| {
                    let Some(panel) = workspace.panel::<Self>(cx) else {
                        return false;
                    };
                    workspace.focus_panel::<Self>(window, cx);
                    panel.update(cx, |panel, cx| {
                        panel.activate_new_thread(true, AgentThreadSource::AgentPanel, window, cx);
                    });
                    true
                })?;
                if opened {
                    return anyhow::Ok(());
                }
                cx.background_executor()
                    .timer(std::time::Duration::from_millis(25))
                    .await;
            }
            anyhow::Ok(())
        })
        .detach();
    }

    /// Say no in one sentence a person can read. omega#99.
    ///
    /// The mode's action gate already refuses the `full_auto_panel` namespace
    /// before any listener runs. This is the second half of the same rule, for
    /// the callers that are not action dispatches: a surface that is only
    /// visually absent is still reachable, and a reachable surface that returns
    /// silently is indistinguishable from a broken one.
    fn refuse_in_zero_base(&self, action_name: &str, cx: &mut Context<Self>) {
        let sentence = omega_zero_base::refusal(action_name);
        log::info!("{sentence}");
        if let Some(workspace) = self.workspace.upgrade() {
            workspace.update(cx, |workspace, cx| {
                struct ZeroBaseRefusalToast;
                workspace.show_toast(
                    workspace::Toast::new(
                        workspace::notifications::NotificationId::unique::<ZeroBaseRefusalToast>(),
                        sentence,
                    )
                    .autohide(),
                    cx,
                );
            });
        }
    }

    /// Show the Full Auto launch surface inside this panel.
    ///
    /// `OMEGA-DELTA-0020`. The owner asked for Full Auto to be folded into the
    /// Omega chat UI rather than living in its own dock panel, so this is
    /// where `full_auto_panel::OpenLauncher` now lands.
    ///
    /// Showing the launch surface is not starting a run: the draft arrives
    /// unsent and "Start Full Auto" is a second, separate human act. Owner
    /// gate 8 — only an explicit human action may start Full Auto authority —
    /// is why the fold adds an entry and not a composer mode flag.
    pub fn open_full_auto(&mut self, focus: bool, window: &mut Window, cx: &mut Context<Self>) {
        // omega#99. Zero base refuses this surface as well as not rendering its
        // entry. The refusal is a sentence, not a silent return: a person who
        // reached this through a keymap of their own is told which mode is on
        // and how to leave it.
        if omega_zero_base::is_active() {
            self.refuse_in_zero_base("full_auto_panel::OpenLauncher", cx);
            return;
        }
        if self.full_auto.is_none() {
            let workspace = self.workspace.clone();
            self.full_auto = Some(cx.new(|cx| FullAutoPanel::new(workspace, window, cx)));
        }
        self.showing_full_auto = true;
        if focus {
            self.focus_handle(cx).focus(window, cx);
        }
        cx.notify();
    }

    /// Toggle the Full Auto surface, for `full_auto_panel::ToggleFocus`.
    ///
    /// The retired dock panel answered that action with
    /// `toggle_panel_focus::<FullAutoPanel>`. Keeping the action working is
    /// deliberate: a user keymap may already name it, and the fold is meant to
    /// move where Full Auto lives, not to take a binding away.
    pub fn toggle_full_auto(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if omega_zero_base::is_active() {
            self.refuse_in_zero_base("full_auto_panel::ToggleFocus", cx);
            return;
        }
        if self.showing_full_auto {
            self.showing_full_auto = false;
            self.focus_handle(cx).focus(window, cx);
            cx.notify();
        } else {
            self.open_full_auto(true, window, cx);
        }
    }

    /// Show or hide zero base's threads sidebar. `OMEGA-DELTA-0118`.
    ///
    /// The owner's words, testing a live build: *"this 'Toggle Threads Sidebar'
    /// does nothing when i click on it but i want it. i want threads sidebar to
    /// see historical chats."* It did nothing because the entry named
    /// `multi_workspace::ToggleWorkspaceSidebar`, and that namespace is outside
    /// zero base's admitted set, so the action gate refused it before any
    /// listener ran. This is the surface the entry names instead.
    /// `OMEGA-DELTA-0130` widened what it toggles: the same action, the same
    /// binding, the same menu entry, now expanding and collapsing a persistent
    /// sidebar whose first section is those threads rather than opening an
    /// overlay that was only threads.
    pub fn toggle_threads_sidebar(&mut self, cx: &mut Context<Self>) {
        self.sidebar.open = !self.sidebar.open;
        // A refusal belongs to the click that produced it. Carrying it across
        // a collapse and an expand would show a sentence about a thread the
        // person is no longer looking at.
        self.threads_sidebar_refusal = None;
        self.save_sidebar_state(cx);
        cx.notify();
    }

    /// Collapse or expand one section. `OMEGA-DELTA-0130`.
    fn toggle_sidebar_section(
        &mut self,
        section: omega_sidebar::SectionId,
        cx: &mut Context<Self>,
    ) {
        self.sidebar.toggle_section(section);
        self.save_sidebar_state(cx);
        cx.notify();
    }

    /// Write the sidebar's state, so a relaunch draws what was left behind.
    ///
    /// Fire and forget, and logged rather than raised. A key-value write that
    /// fails costs the person a collapsed section after a restart; making them
    /// read about it costs them the thing they were doing.
    fn save_sidebar_state(&self, cx: &mut Context<Self>) {
        let Ok(json) = serde_json::to_string(&self.sidebar) else {
            return;
        };
        let kvp = KeyValueStore::global(cx);
        cx.background_spawn(async move {
            kvp.write_kvp(omega_sidebar::STATE_KEY.to_string(), json)
                .await
                .log_err();
        })
        .detach();
    }

    /// Load the one-channel deployment manifest into the versioned registry.
    fn load_public_channels(&mut self, cx: &mut Context<Self>) {
        if !omega_zero_base::is_active() {
            return;
        }
        let http_client = self.project.read(cx).client().http_client();
        self.public_channels_error = None;
        self._public_channels_load = Some(cx.spawn(async move |this, cx| {
            let read = fetch_public_channel_registry(http_client).await;
            this.update(cx, |this, cx| {
                match read {
                    Ok(registry) => {
                        this.stop_all_public_channel_views(cx);
                        this.public_channels =
                            crate::omega_public_channels::PublicChannelController::new(registry);
                        this.public_channels_error = None;
                    }
                    Err(error) => {
                        log::info!("public channel registry could not load: {error:#}");
                        this.public_channels_error =
                            Some("Could not load public channels just now.".into());
                    }
                }
                cx.notify();
            })
            .ok();
        }));
    }

    fn stop_all_public_channel_views(&mut self, cx: &mut Context<Self>) {
        for view in self.public_channel_views.values() {
            view.update(cx, |view, cx| view.pause(cx));
        }
        self.public_channel_views.clear();
        self._public_channel_view_subscriptions.clear();
    }

    /// The rows the threads sidebar draws, newest first.
    ///
    /// Recomputed per render from the metadata store rather than cached. The
    /// store is already in memory and the list is bounded, so a cache would buy
    /// nothing and could disagree with the thread the person just renamed.
    fn threads_sidebar_rows(&self, cx: &App) -> Vec<omega_threads_sidebar::ThreadRow> {
        let Some(store) = ThreadMetadataStore::try_global(cx) else {
            return Vec::new();
        };
        let registered: Vec<AgentId> = self
            .project
            .read(cx)
            .agent_server_store()
            .read(cx)
            .external_agents()
            .cloned()
            .collect();
        omega_threads_sidebar::rows(
            store.read(cx).entries(),
            Utc::now(),
            &omega_executor_selector::unavailable_here(),
            &registered,
        )
    }

    /// Reopen a past conversation from the threads sidebar.
    ///
    /// The executor travels with the thread: `load_agent_thread` is handed the
    /// `agent_id` the store recorded, never the one currently selected. A
    /// session id names a conversation inside the agent server that created it,
    /// so resuming a Codex session on Claude's connection reaches an adapter
    /// that has never heard of it. When the recorded executor cannot run here
    /// at all, the row already carries the sentence saying so and this shows it
    /// rather than dispatching a load that fails in somebody else's error text.
    fn open_thread_from_threads_sidebar(
        &mut self,
        row: &omega_threads_sidebar::ThreadRow,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(refusal) = row.refusal.clone() {
            log::info!(
                "threads sidebar refused {}: {refusal}",
                row.thread_id.to_key_string()
            );
            self.threads_sidebar_refusal = Some(refusal);
            cx.notify();
            return;
        }

        self.threads_sidebar_refusal = None;
        // OMEGA-DELTA-0130. The sidebar stays. It used to close itself here,
        // because it was an overlay covering the thread it had just opened.
        // A persistent sidebar is beside that thread rather than on top of it,
        // and closing it would take away the list the person is picking from.
        self.load_agent_thread(
            Agent::from(row.agent_id.clone()),
            row.thread_id,
            Some(row.folder_paths.clone()),
            Some(row.title.clone()),
            true,
            AgentThreadSource::AgentPanel,
            window,
            cx,
        );
    }

    fn open_device_pairing(&mut self, cx: &mut Context<Self>) {
        // omega#124. Zero base is the default mode and loads no workroom panel,
        // so no Sarah host request had ever started the transport here and this
        // control refused every press. Start the transport on the press, then
        // issue the bootstrap, so the mode a person actually gets can pair.
        cx.spawn(async move |this, cx| {
            let started = crate::omega_host_bridge::ensure_device_pairing_runtime(cx).await;
            this.update(cx, |this, cx| {
                this.device_pairing_surface = Some(match started {
                    Err(error) => DevicePairingSurface::Unavailable(error.to_string().into()),
                    Ok(()) => match omega_effectd::issue_device_pairing_bootstrap(cx).and_then(
                        |bootstrap| bootstrap.qr().map(|qr| (bootstrap, qr)).map_err(Into::into),
                    ) {
                        Ok((bootstrap, qr)) => match render_pairing_qr(&qr) {
                            Ok((image, image_size)) => DevicePairingSurface::Ready {
                                bootstrap,
                                image,
                                image_size,
                            },
                            Err(error) => {
                                DevicePairingSurface::Unavailable(error.to_string().into())
                            }
                        },
                        Err(error) => DevicePairingSurface::Unavailable(error.to_string().into()),
                    },
                });
                cx.notify();
            })
        })
        .detach();
    }

    fn render_device_pairing_surface(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let surface = self.device_pairing_surface.as_ref()?;
        let border = cx.theme().colors().border;
        let content = match surface {
            DevicePairingSurface::Ready {
                bootstrap,
                image,
                image_size,
            } => render_ready_device_pairing_content(
                &bootstrap.magic_dns_name,
                bootstrap.port,
                image.clone(),
                *image_size,
            ),
            DevicePairingSurface::Unavailable(message) => Label::new(message.clone())
                .size(LabelSize::XSmall)
                .color(Color::Muted)
                .into_any_element(),
        };
        Some(
            v_flex()
                .mx_2()
                .mb_2()
                .p_2()
                .gap_2()
                .border_1()
                .border_color(border)
                .rounded_md()
                .child(
                    h_flex()
                        .justify_between()
                        .child(Label::new("Pair phone").size(LabelSize::Small))
                        .child(
                            IconButton::new("close-device-pairing", IconName::Close)
                                .icon_size(IconSize::Small)
                                .tooltip(Tooltip::text("Close"))
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.device_pairing_surface = None;
                                    cx.notify();
                                })),
                        ),
                )
                .child(content)
                .into_any_element(),
        )
    }

    /// Draw zero base's persistent sidebar. `OMEGA-DELTA-0130`.
    ///
    /// # This is a column, not an overlay, and that is the whole decision
    ///
    /// `OMEGA-DELTA-0118` drew this absolutely positioned, and gave the reason:
    /// `OMEGA-DELTA-0105` records that the composer's bottom row already has to
    /// wrap so a narrow dock does not clip **Send**, and an overlay takes width
    /// from nobody. That was the right answer for a surface you open, look at,
    /// and close again.
    ///
    /// It is the wrong answer for a persistent one. An overlay that is always
    /// there is a permanent lid over the left of the transcript — and over the
    /// left of the composer, which is the thing the earlier delta was protecting.
    /// "It happens not to overlap" is not a property of a layout; it is a
    /// property of today's widths.
    ///
    /// So the sidebar is a real column and it **yields**. The shared
    /// [`workbench_shell::WorkbenchLayout`] allocator gives it its width only
    /// while the transcript can still keep
    /// [`omega_sidebar::MIN_CONTENT_WIDTH`], and draws a rail otherwise. The
    /// person's preference is untouched by that, so a window dragged wide
    /// again shows the sidebar they asked for.
    ///
    /// `None` outside zero base. The editor has its own workspace sidebar
    /// there, and `OMEGA-DELTA-0118`'s menu entry already names one action per
    /// mode for exactly that reason. The workbench test seam also enables it so
    /// deterministic shell scenes exercise the production column allocation.
    fn render_sidebar(
        &self,
        layout: omega_sidebar::Layout,
        _window: &Window,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        if !omega_zero_base::is_active() && !self.workbench_shell_enabled {
            return None;
        }

        let (background, border) = {
            let colors = cx.theme().colors();
            (colors.panel_background, colors.border)
        };

        let column = v_flex()
            .h_full()
            .w(layout.width())
            // Never squeezed by what is beside it: the yield is a decision
            // `layout` makes with a floor in it, not one flexbox makes by
            // running out of room.
            .flex_shrink_0()
            .overflow_hidden()
            .bg(background)
            .border_r_1()
            .border_color(border);

        if !layout.is_expanded() {
            return Some(
                column
                    .id("omega-sidebar-rail")
                    .items_center()
                    .pt_1p5()
                    .child(
                        IconButton::new("expand-omega-sidebar", IconName::ChevronRight)
                            .icon_size(IconSize::Small)
                            .tooltip(Tooltip::text("Expand Sidebar"))
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.toggle_threads_sidebar(cx);
                            })),
                    )
                    .child(div().flex_1())
                    .child(
                        div().pb_1().child(
                            IconButton::new("open-omega-settings-rail", IconName::Settings)
                                .icon_size(IconSize::Small)
                                .tooltip(Tooltip::text("Settings"))
                                .on_click(|_, window, cx| {
                                    window.dispatch_action(
                                        zed_actions::OpenSettings.boxed_clone(),
                                        cx,
                                    );
                                }),
                        ),
                    )
                    .into_any_element(),
            );
        }

        let sections = omega_sidebar::SectionId::ALL.iter().fold(
            v_flex()
                .id("omega-sidebar-sections")
                .flex_1()
                .overflow_y_scroll(),
            |sections, section| sections.child(self.render_sidebar_section(*section, cx)),
        );
        let pairing_surface = self.render_device_pairing_surface(cx);

        Some(
            column
                .id("omega-sidebar")
                .child(
                    // `OMEGA-DELTA-0131`. The same height, background and border
                    // as the thread's toolbar beside it, because they are one
                    // line across the window and were visibly not: this header
                    // took its height from its padding, so the rule sat lower
                    // than the toolbar's, and `border_variant` drew it fainter.
                    // Two rules at two heights in two weights read as a seam.
                    h_flex()
                        .w_full()
                        .h(Tab::container_height(cx))
                        .flex_shrink_0()
                        .px_2()
                        .justify_between()
                        .bg(cx.theme().colors().tab_bar_background)
                        .border_b_1()
                        .border_color(cx.theme().colors().border)
                        .child(Label::new("Omega").size(LabelSize::Small))
                        .child(
                            IconButton::new("collapse-omega-sidebar", IconName::ChevronLeft)
                                .icon_size(IconSize::Small)
                                .tooltip(Tooltip::text("Collapse Sidebar"))
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.toggle_threads_sidebar(cx);
                                })),
                        ),
                )
                .child(self.render_sidebar_controls(cx))
                .child(sections)
                .children(pairing_surface)
                .child(
                    div()
                        .flex_shrink_0()
                        .border_t_1()
                        .border_color(border)
                        .p_1()
                        .child(
                            ListItem::new("open-omega-phone-pairing")
                                .aria_role(gpui::Role::Button)
                                .aria_label("Pair phone")
                                .inset(true)
                                .spacing(ListItemSpacing::Sparse)
                                .start_slot(
                                    Icon::new(IconName::Link)
                                        .size(IconSize::Small)
                                        .color(Color::Muted),
                                )
                                .child(Label::new("Pair phone").size(LabelSize::Small))
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.open_device_pairing(cx);
                                })),
                        )
                        .child(
                            ListItem::new("open-omega-settings")
                                .aria_role(gpui::Role::Button)
                                .aria_label("Open Settings")
                                .inset(true)
                                .spacing(ListItemSpacing::Sparse)
                                .start_slot(
                                    Icon::new(IconName::Settings)
                                        .size(IconSize::Small)
                                        .color(Color::Muted),
                                )
                                .child(Label::new("Settings").size(LabelSize::Small))
                                .on_click(|_, window, cx| {
                                    window.dispatch_action(
                                        zed_actions::OpenSettings.boxed_clone(),
                                        cx,
                                    );
                                }),
                        ),
                )
                .into_any_element(),
        )
    }

    fn render_sidebar_controls(&self, cx: &mut Context<Self>) -> AnyElement {
        h_flex()
            .w_full()
            .flex_shrink_0()
            .gap_1()
            .px_2()
            .pt_2()
            .pb_1()
            .child(
                div().flex_1().child(
                    Button::new("search-omega-sidebar", "Search")
                        .full_width()
                        .style(ButtonStyle::Subtle)
                        .size(ButtonSize::Default)
                        .label_size(LabelSize::Small)
                        .start_icon(
                            Icon::new(IconName::MagnifyingGlass)
                                .size(IconSize::Small)
                                .color(Color::Muted),
                        )
                        .key_binding(KeyBinding::for_action(
                            &zed_actions::command_palette::Toggle,
                            cx,
                        ))
                        .on_click(|_, window, cx| {
                            window.dispatch_action(
                                zed_actions::command_palette::Toggle.boxed_clone(),
                                cx,
                            );
                        }),
                ),
            )
            .child(
                IconButton::new("new-omega-sidebar-thread", IconName::Plus)
                    .style(ButtonStyle::Subtle)
                    .icon_size(IconSize::Small)
                    .tooltip(|_, cx| Tooltip::for_action("New Thread", &NewThread, cx))
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.new_thread(&NewThread, window, cx);
                    })),
            )
            .into_any_element()
    }

    /// One vertically collapsible section: a header that toggles, and a body.
    ///
    /// # Adding a fourth
    ///
    /// One arm here, plus a variant and its `key`/`title` in
    /// [`omega_sidebar::SectionId`]. `omega_deltas` asserts that every variant
    /// of that enum is named in this match, so a section added there and
    /// forgotten here fails the suite rather than drawing an empty heading.
    ///
    /// # No arm may refuse
    ///
    /// Every arm returns something drawable. A section that cannot load puts
    /// one quiet note where its rows would be. The sections above and below it
    /// never find out.
    fn render_sidebar_section(
        &self,
        section: omega_sidebar::SectionId,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let collapsed = self.sidebar.is_collapsed(section);
        let hover_background = cx.theme().colors().element_hover;

        let header = h_flex()
            .id(SharedString::new_static(section.key()))
            .w_full()
            .px_2()
            .py_1()
            .gap_1()
            .cursor_pointer()
            .hover(|style| style.bg(hover_background))
            .child(
                Icon::new(if collapsed {
                    IconName::ChevronRight
                } else {
                    IconName::ChevronDown
                })
                .size(IconSize::XSmall)
                .color(Color::Muted),
            )
            .child(
                Label::new(section.title())
                    .size(LabelSize::XSmall)
                    .color(Color::Muted),
            )
            .on_click(cx.listener(move |this, _, _, cx| {
                this.toggle_sidebar_section(section, cx);
            }));

        let mut column = v_flex().w_full().child(header);
        if collapsed {
            return column.into_any_element();
        }

        column = match section {
            omega_sidebar::SectionId::RecentThreads => {
                column.child(self.render_recent_threads_section(cx))
            }
            omega_sidebar::SectionId::PublicChannels => {
                column.child(self.render_public_channel_destinations(cx))
            }
        };
        column.into_any_element()
    }

    fn select_public_channel(
        &mut self,
        channel_id: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let previous_channel_id = self
            .public_channels
            .selected_channel_id()
            .map(str::to_string);
        if previous_channel_id.as_deref() != Some(channel_id)
            && let Some(previous_view) = previous_channel_id
                .as_ref()
                .and_then(|channel_id| self.public_channel_views.get(channel_id))
        {
            previous_view.update(cx, |view, cx| view.pause(cx));
        }
        if self.public_channels.select(channel_id) {
            let Some(channel) = self.public_channels.channel(channel_id).cloned() else {
                return;
            };
            let view = if let Some(view) = self.public_channel_views.get(channel_id) {
                view.clone()
            } else {
                let http_client: Arc<dyn http_client::HttpClient> =
                    self.project.read(cx).client().http_client();
                let view = cx.new(|cx| {
                    crate::omega_public_channel_view::PublicChannelView::new(
                        channel,
                        http_client,
                        cx,
                    )
                });
                let channel_id_for_snapshot = channel_id.to_string();
                let subscription = cx.subscribe(
                    &view,
                    move |this,
                          _view,
                          event: &crate::omega_public_channel_view::PublicChannelViewEvent,
                          cx| {
                        let crate::omega_public_channel_view::PublicChannelViewEvent::SnapshotChanged(
                            snapshot,
                        ) = event;
                        this.public_channels
                            .apply_snapshot(&channel_id_for_snapshot, snapshot.clone());
                        cx.notify();
                    },
                );
                self._public_channel_view_subscriptions
                    .insert(channel_id.to_string(), subscription);
                self.public_channel_views
                    .insert(channel_id.to_string(), view.clone());
                view
            };
            view.update(cx, |view, cx| view.resume(cx));
            self.showing_full_auto = false;
            self.threads_sidebar_refusal = None;
            self.focus_handle.focus(window, cx);
            cx.emit(AgentPanelEvent::ActiveViewChanged);
            cx.notify();
        }
    }

    fn close_selected_public_channel(&mut self, cx: &mut Context<Self>) {
        if let Some(view) = self
            .public_channels
            .selected_channel_id()
            .and_then(|channel_id| self.public_channel_views.get(channel_id))
        {
            view.update(cx, |view, cx| view.pause(cx));
        }
        self.public_channels.clear_selection();
        cx.emit(AgentPanelEvent::ActiveViewChanged);
        cx.notify();
    }

    fn render_public_channel_destinations(&self, cx: &mut Context<Self>) -> AnyElement {
        let destinations = self.public_channels.destinations();
        if destinations.is_empty() {
            let note = self
                .public_channels_error
                .clone()
                .unwrap_or_else(|| "Loading channels…".into());
            return div()
                .w_full()
                .px_2()
                .py_0p5()
                .pb_1()
                .child(Label::new(note).size(LabelSize::XSmall).color(Color::Muted))
                .into_any_element();
        }

        destinations
            .into_iter()
            .enumerate()
            .fold(
                v_flex().w_full().pb_1().gap_0p5(),
                |list, (index, destination)| {
                    let channel_id = destination.channel_id.clone();
                    let channel_id_for_key = channel_id.clone();
                    let lifecycle = if destination.cached {
                        format!("{} · cached", destination.lifecycle.label())
                    } else {
                        destination.lifecycle.label().to_string()
                    };
                    let unread = (destination.unread > 0)
                        .then(|| format!(" · {} unread", destination.unread));
                    let accessible_description = format!(
                        "{} on {} for group {}{}",
                        lifecycle,
                        destination.relay_url,
                        destination.group_id,
                        unread.clone().unwrap_or_default()
                    );
                    let accessible_label = destination.accessible_label();
                    list.child(
                        v_flex()
                            .w_full()
                            .px_1()
                            .on_key_down(cx.listener(
                                move |this, event: &gpui::KeyDownEvent, window, cx| {
                                    if crate::omega_public_channels::is_channel_activation_key(
                                        event.keystroke.key.as_str(),
                                        event.keystroke.modifiers.modified(),
                                    ) {
                                        this.select_public_channel(&channel_id_for_key, window, cx);
                                        cx.stop_propagation();
                                    }
                                },
                            ))
                            .child(
                                Button::new(
                                    ElementId::Name(
                                        format!("omega-public-channel-{}", destination.channel_id)
                                            .into(),
                                    ),
                                    destination.label,
                                )
                                .style(ButtonStyle::Subtle)
                                .size(ButtonSize::Compact)
                                .full_width()
                                .tab_index(index as isize)
                                .toggle_state(destination.selected)
                                .aria_label(accessible_label)
                                .aria_description(accessible_description)
                                .on_click(cx.listener(
                                    move |this, _, window, cx| {
                                        this.select_public_channel(&channel_id, window, cx);
                                    },
                                )),
                            )
                            .child(
                                Label::new(format!("{}{}", lifecycle, unread.unwrap_or_default()))
                                    .size(LabelSize::XSmall)
                                    .color(Color::Muted),
                            ),
                    )
                },
            )
            .into_any_element()
    }

    fn render_selected_public_channel(&self, cx: &mut Context<Self>) -> AnyElement {
        let Some(channel) = self.public_channels.selected_channel() else {
            return div().into_any_element();
        };
        let selected_view = self.public_channel_views.get(&channel.channel_id).cloned();
        let last_sync = selected_view
            .as_ref()
            .and_then(|view| view.read(cx).last_current_at())
            .and_then(|millis| i64::try_from(millis).ok())
            .and_then(DateTime::<Utc>::from_timestamp_millis)
            .map(|time| {
                format!(
                    "Last sync {}",
                    time.to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
                )
            });
        let lifecycle = self
            .public_channels
            .selected_snapshot()
            .map(|snapshot| snapshot.lifecycle.label())
            .unwrap_or("Not connected");
        v_flex()
            .id("omega-selected-public-channel")
            .size_full()
            .overflow_hidden()
            .child(
                h_flex()
                    .w_full()
                    .px_4()
                    .py_3()
                    .justify_between()
                    .border_b_1()
                    .border_color(cx.theme().colors().border)
                    .child(
                        v_flex()
                            .gap_0p5()
                            .child(Label::new(channel.destination_label()).size(LabelSize::Large))
                            .child(
                                Label::new(channel.display_name.clone())
                                    .size(LabelSize::XSmall)
                                    .color(Color::Muted),
                            )
                            .child(
                                Label::new(format!("{} · {}", channel.relay_url, channel.group_id))
                                    .size(LabelSize::XSmall)
                                    .color(Color::Muted),
                            ),
                    )
                    .child(
                        h_flex()
                            .gap_2()
                            .child(
                                Label::new(lifecycle)
                                    .size(LabelSize::XSmall)
                                    .color(Color::Muted),
                            )
                            .when_some(last_sync, |this, last_sync| {
                                this.child(
                                    Label::new(last_sync)
                                        .size(LabelSize::XSmall)
                                        .color(Color::Muted),
                                )
                            })
                            .child(
                                IconButton::new(
                                    "close-omega-selected-public-channel",
                                    IconName::Close,
                                )
                                .icon_size(IconSize::Small)
                                .tooltip(Tooltip::text("Close Channel"))
                                .on_click(cx.listener(
                                    |this, _, _, cx| {
                                        this.close_selected_public_channel(cx);
                                    },
                                )),
                            ),
                    ),
            )
            .child(
                div()
                    .id("omega-selected-public-channel-content")
                    .min_h_0()
                    .flex_1()
                    .when_some(selected_view, |this, view| this.child(view))
                    .when(
                        !self.public_channel_views.contains_key(&channel.channel_id),
                        |this| {
                            this.child(
                                v_flex().size_full().items_center().justify_center().child(
                                    Label::new("The channel view could not start.")
                                        .size(LabelSize::Small)
                                        .color(Color::Muted),
                                ),
                            )
                        },
                    ),
            )
            .into_any_element()
    }

    /// The last [`omega_sidebar::RECENT_THREADS`] conversations.
    ///
    /// The rows are `OMEGA-DELTA-0118`'s, unchanged and not recomputed here:
    /// [`omega_threads_sidebar::rows`] still decides the order, the exclusions,
    /// the ages and the refusals, and this takes the first ten of them. A
    /// second thread list with its own opinion about which threads are
    /// historical is exactly the "one window giving two answers" failure that
    /// delta's notes name.
    fn render_recent_threads_section(&self, cx: &mut Context<Self>) -> AnyElement {
        let rows: Vec<omega_threads_sidebar::ThreadRow> = self
            .threads_sidebar_rows(cx)
            .into_iter()
            .take(omega_sidebar::RECENT_THREADS)
            .collect();

        if rows.is_empty() {
            return div()
                .w_full()
                .px_2()
                .py_0p5()
                .pb_1()
                .child(
                    Label::new("No past conversations yet.")
                        .size(LabelSize::XSmall)
                        .color(Color::Muted),
                )
                .into_any_element();
        }

        let active_thread_id = self.active_thread_id(cx);

        rows.into_iter()
            .enumerate()
            .fold(v_flex().w_full().pb_1(), |list, (index, row)| {
                let is_active = active_thread_id.as_ref() == Some(&row.thread_id);
                let reopenable = row.is_reopenable();
                // A row that will refuse says so unclicked. It already knew, and
                // a list whose dead rows look exactly like its live ones asks a
                // person to find them one click at a time. The note names the
                // executor itself, in the composer selector's `name — reason`
                // form, so it stands in place of the bare name rather than
                // beside it.
                let executor = row.unavailable_note.clone().or(row.executor.clone());
                let age = row.age.clone();
                let title = row.title.clone();
                list.child(
                    ListItem::new(("threads-sidebar-row", index))
                        .toggle_state(is_active)
                        .inset(true)
                        .spacing(ListItemSpacing::Sparse)
                        .on_click(cx.listener(move |this, _, window, cx| {
                            this.open_thread_from_threads_sidebar(&row, window, cx);
                        }))
                        .child(
                            v_flex()
                                .w_full()
                                .gap_0p5()
                                .child(
                                    Label::new(title)
                                        .size(LabelSize::Small)
                                        .color(if is_active {
                                            Color::Accent
                                        } else if reopenable {
                                            Color::Default
                                        } else {
                                            Color::Muted
                                        })
                                        .truncate(),
                                )
                                .child(
                                    h_flex()
                                        .gap_1p5()
                                        .child(
                                            Label::new(age)
                                                .size(LabelSize::XSmall)
                                                .color(Color::Muted),
                                        )
                                        .children(executor.map(|executor| {
                                            Label::new(executor)
                                                .size(LabelSize::XSmall)
                                                .color(Color::Muted)
                                        })),
                                ),
                        ),
                )
            })
            .children(self.threads_sidebar_refusal.clone().map(|refusal| {
                // In place, in the section, where the click was. Not a toast:
                // `OMEGA-DELTA-0053` records that a zero-base window is where a
                // notification is least likely to be where somebody is looking,
                // and omega#119 records what happened when refusals did toast.
                // In the ordinary colour, because the row now says this before
                // the click and nothing has gone wrong when somebody reads the
                // long version of a fact they were already shown. A warning
                // colour for a machine that simply does not have Codex is how a
                // person learns to read past warnings, which is why
                // `OMEGA-DELTA-0054`'s two first-run notices left it too.
                div().w_full().px_2().py_1().child(
                    Label::new(refusal)
                        .size(LabelSize::XSmall)
                        .color(Color::Muted),
                )
            }))
            .into_any_element()
    }

    /// `OMEGA-DELTA-0034`. Connect the native agent, project or no project.
    ///
    /// This was the guard that had to be understood before any of the others
    /// could move, because it is the one that could plausibly have been
    /// protecting something. It is not: `NativeAgentServer::connect` takes the
    /// project as `_project` and never reads it, and `NativeAgent::new_session`
    /// builds a thread from the project entity without requiring a visible
    /// worktree. What the guard actually bought was not spinning up an agent
    /// connection for a window that upstream had decided would never show the
    /// agent — a resource choice, on a premise Omega does not share.
    fn ensure_native_agent_connection(&self, cx: &mut Context<Self>) {
        let fs = self.fs.clone();
        let thread_store = self.thread_store.clone();
        self.connection_store.update(cx, |store, cx| {
            store.request_connection(
                Agent::NativeAgent,
                Agent::NativeAgent.server(fs, thread_store),
                cx,
            );
        });
    }

    /// `OMEGA-DELTA-0034`. The composer, project or no project.
    pub fn activate_draft(
        &mut self,
        focus: bool,
        source: AgentThreadSource,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let draft = self.ensure_draft(source, window, cx);
        if let BaseView::AgentThread { conversation_view } = &self.base_view {
            if conversation_view.entity_id() == draft.entity_id() {
                if focus {
                    self.activation_focus_handle(cx).focus(window, cx);
                }
                return;
            }
        }
        self.set_base_view(
            BaseView::AgentThread {
                conversation_view: draft,
            },
            focus,
            window,
            cx,
        );
    }

    fn ensure_draft(
        &mut self,
        source: AgentThreadSource,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Entity<ConversationView> {
        let desired_agent = self.selected_agent(cx);
        if let Some(draft) = &self.draft_thread {
            let draft_entity = draft.entity_id();
            let agent_matches = *draft.read(cx).agent_key() == desired_agent;
            let has_editor_content = draft.read(cx).root_thread_view().is_some_and(|tv| {
                !tv.read(cx)
                    .message_editor
                    .read(cx)
                    .text(cx)
                    .trim()
                    .is_empty()
            });
            // Only retarget the empty draft when the user is actively
            // viewing it — that's the case where switching agents in the
            // toolbar should replace the draft with one bound to the
            // newly-selected agent. When the draft is parked in its slot
            // while the user is viewing a real thread, `selected_agent`
            // reflects that real thread's agent and must not be allowed
            // to silently rebuild the draft.
            let draft_is_active = matches!(
                &self.base_view,
                BaseView::AgentThread { conversation_view }
                    if conversation_view.entity_id() == draft_entity
            );

            if agent_matches || has_editor_content || !draft_is_active {
                return draft.clone();
            }

            // Clean up the old empty draft's metadata so it doesn't
            // linger as a ghost entry in the sidebar.
            let old_draft_id = draft.read(cx).thread_id;
            ThreadMetadataStore::global(cx).update(cx, |store, cx| {
                store.delete(old_draft_id, cx);
            });

            self.draft_thread = None;
            self._draft_editor_observation = None;
        }

        let thread = self.create_agent_thread_with_server(
            desired_agent,
            None,
            None,
            None,
            None,
            None,
            None,
            source,
            window,
            cx,
        );

        self.draft_thread = Some(thread.conversation_view.clone());
        self.observe_draft_editor(&thread.conversation_view, cx);
        thread.conversation_view
    }

    fn observe_draft_editor(
        &mut self,
        conversation_view: &Entity<ConversationView>,
        cx: &mut Context<Self>,
    ) {
        if let Some(acp_thread) = conversation_view.read(cx).root_thread(cx) {
            self._draft_editor_observation = Some(cx.subscribe(
                &acp_thread,
                |this, acp_thread, event: &AcpThreadEvent, cx| {
                    if !acp_thread.read(cx).is_draft_thread()
                        && this.draft_thread.as_ref().is_some_and(|draft| {
                            draft
                                .read(cx)
                                .root_thread(cx)
                                .is_some_and(|thread| thread.entity_id() == acp_thread.entity_id())
                        })
                    {
                        this.draft_thread = None;
                        this._draft_editor_observation = None;
                        this.serialize(cx);
                        return;
                    }

                    if let AcpThreadEvent::PromptUpdated = event {
                        this.serialize(cx);
                    }
                },
            ));
        } else {
            let cv = conversation_view.clone();
            self._draft_editor_observation = Some(cx.observe(&cv, |this, cv, cx| {
                if cv.read(cx).root_thread(cx).is_some() {
                    this.observe_draft_editor(&cv, cx);
                }
            }));
        }
    }

    /// Sets up an editor observation on the active view that reclaims
    /// it as ephemeral when the editor becomes empty. Only activates
    /// for non-ephemeral draft threads.
    fn observe_active_draft_for_empty_editor(
        &mut self,
        conversation_view: &Entity<ConversationView>,
        cx: &mut Context<Self>,
    ) {
        let thread_id = conversation_view.read(cx).thread_id;
        let is_ephemeral = self
            .draft_thread
            .as_ref()
            .is_some_and(|d| d.read(cx).thread_id == thread_id);
        if is_ephemeral {
            self._active_draft_reclaim_observation = None;
            return;
        }
        let is_draft = conversation_view
            .read(cx)
            .root_thread(cx)
            .is_some_and(|t| t.read(cx).is_draft_thread());
        if !is_draft {
            self._active_draft_reclaim_observation = None;
            return;
        }
        let Some(editor) = conversation_view
            .read(cx)
            .active_thread()
            .map(|tv| tv.read(cx).message_editor.clone())
        else {
            self._active_draft_reclaim_observation = None;
            return;
        };
        let cv = conversation_view.clone();
        self._active_draft_reclaim_observation =
            Some(cx.observe(&editor, move |this, _editor, cx| {
                let editor_has_text = cv.read(cx).active_thread().is_some_and(|tv| {
                    !tv.read(cx)
                        .message_editor
                        .read(cx)
                        .text(cx)
                        .trim()
                        .is_empty()
                });
                if editor_has_text {
                    return;
                }
                if this.ephemeral_draft_thread_id(cx) == Some(thread_id) {
                    return;
                }
                if this.active_thread_id(cx) != Some(thread_id) {
                    return;
                }
                if this.try_make_empty_draft_ephemeral(cv.clone(), cx) {
                    this._active_draft_reclaim_observation = None;
                    cx.emit(AgentPanelEvent::EntryChanged);
                    cx.notify();
                }
            }));
    }

    fn try_make_empty_draft_ephemeral(
        &mut self,
        conversation_view: Entity<ConversationView>,
        cx: &mut Context<Self>,
    ) -> bool {
        let (thread_id, is_draft, is_empty) = {
            let conversation = conversation_view.read(cx);
            let thread_id = conversation.thread_id;
            let is_draft = conversation
                .root_thread(cx)
                .is_some_and(|thread| thread.read(cx).is_draft_thread());
            let is_empty = if let Some(thread_view) = conversation.active_thread() {
                thread_view
                    .read(cx)
                    .message_editor
                    .read(cx)
                    .text(cx)
                    .trim()
                    .is_empty()
            } else {
                !self.draft_has_content(&conversation_view, cx)
            };

            (thread_id, is_draft, is_empty)
        };

        if !is_draft || !is_empty {
            return false;
        }

        self.retained_threads.remove(&thread_id);
        self.set_ephemeral_draft(conversation_view, cx);
        true
    }

    /// Moves a conversation view into the ephemeral `draft_thread` slot,
    /// cleaning up any previous ephemeral draft and deleting the thread's
    /// metadata so it no longer appears in the sidebar.
    fn set_ephemeral_draft(
        &mut self,
        conversation_view: Entity<ConversationView>,
        cx: &mut Context<Self>,
    ) {
        if let Some(old_draft) = self.draft_thread.take() {
            let old_id = old_draft.read(cx).thread_id;
            let new_id = conversation_view.read(cx).thread_id;
            if old_id != new_id {
                ThreadMetadataStore::global(cx).update(cx, |store, cx| {
                    store.delete(old_id, cx);
                });
            }
            self._draft_editor_observation = None;
        }
        self.draft_thread = Some(conversation_view.clone());
        self.observe_draft_editor(&conversation_view, cx);
        self.serialize(cx);
    }

    /// Creates a new retained thread and inserts it into the sidebar without
    /// switching the active view to it. Used by the `create_thread` agent tool,
    /// which passes an initial prompt, and optionally an agent and model
    /// override.
    pub fn create_thread_with_options(
        &mut self,
        options: CreateThreadOptions,
        source: AgentThreadSource,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> ThreadId {
        let (agent, override_used) = if self.project.read(cx).is_via_collab() {
            (Agent::NativeAgent, false)
        } else if let Some(override_agent) = options.agent {
            (override_agent, true)
        } else {
            (self.selected_agent.clone(), false)
        };
        // If the caller explicitly overrode the agent (e.g., the `create_thread`
        // tool wants to spawn a sibling thread using a specific agent), we
        // shouldn't let that change the panel's selected_agent or the
        // last-used-agent preference. Snapshot and restore both.
        let saved_selected_agent = override_used.then(|| self.selected_agent.clone());
        let thread = self.create_agent_thread_with_server(
            agent,
            None,
            None,
            options.work_dirs,
            options.title.clone(),
            options.initial_content,
            options.model,
            source,
            window,
            cx,
        );
        if let Some(original) = saved_selected_agent {
            self.set_selected_agent_and_persist(original, cx);
        }
        let thread_id = thread.conversation_view.read(cx).thread_id;
        self.retained_threads
            .insert(thread_id, thread.conversation_view);
        thread_id
    }

    /// Creates an Omega Agent thread and submits `message` through the normal
    /// message-editor send path. When `reveal` is false, the new thread stays
    /// retained without changing the panel's active view.
    pub fn create_omega_thread_with_message(
        &mut self,
        message: String,
        reveal: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<ThreadId> {
        if !self.has_open_project(cx) {
            return None;
        }
        let thread_id = self.create_thread_with_options(
            CreateThreadOptions {
                initial_content: Some(AgentInitialContent::ContentBlock {
                    blocks: vec![acp::ContentBlock::Text(acp::TextContent::new(message))],
                    auto_submit: true,
                }),
                agent: Some(Agent::NativeAgent),
                ..CreateThreadOptions::default()
            },
            AgentThreadSource::AgentPanel,
            window,
            cx,
        );
        if reveal {
            self.activate_retained_thread(thread_id, true, window, cx);
        }
        Some(thread_id)
    }

    pub fn activate_retained_thread(
        &mut self,
        id: ThreadId,
        focus: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let conversation_view = if let Some(view) = self.retained_threads.remove(&id) {
            self.try_make_empty_draft_ephemeral(view.clone(), cx);
            view
        } else if let Some(draft) = &self.draft_thread {
            if draft.read(cx).thread_id == id {
                draft.clone()
            } else {
                return;
            }
        } else {
            return;
        };
        self.set_base_view(
            BaseView::AgentThread { conversation_view },
            focus,
            window,
            cx,
        );
    }

    pub fn reveal_omega_thread(
        &mut self,
        id: ThreadId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        if self.active_thread_id(cx) == Some(id) {
            return true;
        }
        if self.retained_threads.contains_key(&id) {
            self.activate_retained_thread(id, true, window, cx);
            return self.active_thread_id(cx) == Some(id);
        }
        let exists = ThreadMetadataStore::try_global(cx)
            .is_some_and(|store| store.read(cx).entry(id).is_some());
        if !exists {
            return false;
        }
        self.load_agent_thread(
            Agent::NativeAgent,
            id,
            None,
            None,
            true,
            AgentThreadSource::AgentPanel,
            window,
            cx,
        );
        self.active_thread_id(cx) == Some(id)
    }

    pub fn active_thread_id(&self, cx: &App) -> Option<ThreadId> {
        match &self.base_view {
            BaseView::AgentThread { conversation_view } => {
                Some(conversation_view.read(cx).thread_id)
            }
            _ => None,
        }
    }

    /// Drops a thread — retained or the active ephemeral draft — from
    /// the panel and deletes its metadata row. Used by the sidebar when
    /// the user dismisses a parked draft.
    pub fn remove_thread(&mut self, id: ThreadId, window: &mut Window, cx: &mut Context<Self>) {
        self.remove_thread_internal(id, true, window, cx);
    }

    pub fn remove_thread_without_activating_draft(
        &mut self,
        id: ThreadId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.remove_thread_internal(id, false, window, cx);
    }

    fn remove_thread_internal(
        &mut self,
        id: ThreadId,
        activate_draft_after_remove: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.retained_threads.remove(&id);
        let thread_key = id.to_key_string();
        self.thread_identity_operation_requests.remove(&thread_key);
        self.thread_identity_pending_operations.remove(&thread_key);
        self.thread_identity_operation_errors.remove(&thread_key);
        if let Err(error) = self.workbench_shell.close_thread(&thread_key) {
            log::warn!("failed to close workbench projection for thread {id:?}: {error:#}");
            self.workbench_shell.record_error(error.to_string());
        }
        ThreadMetadataStore::global(cx).update(cx, |store, cx| {
            store.delete(id, cx);
        });

        if self
            .draft_thread
            .as_ref()
            .is_some_and(|d| d.read(cx).thread_id == id)
        {
            self.draft_thread = None;
            self._draft_editor_observation = None;
        }

        if self.active_thread_id(cx) == Some(id) {
            if activate_draft_after_remove {
                self.activate_draft(false, AgentThreadSource::AgentPanel, window, cx);
            } else {
                self.base_view = BaseView::Uninitialized;
                self.refresh_base_view_subscriptions(window, cx);
            }
            self.serialize(cx);
            cx.emit(AgentPanelEvent::ActiveViewChanged);
            cx.notify();
        }
    }

    pub fn ephemeral_draft_thread_id(&self, cx: &App) -> Option<ThreadId> {
        let draft = self.draft_thread.as_ref()?;
        let draft = draft.read(cx);
        draft
            .root_thread(cx)
            .is_some_and(|thread| thread.read(cx).is_draft_thread())
            .then_some(draft.thread_id)
    }

    pub fn active_terminal_id(&self) -> Option<TerminalId> {
        match &self.base_view {
            BaseView::Terminal { terminal_id } => Some(*terminal_id),
            _ => None,
        }
    }

    pub fn has_terminal(&self, terminal_id: TerminalId) -> bool {
        self.terminals.contains_key(&terminal_id)
    }

    pub fn terminals(&self, cx: &App) -> Vec<AgentPanelTerminalInfo> {
        self.terminals
            .iter()
            .map(|(id, terminal)| AgentPanelTerminalInfo {
                id: *id,
                title: terminal.title(cx),
                created_at: terminal.created_at,
                has_notification: terminal.has_notification,
                custom_title: terminal.custom_title(cx),
                working_directory: terminal.working_directory.clone(),
            })
            .collect()
    }

    pub fn editor_text(&self, id: ThreadId, cx: &App) -> Option<String> {
        self.editor_text_if_in_memory(id, cx).flatten()
    }

    pub fn editor_text_if_in_memory(&self, id: ThreadId, cx: &App) -> Option<Option<String>> {
        let cv = self
            .retained_threads
            .get(&id)
            .or_else(|| {
                self.draft_thread
                    .as_ref()
                    .filter(|draft| draft.read(cx).thread_id == id)
            })
            .or_else(|| match &self.base_view {
                BaseView::AgentThread { conversation_view }
                    if conversation_view.read(cx).thread_id == id =>
                {
                    Some(conversation_view)
                }
                _ => None,
            })?;
        let tv = cv.read(cx).root_thread_view()?;
        let text = tv.read(cx).message_editor.read(cx).text(cx);
        if text.trim().is_empty() {
            Some(None)
        } else {
            Some(Some(text))
        }
    }

    pub fn draft_prompt_blocks_if_in_memory(
        &self,
        id: ThreadId,
        cx: &App,
    ) -> Option<Vec<acp::ContentBlock>> {
        let cv = self
            .retained_threads
            .get(&id)
            .or_else(|| {
                self.draft_thread
                    .as_ref()
                    .filter(|draft| draft.read(cx).thread_id == id)
            })
            .or_else(|| match &self.base_view {
                BaseView::AgentThread { conversation_view }
                    if conversation_view.read(cx).thread_id == id =>
                {
                    Some(conversation_view)
                }
                _ => None,
            })?;
        let thread_view = cv.read(cx).root_thread_view()?;
        let thread_view = thread_view.read(cx);
        Some(
            thread_view
                .message_editor
                .read(cx)
                .draft_content_blocks_snapshot(cx),
        )
    }

    fn new_native_agent_thread_from_summary(
        &mut self,
        action: &NewNativeAgentThreadFromSummary,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let session_id = action.from_session_id.clone();

        let Some(content) = Self::initial_content_for_thread_summary(session_id.clone(), cx) else {
            log::error!("No session found for summarization with id {}", session_id);
            return;
        };

        cx.spawn_in(window, async move |this, cx| {
            this.update_in(cx, |this, window, cx| {
                this.external_thread(
                    Some(Agent::NativeAgent),
                    None,
                    None,
                    None,
                    Some(content),
                    true,
                    AgentThreadSource::AgentPanel,
                    window,
                    cx,
                );
                anyhow::Ok(())
            })
        })
        .detach_and_log_err(cx);
    }

    fn initial_content_for_thread_summary(
        session_id: acp::SessionId,
        cx: &App,
    ) -> Option<AgentInitialContent> {
        let thread = ThreadStore::global(cx)
            .read(cx)
            .entries()
            .find(|t| t.id == session_id)?;

        Some(AgentInitialContent::ThreadSummary {
            session_id: thread.id,
            title: Some(thread.title),
        })
    }

    /// Open a thread and submit one message on it, with nobody at the keyboard.
    ///
    /// `OMEGA-DELTA-0093`, omega#100. Synthetic keystrokes are unusable on a
    /// busy desktop — twice, an attempt to prove something about a turn failed
    /// because the keys landed in another application — so every visual claim
    /// about a turn depends on being able to send one without the window
    /// having focus.
    ///
    /// **This is not a second way to send.** It hands
    /// [`AgentInitialContent::ContentBlock`] with `auto_submit` to
    /// [`Self::external_thread`], which is the same call the Git panel's
    /// "review this branch diff" action already makes. The content lands in the
    /// composer through `MessageEditor::set_message`, and the submit is
    /// `ThreadView::send` — the identical function the Enter key reaches. A
    /// control surface that bypassed the production path would prove nothing
    /// about the production path, which is the whole reason this is a thin
    /// wrapper rather than a driver of its own.
    ///
    /// Returns whether a thread was opened. `false` means the panel had no
    /// project, which `external_thread` refuses on: a thread whose file tools
    /// have no worktree is the `OMEGA-DELTA-0054` failure, and a driver that
    /// reported success there would be reporting that a turn had started when
    /// none had.
    pub fn omega_send_first_message(
        &mut self,
        text: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        if !self.has_open_project(cx) {
            return false;
        }
        self.external_thread(
            None,
            None,
            None,
            None,
            Some(AgentInitialContent::ContentBlock {
                blocks: vec![acp::ContentBlock::Text(acp::TextContent::new(text))],
                auto_submit: true,
            }),
            true,
            AgentThreadSource::AgentPanel,
            window,
            cx,
        );
        true
    }

    /// The thread the panel is showing, once it has one.
    ///
    /// `OMEGA-DELTA-0093`. Connecting an agent and completing ACP
    /// initialization are real I/O, so the thread is not there the instant
    /// [`Self::omega_send_first_message`] returns. An unattended driver polls
    /// this rather than sleeping for a duration somebody guessed.
    pub fn omega_active_acp_thread(&self, cx: &App) -> Option<Entity<AcpThread>> {
        Some(
            self.active_conversation_view()?
                .read(cx)
                .active_thread()?
                .read(cx)
                .thread
                .clone(),
        )
    }

    fn external_thread(
        &mut self,
        agent_choice: Option<crate::Agent>,
        resume_thread_id: Option<ThreadId>,
        work_dirs: Option<PathList>,
        title: Option<SharedString>,
        initial_content: Option<AgentInitialContent>,
        focus: bool,
        source: AgentThreadSource,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if resume_thread_id.is_none() && !self.has_open_project(cx) {
            return;
        }

        let agent = agent_choice.unwrap_or_else(|| self.selected_agent(cx));
        let thread = self.create_agent_thread_with_server(
            agent,
            None,
            resume_thread_id,
            work_dirs,
            title,
            initial_content,
            None,
            source,
            window,
            cx,
        );
        self.set_base_view(thread.into(), focus, window, cx);
    }

    fn manage_skills(
        &mut self,
        _action: &ManageSkills,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        window.dispatch_action(
            Box::new(zed_actions::OpenSettingsAt {
                path: zed_actions::AGENT_SKILLS_SETTINGS_PATH.to_string(),
                target: None,
            }),
            cx,
        );
    }

    /// Refresh the native agent's view of available skills
    pub fn refresh_skills(&mut self, cx: &mut Context<Self>) {
        if !self.has_open_project(cx) {
            return;
        }

        self.ensure_native_agent_connection(cx);
        let Some(connect_task) = self.connection_store.update(cx, |store, cx| {
            store
                .entry(&Agent::NativeAgent)
                .map(|entry| entry.read(cx).wait_for_connection())
        }) else {
            return;
        };
        let project = self.project.clone();
        cx.spawn(async move |_this, cx| -> Result<()> {
            let connected = connect_task.await?;
            // OMEGA-DELTA-0035. The store's connection is the router; the
            // native loop is underneath it.
            if let Some(native_connection) =
                crate::omega_router::native_connection(&connected.connection)
            {
                cx.update(|cx| native_connection.refresh_skills_for_project(project, cx));
            }
            Ok(())
        })
        .detach_and_log_err(cx);
    }

    fn expand_message_editor(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(conversation_view) = self.active_conversation_view() else {
            return;
        };

        let Some(active_thread) = conversation_view.read(cx).root_thread_view() else {
            return;
        };

        active_thread.update(cx, |active_thread, cx| {
            active_thread.expand_message_editor(&ExpandMessageEditor, window, cx);
            active_thread.activation_focus_handle(cx).focus(window, cx);
        })
    }

    pub fn toggle_options_menu(
        &mut self,
        _: &ToggleOptionsMenu,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        window.focus(&self.focus_handle, cx);
        self.agent_panel_menu_handle.toggle(window, cx);
    }

    /// `OMEGA-DELTA-0034`. The new-thread menu, project or no project — it is
    /// how the front door reaches Full Auto and the executor choices.
    pub fn toggle_new_thread_menu(
        &mut self,
        _: &ToggleNewThreadMenu,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.new_thread_menu_handle.toggle(window, cx);
    }

    pub fn increase_font_size(
        &mut self,
        action: &IncreaseBufferFontSize,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.handle_font_size_action(action.persist, px(1.0), cx);
    }

    pub fn decrease_font_size(
        &mut self,
        action: &DecreaseBufferFontSize,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.handle_font_size_action(action.persist, px(-1.0), cx);
    }

    fn handle_font_size_action(&mut self, persist: bool, delta: Pixels, cx: &mut Context<Self>) {
        match self.visible_font_size() {
            WhichFontSize::AgentFont => {
                if persist {
                    update_settings_file(self.fs.clone(), cx, move |settings, cx| {
                        let agent_ui_font_size =
                            ThemeSettings::get_global(cx).agent_ui_font_size(cx) + delta;
                        let agent_buffer_font_size =
                            ThemeSettings::get_global(cx).agent_buffer_font_size(cx) + delta;

                        let _ = settings.theme.agent_ui_font_size.insert(
                            f32::from(theme_settings::clamp_font_size(agent_ui_font_size)).into(),
                        );
                        let _ = settings.theme.agent_buffer_font_size.insert(
                            f32::from(theme_settings::clamp_font_size(agent_buffer_font_size))
                                .into(),
                        );
                    });
                } else {
                    theme_settings::adjust_agent_ui_font_size(cx, |size| size + delta);
                    theme_settings::adjust_agent_buffer_font_size(cx, |size| size + delta);
                }
            }
            WhichFontSize::None => {
                // The agent panel does not own this font size (e.g. when a
                // terminal is the visible surface). Let the action bubble up
                // to the workspace handler so the global buffer font size is
                // adjusted instead.
                cx.propagate();
            }
        }
    }

    pub fn reset_font_size(
        &mut self,
        action: &ResetBufferFontSize,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match self.visible_font_size() {
            WhichFontSize::AgentFont => {
                if action.persist {
                    update_settings_file(self.fs.clone(), cx, move |settings, _| {
                        settings.theme.agent_ui_font_size = None;
                        settings.theme.agent_buffer_font_size = None;
                    });
                } else {
                    theme_settings::reset_agent_ui_font_size(cx);
                    theme_settings::reset_agent_buffer_font_size(cx);
                }
            }
            WhichFontSize::None => {
                // Let the workspace handler reset the global buffer font size
                // that the terminal uses.
                cx.propagate();
            }
        }
    }

    pub fn reset_agent_zoom(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        theme_settings::reset_agent_ui_font_size(cx);
        theme_settings::reset_agent_buffer_font_size(cx);
    }

    pub fn toggle_zoom(&mut self, _: &ToggleZoom, window: &mut Window, cx: &mut Context<Self>) {
        if self.zoomed {
            cx.emit(PanelEvent::ZoomOut);
        } else {
            if !self.focus_handle(cx).contains_focused(window, cx) {
                self.activation_focus_handle(cx).focus(window, cx);
            }
            cx.emit(PanelEvent::ZoomIn);
        }
    }

    pub(crate) fn open_active_thread_as_markdown(
        &mut self,
        _: &OpenActiveThreadAsMarkdown,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(workspace) = self.workspace.upgrade()
            && let Some(conversation_view) = self.active_conversation_view()
            && let Some(active_thread) = conversation_view.read(cx).active_thread().cloned()
        {
            active_thread.update(cx, |thread, cx| {
                thread
                    .open_thread_as_markdown(workspace, window, cx)
                    .detach_and_log_err(cx);
            });
        }
    }

    pub fn open_thread_as_markdown(
        &mut self,
        thread_id: ThreadId,
        workspace: Entity<Workspace>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some(conversation_view) = self.conversation_view_for_id(&thread_id, cx).cloned() else {
            return false;
        };
        let Some(thread_view) = conversation_view.read(cx).root_thread_view() else {
            return false;
        };
        thread_view.update(cx, |thread, cx| {
            thread
                .open_thread_as_markdown(workspace, window, cx)
                .detach_and_log_err(cx);
        });
        true
    }

    fn copy_thread_to_clipboard(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(thread) = self.active_native_agent_thread(cx) else {
            Self::show_deferred_toast(&self.workspace, "No active native thread to copy", cx);
            return;
        };

        let workspace = self.workspace.clone();
        let load_task = thread.read(cx).to_db(cx);

        cx.spawn_in(window, async move |_this, cx| {
            let db_thread = load_task.await;
            let shared_thread = SharedThread::from_db_thread(&db_thread);
            let thread_data = shared_thread.to_bytes()?;
            let encoded = base64::Engine::encode(&base64::prelude::BASE64_STANDARD, &thread_data);

            cx.update(|_window, cx| {
                cx.write_to_clipboard(ClipboardItem::new_string(encoded));
                if let Some(workspace) = workspace.upgrade() {
                    workspace.update(cx, |workspace, cx| {
                        struct ThreadCopiedToast;
                        workspace.show_toast(
                            workspace::Toast::new(
                                workspace::notifications::NotificationId::unique::<ThreadCopiedToast>(),
                                "Thread copied to clipboard (base64 encoded)",
                            )
                            .autohide(),
                            cx,
                        );
                    });
                }
            })?;

            anyhow::Ok(())
        })
        .detach_and_log_err(cx);
    }

    fn show_deferred_toast(
        workspace: &WeakEntity<workspace::Workspace>,
        message: &'static str,
        cx: &mut App,
    ) {
        let workspace = workspace.clone();
        cx.defer(move |cx| {
            if let Some(workspace) = workspace.upgrade() {
                workspace.update(cx, |workspace, cx| {
                    struct ClipboardToast;
                    workspace.show_toast(
                        workspace::Toast::new(
                            workspace::notifications::NotificationId::unique::<ClipboardToast>(),
                            message,
                        )
                        .autohide(),
                        cx,
                    );
                });
            }
        });
    }

    fn load_thread_from_clipboard(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.has_open_project(cx) {
            Self::show_deferred_toast(&self.workspace, "Open a project to load a thread", cx);
            return;
        }

        let Some(clipboard) = cx.read_from_clipboard() else {
            Self::show_deferred_toast(&self.workspace, "No clipboard content available", cx);
            return;
        };

        let Some(encoded) = clipboard.text() else {
            Self::show_deferred_toast(&self.workspace, "Clipboard does not contain text", cx);
            return;
        };

        let thread_data = match base64::Engine::decode(&base64::prelude::BASE64_STANDARD, &encoded)
        {
            Ok(data) => data,
            Err(_) => {
                Self::show_deferred_toast(
                    &self.workspace,
                    "Failed to decode clipboard content (expected base64)",
                    cx,
                );
                return;
            }
        };

        let shared_thread = match SharedThread::from_bytes(&thread_data) {
            Ok(thread) => thread,
            Err(_) => {
                Self::show_deferred_toast(
                    &self.workspace,
                    "Failed to parse thread data from clipboard",
                    cx,
                );
                return;
            }
        };

        let db_thread = shared_thread.to_db_thread();
        let session_id = acp::SessionId::new(uuid::Uuid::new_v4().to_string());
        let thread_store = self.thread_store.clone();
        let title = db_thread.title.clone();
        let workspace = self.workspace.clone();

        cx.spawn_in(window, async move |this, cx| {
            thread_store
                .update(&mut cx.clone(), |store, cx| {
                    store.save_thread(session_id.clone(), db_thread, Default::default(), cx)
                })
                .await?;

            this.update_in(cx, |this, window, cx| {
                this.open_thread(session_id, None, Some(title), window, cx);
            })?;

            this.update_in(cx, |_, _window, cx| {
                if let Some(workspace) = workspace.upgrade() {
                    workspace.update(cx, |workspace, cx| {
                        struct ThreadLoadedToast;
                        workspace.show_toast(
                            workspace::Toast::new(
                                workspace::notifications::NotificationId::unique::<ThreadLoadedToast>(),
                                "Thread loaded from clipboard",
                            )
                            .autohide(),
                            cx,
                        );
                    });
                }
            })?;

            anyhow::Ok(())
        })
        .detach_and_log_err(cx);
    }

    fn show_thread_metadata(
        &mut self,
        _: &ShowThreadMetadata,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(thread_id) = self.active_thread_id(cx) else {
            Self::show_deferred_toast(&self.workspace, "No active thread", cx);
            return;
        };

        let Some(store) = ThreadMetadataStore::try_global(cx) else {
            Self::show_deferred_toast(&self.workspace, "Thread metadata store not available", cx);
            return;
        };

        let Some(metadata) = store.read(cx).entry(thread_id).cloned() else {
            Self::show_deferred_toast(&self.workspace, "No metadata found for active thread", cx);
            return;
        };

        let json = thread_metadata_to_debug_json(&metadata);
        let text = serde_json::to_string_pretty(&json).unwrap_or_default();
        let title = format!("Thread Metadata: {}", metadata.display_title());

        self.open_json_buffer(title, text, window, cx);
    }

    fn show_all_sidebar_thread_metadata(
        &mut self,
        _: &ShowAllSidebarThreadMetadata,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(store) = ThreadMetadataStore::try_global(cx) else {
            Self::show_deferred_toast(&self.workspace, "Thread metadata store not available", cx);
            return;
        };

        let entries: Vec<serde_json::Value> = store
            .read(cx)
            .entries()
            .filter(|t| !t.archived)
            .map(thread_metadata_to_debug_json)
            .collect();

        let json = serde_json::Value::Array(entries);
        let text = serde_json::to_string_pretty(&json).unwrap_or_default();

        self.open_json_buffer("All Sidebar Thread Metadata".to_string(), text, window, cx);
    }

    fn open_json_buffer(
        &self,
        title: String,
        text: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let json_language = self.language_registry.language_for_name("JSON");
        let project = self.project.clone();
        let workspace = self.workspace.clone();

        window
            .spawn(cx, async move |cx| {
                let json_language = json_language.await.ok();

                let buffer = project
                    .update(cx, |project, cx| {
                        project.create_buffer(json_language, false, cx)
                    })
                    .await?;

                buffer.update(cx, |buffer, cx| {
                    buffer.set_text(text, cx);
                    buffer.set_capability(language::Capability::ReadWrite, cx);
                });

                workspace.update_in(cx, |workspace, window, cx| {
                    let buffer =
                        cx.new(|cx| MultiBuffer::singleton(buffer, cx).with_title(title.clone()));

                    workspace.add_item_to_active_pane(
                        Box::new(cx.new(|cx| {
                            let mut editor =
                                Editor::for_multibuffer(buffer, Some(project.clone()), window, cx);
                            editor.set_breadcrumb_header(title);
                            editor.disable_mouse_wheel_zoom();
                            editor
                        })),
                        None,
                        true,
                        window,
                        cx,
                    );
                })?;

                anyhow::Ok(())
            })
            .detach_and_log_err(cx);
    }

    pub fn workspace_id(&self) -> Option<WorkspaceId> {
        self.workspace_id
    }

    pub fn retained_threads(&self) -> &HashMap<ThreadId, Entity<ConversationView>> {
        &self.retained_threads
    }

    pub fn active_conversation_view(&self) -> Option<&Entity<ConversationView>> {
        match &self.base_view {
            BaseView::AgentThread { conversation_view } => Some(conversation_view),
            _ => None,
        }
    }

    pub(crate) fn visible_conversation_view(&self) -> Option<&Entity<ConversationView>> {
        match self.visible_surface() {
            VisibleSurface::AgentThread(conversation_view) => Some(conversation_view),
            _ => None,
        }
    }

    pub fn visible_terminal_view(&self) -> Option<&Entity<TerminalView>> {
        match self.visible_surface() {
            VisibleSurface::Terminal(terminal_view) => Some(terminal_view),
            _ => None,
        }
    }

    fn toggle_terminal_thread_search(
        &mut self,
        _: &crate::ToggleSearch,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(terminal) = self
            .active_terminal_id()
            .and_then(|terminal_id| self.terminals.get_mut(&terminal_id))
        else {
            cx.propagate();
            return;
        };

        let terminal_view = terminal.view.clone();
        let search_bar = terminal
            .search_bar
            .get_or_insert_with(|| cx.new(|cx| BufferSearchBar::new(None, window, cx)))
            .clone();
        let deployed = search_bar.update(cx, |search_bar, cx| {
            let terminal_item: &dyn ItemHandle = &terminal_view;
            search_bar.set_active_pane_item(Some(terminal_item), window, cx);
            search_bar.deploy(&DeployBufferSearch::find(), None, window, cx)
        });
        if deployed {
            cx.stop_propagation();
        } else {
            cx.propagate();
        }
    }

    pub fn conversation_view_for_id(
        &self,
        thread_id: &ThreadId,
        cx: &App,
    ) -> Option<&Entity<ConversationView>> {
        self.retained_threads.get(thread_id).or_else(|| {
            if let Some(view) = self.active_conversation_view()
                && view.read(cx).thread_id == *thread_id
            {
                Some(view)
            } else {
                None
            }
        })
    }

    pub fn regenerate_thread_title(
        &mut self,
        thread_id: ThreadId,
        cx: &mut Context<Self>,
    ) -> ThreadTitleRegenerationResult {
        let Some(conversation_view) = self.conversation_view_for_id(&thread_id, cx).cloned() else {
            return ThreadTitleRegenerationResult::NotOpen;
        };
        Self::regenerate_conversation_thread_title(conversation_view, cx)
    }

    fn regenerate_conversation_thread_title(
        conversation_view: Entity<ConversationView>,
        cx: &mut App,
    ) -> ThreadTitleRegenerationResult {
        let Some(thread) = conversation_view.read(cx).as_native_thread(cx) else {
            return ThreadTitleRegenerationResult::NotOpen;
        };
        let thread_id = conversation_view.read(cx).parent_id();
        thread.update(cx, |thread, cx| {
            if thread.is_generating_title() {
                ThreadTitleRegenerationResult::AlreadyGenerating
            } else if thread.summarization_model().is_none() {
                ThreadTitleRegenerationResult::NoModel
            } else if thread.regenerate_title_with_callback(cx, move |title, cx| {
                ThreadMetadataStore::global(cx).update(cx, |store, cx| {
                    store.set_generated_title(thread_id, title, cx);
                });
            }) {
                ThreadTitleRegenerationResult::Started
            } else {
                ThreadTitleRegenerationResult::AlreadyGenerating
            }
        })
    }

    pub fn conversation_views(&self) -> Vec<Entity<ConversationView>> {
        self.active_conversation_view()
            .into_iter()
            .cloned()
            .chain(self.retained_threads.values().cloned())
            .collect()
    }

    pub fn active_thread_view(&self, cx: &App) -> Option<Entity<ThreadView>> {
        let server_view = self.active_conversation_view()?;
        server_view.read(cx).root_thread_view()
    }

    pub fn active_agent_thread(&self, cx: &App) -> Option<Entity<AcpThread>> {
        match &self.base_view {
            BaseView::AgentThread { conversation_view } => {
                conversation_view.read(cx).root_thread(cx)
            }
            _ => None,
        }
    }

    pub fn is_retained_thread(&self, id: &ThreadId) -> bool {
        self.retained_threads.contains_key(id)
    }

    pub fn cancel_thread(&self, thread_id: &ThreadId, cx: &mut Context<Self>) -> bool {
        let conversation_views = self
            .active_conversation_view()
            .into_iter()
            .chain(self.retained_threads.values());

        for conversation_view in conversation_views {
            if *thread_id == conversation_view.read(cx).thread_id {
                if let Some(thread_view) = conversation_view.read(cx).root_thread_view() {
                    thread_view.update(cx, |view, cx| view.cancel_generation(cx));
                    return true;
                }
            }
        }
        false
    }

    fn update_thread_work_dirs(&self, cx: &mut Context<Self>) {
        let default_work_dirs = self.project.read(cx).default_path_list(cx);

        if let Some(conversation_view) = self.active_conversation_view() {
            if conversation_view.read(cx).work_dirs().is_empty() {
                conversation_view.update(cx, |conversation_view, cx| {
                    conversation_view.set_work_dirs(default_work_dirs.clone(), cx);
                });
            }
        }

        for conversation_view in self.retained_threads.values() {
            if conversation_view.read(cx).work_dirs().is_empty() {
                conversation_view.update(cx, |conversation_view, cx| {
                    conversation_view.set_work_dirs(default_work_dirs.clone(), cx);
                });
            }
        }
    }

    fn retain_running_thread(&mut self, old_view: BaseView, cx: &mut Context<Self>) {
        let BaseView::AgentThread { conversation_view } = old_view else {
            return;
        };

        if self
            .draft_thread
            .as_ref()
            .is_some_and(|d| d.entity_id() == conversation_view.entity_id())
        {
            if self.draft_has_content(&conversation_view, cx) {
                let thread_id = conversation_view.read(cx).thread_id;
                self.draft_thread = None;
                self._draft_editor_observation = None;
                self.retained_threads.insert(thread_id, conversation_view);
                self.cleanup_retained_threads(cx);
            }
            return;
        }

        let thread_id = conversation_view.read(cx).thread_id;

        if self.retained_threads.contains_key(&thread_id) {
            return;
        }

        self.retained_threads.insert(thread_id, conversation_view);
        self.cleanup_retained_threads(cx);
    }

    fn cleanup_retained_threads(&mut self, cx: &App) {
        let mut potential_removals = self
            .retained_threads
            .iter()
            .filter(|(_id, view)| {
                let Some(thread_view) = view.read(cx).root_thread_view() else {
                    return true;
                };
                let thread = thread_view.read(cx).thread.read(cx);
                thread.connection().supports_load_session() && thread.status() == ThreadStatus::Idle
            })
            .collect::<Vec<_>>();

        let max_idle = MaxIdleRetainedThreads::global(cx);

        potential_removals.sort_unstable_by_key(|(_, view)| view.read(cx).updated_at(cx));
        let n = potential_removals.len().saturating_sub(max_idle);
        let to_remove = potential_removals
            .into_iter()
            .map(|(id, _)| *id)
            .take(n)
            .collect::<Vec<_>>();
        for id in to_remove {
            self.retained_threads.remove(&id);
        }
    }

    pub(crate) fn active_native_agent_thread(&self, cx: &App) -> Option<Entity<agent::Thread>> {
        match &self.base_view {
            BaseView::AgentThread { conversation_view } => {
                conversation_view.read(cx).as_native_thread(cx)
            }
            _ => None,
        }
    }

    fn set_base_view(
        &mut self,
        new_view: BaseView,
        focus: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // Opening a thread or terminal leaves the Full Auto surface. The run
        // itself keeps going — `omega-effectd` owns it, not this panel — and
        // the surface is retained, so returning to it restores the draft and
        // the selected run.
        self.showing_full_auto = false;
        if let Some(view) = self
            .public_channels
            .selected_channel_id()
            .and_then(|channel_id| self.public_channel_views.get(channel_id))
        {
            view.update(cx, |view, cx| view.pause(cx));
        }
        self.public_channels.clear_selection();

        let old_view = std::mem::replace(&mut self.base_view, new_view);
        self.retain_running_thread(old_view, cx);

        if let BaseView::AgentThread { conversation_view } = &self.base_view {
            let conversation_view = conversation_view.read(cx);
            let thread_agent = conversation_view.agent_key().clone();
            if self.selected_agent != thread_agent {
                self.selected_agent = thread_agent;
                self.serialize(cx);
            }
        }

        self.refresh_base_view_subscriptions(window, cx);
        self.sync_workbench_shell(window, cx);

        if focus {
            if matches!(
                self.workbench_shell.focus_target(),
                workbench_shell::WorkbenchFocusTarget::Surface(_)
            ) && let Some(host) = self.workbench_shell.visible_host()
            {
                host.focus_handle(cx).focus(window, cx);
            } else {
                self.activation_focus_handle(cx).focus(window, cx);
            }
        }
        cx.emit(AgentPanelEvent::ActiveViewChanged);
    }

    fn refresh_base_view_subscriptions(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self._base_view_observation = match &self.base_view {
            BaseView::AgentThread { conversation_view } => {
                self._thread_view_subscription =
                    Self::subscribe_to_active_thread_view(conversation_view, window, cx);
                let focus_handle = conversation_view.focus_handle(cx);
                self._active_thread_focus_subscription =
                    Some(cx.on_focus_in(&focus_handle, window, |_this, _window, cx| {
                        cx.emit(AgentPanelEvent::ActiveViewFocused);
                        cx.notify();
                    }));
                let cv = conversation_view.clone();
                self.observe_active_draft_for_empty_editor(&cv, cx);
                Some(cx.observe_in(&cv, window, |this, server_view, window, cx| {
                    this._thread_view_subscription =
                        Self::subscribe_to_active_thread_view(&server_view, window, cx);
                    this.observe_active_draft_for_empty_editor(&server_view, cx);
                    cx.emit(AgentPanelEvent::ActiveViewChanged);
                    this.serialize(cx);
                    cx.notify();
                }))
            }
            BaseView::Terminal { terminal_id } => {
                self._thread_view_subscription = None;
                if let Some(terminal) = self.terminals.get(terminal_id) {
                    let terminal_id = *terminal_id;
                    let focus_handle = terminal.view.focus_handle(cx);
                    self._active_thread_focus_subscription =
                        Some(
                            cx.on_focus_in(&focus_handle, window, move |this, _window, cx| {
                                if let Some(terminal) = this.terminals.get_mut(&terminal_id) {
                                    terminal.has_notification = false;
                                }
                                cx.emit(AgentPanelEvent::ActiveViewFocused);
                                cx.notify();
                            }),
                        );
                } else {
                    self._active_thread_focus_subscription = None;
                }
                None
            }
            BaseView::Uninitialized => {
                self._thread_view_subscription = None;
                self._active_thread_focus_subscription = None;
                None
            }
        };
        self.serialize(cx);
    }

    fn visible_surface(&self) -> VisibleSurface<'_> {
        match &self.base_view {
            BaseView::Uninitialized => VisibleSurface::Uninitialized,
            BaseView::AgentThread { conversation_view } => {
                VisibleSurface::AgentThread(conversation_view)
            }
            BaseView::Terminal { terminal_id } => self
                .terminals
                .get(terminal_id)
                .map(|terminal| VisibleSurface::Terminal(&terminal.view))
                .unwrap_or(VisibleSurface::Uninitialized),
        }
    }

    fn visible_font_size(&self) -> WhichFontSize {
        self.base_view.which_font_size_used()
    }

    fn subscribe_to_active_thread_view(
        server_view: &Entity<ConversationView>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<Subscription> {
        server_view.read(cx).root_thread_view().map(|tv| {
            cx.subscribe_in(
                &tv,
                window,
                |this, _view, event: &AcpThreadViewEvent, _window, cx| match event {
                    AcpThreadViewEvent::Interacted => {
                        let Some(thread_id) = this.active_thread_id(cx) else {
                            return;
                        };
                        // If the draft was the active thread, it has now been
                        // promoted to a real thread. Clear the ephemeral
                        // pointer; the ConversationView itself stays put as
                        // the active base view.
                        if this
                            .draft_thread
                            .as_ref()
                            .is_some_and(|draft| draft.read(cx).thread_id == thread_id)
                        {
                            this.draft_thread = None;
                            this._draft_editor_observation = None;
                        }
                        this.retained_threads.remove(&thread_id);
                        cx.emit(AgentPanelEvent::ThreadInteracted { thread_id });
                    }
                },
            )
        })
    }

    fn migrate_agent_server_from_extensions(&mut self, id: Arc<str>, cx: &mut Context<Self>) {
        self.project.update(cx, |project, cx| {
            project.agent_server_store().update(cx, |store, cx| {
                store.migrate_agent_server_from_extensions(id, project.fs().clone(), cx);
            });
        });
    }

    pub fn new_agent_thread_with_external_source_prompt(
        &mut self,
        external_source_prompt: Option<ExternalSourcePrompt>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.external_thread(
            None,
            None,
            None,
            None,
            external_source_prompt.map(AgentInitialContent::from),
            true,
            AgentThreadSource::AgentPanel,
            window,
            cx,
        );
    }

    pub fn load_agent_thread(
        &mut self,
        agent: Agent,
        thread_id: ThreadId,
        work_dirs: Option<PathList>,
        title: Option<SharedString>,
        focus: bool,
        source: AgentThreadSource,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(store) = ThreadMetadataStore::try_global(cx) {
            store.update(cx, |store, cx| {
                store.unarchive(thread_id, cx);
            });
        }

        // Check if the active view already holds this thread.
        if let BaseView::AgentThread { conversation_view } = &self.base_view {
            if conversation_view.read(cx).thread_id == thread_id {
                cx.emit(AgentPanelEvent::ActiveViewChanged);
                return;
            }
        }

        // Check if the thread is already in memory — either as the
        // ephemeral draft pointer or in retained_threads. Either way we
        // can just reactivate without touching storage.
        if let Some(draft) = self.draft_thread.clone()
            && draft.read(cx).thread_id == thread_id
        {
            self.set_base_view(
                BaseView::AgentThread {
                    conversation_view: draft,
                },
                focus,
                window,
                cx,
            );
            return;
        }
        if let Some(conversation_view) = self.retained_threads.remove(&thread_id) {
            self.try_make_empty_draft_ephemeral(conversation_view.clone(), cx);
            self.set_base_view(
                BaseView::AgentThread { conversation_view },
                focus,
                window,
                cx,
            );
            return;
        }

        // Not in memory. Build a fresh ConversationView. For drafts we
        // also seed the message editor with any prompt text the user had
        // typed before closing the window (persisted in the scoped kvp
        // draft-prompt store).
        let is_draft = ThreadMetadataStore::try_global(cx)
            .and_then(|store| store.read(cx).entry(thread_id).map(|m| m.is_draft()))
            .unwrap_or(false);
        let initial_content = is_draft
            .then(|| crate::draft_prompt_store::read(thread_id, cx))
            .flatten()
            .map(|blocks| AgentInitialContent::ContentBlock {
                blocks,
                auto_submit: false,
            });

        self.external_thread(
            Some(agent),
            Some(thread_id),
            work_dirs,
            title,
            initial_content,
            focus,
            source,
            window,
            cx,
        );
    }

    pub(crate) fn create_agent_thread_with_server(
        &mut self,
        agent: Agent,
        server_override: Option<Rc<dyn AgentServer>>,
        resume_thread_id: Option<ThreadId>,
        work_dirs: Option<PathList>,
        title: Option<SharedString>,
        initial_content: Option<AgentInitialContent>,
        model_override: Option<String>,
        source: AgentThreadSource,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AgentThread {
        let resume_session_id = resume_thread_id.and_then(|tid| {
            ThreadMetadataStore::try_global(cx)
                .and_then(|store| store.read(cx).entry(tid).and_then(|m| m.session_id.clone()))
        });
        self.create_agent_thread_inner(
            agent,
            server_override,
            resume_thread_id,
            resume_session_id,
            work_dirs,
            title,
            initial_content,
            model_override,
            source,
            window,
            cx,
        )
    }

    /// Legacy entry that resumes a thread by raw ACP session id when no
    /// local [`ThreadMetadata`] row exists yet (share-link imports and
    /// clipboard imports).
    ///
    /// TODO(legacy-session-id): migrate remaining callers (share-link
    /// handler, clipboard import) to mint a [`ThreadId`] + seed metadata
    /// so they can route through [`create_agent_thread_with_server`] and
    /// this entry can be deleted.
    fn create_agent_thread_with_server_for_external_session(
        &mut self,
        agent: Agent,
        server_override: Option<Rc<dyn AgentServer>>,
        resume_session_id: acp::SessionId,
        work_dirs: Option<PathList>,
        title: Option<SharedString>,
        initial_content: Option<AgentInitialContent>,
        source: AgentThreadSource,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AgentThread {
        self.create_agent_thread_inner(
            agent,
            server_override,
            None,
            Some(resume_session_id),
            work_dirs,
            title,
            initial_content,
            None,
            source,
            window,
            cx,
        )
    }

    fn create_agent_thread_inner(
        &mut self,
        agent: Agent,
        server_override: Option<Rc<dyn AgentServer>>,
        resume_thread_id: Option<ThreadId>,
        resume_session_id: Option<acp::SessionId>,
        work_dirs: Option<PathList>,
        title: Option<SharedString>,
        initial_content: Option<AgentInitialContent>,
        model_override: Option<String>,
        source: AgentThreadSource,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AgentThread {
        let thread_id = resume_thread_id.unwrap_or_else(ThreadId::new);
        let workspace = self.workspace.clone();
        let project = self.project.clone();

        self.set_selected_agent_and_persist(agent.clone(), cx);

        let server = server_override
            .unwrap_or_else(|| agent.server(self.fs.clone(), self.thread_store.clone()));
        // OMEGA-DELTA-0035. Wrapped or bare, the native agent gets the thread
        // store; a bare downcast here would silently hand the router `None` and
        // lose native thread persistence.
        let thread_store =
            crate::omega_router::is_native_agent_server(&server).then(|| self.thread_store.clone());

        let connection_store = self.connection_store.clone();

        let conversation_view = cx.new(|cx| {
            crate::ConversationView::new(
                server,
                connection_store,
                agent,
                resume_session_id,
                Some(thread_id),
                work_dirs,
                title,
                initial_content,
                workspace.clone(),
                project,
                thread_store,
                source,
                window,
                cx,
            )
        });

        cx.observe_in(
            &conversation_view,
            window,
            |this, server_view, window, cx| {
                let is_active = this
                    .active_conversation_view()
                    .is_some_and(|active| active.entity_id() == server_view.entity_id());
                if is_active {
                    cx.emit(AgentPanelEvent::ActiveViewChanged);
                    this.serialize(cx);
                } else {
                    cx.emit(AgentPanelEvent::EntryChanged);
                }
                this.ensure_sibling_host_installed(&server_view, window, cx);
                cx.notify();
            },
        )
        .detach();

        // Try installing the host eagerly as well, in case the connection is
        // already established by the time the observe fires.
        self.ensure_sibling_host_installed(&conversation_view, window, cx);

        if let Some(model) = model_override {
            // The native thread is constructed asynchronously after the
            // connection establishes. Wait for the first `RootThreadUpdated`
            // event that yields a native thread, then apply the override once.
            let applied = Cell::new(false);
            cx.subscribe(
                &conversation_view,
                move |_this, view, _event: &RootThreadUpdated, cx| {
                    if applied.get() {
                        return;
                    }
                    let Some(native_thread) = view.read(cx).as_native_thread(cx) else {
                        return;
                    };
                    apply_native_model_override(&native_thread, &model, cx);
                    applied.set(true);
                },
            )
            .detach();
        }

        AgentThread { conversation_view }
    }

    fn ensure_sibling_host_installed(
        &self,
        conversation_view: &Entity<ConversationView>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !cx.has_flag::<CreateThreadToolFeatureFlag>() {
            return;
        }
        let Some(native_connection) = conversation_view.read(cx).as_native_connection(cx) else {
            return;
        };
        let host = Rc::new(AgentPanelSiblingHost::new(
            cx.weak_entity(),
            window.window_handle(),
        )) as Rc<dyn agent::SiblingThreadHost>;
        native_connection.0.update(cx, |native_agent, _cx| {
            native_agent.set_sibling_thread_host(host);
        });
    }

    fn active_thread_has_messages(&self, cx: &App) -> bool {
        self.active_agent_thread(cx)
            .is_some_and(|thread| !thread.read(cx).entries().is_empty())
    }

    /// Whether the active view is in the **ephemeral** new-draft slot
    pub fn active_view_is_new_draft(&self, cx: &App) -> bool {
        self.draft_thread.as_ref().is_some_and(|draft| {
            draft
                .read(cx)
                .root_thread(cx)
                .is_some_and(|thread| thread.read(cx).is_draft_thread())
                && self
                    .active_conversation_view()
                    .is_some_and(|active| active.entity_id() == draft.entity_id())
        })
    }
    /// Whether the active thread is any kind of draft
    pub fn active_thread_is_draft(&self, cx: &App) -> bool {
        self.active_agent_thread(cx)
            .is_some_and(|thread| thread.read(cx).is_draft_thread())
    }
}

/// Apply a `provider/model-id` model override to a freshly-created native thread.
/// Best-effort: logs an error and leaves the default model in place if the
/// string can't be parsed or the model isn't registered.
pub(crate) fn apply_native_model_override(
    thread: &Entity<agent::Thread>,
    model_id: &str,
    cx: &mut App,
) {
    let Some(selected) = parse_provider_slash_model(model_id) else {
        log::warn!(
            "create_thread: could not parse model override {model_id:?}; expected `provider/model-id`"
        );
        return;
    };
    let configured = LanguageModelRegistry::global(cx)
        .update(cx, |registry, cx| registry.select_model(&selected, cx));
    let Some(configured) = configured else {
        log::warn!(
            "create_thread: no model registered for {model_id:?}; using thread's default model"
        );
        return;
    };
    thread.update(cx, |thread, cx| {
        thread.set_model(configured.model, cx);
    });
}

fn parse_provider_slash_model(input: &str) -> Option<language_model::SelectedModel> {
    let (provider, model) = input.split_once('/')?;
    if provider.is_empty() || model.is_empty() {
        return None;
    }
    Some(language_model::SelectedModel {
        provider: language_model::LanguageModelProviderId::from(provider.to_string()),
        model: language_model::LanguageModelId::from(model.to_string()),
    })
}

/// Bridges agent-side `SiblingThreadHost` calls to `AgentPanel`. Constructed
/// and installed on a `NativeAgent` by the agent panel when a native-agent
/// thread is created.
pub(crate) struct AgentPanelSiblingHost {
    panel: WeakEntity<AgentPanel>,
    window: gpui::AnyWindowHandle,
}

impl AgentPanelSiblingHost {
    pub(crate) fn new(panel: WeakEntity<AgentPanel>, window: gpui::AnyWindowHandle) -> Self {
        Self { panel, window }
    }
}

impl agent::SiblingThreadHost for AgentPanelSiblingHost {
    fn create_sibling_thread(
        &self,
        request: agent::SiblingThreadRequest,
        cx: &mut gpui::AsyncApp,
    ) -> Task<Result<agent::SiblingThreadInfo>> {
        let panel = self.panel.clone();
        let window = self.window;
        cx.spawn(async move |cx| {
            let agent_choice = match request.agent_id.as_deref() {
                None => None,
                Some(id) if id == agent::OMEGA_AGENT_ID.as_ref() => Some(Agent::NativeAgent),
                Some(id) => {
                    // Reject unknown agent ids up front so the model gets a
                    // structured error pointing at `list_agents_and_models`,
                    // rather than a thread that silently fails to launch in
                    // the user's sidebar.
                    let known = panel
                        .read_with(cx, |panel, cx| {
                            let store = panel.project.read(cx).agent_server_store().clone();
                            store
                                .read(cx)
                                .external_agents()
                                .any(|known_id| known_id.0.as_ref() == id)
                        })
                        .unwrap_or(false);
                    if !known {
                        return Err(anyhow!(
                            "Unknown agent id {id:?}. Call `list_agents_and_models` \
                             to see the agents available for `create_thread`."
                        ));
                    }
                    Some(Agent::Custom {
                        id: project::AgentId(id.to_string().into()),
                    })
                }
            };

            let initial_content = AgentInitialContent::ContentBlock {
                blocks: vec![acp::ContentBlock::Text(acp::TextContent::new(
                    request.prompt.clone(),
                ))],
                auto_submit: true,
            };

            let title: SharedString = request.title.clone();
            let options = CreateThreadOptions {
                title: Some(title.clone()),
                initial_content: Some(initial_content),
                agent: agent_choice.clone(),
                model: request.model.clone(),
                work_dirs: None,
            };

            // If the caller asked for a fresh worktree, open a new workspace
            // backed by a linked git worktree of each git repo in the parent
            // project — the same flow the user gets when they pick "Create
            // worktree" from the worktree picker. The sibling thread is then
            // created inside the new workspace's agent panel, so it lives
            // alongside any threads the user would create there manually.
            let mut worktree_warning: Option<String> = None;
            let target_panel = if request.use_new_worktree {
                let workspace = panel.read_with(cx, |panel, _cx| panel.workspace.clone())?;
                let workspace = workspace
                    .upgrade()
                    .ok_or_else(|| anyhow!("Source workspace is no longer available"))?;
                // The branch target follows the existing UI semantics: when
                // `base_ref` is set, treat it as the ref to base off of
                // (resolved like `git switch --detach <ref>`); otherwise base
                // off the current HEAD. Either way the new worktrees are in
                // detached HEAD state — the agent can attach to a branch via
                // git afterwards.
                let branch_target = match request.base_ref.as_ref() {
                    Some(ref_name) => zed_actions::NewWorktreeBranchTarget::ExistingBranch {
                        name: ref_name.clone(),
                    },
                    None => zed_actions::NewWorktreeBranchTarget::CurrentBranch,
                };
                let action = zed_actions::CreateWorktree {
                    worktree_name: request.worktree_name.clone(),
                    branch_target,
                };
                let creation = window.update(cx, |_root, window, cx| {
                    workspace.update(cx, |workspace, cx| {
                        git_ui::worktree_service::create_worktree_workspace(
                            workspace, &action, window, None, cx,
                        )
                    })
                })?;
                let created = creation
                    .await
                    .context("failed to create worktree workspace")?;
                // The creation flow tells us when the project had multiple
                // worktrees of the same underlying repo, which it consolidates
                // into one new worktree — flag it so the calling agent knows
                // the result may not reflect every source worktree's state.
                if created.consolidated_worktrees {
                    worktree_warning = Some(
                        "The project contained multiple worktrees backed by the same git \
                         repository, so they were consolidated into a single new worktree. \
                         The new thread's worktree is based on one of them and may not \
                         reflect the exact state of the others."
                            .to_string(),
                    );
                }
                // Locate the agent panel on the new workspace. We rely on
                // the panel having registered by the time
                // `create_worktree_workspace` returns — `open_worktree_workspace`
                // explicitly awaits `take_panels_task` and the initial scan.
                created
                    .workspace
                    .read_with(cx, |workspace, cx| workspace.panel::<AgentPanel>(cx))
                    .ok_or_else(|| anyhow!("new workspace did not register an agent panel"))?
                    .downgrade()
            } else {
                panel.clone()
            };
            // Both the source panel and any newly-opened worktree workspace
            // live in the same OS window (the new workspace is a tab on the
            // existing MultiWorkspace), so the original window handle is
            // still the right context for the `create_thread_with_options`
            // call regardless of which panel ends up the target.
            let target_window = window;

            // We deliberately don't wait for the new thread's session to
            // become available here: there are currently no agent tools that
            // operate on sibling threads by session ID, so requiring one would
            // just introduce a race for no benefit.
            let resolved_agent_id = target_window.update(cx, |_root, window, cx| {
                target_panel.update(cx, |panel, cx| {
                    panel.create_thread_with_options(
                        options,
                        AgentThreadSource::AgentPanel,
                        window,
                        cx,
                    );
                    let resolved_agent = agent_choice
                        .clone()
                        .unwrap_or_else(|| panel.selected_agent.clone());
                    resolved_agent.id()
                })
            })??;

            Ok(agent::SiblingThreadInfo {
                title,
                agent_id: resolved_agent_id.0.to_string(),
                model: request.model,
                warning: worktree_warning,
            })
        })
    }

    fn list_available_agents(&self, cx: &mut App) -> Result<agent::AvailableAgents> {
        let panel = self
            .panel
            .upgrade()
            .ok_or_else(|| anyhow!("Agent panel is no longer available"))?;

        let mut agents = Vec::new();

        // Native Omega Agent executor — always available, and we can enumerate models
        // directly from the language model registry.
        let native_models = {
            let registry = LanguageModelRegistry::read_global(cx);
            let default = registry.default_model();
            let mut models = Vec::new();
            for provider in registry.providers() {
                if !provider.is_authenticated(cx) {
                    continue;
                }
                let provider_id = provider.id();
                for model in provider.provided_models(cx) {
                    let id = format!("{}/{}", provider_id.0, model.id().0);
                    let is_default = default
                        .as_ref()
                        .map(|cm| cm.provider.id() == provider_id && cm.model.id() == model.id())
                        .unwrap_or(false);
                    models.push(agent::AvailableModel {
                        id,
                        name: model.name().0,
                        is_default,
                    });
                }
            }
            models
        };
        agents.push(agent::AvailableAgent {
            id: agent::OMEGA_AGENT_ID.to_string(),
            name: Agent::NativeAgent.label(),
            is_native: true,
            models: native_models,
        });

        let project = panel.read(cx).project.clone();
        let agent_server_store = project.read(cx).agent_server_store().clone();
        let store = agent_server_store.read(cx);
        for agent_id in store.external_agents() {
            let display = store
                .agent_display_name(agent_id)
                .unwrap_or_else(|| agent_id.0.clone());
            agents.push(agent::AvailableAgent {
                id: agent_id.0.to_string(),
                name: display,
                is_native: false,
                // External agents pick their own models dynamically; we don't
                // try to enumerate them ahead of time.
                models: Vec::new(),
            });
        }

        Ok(agent::AvailableAgents { agents })
    }
}

impl Focusable for AgentPanel {
    fn focus_handle(&self, cx: &App) -> FocusHandle {
        if self.public_channels.selected_channel().is_some() {
            return self.focus_handle.clone();
        }
        // The Full Auto surface owns focus while it is showing, so its
        // objective editor is what a keystroke reaches. `OMEGA-DELTA-0020`.
        if self.showing_full_auto
            && let Some(full_auto) = &self.full_auto
        {
            return full_auto.focus_handle(cx);
        }
        match self.visible_surface() {
            VisibleSurface::Uninitialized => self.focus_handle.clone(),
            VisibleSurface::AgentThread(conversation_view) => conversation_view.focus_handle(cx),
            VisibleSurface::Terminal(terminal_view) => terminal_view.focus_handle(cx),
        }
    }
}

fn agent_panel_dock_position(cx: &App) -> DockPosition {
    AgentSettings::get_global(cx).dock.into()
}

pub enum AgentPanelEvent {
    ActiveViewChanged,
    ActiveViewFocused,
    EntryChanged,
    TerminalCloseRequested { metadata: TerminalThreadMetadata },
    ThreadInteracted { thread_id: ThreadId },
}

impl EventEmitter<PanelEvent> for AgentPanel {}
impl EventEmitter<AgentPanelEvent> for AgentPanel {}

impl Panel for AgentPanel {
    fn persistent_name() -> &'static str {
        "AgentPanel"
    }

    fn panel_key() -> &'static str {
        AGENT_PANEL_KEY
    }

    fn activation_focus_handle(&self, cx: &App) -> FocusHandle {
        if self.public_channels.selected_channel().is_some() {
            return self.focus_handle.clone();
        }
        match self.visible_surface() {
            VisibleSurface::Uninitialized => self.focus_handle.clone(),
            VisibleSurface::AgentThread(conversation_view) => {
                conversation_view.read(cx).activation_focus_handle(cx)
            }
            VisibleSurface::Terminal(terminal_view) => terminal_view.focus_handle(cx),
        }
    }

    fn position(&self, _window: &Window, cx: &App) -> DockPosition {
        agent_panel_dock_position(cx)
    }

    fn position_is_valid(&self, position: DockPosition) -> bool {
        position != DockPosition::Bottom
    }

    fn set_position(&mut self, position: DockPosition, _: &mut Window, cx: &mut Context<Self>) {
        let side = match position {
            DockPosition::Left => "left",
            DockPosition::Right | DockPosition::Bottom => "right",
        };
        telemetry::event!("Agent Panel Side Changed", side = side);
        settings::update_settings_file(self.fs.clone(), cx, move |settings, _| {
            settings
                .agent
                .get_or_insert_default()
                .set_dock(position.into());
        });
    }

    fn default_size(&self, window: &Window, cx: &App) -> Pixels {
        let settings = AgentSettings::get_global(cx);
        match self.position(window, cx) {
            DockPosition::Left | DockPosition::Right => settings.default_width,
            DockPosition::Bottom => settings.default_height,
        }
    }

    fn min_size(&self, window: &Window, cx: &App) -> Option<Pixels> {
        match self.position(window, cx) {
            DockPosition::Left | DockPosition::Right => Some(MIN_PANEL_WIDTH),
            DockPosition::Bottom => None,
        }
    }

    fn supports_flexible_size(&self) -> bool {
        true
    }

    fn has_flexible_size(&self, _window: &Window, cx: &App) -> bool {
        AgentSettings::get_global(cx).flexible
    }

    fn set_flexible_size(&mut self, flexible: bool, _window: &mut Window, cx: &mut Context<Self>) {
        settings::update_settings_file(self.fs.clone(), cx, move |settings, _| {
            settings
                .agent
                .get_or_insert_default()
                .set_flexible_size(flexible);
        });
    }

    fn set_active(&mut self, active: bool, window: &mut Window, cx: &mut Context<Self>) {
        self.is_active = active;
        if active {
            self.ensure_thread_initialized(window, cx);
        }
    }

    fn remote_id() -> Option<proto::PanelId> {
        Some(proto::PanelId::AssistantPanel)
    }

    fn icon(&self, _window: &Window, cx: &App) -> Option<IconName> {
        (self.enabled(cx) && AgentSettings::get_global(cx).button)
            .then_some(IconName::OmegaAssistant)
    }

    fn icon_tooltip(&self, _window: &Window, _cx: &App) -> Option<&'static str> {
        Some("Agent Panel")
    }

    fn toggle_action(&self) -> Box<dyn Action> {
        Box::new(ToggleFocus)
    }

    fn activation_priority(&self) -> u32 {
        0
    }

    fn enabled(&self, cx: &App) -> bool {
        AgentSettings::get_global(cx).enabled(cx)
    }

    fn is_agent_panel(&self) -> bool {
        true
    }

    fn hide_button_setting(&self, _: &App) -> Option<workspace::HideStatusItem> {
        Some(workspace::HideStatusItem::new(|settings| {
            settings.agent.get_or_insert_default().button = Some(false);
        }))
    }

    fn is_zoomed(&self, _window: &Window, _cx: &App) -> bool {
        self.zoomed
    }

    fn set_zoomed(&mut self, zoomed: bool, _window: &mut Window, cx: &mut Context<Self>) {
        self.zoomed = zoomed;
        cx.notify();
    }
}

impl AgentPanel {
    fn ensure_thread_initialized(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        // `OMEGA-DELTA-0020`. A user who opened Full Auto as the first thing
        // in a fresh panel leaves `base_view` uninitialized, so re-activating
        // the panel would create a thread here, and `set_base_view` would
        // clear `showing_full_auto` and bounce them out of the surface they
        // were looking at. Initializing a thread can wait until they ask for
        // one.
        if self.showing_full_auto {
            return;
        }
        if matches!(self.base_view, BaseView::Uninitialized) {
            if self.pending_terminal_spawn.is_some() {
                return;
            }
            if self.should_create_terminal_for_new_entry(cx) {
                let terminal_id = TerminalId::new();
                self.pending_terminal_spawn = Some(terminal_id);
                cx.defer_in(window, move |this, window, cx| {
                    if matches!(this.base_view, BaseView::Uninitialized)
                        && this.pending_terminal_spawn == Some(terminal_id)
                        && this.should_create_terminal_for_new_entry(cx)
                    {
                        this.create_initial_terminal(
                            terminal_id,
                            AgentThreadSource::AgentPanel,
                            window,
                            cx,
                        );
                    } else if this.pending_terminal_spawn == Some(terminal_id) {
                        this.pending_terminal_spawn = None;
                    }
                });
            } else {
                self.activate_draft(false, AgentThreadSource::AgentPanel, window, cx);
            }
        }
    }

    fn create_initial_terminal(
        &mut self,
        terminal_id: TerminalId,
        source: AgentThreadSource,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.supports_terminal(cx) {
            if self.pending_terminal_spawn == Some(terminal_id) {
                self.pending_terminal_spawn = None;
            }
            return;
        }
        let working_directory = self.terminal_working_directory(None, cx);
        self.spawn_initial_terminal(terminal_id, working_directory, source, window, cx);
    }

    #[cfg(not(test))]
    fn spawn_initial_terminal(
        &mut self,
        terminal_id: TerminalId,
        working_directory: Option<PathBuf>,
        source: AgentThreadSource,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.spawn_terminal(
            terminal_id,
            working_directory,
            None,
            None,
            None,
            true,
            false,
            true,
            source,
            window,
            cx,
        );
    }

    #[cfg(test)]
    fn spawn_initial_terminal(
        &mut self,
        terminal_id: TerminalId,
        working_directory: Option<PathBuf>,
        source: AgentThreadSource,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Err(error) = self.insert_display_only_terminal(
            terminal_id,
            working_directory,
            None,
            None,
            None,
            true,
            false,
            true,
            source,
            window,
            cx,
        ) {
            log::error!("failed to spawn test agent panel terminal: {error:#}");
            if self.pending_terminal_spawn == Some(terminal_id) {
                self.pending_terminal_spawn = None;
                cx.notify();
            }
        }
    }

    fn destination_has_meaningful_state(&self, cx: &App) -> bool {
        if !self.retained_threads.is_empty() || !self.terminals.is_empty() {
            return true;
        }

        match &self.base_view {
            BaseView::Uninitialized => false,
            BaseView::Terminal { .. } => true,
            BaseView::AgentThread { conversation_view } => {
                let has_entries = conversation_view
                    .read(cx)
                    .root_thread_view()
                    .is_some_and(|tv| !tv.read(cx).thread.read(cx).entries().is_empty());
                if has_entries {
                    return true;
                }

                conversation_view
                    .read(cx)
                    .root_thread_view()
                    .is_some_and(|thread_view| {
                        let thread_view = thread_view.read(cx);
                        thread_view
                            .thread
                            .read(cx)
                            .draft_prompt()
                            .is_some_and(|draft| !draft.is_empty())
                            || !thread_view
                                .message_editor
                                .read(cx)
                                .text(cx)
                                .trim()
                                .is_empty()
                    })
            }
        }
    }

    fn active_initial_content(&self, cx: &App) -> Option<AgentInitialContent> {
        let thread_view = self.active_thread_view(cx)?;
        let thread_view = thread_view.read(cx);
        let saved = thread_view
            .thread
            .read(cx)
            .draft_prompt()
            .map(|blocks| blocks.to_vec())
            .filter(|blocks| !blocks.is_empty());
        let blocks = saved.unwrap_or_else(|| {
            thread_view
                .message_editor
                .read(cx)
                .draft_content_blocks_snapshot(cx)
        });
        if blocks.is_empty() {
            return None;
        }
        Some(AgentInitialContent::ContentBlock {
            blocks,
            auto_submit: false,
        })
    }

    fn source_panel_initialization(
        source_workspace: &WeakEntity<Workspace>,
        cx: &App,
    ) -> Option<SourcePanelInitialization> {
        let source_workspace = source_workspace.upgrade()?;
        let source_panel = source_workspace.read(cx).panel::<AgentPanel>(cx)?;
        let source_panel = source_panel.read(cx);
        Some(SourcePanelInitialization {
            agent: source_panel.selected_agent(cx),
            initial_content: source_panel.active_initial_content(cx),
        })
    }

    pub fn initialize_from_source_workspace_if_needed(
        &mut self,
        source_workspace: WeakEntity<Workspace>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        if !self.has_open_project(cx) {
            return false;
        }

        if self.destination_has_meaningful_state(cx) {
            return false;
        }

        let Some(initialization) = Self::source_panel_initialization(&source_workspace, cx) else {
            return false;
        };

        let mut initialized = false;
        if self.selected_agent != initialization.agent {
            self.selected_agent = initialization.agent.clone();
            self.serialize(cx);
            initialized = true;
        }

        if let Some(initial_content) = initialization.initial_content {
            let thread = self.create_agent_thread_with_server(
                initialization.agent,
                None,
                None,
                None,
                None,
                Some(initial_content),
                None,
                AgentThreadSource::AgentPanel,
                window,
                cx,
            );
            self.draft_thread = Some(thread.conversation_view.clone());
            self.observe_draft_editor(&thread.conversation_view, cx);
            self.set_base_view(thread.into(), false, window, cx);
            true
        } else {
            if initialized
                && matches!(
                    &self.base_view,
                    BaseView::AgentThread { conversation_view }
                        if self.draft_thread.as_ref().is_some_and(|draft| {
                            draft.entity_id() == conversation_view.entity_id()
                        })
                )
            {
                self.activate_draft(false, AgentThreadSource::AgentPanel, window, cx);
            } else if initialized {
                cx.notify();
            }
            initialized
        }
    }

    fn is_title_editor_focused(&self, window: &Window, cx: &Context<Self>) -> bool {
        match self.visible_surface() {
            VisibleSurface::AgentThread(conversation_view) => conversation_view
                .read(cx)
                .root_thread_view()
                .is_some_and(|view| view.read(cx).title_editor.read(cx).is_focused(window)),
            VisibleSurface::Terminal(_) => self
                .active_terminal_id()
                .and_then(|id| self.terminals.get(&id))
                .and_then(|terminal| terminal.title_editor.as_ref())
                .is_some_and(|editor| editor.read(cx).is_focused(window)),
            _ => false,
        }
    }

    fn should_show_title_edit(&self, window: &Window, cx: &Context<Self>) -> bool {
        // A Full Auto run's title is edited on its own launch surface, so the
        // toolbar's thread-title editor must not appear over it.
        // OMEGA-DELTA-0034. A project-optional thread has a title like any
        // other, so editing it does not wait for a worktree.
        !self.showing_full_auto
            && matches!(
                self.visible_surface(),
                VisibleSurface::AgentThread(_) | VisibleSurface::Terminal(_)
            )
            && !self.is_title_editor_focused(window, cx)
    }

    fn render_title_view(&self, window: &mut Window, cx: &Context<Self>) -> AnyElement {
        if self.showing_full_auto {
            return Label::new("Full Auto").truncate().into_any_element();
        }
        let content = match self.visible_surface() {
            VisibleSurface::AgentThread(conversation_view) => {
                let server_view_ref = conversation_view.read(cx);
                let native_thread = server_view_ref.as_native_thread(cx);
                let is_generating_title = native_thread
                    .as_ref()
                    .is_some_and(|thread| thread.read(cx).is_generating_title());
                let title_generation_error = native_thread
                    .as_ref()
                    .and_then(|thread| thread.read(cx).title_generation_error());

                if let Some(title_editor) = server_view_ref
                    .root_thread_view()
                    .map(|r| r.read(cx).title_editor.clone())
                {
                    if is_generating_title {
                        Label::new(server_view_ref.title(cx))
                            .color(Color::Muted)
                            .truncate()
                            .with_animation(
                                "generating_title",
                                Animation::new(Duration::from_secs(2))
                                    .repeat()
                                    .with_easing(pulsating_between(0.4, 0.8)),
                                |label, delta| label.alpha(delta),
                            )
                            .into_any_element()
                    } else {
                        let editable_title = div()
                            .flex_1()
                            .on_action({
                                let conversation_view = conversation_view.downgrade();
                                move |_: &menu::Confirm, window, cx| {
                                    if let Some(conversation_view) = conversation_view.upgrade() {
                                        conversation_view
                                            .read(cx)
                                            .activation_focus_handle(cx)
                                            .focus(window, cx);
                                    }
                                }
                            })
                            .on_action({
                                let conversation_view = conversation_view.downgrade();
                                move |_: &editor::actions::Cancel, window, cx| {
                                    if let Some(conversation_view) = conversation_view.upgrade() {
                                        conversation_view
                                            .read(cx)
                                            .activation_focus_handle(cx)
                                            .focus(window, cx);
                                    }
                                }
                            })
                            .child(title_editor);

                        if let Some(title_generation_error) = title_generation_error {
                            h_flex()
                                .w_full()
                                .gap_1()
                                .child(editable_title)
                                .child(
                                    IconButton::new("retry-thread-title", IconName::XCircle)
                                        .icon_color(Color::Error)
                                        .icon_size(IconSize::Small)
                                        .tooltip(move |_window, cx| {
                                            Tooltip::with_meta(
                                                "Title generation failed. Click to retry.",
                                                None,
                                                title_generation_error.clone(),
                                                cx,
                                            )
                                        })
                                        .on_click({
                                            let conversation_view = conversation_view.clone();
                                            let workspace = self.workspace.clone();
                                            move |_event, _window, cx| {
                                                Self::handle_regenerate_thread_title(
                                                    conversation_view.clone(),
                                                    workspace.clone(),
                                                    cx,
                                                );
                                            }
                                        }),
                                )
                                .into_any_element()
                        } else {
                            editable_title.w_full().into_any_element()
                        }
                    }
                } else {
                    Label::new(conversation_view.read(cx).title(cx))
                        .color(Color::Muted)
                        .truncate()
                        .into_any_element()
                }
            }
            VisibleSurface::Terminal(_) => {
                if let Some((terminal_id, title_editor, title)) =
                    self.active_terminal_id().and_then(|terminal_id| {
                        self.terminals.get(&terminal_id).map(|terminal| {
                            (
                                terminal_id,
                                terminal.title_editor.clone(),
                                terminal.title(cx),
                            )
                        })
                    })
                {
                    if let Some(title_editor) = title_editor {
                        div()
                            .flex_1()
                            .on_action(cx.listener(move |this, _: &menu::Confirm, window, cx| {
                                this.stop_editing_terminal_title(terminal_id, true, window, cx);
                            }))
                            .on_action(cx.listener(
                                move |this, _: &editor::actions::Cancel, window, cx| {
                                    this.stop_editing_terminal_title(terminal_id, true, window, cx);
                                },
                            ))
                            .child(title_editor)
                            .into_any_element()
                    } else {
                        div()
                            .id("terminal-title")
                            .flex_1()
                            .cursor_text()
                            .overflow_x_scroll()
                            .child(Label::new(title).color(Color::Muted).single_line())
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.edit_terminal_title(terminal_id, window, cx);
                            }))
                            .into_any_element()
                    }
                } else {
                    Label::new("Terminal").into_any_element()
                }
            }

            VisibleSurface::Uninitialized => Label::new("Agent").truncate().into_any_element(),
        };

        let toolbar_bg = cx.theme().colors().tab_bar_background;
        let gradient_overlay = GradientFade::new(toolbar_bg, toolbar_bg, toolbar_bg)
            .width(px(64.0))
            .right(px(0.0))
            .gradient_stop(0.75);
        // The fade gradient renders as a visible patch on transparent windows
        // (the title already truncates).
        let opaque_window =
            cx.theme().window_background_appearance() == gpui::WindowBackgroundAppearance::Opaque;

        h_flex()
            .key_context("TitleEditor")
            .group("title_editor")
            .flex_grow_1()
            .w_full()
            .min_w_0()
            .max_w_full()
            .overflow_x_hidden()
            .child(content)
            .when(self.should_show_title_edit(window, cx), |this| {
                this.when(opaque_window, |this| this.child(gradient_overlay))
                    .child(
                        h_flex()
                            .visible_on_hover("title_editor")
                            .absolute()
                            .right_0()
                            .h_full()
                            .bg(cx.theme().colors().tab_bar_background)
                            .child(
                                IconButton::new("edit_tile", IconName::Pencil)
                                    .icon_size(IconSize::Small)
                                    .tooltip(Tooltip::text("Edit Thread Title")),
                            ),
                    )
            })
            .into_any()
    }

    fn show_no_thread_summary_model_toast(workspace: Entity<Workspace>, cx: &mut App) {
        workspace.update(cx, |workspace, cx| {
            let toast = StatusToast::new(
                "No model is configured for summarizing thread titles.",
                cx,
                |this, _cx| {
                    this.icon(
                        Icon::new(IconName::Warning)
                            .size(IconSize::Small)
                            .color(Color::Warning),
                    )
                    .dismiss_button(true)
                },
            );
            workspace.toggle_status_toast(toast, cx);
        });
    }

    fn handle_regenerate_thread_title(
        conversation_view: Entity<ConversationView>,
        workspace: WeakEntity<Workspace>,
        cx: &mut App,
    ) {
        match Self::regenerate_conversation_thread_title(conversation_view, cx) {
            ThreadTitleRegenerationResult::NoModel => {
                if let Some(workspace) = workspace.upgrade() {
                    Self::show_no_thread_summary_model_toast(workspace, cx);
                }
            }
            ThreadTitleRegenerationResult::NotOpen
            | ThreadTitleRegenerationResult::Started
            | ThreadTitleRegenerationResult::AlreadyGenerating => {}
        }
    }

    fn render_panel_options_menu(
        &self,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let focus_handle = self.focus_handle(cx);
        // Resolve menu shortcuts at the thread root; the active editor can
        // shadow panel-level commands such as ManageSkills.
        let menu_action_context = match &self.base_view {
            BaseView::AgentThread { conversation_view } => conversation_view
                .read(cx)
                .active_thread()
                .map(|thread| thread.read(cx).focus_handle.clone())
                .unwrap_or_else(|| focus_handle.clone()),
            _ => focus_handle.clone(),
        };
        let showing_terminal = matches!(self.visible_surface(), VisibleSurface::Terminal(_));

        let conversation_view = match &self.base_view {
            BaseView::AgentThread { conversation_view } => Some(conversation_view.clone()),
            _ => None,
        };

        let can_regenerate_thread_title =
            conversation_view.as_ref().is_some_and(|conversation_view| {
                let conversation_view = conversation_view.read(cx);
                conversation_view.has_user_submitted_prompt(cx)
                    && conversation_view
                        .as_native_thread(cx)
                        .is_some_and(|thread| !thread.read(cx).is_generating_title())
            });

        let has_thread_messages = conversation_view.as_ref().is_some_and(|conversation_view| {
            conversation_view.read(cx).has_user_submitted_prompt(cx)
        });

        let has_auth_methods = match &self.base_view {
            BaseView::AgentThread { conversation_view } => {
                conversation_view.read(cx).has_auth_methods()
            }
            _ => false,
        };
        let supports_logout = self
            .active_conversation_view()
            .is_some_and(|conversation_view| conversation_view.read(cx).supports_logout());

        let project_agents_md_path = project_agents_md_path(&self.project, true, cx);

        let global_agents_md_loaded = UserAgentsMd::global(cx)
            .and_then(|md| md.content())
            .is_some();

        // OMEGA-DELTA-0125. Does this window have the editor's extension
        // surface to send a click to?
        //
        // The owner, on a live build: *"literally nothing in this top right
        // menu does anything when i click on it. if its easy to reenable those
        // things to actually work, do it, otherwise hide the menu."* Four
        // entries used to reach the refused `omega` namespace. Settings now
        // opens its own visible window and its three navigation actions are
        // admitted individually. Extensions still belongs to the full editor,
        // so that is the only entry this seal guard hides.
        //
        // The seal, not the mode: before it the ordinary workspace still
        // renders, and these entries work there exactly as upstream.
        let offers_editor_surfaces = !omega_zero_base::is_sealed();

        let workspace = self.workspace.clone();

        PopoverMenu::new("agent-options-menu")
            .trigger_with_tooltip(
                IconButton::new("agent-options-menu", IconName::Ellipsis)
                    .icon_size(IconSize::Small),
                move |_window, cx| {
                    Tooltip::for_action_in(
                        "Toggle Agent Menu",
                        &ToggleOptionsMenu,
                        &focus_handle,
                        cx,
                    )
                },
            )
            .anchor(Anchor::TopRight)
            .with_handle(self.agent_panel_menu_handle.clone())
            .menu({
                move |window, cx| {
                    Some(ContextMenu::build(window, cx, |mut menu, _window, cx| {
                        menu = menu.context(menu_action_context.clone());

                        if has_thread_messages {
                            menu = menu.header("Current Thread");

                            if let Some(conversation_view) = conversation_view.as_ref() {
                                if can_regenerate_thread_title {
                                    menu = menu.entry("Regenerate Thread Title", None, {
                                        let conversation_view = conversation_view.clone();
                                        let workspace = workspace.clone();
                                        move |_, cx| {
                                            Self::handle_regenerate_thread_title(
                                                conversation_view.clone(),
                                                workspace.clone(),
                                                cx,
                                            );
                                        }
                                    });
                                }

                                let root_thread_view =
                                    conversation_view.read(cx).root_thread_view();
                                if let Some(thread_view) = root_thread_view {
                                    let workspace = workspace.clone();
                                    menu = menu.entry("Open Thread as Markdown", None, {
                                        move |window, cx| {
                                            if let Some(workspace) = workspace.upgrade() {
                                                thread_view.update(cx, |thread_view, cx| {
                                                    thread_view
                                                        .open_thread_as_markdown(
                                                            workspace, window, cx,
                                                        )
                                                        .detach_and_log_err(cx);
                                                });
                                            }
                                        }
                                    });
                                }

                                menu = menu.separator();
                            }
                        }

                        if !showing_terminal {
                            menu = menu.header("MCP Servers").action(
                                "Add Server…",
                                Box::new(zed_actions::OpenSettingsAt {
                                    path: "context_servers".to_string(),
                                    target: None,
                                }),
                            );
                            if offers_editor_surfaces {
                                menu = menu.action(
                                    "Install New Servers…",
                                    Box::new(zed_actions::Extensions {
                                        category_filter: Some(
                                            zed_actions::ExtensionCategoryFilter::ContextServers,
                                        ),
                                        id: None,
                                    }),
                                );
                            }
                            menu = menu.separator();

                            menu = menu
                                .header("Context")
                                .action("Skills", Box::new(ManageSkills));

                            if project_agents_md_path.is_some() || global_agents_md_loaded {
                                if global_agents_md_loaded {
                                    let workspace = workspace.clone();

                                    menu = menu.custom_entry(
                                        |_window, _cx| {
                                            h_flex()
                                                .w_full()
                                                .gap_1()
                                                .child(Label::new("Open Global Rules"))
                                                .child(
                                                    Label::new("(AGENTS.md)")
                                                        .color(Color::Muted)
                                                        .size(LabelSize::Small),
                                                )
                                                .into_any_element()
                                        },
                                        move |window, cx| {
                                            workspace
                                                .update(cx, |workspace, cx| {
                                                    open_global_rules(workspace, window, cx);
                                                })
                                                .log_err();
                                        },
                                    );
                                }

                                if project_agents_md_path.is_some() {
                                    let workspace = workspace.clone();
                                    menu = menu.custom_entry(
                                        |_window, _cx| {
                                            h_flex()
                                                .w_full()
                                                .gap_1()
                                                .child(Label::new("Open Project Rules"))
                                                .child(
                                                    Label::new("(AGENTS.md)")
                                                        .color(Color::Muted)
                                                        .size(LabelSize::Small),
                                                )
                                                .into_any_element()
                                        },
                                        move |window, cx| {
                                            workspace
                                                .update(cx, |workspace, cx| {
                                                    open_project_rules(workspace, window, cx);
                                                })
                                                .log_err();
                                        },
                                    );
                                }
                            }

                            menu = menu.separator();

                            // Profiles stays. It is `agent::ManageProfiles`,
                            // which the gate admits, and it opens a modal —
                            // and the modal layer is rendered by
                            // `MultiWorkspace` outside the seal, which is why
                            // the command palette still works. It is the one
                            // entry in this menu that was never broken.
                            menu = menu.action("Profiles", Box::new(ManageProfiles::default()));
                        }

                        menu = menu.action("Settings", Box::new(OpenSettings));

                        // OMEGA-DELTA-0118. The entry names the action that
                        // works in the mode this window is in.
                        //
                        // It used to name `multi_workspace::ToggleWorkspaceSidebar`
                        // in both, and in zero base that namespace is refused
                        // at dispatch — a control that is drawn and denied,
                        // which is the failure `OMEGA-DELTA-0053` names. The
                        // editor keeps the workspace sidebar, which is the
                        // project switcher and is the right answer there;
                        // zero base has no `MultiWorkspace` surface and gets
                        // this panel's own.
                        menu = menu.separator().action(
                            "Toggle Threads Sidebar",
                            if omega_zero_base::is_active() {
                                Box::new(ToggleThreadsSidebar) as Box<dyn Action>
                            } else {
                                Box::new(ToggleWorkspaceSidebar) as Box<dyn Action>
                            },
                        );

                        if has_auth_methods || supports_logout {
                            menu = menu.separator()
                        }
                        if has_auth_methods {
                            menu = menu.action("Reauthenticate", Box::new(ReauthenticateAgent))
                        }
                        if supports_logout {
                            menu = menu.action("Log Out", Box::new(LogoutAgent))
                        }

                        menu
                    }))
                }
            })
    }

    fn render_no_project_state(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let focus_handle = self.focus_handle(cx);

        ProjectEmptyState::new(
            "Agent Panel",
            focus_handle.clone(),
            KeyBinding::for_action_in(&workspace::Open::default(), &focus_handle, cx),
        )
        .on_open_project(|_, window, cx| {
            telemetry::event!("Agent Panel Add Project Clicked");
            window.dispatch_action(workspace::Open::default().boxed_clone(), cx);
        })
        .on_clone_repo(|_, window, cx| {
            telemetry::event!("Agent Panel Clone Repo Clicked");
            window.dispatch_action(git::Clone.boxed_clone(), cx);
        })
    }

    fn render_toolbar(&self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let agent_server_store = self.project.read(cx).agent_server_store().clone();

        let focus_handle = self.focus_handle(cx);

        // OMEGA-DELTA-0034. A thread is creatable with no project open, so the
        // toolbar's create controls are live on the front door. A leftover
        // `has_open_project` here would leave the composer typable and the `+`
        // beside it disabled.
        let can_create_entries = true;
        let supports_terminal = self.supports_terminal(cx);
        let showing_terminal = matches!(self.visible_surface(), VisibleSurface::Terminal(_));

        // `OMEGA-DELTA-0131`. The same accessor a new thread is built from, not
        // the stored field. This heading names the thread pressing `+` would
        // open, and in zero base that is Omega's router whatever the field
        // holds — the field may be holding the agent of a *reopened* thread,
        // which is a different question.
        let agent_for_new_threads = self.selected_agent(cx);
        let (selected_agent_custom_icon, selected_agent_label) = if showing_terminal {
            (None, SharedString::from("Terminal"))
        } else if let Agent::Custom { id, .. } = &agent_for_new_threads {
            let store = agent_server_store.read(cx);
            let icon = store.agent_icon(&id);

            let label = store
                .agent_display_name(&id)
                .unwrap_or_else(|| agent_for_new_threads.label());
            (icon, label)
        } else {
            (None, agent_for_new_threads.label())
        };

        let new_thread_menu_builder: Rc<
            dyn Fn(&mut Window, &mut App) -> Option<Entity<ContextMenu>>,
        > = {
            let selected_agent = self.selected_agent.clone();
            let is_agent_selected = move |agent: Agent| selected_agent == agent;

            let workspace = self.workspace.clone();
            let is_via_collab = workspace
                .update(cx, |workspace, cx| {
                    workspace.project().read(cx).is_via_collab()
                })
                .unwrap_or_default();

            let focus_handle = focus_handle.clone();
            let agent_server_store = agent_server_store;

            Rc::new(move |window, cx| {
                Some(ContextMenu::build(window, cx, |menu, _window, cx| {
                    menu.context(focus_handle.clone())
                        .item(
                            ContextMenuEntry::new("Omega Agent")
                                .when(
                                    !showing_terminal && is_agent_selected(Agent::NativeAgent),
                                    |this| this.action(Box::new(NewThread)),
                                )
                                .icon(IconName::OmegaAgent)
                                .icon_color(Color::Muted)
                                .handler({
                                    let workspace = workspace.clone();
                                    move |window, cx| {
                                        if let Some(workspace) = workspace.upgrade() {
                                            workspace.update(cx, |workspace, cx| {
                                                if let Some(panel) =
                                                    workspace.panel::<AgentPanel>(cx)
                                                {
                                                    panel.update(cx, |panel, cx| {
                                                        panel.selected_agent = Agent::NativeAgent;
                                                        panel.activate_new_thread(
                                                            true,
                                                            AgentThreadSource::AgentPanel,
                                                            window,
                                                            cx,
                                                        );
                                                    });
                                                }
                                            });
                                        }
                                    }
                                }),
                        )
                        // omega#99. Zero base does not render the Full Auto
                        // entry, and the Agent Computer and Sarah entries go
                        // with it because their panels are not loaded either.
                        // Not rendering is only half of it: `open_full_auto`
                        // below refuses too, and the mode's action gate refuses
                        // the `full_auto_panel` namespace, because a surface
                        // that is only visually absent is still one key press
                        // away.
                        .when(!omega_zero_base::is_active(), |menu| {
                            menu.item(
                                ContextMenuEntry::new("Full Auto")
                                    .icon(IconName::OmegaAgent)
                                    .icon_color(Color::Accent)
                                    .handler({
                                        move |window, cx| {
                                            window.dispatch_action(Box::new(OpenLauncher), cx);
                                        }
                                    }),
                            )
                            .item(
                                ContextMenuEntry::new("Agent Computer")
                                    .icon(IconName::OmegaAgent)
                                    .icon_color(Color::Accent)
                                    .handler({
                                        move |window, cx| {
                                            window.dispatch_action(
                                                Box::new(OpenAgentComputerPanel),
                                                cx,
                                            );
                                        }
                                    }),
                            )
                            .item(
                                ContextMenuEntry::new("Sarah")
                                    .icon(IconName::OmegaAgent)
                                    .icon_color(Color::Accent)
                                    .handler({
                                        move |window, cx| {
                                            window.dispatch_action(
                                                Box::new(OpenSarahWorkroomPanel),
                                                cx,
                                            );
                                        }
                                    }),
                            )
                        })
                        .when(supports_terminal, |menu| {
                            menu.item(
                                ContextMenuEntry::new("Terminal")
                                    .when(showing_terminal, |this| this.action(Box::new(NewThread)))
                                    .when(!showing_terminal, |this| {
                                        this.action(Box::new(NewTerminalThread))
                                    })
                                    .icon(IconName::Terminal)
                                    .icon_color(Color::Muted)
                                    .handler({
                                        let workspace = workspace.clone();
                                        move |window, cx| {
                                            if let Some(workspace) = workspace.upgrade() {
                                                workspace.update(cx, |workspace, cx| {
                                                    if let Some(panel) =
                                                        workspace.panel::<AgentPanel>(cx)
                                                    {
                                                        panel.update(cx, |panel, cx| {
                                                            panel.new_terminal(
                                                                Some(workspace),
                                                                AgentThreadSource::AgentPanel,
                                                                window,
                                                                cx,
                                                            );
                                                        });
                                                    }
                                                });
                                            }
                                        }
                                    }),
                            )
                        })
                        .map(|mut menu| {
                            let agent_server_store = agent_server_store.read(cx);
                            let registry_store = project::AgentRegistryStore::try_global(cx);
                            let registry_store_ref = registry_store.as_ref().map(|s| s.read(cx));

                            struct AgentMenuItem {
                                id: AgentId,
                                display_name: SharedString,
                            }

                            let agent_items = agent_server_store
                                .external_agents()
                                .map(|agent_id| {
                                    let display_name = agent_server_store
                                        .agent_display_name(agent_id)
                                        .or_else(|| {
                                            registry_store_ref
                                                .as_ref()
                                                .and_then(|store| store.agent(agent_id))
                                                .map(|a| a.name().clone())
                                        })
                                        .unwrap_or_else(|| agent_id.0.clone());
                                    AgentMenuItem {
                                        id: agent_id.clone(),
                                        display_name,
                                    }
                                })
                                .sorted_unstable_by_key(|e| e.display_name.to_lowercase())
                                .collect::<Vec<_>>();

                            if !agent_items.is_empty() {
                                menu = menu.separator().header("External Agents");
                            }
                            for item in &agent_items {
                                let mut entry = ContextMenuEntry::new(item.display_name.clone());

                                let icon_path =
                                    agent_server_store.agent_icon(&item.id).or_else(|| {
                                        registry_store_ref
                                            .as_ref()
                                            .and_then(|store| store.agent(&item.id))
                                            .and_then(|a| a.icon_path().cloned())
                                    });

                                if let Some(icon_path) = icon_path {
                                    entry = entry.custom_icon_svg(icon_path);
                                } else {
                                    entry = entry.icon(IconName::Sparkle);
                                }

                                entry = entry
                                    .when(
                                        !showing_terminal
                                            && is_agent_selected(Agent::Custom {
                                                id: item.id.clone(),
                                            }),
                                        |this| this.action(Box::new(NewThread)),
                                    )
                                    .icon_color(Color::Muted)
                                    .disabled(is_via_collab)
                                    .handler({
                                        let workspace = workspace.clone();
                                        let agent_id = item.id.clone();
                                        move |window, cx| {
                                            if let Some(workspace) = workspace.upgrade() {
                                                workspace.update(cx, |workspace, cx| {
                                                    if let Some(panel) =
                                                        workspace.panel::<AgentPanel>(cx)
                                                    {
                                                        panel.update(cx, |panel, cx| {
                                                            panel.new_external_agent_thread(
                                                                &NewExternalAgentThread {
                                                                    agent: agent_id.clone(),
                                                                },
                                                                window,
                                                                cx,
                                                            );
                                                        });
                                                    }
                                                });
                                            }
                                        }
                                    });

                                menu = menu.item(entry);
                            }

                            menu
                        })
                        .separator()
                        .item(
                            ContextMenuEntry::new("Add More Agents")
                                .icon(IconName::Plus)
                                .icon_color(Color::Muted)
                                .handler({
                                    move |window, cx| {
                                        window
                                            .dispatch_action(Box::new(zed_actions::AcpRegistry), cx)
                                    }
                                }),
                        )
                }))
            })
        };

        let is_thread_loading = self
            .active_conversation_view()
            .map(|thread| thread.read(cx).is_loading())
            .unwrap_or(false);

        let has_custom_icon = selected_agent_custom_icon.is_some();
        let selected_agent_builtin_icon = if showing_terminal {
            Some(IconName::Terminal)
        } else {
            self.selected_agent.icon()
        };
        let selected_agent_label_for_tooltip = selected_agent_label.clone();

        let selected_agent = div()
            .id("selected_agent_icon")
            .px_0p5()
            .when_some(selected_agent_custom_icon, |this, icon_path| {
                this.child(
                    Icon::from_external_svg(icon_path)
                        .color(Color::Muted)
                        .size(IconSize::Small),
                )
            })
            .when(!has_custom_icon, |this| {
                this.when_some(selected_agent_builtin_icon, |this, icon| {
                    this.child(Icon::new(icon).color(Color::Muted))
                })
            })
            .tooltip(move |_, cx| {
                Tooltip::with_meta(
                    selected_agent_label_for_tooltip.clone(),
                    None,
                    "Selected Agent",
                    cx,
                )
            });

        let selected_agent = if is_thread_loading {
            selected_agent
                .with_animation(
                    "pulsating-icon",
                    Animation::new(Duration::from_secs(1))
                        .repeat()
                        .with_easing(pulsating_between(0.2, 0.6)),
                    |icon, delta| icon.opacity(delta),
                )
                .into_any_element()
        } else {
            selected_agent.into_any_element()
        };

        enum ToolbarMode {
            Terminal,
            EmptyThread,
            ActiveThread,
        }

        let mode = if matches!(self.base_view, BaseView::Terminal { .. }) {
            ToolbarMode::Terminal
        } else if self.active_thread_has_messages(cx) {
            ToolbarMode::ActiveThread
        } else {
            ToolbarMode::EmptyThread
        };

        let is_full_screen = self.is_zoomed(window, cx);
        let (icon_id, icon_name, tooltip_text) = if is_full_screen {
            (
                "disable-full-screen",
                IconName::Minimize,
                "Disable Full Screen",
            )
        } else {
            (
                "enable-full-screen",
                IconName::Maximize,
                "Enable Full Screen",
            )
        };
        let full_screen_button = IconButton::new(icon_id, icon_name)
            .icon_size(IconSize::Small)
            .tooltip(move |_, cx| Tooltip::for_action(tooltip_text, &ToggleZoom, cx))
            .on_click(cx.listener(move |this, _, window, cx| {
                this.toggle_zoom(&ToggleZoom, window, cx);
            }));

        let max_content_width = AgentSettings::get_global(cx).max_content_width;

        let base_container = h_flex()
            .size_full()
            .when(
                matches!(mode, ToolbarMode::EmptyThread | ToolbarMode::ActiveThread),
                |this| this.when_some(max_content_width, |this, max_w| this.max_w(max_w).mx_auto()),
            )
            .flex_none()
            .justify_between();

        let empty_thread_title = matches!(mode, ToolbarMode::EmptyThread).then(|| {
            Label::new(format!("New {} Thread", selected_agent_label))
                .color(Color::Muted)
                .truncate()
                .into_any_element()
        });

        let thread_identity = self.render_thread_identity(window, cx);

        let toolbar_content = {
            let new_thread_menu = PopoverMenu::new("new_thread_menu")
                .trigger_with_tooltip(
                    IconButton::new("omega.workbench.control.new-thread-menu", IconName::Plus)
                        .debug_selector(|| "omega.workbench.control.new-thread-menu".into())
                        .aria_label("New Thread")
                        .aria_expanded(self.new_thread_menu_handle.is_deployed())
                        .tab_index(0isize)
                        .icon_size(IconSize::Small),
                    {
                        move |_window, cx| {
                            Tooltip::for_action_in(
                                "New Thread\u{2026}",
                                &ToggleNewThreadMenu,
                                &focus_handle,
                                cx,
                            )
                        }
                    },
                )
                .anchor(Anchor::TopRight)
                .with_handle(self.new_thread_menu_handle.clone())
                .menu(move |window, cx| new_thread_menu_builder(window, cx));

            let sandbox_status = self
                .active_conversation_view()
                .and_then(|conversation_view| conversation_view.read(cx).root_thread_view())
                .and_then(|thread_view| {
                    thread_view.update(cx, |thread_view, cx| thread_view.render_sandbox_status(cx))
                });

            base_container
                .child(
                    h_flex()
                        .relative()
                        .h_full()
                        .flex_1()
                        .min_w_0()
                        .overflow_hidden()
                        .gap(DynamicSpacing::Base04.rems(cx))
                        .pl(DynamicSpacing::Base04.rems(cx))
                        .child(selected_agent.into_any_element())
                        .child(match empty_thread_title {
                            Some(title) => title,
                            None => self.render_title_view(window, cx),
                        })
                        .children(thread_identity),
                )
                .child(
                    h_flex()
                        .px_1()
                        .h_full()
                        .flex_none()
                        .gap_1()
                        .children(sandbox_status)
                        .when(can_create_entries, |this| this.child(new_thread_menu))
                        .child(full_screen_button)
                        .child(self.render_panel_options_menu(window, cx)),
                )
                .into_any_element()
        };

        h_flex()
            .id("agent-panel-toolbar")
            .debug_selector(|| "omega.workbench.toolbar".into())
            .h(Tab::container_height(cx))
            .flex_shrink_0()
            .max_w_full()
            .bg(cx.theme().colors().tab_bar_background)
            .border_b_1()
            .border_color(cx.theme().colors().border)
            .child(toolbar_content)
    }

    fn thread_identity_target_selection_unavailable_reason(
        &self,
        cx: &App,
    ) -> Option<SharedString> {
        if self.active_thread_id(cx).is_some_and(|thread_id| {
            self.thread_identity_pending_operations
                .contains_key(&thread_id.to_key_string())
        }) {
            return Some("Wait for the branch checkout to finish".into());
        }
        let phase_reason =
            self.workbench_shell
                .identity()
                .and_then(|identity| match identity.phase {
                    IdentityPhase::Loading => {
                        Some("Wait for repository identity to finish loading".into())
                    }
                    IdentityPhase::Stale => Some(
                        "Wait for repository identity to refresh before changing its target".into(),
                    ),
                    IdentityPhase::Offline => {
                        Some("Reconnect the project before changing the thread target".into())
                    }
                    IdentityPhase::Reconnecting => Some(
                        "Wait for the project to reconnect before changing the thread target"
                            .into(),
                    ),
                    IdentityPhase::NoProject
                    | IdentityPhase::Ready
                    | IdentityPhase::Missing
                    | IdentityPhase::Error(_)
                    | IdentityPhase::Inconsistent(_) => None,
                });
        phase_reason.or_else(|| {
            self.active_conversation_view()
                .and_then(|conversation_view| {
                    conversation_view
                        .read(cx)
                        .work_dir_retarget_unavailable_reason(cx)
                })
        })
    }

    fn thread_identity_branch_selection_unavailable_reason(
        &self,
        cx: &App,
    ) -> Option<SharedString> {
        if self.active_thread_id(cx).is_some_and(|thread_id| {
            self.thread_identity_pending_operations
                .contains_key(&thread_id.to_key_string())
        }) {
            return Some("Wait for the branch checkout to finish".into());
        }
        let phase_reason =
            self.workbench_shell
                .identity()
                .and_then(|identity| match identity.phase {
                    IdentityPhase::Loading => {
                        Some("Wait for repository identity to finish loading".into())
                    }
                    IdentityPhase::Stale => Some(
                        "Wait for repository identity to refresh before changing branches".into(),
                    ),
                    IdentityPhase::Offline => {
                        Some("Reconnect the project before changing branches".into())
                    }
                    IdentityPhase::Reconnecting => {
                        Some("Wait for the project to reconnect before changing branches".into())
                    }
                    IdentityPhase::Missing => {
                        Some("Choose an available repository before changing branches".into())
                    }
                    IdentityPhase::NoProject => {
                        Some("Open a project before changing branches".into())
                    }
                    IdentityPhase::Inconsistent(_) => {
                        Some("Reconnect this thread before changing branches".into())
                    }
                    IdentityPhase::Ready | IdentityPhase::Error(_) => None,
                });
        phase_reason.or_else(|| {
            self.active_conversation_view()
                .and_then(|conversation_view| {
                    conversation_view
                        .read(cx)
                        .identity_mutation_unavailable_reason(cx)
                })
        })
    }

    fn render_thread_identity(
        &self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        if !self.workbench_shell_enabled {
            return None;
        }
        let Some(identity) = self.workbench_shell.identity().cloned() else {
            return None;
        };
        let Some(selected) = identity.selected.clone() else {
            let status = identity.phase.label();
            return Some(
                h_flex()
                    .id("omega.workbench.thread-identity")
                    .debug_selector(|| "omega.workbench.thread-identity".into())
                    .flex_none()
                    .gap_1()
                    .child(Label::new("·").color(Color::Muted).size(LabelSize::Small))
                    .child(
                        Button::new(
                            "omega.workbench.control.identity.repository",
                            "Choose a folder",
                        )
                        .debug_selector(|| "omega.workbench.control.identity.repository".into())
                        .style(ButtonStyle::Subtle)
                        .size(ButtonSize::Compact)
                        .label_size(LabelSize::Small)
                        .color(Color::Muted)
                        .tab_index(0isize)
                        .aria_label("Choose a repository folder")
                        .aria_description(
                            "Choose the folder this thread can read, search, and run in",
                        )
                        .tooltip(|_, cx| {
                            Tooltip::with_meta(
                                "Choose a folder",
                                None,
                                "Choose the folder this thread can read, search, and run in",
                                cx,
                            )
                        })
                        .on_click(|_, window, cx| {
                            window.dispatch_action(
                                Box::new(workspace::Open {
                                    create_new_window: Some(false),
                                }),
                                cx,
                            );
                        }),
                    )
                    .when_some(status, |this, status| {
                        this.child(
                            div()
                                .id("omega.workbench.identity.status")
                                .debug_selector(|| "omega.workbench.identity.status".into())
                                .role(gpui::Role::Status)
                                .aria_label(status.clone())
                                .child(
                                    Label::new(status)
                                        .size(LabelSize::Small)
                                        .color(Color::Muted),
                                ),
                        )
                    })
                    .into_any_element(),
            );
        };

        let panel = cx.entity().downgrade();
        let candidates = identity.candidates.clone();
        let selected_binding = selected.binding.clone();
        let mut seen_repositories = HashSet::default();
        let repository_candidates = candidates
            .iter()
            .filter(|candidate| seen_repositories.insert(candidate.binding.repository_id.clone()))
            .cloned()
            .collect::<Vec<_>>();
        let mut seen_worktrees = HashSet::default();
        let worktree_candidates = candidates
            .iter()
            .filter(|candidate| {
                candidate.binding.repository_id == selected_binding.repository_id
                    && seen_worktrees.insert(candidate.binding.worktree_id.clone())
            })
            .cloned()
            .collect::<Vec<_>>();
        let visible = self.workbench_shell.projection().visible_projection()?;
        if visible.binding.as_ref() != identity.binding() {
            return None;
        }
        let source_thread_id = visible.thread_id;
        let source_projection_binding = visible.binding;
        let source_binding_generation = visible.generation;
        let menu_builder =
            |candidates: Vec<ThreadIdentityCandidate>,
             selector_for: fn(&omega_workbench_state::RepositoryBinding) -> String| {
                let panel = panel.clone();
                let selected_binding = selected_binding.clone();
                let source_thread_id = source_thread_id.clone();
                let source_projection_binding = source_projection_binding.clone();
                Rc::new(move |window: &mut Window, cx: &mut App| {
                    let panel = panel.clone();
                    let selected_binding = selected_binding.clone();
                    let candidates = candidates.clone();
                    let source_thread_id = source_thread_id.clone();
                    let source_projection_binding = source_projection_binding.clone();
                    Some(ContextMenu::build(
                        window,
                        cx,
                        move |mut menu, _window, _cx| {
                            for candidate in &candidates {
                                let binding = candidate.binding.clone();
                                let label = format!(
                                    "{} — {}",
                                    candidate.repository_name, candidate.worktree_path
                                );
                                let selector = selector_for(&binding);
                                let panel = panel.clone();
                                let source_thread_id = source_thread_id.clone();
                                let source_projection_binding = source_projection_binding.clone();
                                menu = menu.item(
                                    ContextMenuEntry::new(label)
                                        .debug_selector(selector)
                                        .icon(if binding == selected_binding {
                                            IconName::Check
                                        } else {
                                            IconName::Folder
                                        })
                                        .handler(move |_window, cx| {
                                            panel
                                                .update(cx, |panel, cx| {
                                                    panel.select_thread_identity(
                                                        &source_thread_id,
                                                        source_projection_binding.as_ref(),
                                                        source_binding_generation,
                                                        binding.clone(),
                                                        cx,
                                                    );
                                                })
                                                .log_err();
                                        }),
                                );
                            }
                            menu
                        },
                    ))
                })
                    as Rc<dyn Fn(&mut Window, &mut App) -> Option<Entity<ContextMenu>> + 'static>
            };
        let repository_menu_builder = menu_builder(repository_candidates, |binding| {
            format!(
                "omega.workbench.control.repository.{}",
                binding.repository_id
            )
        });
        let worktree_menu_builder = menu_builder(worktree_candidates, |binding| {
            format!("omega.workbench.control.worktree.{}", binding.worktree_id)
        });

        let full_identity = selected.accessible_label();
        let target_selection_unavailable_reason =
            self.thread_identity_target_selection_unavailable_reason(cx);
        let compact = window.viewport_size().width < px(980.);
        let repository_label = if compact {
            selected.repository_name.clone()
        } else {
            format!("{} / {}", selected.project_name, selected.repository_name).into()
        };
        let repository_label =
            util::truncate_and_trailoff(&repository_label, if compact { 18 } else { 26 });
        let repository_target_selection_unavailable_reason =
            target_selection_unavailable_reason.clone();
        let repository_menu = PopoverMenu::new("omega.workbench.identity.repository-picker")
            .with_handle(self.thread_repository_menu_handle.clone())
            .menu({
                let repository_menu_builder = repository_menu_builder.clone();
                move |window, cx| repository_menu_builder(window, cx)
            })
            .trigger_with_tooltip(
                Button::new(
                    "omega.workbench.control.identity.repository",
                    repository_label,
                )
                .debug_selector(|| "omega.workbench.control.identity.repository".into())
                .style(ButtonStyle::Subtle)
                .size(ButtonSize::Compact)
                .label_size(LabelSize::Small)
                .disabled(target_selection_unavailable_reason.is_some())
                .truncate(true)
                .tab_index(0isize)
                .aria_expanded(self.thread_repository_menu_handle.is_deployed())
                .aria_label(format!("Repository {}", selected.repository_name))
                .aria_description(full_identity.clone())
                .end_icon(
                    Icon::new(IconName::ChevronDown)
                        .size(IconSize::XSmall)
                        .color(Color::Muted),
                ),
                move |_, cx| {
                    let description = repository_target_selection_unavailable_reason
                        .clone()
                        .unwrap_or_else(|| "Select the repository for this thread".into());
                    Tooltip::with_meta(
                        full_identity.clone(),
                        Some(&workbench_shell::ToggleRepositoryPicker),
                        description,
                        cx,
                    )
                },
            )
            .anchor(Anchor::TopLeft);
        let worktree_menu = PopoverMenu::new("omega.workbench.identity.worktree-picker")
            .with_handle(self.thread_worktree_menu_handle.clone())
            .menu({
                let worktree_menu_builder = worktree_menu_builder.clone();
                move |window, cx| worktree_menu_builder(window, cx)
            })
            .trigger_with_tooltip(
                Button::new(
                    "omega.workbench.control.identity.worktree",
                    util::truncate_and_trailoff(
                        &selected.worktree_name,
                        if compact { 12 } else { 18 },
                    ),
                )
                .debug_selector(|| "omega.workbench.control.identity.worktree".into())
                .style(ButtonStyle::Subtle)
                .size(ButtonSize::Compact)
                .label_size(LabelSize::Small)
                .disabled(target_selection_unavailable_reason.is_some())
                .truncate(true)
                .tab_index(0isize)
                .aria_expanded(self.thread_worktree_menu_handle.is_deployed())
                .aria_label(format!("Worktree {}", selected.worktree_name))
                .aria_description(format!("Worktree path {}", selected.worktree_path))
                .end_icon(
                    Icon::new(IconName::ChevronDown)
                        .size(IconSize::XSmall)
                        .color(Color::Muted),
                ),
                {
                    let worktree_path = selected.worktree_path.clone();
                    move |_, cx| {
                        let description = target_selection_unavailable_reason
                            .clone()
                            .unwrap_or_else(|| "Select the worktree for this thread".into());
                        Tooltip::with_meta(
                            worktree_path.clone(),
                            Some(&workbench_shell::ToggleWorktreePicker),
                            description,
                            cx,
                        )
                    }
                },
            )
            .anchor(Anchor::TopLeft);

        let repository = selected.git_repository_id.and_then(|id| {
            self.project
                .read(cx)
                .git_store()
                .read(cx)
                .repositories()
                .get(&project::git_store::RepositoryId(id))
                .cloned()
        });
        let branch_label = selected.branch.label();
        let branch_menu_label: SharedString =
            util::truncate_and_trailoff(&branch_label, if compact { 16 } else { 24 }).into();
        let branch_selection = {
            let panel = cx.entity().downgrade();
            let repository = repository.clone();
            let conversation_view = self.active_conversation_view().cloned();
            let expected_thread_id = self.active_thread_id(cx)?;
            let expected_thread_id = expected_thread_id.to_key_string();
            let expected_binding = selected.binding.clone();
            let expected_projection_binding = identity.binding().cloned();
            let expected_binding_generation = self
                .workbench_shell
                .projection()
                .visible_projection()
                .map(|visible| visible.generation)
                .unwrap_or_default();
            let selected_branch = match &selected.branch {
                BranchIdentity::Branch(branch) => Some(branch.clone()),
                _ => None,
            };
            let on_select: git_ui::branch_picker::SelectBranchCallback =
                Arc::new(move |branch, _window, cx| {
                    let Some(repository) = repository.clone() else {
                        return;
                    };
                    let conversation_view = conversation_view.clone();
                    let request_id = panel
                        .update(cx, |panel, cx| {
                            if !panel.binding_epoch_matches(
                                &expected_thread_id,
                                expected_projection_binding.as_ref(),
                                expected_binding_generation,
                                true,
                                cx,
                            ) || panel
                                .thread_identity_branch_selection_unavailable_reason(cx)
                                .is_some()
                            {
                                return None;
                            }
                            panel.thread_identity_operation_request =
                                panel.thread_identity_operation_request.saturating_add(1);
                            let request_id = panel.thread_identity_operation_request;
                            panel
                                .thread_identity_operation_requests
                                .insert(expected_thread_id.clone(), request_id);
                            panel
                                .thread_identity_pending_operations
                                .insert(expected_thread_id.clone(), request_id);
                            panel
                                .thread_identity_operation_errors
                                .remove(&expected_thread_id);
                            if let Some(conversation_view) = &conversation_view {
                                conversation_view.update(cx, |conversation_view, cx| {
                                    conversation_view.set_repository_mutation_pending(true, cx);
                                });
                            }
                            panel.thread_identity_observation_revision =
                                panel.thread_identity_observation_revision.saturating_add(1);
                            cx.notify();
                            Some(request_id)
                        })
                        .ok()
                        .flatten();
                    let Some(request_id) = request_id else {
                        return;
                    };
                    let panel = panel.clone();
                    let expected_thread_id = expected_thread_id.clone();
                    let expected_binding = expected_binding.clone();
                    let expected_projection_binding = expected_projection_binding.clone();
                    let conversation_view = conversation_view.clone();
                    cx.spawn(async move |cx| {
                        let result: Result<()> = async {
                            repository
                                .update(cx, |repository, _cx| {
                                    repository.change_branch(branch.name().to_string())
                                })
                                .await??;
                            Ok(())
                        }
                        .await;
                        if let Some(conversation_view) = &conversation_view {
                            conversation_view.update(cx, |conversation_view, cx| {
                                conversation_view.set_repository_mutation_pending(false, cx);
                            });
                        }
                        panel
                            .update(cx, |panel, cx| {
                                if panel
                                    .thread_identity_pending_operations
                                    .get(&expected_thread_id)
                                    == Some(&request_id)
                                {
                                    panel
                                        .thread_identity_pending_operations
                                        .remove(&expected_thread_id);
                                }
                                if panel
                                    .thread_identity_operation_requests
                                    .get(&expected_thread_id)
                                    != Some(&request_id)
                                    || !panel.binding_epoch_matches(
                                        &expected_thread_id,
                                        expected_projection_binding.as_ref(),
                                        expected_binding_generation,
                                        false,
                                        cx,
                                    )
                                {
                                    return;
                                }
                                let result = result.and_then(|()| {
                                    panel.workbench_shell.refresh_binding_epoch(
                                        &expected_thread_id,
                                        &expected_binding,
                                        expected_binding_generation,
                                        cx,
                                    )
                                });
                                if let Some(error) = result.err() {
                                    panel.thread_identity_operation_errors.insert(
                                        expected_thread_id.clone(),
                                        ThreadIdentityOperationError {
                                            source_binding: expected_projection_binding.clone(),
                                            attempted_binding: expected_binding.clone(),
                                            binding_generation: expected_binding_generation,
                                            request_id,
                                            inconsistent: false,
                                            message: error.to_string().into(),
                                        },
                                    );
                                } else {
                                    panel
                                        .thread_identity_operation_requests
                                        .remove(&expected_thread_id);
                                    panel
                                        .thread_identity_operation_errors
                                        .remove(&expected_thread_id);
                                }
                                panel.thread_identity_observation_revision =
                                    panel.thread_identity_observation_revision.saturating_add(1);
                                cx.notify();
                            })
                            .log_err();
                    })
                    .detach();
                });
            (selected_branch, on_select)
        };
        let branch_selection_unavailable_reason =
            self.thread_identity_branch_selection_unavailable_reason(cx);
        let branch_menu = (!matches!(selected.branch, BranchIdentity::NoGit)
            && matches!(
                identity.phase,
                IdentityPhase::Ready | IdentityPhase::Error(_)
            )
            && branch_selection_unavailable_reason.is_none())
        .then(|| {
            let branch_accessible_label = branch_label.clone();
            let branch_tooltip_label = branch_label.clone();
            PopoverMenu::new("omega.workbench.identity.branch-picker")
                .with_handle(self.thread_branch_menu_handle.clone())
                .menu({
                    let workspace = self.workspace.clone();
                    let repository = repository.clone();
                    let selected_branch = branch_selection.0.clone();
                    let on_select = branch_selection.1.clone();
                    move |window, cx| {
                        Some(git_ui::branch_picker::select_popover(
                            workspace.clone(),
                            repository.clone(),
                            selected_branch.clone(),
                            on_select.clone(),
                            window,
                            cx,
                        ))
                    }
                })
                .trigger_with_tooltip(
                    Button::new(
                        "omega.workbench.control.identity.branch",
                        branch_menu_label.clone(),
                    )
                    .debug_selector(|| "omega.workbench.control.identity.branch".into())
                    .style(ButtonStyle::Subtle)
                    .size(ButtonSize::Compact)
                    .label_size(LabelSize::Small)
                    .truncate(true)
                    .tab_index(0isize)
                    .aria_expanded(self.thread_branch_menu_handle.is_deployed())
                    .aria_label(format!("Branch {branch_accessible_label}"))
                    .aria_description(format!(
                        "Git branch state for repository {}",
                        selected.repository_name
                    )),
                    move |_, cx| {
                        Tooltip::with_meta(
                            branch_tooltip_label.clone(),
                            Some(&workbench_shell::ToggleBranchPicker),
                            "Switch branch with the Git branch picker",
                            cx,
                        )
                    },
                )
                .anchor(Anchor::TopLeft)
        });
        let status = identity.phase.label();
        let branch_unavailable_reason = branch_selection_unavailable_reason
            .or(status.clone())
            .unwrap_or_else(|| {
                if matches!(selected.branch, BranchIdentity::NoGit) {
                    "The selected worktree has no Git repository".into()
                } else {
                    "Branch selection is unavailable".into()
                }
            });
        let git = selected.git;

        Some(
            h_flex()
                .id("omega.workbench.thread-identity")
                .debug_selector(|| "omega.workbench.thread-identity".into())
                .flex_none()
                .max_w(if compact { rems(24.) } else { rems(42.) })
                .overflow_hidden()
                .gap_0p5()
                .child(Label::new("·").color(Color::Muted).size(LabelSize::Small))
                .child(repository_menu)
                .child(Label::new("/").color(Color::Muted).size(LabelSize::Small))
                .child(worktree_menu)
                .child(Label::new("@").color(Color::Muted).size(LabelSize::Small))
                .child(
                    branch_menu
                        .map(IntoElement::into_any_element)
                        .unwrap_or_else(|| {
                            let branch_tooltip_label = branch_label.clone();
                            Button::new(
                                "omega.workbench.control.identity.branch",
                                branch_menu_label.clone(),
                            )
                            .debug_selector(|| "omega.workbench.control.identity.branch".into())
                            .style(ButtonStyle::Subtle)
                            .size(ButtonSize::Compact)
                            .label_size(LabelSize::Small)
                            .truncate(true)
                            .disabled(true)
                            .tab_index(0isize)
                            .aria_label(format!("Branch {branch_label}"))
                            .aria_description(branch_unavailable_reason.clone())
                            .tooltip(move |_, cx| {
                                Tooltip::with_meta(
                                    branch_tooltip_label.clone(),
                                    None,
                                    branch_unavailable_reason.clone(),
                                    cx,
                                )
                            })
                            .into_any_element()
                        }),
                )
                .when(git.dirty_files > 0, |this| {
                    this.child(
                        div()
                            .id("omega.workbench.identity.indicator.dirty")
                            .debug_selector(|| "omega.workbench.identity.indicator.dirty".into())
                            .role(gpui::Role::Status)
                            .aria_label(format!("{} changed files", git.dirty_files))
                            .child(
                                Label::new(format!("±{}", git.dirty_files))
                                    .size(LabelSize::Small)
                                    .color(Color::Warning),
                            ),
                    )
                })
                .when(git.conflicts > 0, |this| {
                    this.child(
                        div()
                            .id("omega.workbench.identity.indicator.conflict")
                            .debug_selector(|| "omega.workbench.identity.indicator.conflict".into())
                            .role(gpui::Role::Status)
                            .aria_label(format!("{} conflicted files", git.conflicts))
                            .child(
                                Label::new(format!("!{}", git.conflicts))
                                    .size(LabelSize::Small)
                                    .color(Color::Error),
                            ),
                    )
                })
                .when(git.ahead > 0, |this| {
                    this.child(
                        div()
                            .id("omega.workbench.identity.indicator.ahead")
                            .debug_selector(|| "omega.workbench.identity.indicator.ahead".into())
                            .role(gpui::Role::Status)
                            .aria_label(format!("{} commits ahead", git.ahead))
                            .child(
                                Label::new(format!("↑{}", git.ahead))
                                    .size(LabelSize::Small)
                                    .color(Color::Muted),
                            ),
                    )
                })
                .when(git.behind > 0, |this| {
                    this.child(
                        div()
                            .id("omega.workbench.identity.indicator.behind")
                            .debug_selector(|| "omega.workbench.identity.indicator.behind".into())
                            .role(gpui::Role::Status)
                            .aria_label(format!("{} commits behind", git.behind))
                            .child(
                                Label::new(format!("↓{}", git.behind))
                                    .size(LabelSize::Small)
                                    .color(Color::Muted),
                            ),
                    )
                })
                .when_some(status, |this, status| {
                    this.child(
                        div()
                            .id("omega.workbench.identity.status")
                            .debug_selector(|| "omega.workbench.identity.status".into())
                            .role(gpui::Role::Status)
                            .aria_label(status.clone())
                            .child(
                                Label::new(status)
                                    .size(LabelSize::Small)
                                    .color(Color::Warning),
                            ),
                    )
                })
                .into_any_element(),
        )
    }

    fn dismiss_ai_onboarding(&mut self, cx: &mut Context<Self>) {
        self.new_user_onboarding_upsell_dismissed
            .store(true, Ordering::Release);
        OnboardingUpsell::set_dismissed(true, cx);
        cx.notify();
    }

    fn should_render_new_user_onboarding(&mut self, cx: &mut Context<Self>) -> bool {
        // omega#99. Zero base never draws the onboarding card.
        //
        // The card is the agent panel's own, not zero base's, but it appeared
        // in zero base's window and so it was zero base's problem: it read
        // `Configured AI providers: Ollama` while the disclosure line and the
        // composer beside it both named `google/gemini-3.6-flash`. A window
        // that gives two answers to "which model am I about to talk to" is
        // worse than one that gives none, and the owner named it as the worst
        // thing on the screen.
        //
        // Auto-detection replaces the question. `has_configured_ai_provider`
        // is the onboarding screen's own enumeration, not a second one: when a
        // usable provider exists the surface says nothing at all, and when
        // none exists the composer says so in one line rather than a modal
        // card, because a card is a decision point and the composer is where
        // the person already is. `render_zero_base_provider_notice` in the
        // thread view is that line.
        if omega_zero_base::is_active() {
            return false;
        }

        if self
            .new_user_onboarding_upsell_dismissed
            .load(Ordering::Acquire)
        {
            return false;
        }

        let has_configured_non_zed_providers = ai_onboarding::has_configured_ai_provider(cx);

        match &self.base_view {
            BaseView::Uninitialized | BaseView::Terminal { .. } => false,
            BaseView::AgentThread { conversation_view } => {
                if conversation_view.read(cx).as_native_thread(cx).is_some() {
                    let history_is_empty = ThreadStore::global(cx).read(cx).is_empty();
                    history_is_empty || !has_configured_non_zed_providers
                } else {
                    false
                }
            }
        }
    }

    fn render_new_user_onboarding(
        &mut self,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<impl IntoElement> {
        if !self.should_render_new_user_onboarding(cx) {
            return None;
        }

        Some(
            div()
                .bg(cx.theme().colors().editor_background)
                .child(self.new_user_onboarding.clone()),
        )
    }

    fn render_drag_target(&self, cx: &Context<Self>) -> Div {
        let is_local = self.project.read(cx).is_local();
        div()
            .invisible()
            .absolute()
            .top_0()
            .right_0()
            .bottom_0()
            .left_0()
            .bg(cx.theme().colors().drop_target_background)
            .drag_over::<DraggedTab>(|this, _, _, _| this.visible())
            .drag_over::<DraggedSelection>(|this, _, _, _| this.visible())
            .when(is_local, |this| {
                this.drag_over::<ExternalPaths>(|this, _, _, _| this.visible())
            })
            .on_drop(cx.listener(move |this, tab: &DraggedTab, window, cx| {
                let item = tab.pane.read(cx).item_for_index(tab.ix);
                let project_paths = item
                    .and_then(|item| item.project_path(cx))
                    .into_iter()
                    .collect::<Vec<_>>();
                this.handle_drop(project_paths, vec![], window, cx);
            }))
            .on_drop(
                cx.listener(move |this, selection: &DraggedSelection, window, cx| {
                    let project_paths = selection
                        .items()
                        .filter_map(|item| this.project.read(cx).path_for_entry(item.entry_id, cx))
                        .collect::<Vec<_>>();
                    this.handle_drop(project_paths, vec![], window, cx);
                }),
            )
            .on_drop(cx.listener(move |this, paths: &ExternalPaths, window, cx| {
                this.handle_external_paths_drop(paths, window, cx);
            }))
    }

    fn handle_external_paths_drop(
        &mut self,
        paths: &ExternalPaths,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if matches!(&self.base_view, BaseView::Terminal { .. }) {
            // Terminal drops should match normal terminal views by pasting raw OS paths.
            // The agent-thread path below converts paths to project paths, which can add
            // worktrees and is only needed when attaching files to a conversation.
            self.paste_external_paths_into_active_terminal(paths, window, cx);
            return;
        }

        let BaseView::AgentThread { conversation_view } = &self.base_view else {
            return;
        };
        let conversation_view = conversation_view.clone();
        let tasks = paths
            .paths()
            .iter()
            .map(|path| Workspace::project_path_for_path(self.project.clone(), path, false, cx))
            .collect::<Vec<_>>();
        cx.spawn_in(window, async move |_this, cx| {
            let mut paths = vec![];
            let mut added_worktrees = vec![];
            let opened_paths = futures::future::join_all(tasks).await;
            for entry in opened_paths {
                if let Some((worktree, project_path)) = entry.log_err() {
                    added_worktrees.push(worktree);
                    paths.push(project_path);
                }
            }
            conversation_view
                .update_in(cx, |conversation_view, window, cx| {
                    conversation_view.insert_dragged_files(paths, added_worktrees, window, cx);
                })
                .log_err();
        })
        .detach();
    }

    fn paste_external_paths_into_active_terminal(
        &mut self,
        paths: &ExternalPaths,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let BaseView::Terminal { terminal_id } = &self.base_view else {
            return;
        };

        if !self.project.read(cx).is_local() {
            return;
        }

        let Some(terminal_view) = self
            .terminals
            .get(terminal_id)
            .map(|terminal| terminal.view.clone())
        else {
            return;
        };

        terminal_view.update(cx, |terminal_view, cx| {
            terminal_view.add_paths_to_terminal(paths.paths(), window, cx);
        });
    }

    fn handle_drop(
        &mut self,
        paths: Vec<ProjectPath>,
        added_worktrees: Vec<Entity<Worktree>>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match &self.base_view {
            BaseView::AgentThread { conversation_view } => {
                conversation_view.update(cx, |conversation_view, cx| {
                    conversation_view.insert_dragged_files(paths, added_worktrees, window, cx);
                });
            }
            BaseView::Terminal { terminal_id } => {
                let paths = {
                    let project = self.project.read(cx);
                    paths
                        .iter()
                        .filter_map(|project_path| project.absolute_path(project_path, cx))
                        .collect::<Vec<_>>()
                };

                if paths.is_empty() {
                    return;
                }

                if let Some(terminal_view) = self
                    .terminals
                    .get(terminal_id)
                    .map(|terminal| terminal.view.clone())
                {
                    terminal_view.update(cx, |terminal_view, cx| {
                        terminal_view.add_paths_to_terminal(&paths, window, cx);
                    });
                }
            }
            BaseView::Uninitialized => {}
        }
    }

    fn key_context(&self) -> KeyContext {
        let mut key_context = KeyContext::new_with_defaults();
        key_context.add("AgentPanel");
        // OMEGA-DELTA-0118. So a shipped binding can name a key that means one
        // thing in zero base and something else in the editor, without either
        // of them being deleted. `cmd-alt-j` opens the workspace sidebar in the
        // editor and this panel's threads sidebar in zero base, which is the
        // only sidebar a sealed window has.
        if omega_zero_base::is_active() {
            key_context.add("ZeroBase");
        }
        key_context
    }

    fn workbench_thread_context(
        &self,
        cx: &App,
    ) -> Result<(Option<String>, ThreadIdentityObservation)> {
        let revision = self.thread_identity_observation_revision;
        let Some(thread_id) = self.active_thread_id(cx) else {
            return Ok((
                None,
                ThreadIdentityObservation {
                    revision,
                    phase: IdentityPhase::NoProject,
                    candidates: Vec::new(),
                },
            ));
        };
        #[cfg(any(test, feature = "test-support"))]
        if let Some(observation) = self.workbench_identity_observation_override.clone() {
            return Ok((Some(thread_id.to_key_string()), observation));
        }

        let preferred_paths = self
            .active_conversation_view()
            .map(|conversation_view| {
                conversation_view
                    .read(cx)
                    .work_dirs()
                    .ordered_paths()
                    .cloned()
                    .collect::<Vec<_>>()
            })
            .or_else(|| {
                self.omega_active_acp_thread(cx).and_then(|thread| {
                    thread
                        .read(cx)
                        .work_dirs()
                        .map(|paths| paths.ordered_paths().cloned().collect::<Vec<_>>())
                })
            })
            .unwrap_or_default();
        let project = self.project.read(cx);
        let remote_phase = project
            .remote_connection_state(cx)
            .and_then(|state| match state {
                remote::ConnectionState::Connecting | remote::ConnectionState::Reconnecting => {
                    Some(IdentityPhase::Reconnecting)
                }
                remote::ConnectionState::Connected => None,
                remote::ConnectionState::HeartbeatMissed => Some(IdentityPhase::Stale),
                remote::ConnectionState::Disconnected => Some(IdentityPhase::Offline),
            });
        let initial_scan_completed = project.worktree_store().read(cx).initial_scan_completed();
        let git_store = project.git_store();
        let mut worktrees = project.visible_worktrees(cx).collect::<Vec<_>>();
        worktrees.sort_by(|left, right| {
            let left_path = left.read(cx).abs_path();
            let right_path = right.read(cx).abs_path();
            let left_preference = preferred_paths
                .iter()
                .position(|path| path == left_path.as_ref())
                .unwrap_or(usize::MAX);
            let right_preference = preferred_paths
                .iter()
                .position(|path| path == right_path.as_ref())
                .unwrap_or(usize::MAX);
            left_preference
                .cmp(&right_preference)
                .then_with(|| left_path.cmp(&right_path))
        });
        let home = std::env::var_os("HOME").map(PathBuf::from);
        let mut candidates = Vec::new();
        let mut branch_errors = Vec::new();
        let mut loading_bindings = Vec::new();
        for worktree in worktrees {
            let worktree = worktree.read(cx);
            let worktree_id = worktree.id();
            let worktree_name: SharedString = worktree.root_name_str().to_string().into();
            let worktree_path = worktree.abs_path();
            let worktree_display: SharedString =
                omega_workdir::display_for_person(&worktree_path, home.as_deref()).into();
            let repository_ids = git_store.read(cx).repository_ids_for_worktree(worktree_id);
            if repository_ids.is_empty() {
                let binding = omega_workbench_state::RepositoryBinding::new(
                    format!("project-worktree-{worktree_id}"),
                    format!("worktree-{worktree_id}"),
                )
                .map_err(anyhow::Error::new)?;
                candidates.push(ThreadIdentityCandidate {
                    binding,
                    git_repository_id: None,
                    project_name: worktree_name.clone(),
                    repository_name: "No Git".into(),
                    worktree_name,
                    worktree_abs_path: worktree_path.as_ref().to_path_buf(),
                    worktree_path: worktree_display,
                    branch: BranchIdentity::NoGit,
                    git: GitIdentitySummary::default(),
                    source_revision: revision,
                });
                continue;
            }

            for repository_id in repository_ids {
                let repository = git_store
                    .read(cx)
                    .repositories()
                    .get(&repository_id)
                    .cloned()
                    .ok_or_else(|| {
                        anyhow!(
                            "repository {repository_id:?} disappeared while projecting identity"
                        )
                    })?;
                let repository = repository.read(cx);
                let snapshot = repository.snapshot();
                let repository_identity_path =
                    project::git_store::repo_identity_path(&snapshot.common_dir_abs_path);
                let repository_digest =
                    Sha256::digest(repository_identity_path.to_string_lossy().as_bytes());
                let binding = omega_workbench_state::RepositoryBinding::new(
                    format!("git-repository-{repository_digest:x}"),
                    format!("worktree-{worktree_id}"),
                )
                .map_err(anyhow::Error::new)?;
                if snapshot.scan_id == 0 {
                    loading_bindings.push(binding.clone());
                }
                if let Some(error) = snapshot.branch_list_error.clone() {
                    branch_errors.push((binding.clone(), error));
                }
                let branch = BranchIdentity::from_git(
                    snapshot.branch.as_ref().map(|branch| branch.name()),
                    snapshot
                        .head_commit
                        .as_ref()
                        .map(|commit| commit.sha.as_ref()),
                );
                let tracking = snapshot
                    .branch
                    .as_ref()
                    .and_then(|branch| branch.tracking_status());
                let summary = snapshot.status_summary();
                candidates.push(ThreadIdentityCandidate {
                    binding,
                    git_repository_id: Some(repository_id.0),
                    project_name: worktree_name.clone(),
                    repository_name: snapshot.display_name(),
                    worktree_name: worktree_name.clone(),
                    worktree_abs_path: worktree_path.as_ref().to_path_buf(),
                    worktree_path: worktree_display.clone(),
                    branch,
                    git: GitIdentitySummary {
                        dirty_files: summary.count,
                        conflicts: summary.conflict,
                        ahead: tracking.map_or(0, |tracking| tracking.ahead as usize),
                        behind: tracking.map_or(0, |tracking| tracking.behind as usize),
                    },
                    source_revision: snapshot.scan_id.max(revision),
                });
            }
        }

        let thread_key = thread_id.to_key_string();
        let selected_binding = self
            .workbench_shell
            .identity_thread_id()
            .filter(|active_thread_id| *active_thread_id == thread_key)
            .and_then(|_| self.workbench_shell.identity())
            .and_then(|identity| identity.selected.as_ref())
            .map(|selected| &selected.binding)
            .filter(|binding| {
                candidates
                    .iter()
                    .any(|candidate| &candidate.binding == *binding)
            })
            .cloned()
            .or_else(|| {
                candidates
                    .first()
                    .map(|candidate| candidate.binding.clone())
            });
        let operation_error = self
            .thread_identity_operation_errors
            .get(&thread_key)
            .filter(|error| {
                self.thread_identity_operation_requests.get(&thread_key) == Some(&error.request_id)
                    && self
                        .workbench_shell
                        .projection()
                        .threads
                        .get(&thread_key)
                        .is_some_and(|thread| {
                            thread.generation == error.binding_generation
                                && thread.binding == error.source_binding
                        })
                    && candidates
                        .iter()
                        .any(|candidate| candidate.binding == error.attempted_binding)
            })
            .map(|error| (error.inconsistent, error.message.clone()));
        let operation_pending = self
            .thread_identity_pending_operations
            .contains_key(&thread_key);
        let branch_error = selected_binding.as_ref().and_then(|binding| {
            branch_errors
                .iter()
                .find(|(error_binding, _)| error_binding == binding)
                .map(|(_, error)| error.clone())
        });
        let selected_repository_is_loading = selected_binding
            .as_ref()
            .is_some_and(|binding| loading_bindings.contains(binding));
        let phase = remote_phase.unwrap_or_else(|| {
            if operation_pending {
                IdentityPhase::Loading
            } else if let Some((inconsistent, error)) = operation_error {
                if inconsistent {
                    IdentityPhase::Inconsistent(error)
                } else {
                    IdentityPhase::Error(error)
                }
            } else if let Some(error) = branch_error {
                IdentityPhase::Error(error)
            } else if !initial_scan_completed || selected_repository_is_loading {
                IdentityPhase::Loading
            } else if candidates.is_empty() {
                IdentityPhase::NoProject
            } else {
                IdentityPhase::Ready
            }
        });
        #[cfg(any(test, feature = "test-support"))]
        let phase = self
            .workbench_identity_phase_override
            .clone()
            .unwrap_or(phase);
        Ok((
            Some(thread_id.to_key_string()),
            ThreadIdentityObservation {
                revision,
                phase,
                candidates,
            },
        ))
    }

    fn sync_workbench_shell(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let files_panel_was_focused = self
            .workbench_files_panel
            .as_ref()
            .is_some_and(|panel| panel.focus_handle(cx).contains_focused(window, cx));
        let search_surface_was_focused = self
            .workbench_shell
            .search_surface_for_active_binding(cx)
            .is_some_and(|surface| surface.focus_handle(cx).contains_focused(window, cx));
        let review_surface_was_focused = self
            .workbench_shell
            .review_surface_for_active_binding(cx)
            .is_some_and(|surface| surface.focus_handle(cx).contains_focused(window, cx));
        let git_surface_was_focused = self
            .workbench_shell
            .git_surface_for_active_binding(cx)
            .is_some_and(|surface| surface.read(cx).contains_focus(window, cx));
        let terminal_surface_was_focused = self
            .workbench_terminal_surface
            .as_ref()
            .is_some_and(|surface| surface.read(cx).contains_focus(window, cx));
        let context = self.workbench_thread_context(cx);
        let result = match context {
            Ok((thread_id, observation)) => {
                if let Some(thread_id) = thread_id.as_deref() {
                    self.restore_workbench_selection_from_disk_if_needed(thread_id, cx);
                }
                self.workbench_shell
                    .sync_active_thread(thread_id, observation)
            }
            Err(error) => Err(error),
        };
        if let Err(error) = result {
            log::warn!("failed to synchronize workbench shell: {error:#}");
            self.workbench_shell.record_error(error.to_string());
        }
        self.synchronize_thread_outline(cx);
        if self.workbench_shell_enabled
            && self.workbench_git_panel_handed_off
            && let Some(panel) = self.workbench_git_panel.clone()
        {
            let desired_scope = self
                .workbench_git_has_authority(cx)
                .then(|| {
                    Some(GitPanelRepositoryScope {
                        repository_id: self.active_workbench_git_repository_id(cx)?,
                        worktree_id: self.active_workbench_worktree_id(cx)?,
                        generation: self
                            .workbench_shell
                            .projection()
                            .visible_projection()?
                            .generation,
                    })
                })
                .flatten();
            panel.update(cx, |panel, cx| {
                if let Some(scope) = desired_scope {
                    if let Err(error) = panel.set_repository_scope(Some(scope), window, cx) {
                        panel.set_repository_scope_unavailable(scope, window, cx);
                        log::warn!("failed to synchronize the exact native Git scope: {error:#}");
                    }
                } else if let Some(scope) = panel.repository_scope() {
                    panel.set_repository_scope_unavailable(scope, window, cx);
                }
            });
        }
        if self.workbench_shell_enabled
            && self.workbench_files_panel_handed_off
            && let Some(panel) = self.workbench_files_panel.clone()
        {
            let compatible_worktree_id = self.workbench_files_compatible_worktree_id(cx);
            let worktree_id = self
                .workbench_files_are_actionable(cx)
                .then(|| self.active_workbench_worktree_id(cx))
                .flatten();
            panel.update(cx, |panel, cx| match worktree_id {
                Some(worktree_id) => {
                    panel.set_worktree_scope(Some(worktree_id), window, cx);
                }
                None => {
                    panel.set_worktree_scope_unavailable(compatible_worktree_id, window, cx);
                }
            });
            if worktree_id.is_some()
                && let Err(error) = self.workbench_shell.ensure_visible_files_host(panel, cx)
            {
                log::warn!("failed to synchronize the native Files host: {error:#}");
                self.workbench_shell.record_error(error.to_string());
                if let Err(collapse_error) = self.workbench_shell.collapse_dock() {
                    log::warn!(
                        "failed to collapse Files after host synchronization failed: \
                         {collapse_error:#}"
                    );
                }
            }
            let identity_error =
                self.workbench_shell
                    .identity()
                    .and_then(|identity| match &identity.phase {
                        IdentityPhase::Error(error) => Some(error.clone()),
                        _ => None,
                    });
            self.workbench_shell
                .set_visible_files_identity_error(identity_error, window, cx);
        }
        let search_is_visible = self
            .workbench_shell
            .projection()
            .visible_projection()
            .is_some_and(|visible| {
                visible.dock_open
                    && visible.effective_surface == Some(omega_workbench_state::WorkSurface::Search)
            });
        if self.workbench_shell_enabled
            && search_is_visible
            && self.workbench_search_has_authority(cx)
        {
            let search_host_was_missing = self
                .workbench_shell
                .search_surface_for_active_binding(cx)
                .is_none();
            match self
                .prepare_search_surface(window, cx)
                .and_then(|search_surface| {
                    self.workbench_shell
                        .ensure_visible_search_host(search_surface, cx)
                }) {
                Ok(Some(host)) if search_surface_was_focused && search_host_was_missing => {
                    host.focus_handle(cx).focus(window, cx);
                }
                Ok(_) => {}
                Err(error) => {
                    log::warn!("failed to synchronize the native Search host: {error:#}");
                    self.workbench_shell.record_error(error.to_string());
                    if let Err(collapse_error) = self.workbench_shell.collapse_dock() {
                        log::warn!(
                            "failed to collapse Search after host synchronization failed: \
                             {collapse_error:#}"
                        );
                    }
                    self.focus_thread_transcript(window, cx);
                }
            }
        }
        if let Some(search_surface) = self.workbench_shell.search_surface_for_active_binding(cx) {
            let worktree_id = self.active_workbench_worktree_id(cx);
            let search_is_authoritative = self.workbench_search_has_authority(cx);
            let search_view = search_surface.read(cx).search_view().clone();
            Self::synchronize_native_search_scope(
                &search_view,
                worktree_id,
                !search_is_authoritative,
                cx,
            );
        }
        if let Some(review_surface) = self.workbench_shell.review_surface_for_active_binding(cx)
            && let Some(binding) = review_surface.read(cx).binding(cx)
        {
            let generation = binding.checkpoint.generation();
            if self.workbench_shell.projection().connection
                != omega_workbench_state::ConnectionPhase::Online
            {
                review_surface.update(cx, |review_surface, cx| {
                    review_surface.set_offline(generation, cx);
                });
                self.workbench_shell.set_active_review_content_state(
                    workbench_shell::SurfaceContentState::Offline,
                    window,
                    cx,
                );
            } else if self.workbench_review_has_authority(cx) {
                if matches!(
                    review_surface.read(cx).lifecycle(cx),
                    crate::AgentDiffLifecycle::Offline
                        | crate::AgentDiffLifecycle::UnavailableCheckpoint(_)
                ) {
                    review_surface.update(cx, |review_surface, cx| {
                        review_surface.set_online(generation, window, cx);
                    });
                }
                self.workbench_shell.set_active_review_content_state(
                    workbench_shell::SurfaceContentState::Ready,
                    window,
                    cx,
                );
            } else {
                let message: gpui::SharedString =
                    "The active Review checkpoint is unavailable".into();
                review_surface.update(cx, |review_surface, cx| {
                    review_surface.set_checkpoint_unavailable(generation, message.clone(), cx);
                });
                self.workbench_shell.set_active_review_content_state(
                    workbench_shell::SurfaceContentState::Error(message),
                    window,
                    cx,
                );
            }
        }
        if let Some(plan_surface) = self.workbench_shell.plan_surface_for_active_binding(cx) {
            let generation = plan_surface.read(cx).binding().generation;
            let lifecycle = self.native_plan_lifecycle(cx);
            plan_surface.update(cx, |plan_surface, cx| {
                plan_surface.set_lifecycle(generation, lifecycle, cx);
            });
        }
        let review_is_visible = self
            .workbench_shell
            .projection()
            .visible_projection()
            .is_some_and(|visible| {
                visible.dock_open
                    && visible.effective_surface == Some(omega_workbench_state::WorkSurface::Review)
            });
        if self.workbench_shell_enabled
            && review_is_visible
            && self.workbench_review_has_authority(cx)
        {
            let review_host_was_missing = self
                .workbench_shell
                .review_surface_for_active_binding(cx)
                .is_none();
            match self
                .prepare_review_surface(window, cx)
                .and_then(|review_surface| {
                    self.workbench_shell
                        .ensure_visible_review_host(review_surface, cx)
                }) {
                Ok(Some(host)) if review_surface_was_focused && review_host_was_missing => {
                    host.focus_handle(cx).focus(window, cx);
                }
                Ok(_) => {}
                Err(error) => {
                    log::warn!("failed to synchronize the native Review host: {error:#}");
                    self.workbench_shell.record_error(error.to_string());
                    if let Err(collapse_error) = self.workbench_shell.collapse_dock() {
                        log::warn!(
                            "failed to collapse Review after host synchronization failed: \
                             {collapse_error:#}"
                        );
                    }
                    self.focus_thread_transcript(window, cx);
                }
            }
        }
        if let Some(git_surface) = self.workbench_shell.git_surface_for_active_binding(cx) {
            let binding = git_surface.read(cx).binding().cloned();
            if let Some(binding) = binding {
                let lifecycle = self.native_git_lifecycle(cx);
                git_surface.update(cx, |git_surface, cx| {
                    git_surface.set_lifecycle(
                        binding.generation,
                        binding.git_repository_id,
                        lifecycle,
                        cx,
                    );
                });
            }
        }
        let git_is_visible = self
            .workbench_shell
            .projection()
            .visible_projection()
            .is_some_and(|visible| {
                visible.dock_open
                    && visible.effective_surface == Some(omega_workbench_state::WorkSurface::Git)
            });
        if self.workbench_shell_enabled && git_is_visible && self.workbench_git_has_authority(cx) {
            let git_host_was_missing = self
                .workbench_shell
                .git_surface_for_active_binding(cx)
                .is_none();
            match self
                .prepare_git_surface(window, cx)
                .and_then(|git_surface| {
                    let binding = git_surface
                        .read(cx)
                        .binding()
                        .cloned()
                        .ok_or_else(|| anyhow!("the native Git surface is not bound"))?;
                    let lifecycle = self.native_git_lifecycle(cx);
                    git_surface.update(cx, |git_surface, cx| {
                        git_surface.set_lifecycle(
                            binding.generation,
                            binding.git_repository_id,
                            lifecycle,
                            cx,
                        );
                    });
                    self.workbench_shell
                        .ensure_visible_git_host(git_surface, cx)
                }) {
                Ok(Some(host)) if git_surface_was_focused && git_host_was_missing => {
                    host.focus_handle(cx).focus(window, cx);
                }
                Ok(_) => {}
                Err(error) => {
                    log::warn!("failed to synchronize the native Git host: {error:#}");
                    self.workbench_shell.record_error(error.to_string());
                    if let Err(collapse_error) = self.workbench_shell.collapse_dock() {
                        log::warn!(
                            "failed to collapse Git after host synchronization failed: \
                             {collapse_error:#}"
                        );
                    }
                    self.focus_thread_transcript(window, cx);
                }
            }
        }
        let terminal_is_visible = self
            .workbench_shell
            .projection()
            .visible_projection()
            .is_some_and(|visible| {
                visible.dock_open
                    && visible.effective_surface
                        == Some(omega_workbench_state::WorkSurface::Terminal)
            });
        if let Some(terminal_surface) = self.workbench_terminal_surface.clone() {
            let generation = terminal_surface.read(cx).binding().generation;
            let owner_state = self.native_terminal_owner_state();
            terminal_surface.update(cx, |terminal_surface, cx| {
                terminal_surface.set_owner_state(generation, owner_state, cx);
            });
        }
        if self.workbench_shell_enabled && terminal_is_visible {
            let terminal_host_was_missing = self
                .workbench_shell
                .terminal_surface_for_active_binding(cx)
                .is_none();
            match self
                .prepare_terminal_surface(window, cx)
                .and_then(|terminal_surface| {
                    self.workbench_shell
                        .ensure_visible_terminal_host(terminal_surface, cx)
                }) {
                Ok(Some(host)) if terminal_surface_was_focused && terminal_host_was_missing => {
                    host.focus_handle(cx).focus(window, cx);
                }
                Ok(_) => {}
                Err(error) => {
                    log::warn!("failed to synchronize the native Terminal host: {error:#}");
                    self.workbench_shell.record_error(error.to_string());
                    self.focus_thread_transcript(window, cx);
                }
            }
        }
        if let Some(panel) = self.workbench_terminal_panel.as_ref() {
            let snapshot = panel.read(cx).snapshot(cx);
            if let Some(surface) = self.workbench_terminal_surface.as_ref() {
                surface.update(cx, |surface, cx| {
                    surface.reconcile_terminal_owners(&snapshot, cx);
                });
            }
            let running_count = snapshot.running_terminal_count();
            let badge = (running_count > 0).then(|| workbench_shell::SurfaceBadge::Count {
                count: running_count,
                tone: workbench_shell::BadgeTone::Accent,
                label: format!("{running_count} running terminal processes").into(),
            });
            #[cfg(any(test, feature = "test-support"))]
            let badge = self.workbench_terminal_badge_override.clone().or(badge);
            self.workbench_shell
                .set_badge(omega_workbench_state::WorkSurface::Terminal, badge);
        }
        let files_is_visible = self
            .workbench_shell
            .projection()
            .visible_projection()
            .is_some_and(|visible| {
                visible.dock_open
                    && visible.effective_surface == Some(omega_workbench_state::WorkSurface::Files)
            });
        if files_panel_was_focused && !self.workbench_files_are_actionable(cx) {
            if files_is_visible {
                if let Some(host) = self.workbench_shell.visible_host().cloned() {
                    host.focus_handle(cx).focus(window, cx);
                } else {
                    self.focus_thread_transcript(window, cx);
                }
            } else {
                self.focus_thread_transcript(window, cx);
            }
        } else if files_panel_was_focused && !files_is_visible {
            self.focus_thread_transcript(window, cx);
        }
        if search_surface_was_focused
            && (!search_is_visible || !self.workbench_search_has_authority(cx))
        {
            self.focus_thread_transcript(window, cx);
        }
        if review_surface_was_focused
            && (!review_is_visible || !self.workbench_review_has_authority(cx))
        {
            self.focus_thread_transcript(window, cx);
        }
        if git_surface_was_focused && (!git_is_visible || !self.workbench_git_has_authority(cx)) {
            self.focus_thread_transcript(window, cx);
        }
        if terminal_surface_was_focused && !terminal_is_visible {
            self.focus_thread_transcript(window, cx);
        }
    }

    fn synchronize_thread_outline(&mut self, cx: &mut Context<Self>) {
        let Some(visible) = self.workbench_shell.projection().visible_projection() else {
            self.thread_outline
                .update(cx, |outline, cx| outline.unbind(cx));
            return;
        };
        let Some(thread) = self.active_agent_thread(cx) else {
            self.thread_outline
                .update(cx, |outline, cx| outline.unbind(cx));
            return;
        };
        let lifecycle = match self.workbench_shell.projection().connection {
            omega_workbench_state::ConnectionPhase::Offline
            | omega_workbench_state::ConnectionPhase::StaleProjection => {
                crate::thread_outline::ThreadOutlineLifecycle::Stale
            }
            omega_workbench_state::ConnectionPhase::Reconnecting => {
                crate::thread_outline::ThreadOutlineLifecycle::Reconnecting
            }
            omega_workbench_state::ConnectionPhase::Online => match thread.read(cx).status() {
                ThreadStatus::Generating => {
                    crate::thread_outline::ThreadOutlineLifecycle::Streaming
                }
                ThreadStatus::Idle => crate::thread_outline::ThreadOutlineLifecycle::Ready,
            },
        };
        let binding = crate::thread_outline::ThreadOutlineBinding {
            thread_id: visible.thread_id,
            repository: visible.binding,
            generation: visible.generation,
        };
        self.thread_outline.update(cx, |outline, cx| {
            outline.bind_thread(binding, thread, lifecycle, cx);
        });
    }

    fn active_workbench_worktree_id(&self, cx: &App) -> Option<WorktreeId> {
        let selected_path = self
            .workbench_shell
            .identity()
            .and_then(|identity| identity.selected.as_ref())
            .map(|selected| selected.worktree_abs_path.as_path())?;
        self.project
            .read(cx)
            .visible_worktrees(cx)
            .find(|worktree| worktree.read(cx).abs_path().as_ref() == selected_path)
            .map(|worktree| worktree.read(cx).id())
    }

    fn active_workbench_worktree_abs_path(&self) -> Option<PathBuf> {
        self.workbench_shell
            .identity()
            .and_then(|identity| identity.selected.as_ref())
            .map(|selected| selected.worktree_abs_path.clone())
    }

    fn active_workbench_git_repository_id(&self, cx: &App) -> Option<RepositoryId> {
        let visible = self.workbench_shell.projection().visible_projection()?;
        let selected = self.workbench_shell.identity()?.selected.as_ref()?;
        if visible.binding.as_ref() != Some(&selected.binding) {
            return None;
        }
        let repository_id = RepositoryId(selected.git_repository_id?);
        self.project
            .read(cx)
            .git_store()
            .read(cx)
            .repositories()
            .contains_key(&repository_id)
            .then_some(repository_id)
    }

    fn workbench_files_have_authority(&self, cx: &App) -> bool {
        self.workbench_repository_surface_has_authority(
            omega_workbench_state::WorkSurface::Files,
            cx,
        )
    }

    fn workbench_search_has_authority(&self, cx: &App) -> bool {
        self.workbench_repository_surface_has_authority(
            omega_workbench_state::WorkSurface::Search,
            cx,
        )
    }

    fn workbench_review_has_authority(&self, cx: &App) -> bool {
        self.workbench_repository_surface_has_authority(
            omega_workbench_state::WorkSurface::Review,
            cx,
        ) && self.active_agent_thread(cx).is_some()
    }

    fn workbench_git_has_authority(&self, cx: &App) -> bool {
        self.workbench_repository_surface_has_authority(omega_workbench_state::WorkSurface::Git, cx)
            && self.active_workbench_git_repository_id(cx).is_some()
    }

    fn workbench_terminal_has_authority(&self, cx: &App) -> bool {
        self.workbench_repository_surface_has_authority(
            omega_workbench_state::WorkSurface::Terminal,
            cx,
        ) && self.active_workbench_worktree_id(cx).is_some()
    }

    fn workbench_repository_surface_has_authority(
        &self,
        surface: omega_workbench_state::WorkSurface,
        cx: &App,
    ) -> bool {
        self.workbench_shell.projection().connection
            == omega_workbench_state::ConnectionPhase::Online
            && self
                .workbench_shell
                .capability(surface)
                .is_some_and(|capability| capability.availability.is_available())
            && self
                .workbench_shell
                .identity()
                .is_some_and(|identity| matches!(&identity.phase, IdentityPhase::Ready))
            && self.active_workbench_worktree_id(cx).is_some()
    }

    fn workbench_files_are_actionable(&self, cx: &App) -> bool {
        self.workbench_files_have_authority(cx)
            && !matches!(
                self.workbench_shell
                    .active_surface_content_state(omega_workbench_state::WorkSurface::Files, cx,),
                Some(
                    workbench_shell::SurfaceContentState::Loading
                        | workbench_shell::SurfaceContentState::Error(_)
                        | workbench_shell::SurfaceContentState::Offline
                )
            )
    }

    fn workbench_files_compatible_worktree_id(&self, cx: &App) -> Option<WorktreeId> {
        let worktree_id = self.active_workbench_worktree_id(cx)?;
        let identity = self.workbench_shell.identity()?;
        (!matches!(
            &identity.phase,
            IdentityPhase::NoProject | IdentityPhase::Missing | IdentityPhase::Inconsistent(_)
        ))
        .then_some(worktree_id)
    }

    fn handle_workbench_files_panel_event(
        &mut self,
        event: &PanelEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.workbench_files_panel_handed_off {
            return;
        }
        match event {
            PanelEvent::Activate => {
                if !self.workbench_files_are_actionable(cx) {
                    let files_is_open = self
                        .workbench_shell
                        .projection()
                        .visible_projection()
                        .is_some_and(|visible| {
                            visible.dock_open
                                && visible.effective_surface
                                    == Some(omega_workbench_state::WorkSurface::Files)
                        });
                    if files_is_open && let Err(error) = self.workbench_shell.collapse_dock() {
                        log::warn!("failed to close unavailable Files: {error:#}");
                        self.workbench_shell.record_error(error.to_string());
                    }
                    self.focus_thread_transcript(window, cx);
                    return;
                }
                let files_is_open = self
                    .workbench_shell
                    .projection()
                    .visible_projection()
                    .is_some_and(|visible| {
                        visible.dock_open
                            && visible.effective_surface
                                == Some(omega_workbench_state::WorkSurface::Files)
                    });
                if files_is_open {
                    if let Some(panel) = self.workbench_files_panel.as_ref() {
                        panel.focus_handle(cx).focus(window, cx);
                    }
                    cx.notify();
                } else {
                    self.select_work_surface(omega_workbench_state::WorkSurface::Files, window, cx);
                }
            }
            PanelEvent::Close => {
                let files_is_open = self
                    .workbench_shell
                    .projection()
                    .visible_projection()
                    .is_some_and(|visible| {
                        visible.dock_open
                            && visible.effective_surface
                                == Some(omega_workbench_state::WorkSurface::Files)
                    });
                if files_is_open {
                    if let Err(error) = self.workbench_shell.collapse_dock() {
                        log::warn!("failed to close the native Files surface: {error:#}");
                        self.workbench_shell.record_error(error.to_string());
                    }
                    self.focus_thread_transcript(window, cx);
                }
            }
            PanelEvent::ZoomIn | PanelEvent::ZoomOut => {}
        }
    }

    fn handle_workbench_git_panel_event(
        &mut self,
        event: &PanelEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.workbench_git_panel_handed_off {
            return;
        }
        let git_is_open = self
            .workbench_shell
            .projection()
            .visible_projection()
            .is_some_and(|visible| {
                visible.dock_open
                    && visible.effective_surface == Some(omega_workbench_state::WorkSurface::Git)
            });
        match event {
            PanelEvent::Activate => {
                if !self.workbench_git_has_authority(cx) {
                    if git_is_open && let Err(error) = self.workbench_shell.collapse_dock() {
                        log::warn!("failed to close unavailable Git: {error:#}");
                        self.workbench_shell.record_error(error.to_string());
                    }
                    self.focus_thread_transcript(window, cx);
                } else if git_is_open {
                    if let Some(panel) = self.workbench_git_panel.as_ref() {
                        panel.read(cx).activation_focus_handle(cx).focus(window, cx);
                    }
                    cx.notify();
                } else {
                    self.select_work_surface(omega_workbench_state::WorkSurface::Git, window, cx);
                }
            }
            PanelEvent::Close => {
                if git_is_open {
                    if let Err(error) = self.workbench_shell.collapse_dock() {
                        log::warn!("failed to close the native Git surface: {error:#}");
                        self.workbench_shell.record_error(error.to_string());
                    }
                    self.focus_thread_transcript(window, cx);
                }
            }
            PanelEvent::ZoomIn | PanelEvent::ZoomOut => {}
        }
    }

    fn handle_workbench_terminal_panel_event(
        &mut self,
        event: &PanelEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.workbench_terminal_panel_handed_off {
            return;
        }
        let terminal_is_open = self
            .workbench_shell
            .projection()
            .visible_projection()
            .is_some_and(|visible| {
                visible.dock_open
                    && visible.effective_surface
                        == Some(omega_workbench_state::WorkSurface::Terminal)
            });
        match event {
            PanelEvent::Activate => {
                if terminal_is_open {
                    if let Some(panel) = self.workbench_terminal_panel.as_ref() {
                        panel.read(cx).activation_focus_handle(cx).focus(window, cx);
                    }
                    cx.notify();
                } else if self.workbench_terminal_has_authority(cx) {
                    self.select_work_surface(
                        omega_workbench_state::WorkSurface::Terminal,
                        window,
                        cx,
                    );
                } else {
                    self.focus_thread_transcript(window, cx);
                }
            }
            PanelEvent::Close => {
                if terminal_is_open {
                    if let Err(error) = self.workbench_shell.collapse_dock() {
                        log::warn!("failed to close the native Terminal surface: {error:#}");
                        self.workbench_shell.record_error(error.to_string());
                    }
                    self.focus_thread_transcript(window, cx);
                }
            }
            PanelEvent::ZoomIn | PanelEvent::ZoomOut => {}
        }
    }

    fn prepare_files_surface(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Result<(Entity<ProjectPanel>, Option<WorktreeId>, bool)> {
        if !self.workbench_files_have_authority(cx) {
            anyhow::bail!("the native Files surface has no usable repository authority");
        }
        let worktree_id = self
            .active_workbench_worktree_id(cx)
            .ok_or_else(|| anyhow!("the active thread worktree is unavailable"))?;
        let panel = if let Some(panel) = self.workbench_files_panel.clone() {
            panel
        } else {
            let panel = self
                .workspace
                .upgrade()
                .and_then(|workspace| workspace.read(cx).panel::<ProjectPanel>(cx))
                .ok_or_else(|| anyhow!("the native Files surface is still loading"))?;
            self.workbench_files_panel = Some(panel.clone());
            self._workbench_files_panel_observation =
                Some(cx.observe(&panel, |_this, _panel, cx| cx.notify()));
            self._workbench_files_panel_event_subscription = Some(cx.subscribe_in(
                &panel,
                window,
                |this, _panel, event: &PanelEvent, window, cx| {
                    this.handle_workbench_files_panel_event(event, window, cx);
                },
            ));
            panel
        };
        let previous_scope = panel.read(cx).worktree_scope();
        let previous_scope_was_unavailable = matches!(
            panel.read(cx).scope_state(),
            project_panel::ProjectPanelScopeState::Unavailable
        );
        let files_are_actionable = self.workbench_files_are_actionable(cx);
        panel.update(cx, |panel, cx| {
            panel.route_open_terminal_to_thread(true);
            if files_are_actionable {
                panel.set_worktree_scope(Some(worktree_id), window, cx);
            } else {
                panel.set_worktree_scope_unavailable(Some(worktree_id), window, cx);
            }
        });
        Ok((panel, previous_scope, previous_scope_was_unavailable))
    }

    fn prepare_search_surface(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Result<Entity<workbench_shell::NativeSearchSurface>> {
        if !self.workbench_search_has_authority(cx) {
            anyhow::bail!("the native Search surface has no usable repository authority");
        }
        let worktree_id = self
            .active_workbench_worktree_id(cx)
            .ok_or_else(|| anyhow!("the active thread worktree is unavailable"))?;
        if let Some(search_surface) = self.workbench_shell.search_surface_for_active_binding(cx) {
            let search_view = search_surface.read(cx).search_view().clone();
            Self::synchronize_native_search_scope(&search_view, Some(worktree_id), false, cx);
            return Ok(search_surface);
        }
        if self.workspace.upgrade().is_none() {
            anyhow::bail!("the workspace closed while opening Search");
        }
        let search_surface = cx.new(|cx| {
            workbench_shell::NativeSearchSurface::new(
                self.workspace.clone(),
                self.project.clone(),
                window,
                cx,
            )
        });
        let search_view = search_surface.read(cx).search_view().clone();
        Self::synchronize_native_search_scope(&search_view, Some(worktree_id), false, cx);
        Ok(search_surface)
    }

    fn prepare_review_surface(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Result<Entity<workbench_shell::NativeReviewSurface>> {
        if !self.workbench_review_has_authority(cx) {
            anyhow::bail!("the native Review surface has no usable thread repository authority");
        }
        let visible = self
            .workbench_shell
            .projection()
            .visible_projection()
            .ok_or_else(|| anyhow!("open a thread before preparing native Review"))?;
        let repository = visible
            .binding
            .clone()
            .ok_or_else(|| anyhow!("the active thread has no repository binding"))?;
        let generation = visible.generation;
        let thread_id = self
            .active_thread_id(cx)
            .ok_or_else(|| anyhow!("the active thread has no typed thread identity"))?;
        if thread_id.to_key_string() != visible.thread_id {
            anyhow::bail!(
                "the active typed thread changed before native Review could bind to its projection"
            );
        }
        let worktree_id = self
            .active_workbench_worktree_id(cx)
            .ok_or_else(|| anyhow!("the active thread worktree is unavailable"))?;
        let thread = self
            .active_agent_thread(cx)
            .ok_or_else(|| anyhow!("the active thread cannot provide native Review"))?;
        let binding = crate::AgentDiffBinding::for_thread(
            thread_id,
            repository,
            worktree_id,
            generation,
            &thread,
            cx,
        );
        if let Some(review_surface) = self.workbench_shell.review_surface_for_active_binding(cx) {
            review_surface.update(cx, |review_surface, cx| {
                review_surface.bind(binding, window, cx)
            })?;
            return Ok(review_surface);
        }
        let workspace = self
            .workspace
            .upgrade()
            .ok_or_else(|| anyhow!("the workspace closed while opening Review"))?;
        let review_surface =
            cx.new(|cx| workbench_shell::NativeReviewSurface::new(workspace, thread, window, cx));
        review_surface.update(cx, |review_surface, cx| {
            review_surface.bind(binding, window, cx)
        })?;
        Ok(review_surface)
    }

    fn prepare_git_surface(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Result<Entity<workbench_shell::NativeGitSurface>> {
        if !self.workbench_git_has_authority(cx) {
            anyhow::bail!("the native Git surface has no usable repository authority");
        }
        let visible = self
            .workbench_shell
            .projection()
            .visible_projection()
            .ok_or_else(|| anyhow!("open a thread before preparing native Git"))?;
        let repository = visible
            .binding
            .clone()
            .ok_or_else(|| anyhow!("the active thread has no repository binding"))?;
        let generation = visible.generation;
        let thread_id = visible.thread_id;
        let worktree_id = self
            .active_workbench_worktree_id(cx)
            .ok_or_else(|| anyhow!("the active thread worktree is unavailable"))?;
        let git_repository_id = self
            .active_workbench_git_repository_id(cx)
            .ok_or_else(|| anyhow!("the active thread Git repository is unavailable"))?;
        let repository_ids = self
            .project
            .read(cx)
            .git_store()
            .read(cx)
            .repository_ids_for_worktree(worktree_id);
        if !repository_ids.contains(&git_repository_id) {
            anyhow::bail!(
                "repository {git_repository_id:?} does not belong to worktree {worktree_id:?}"
            );
        }
        let panel = if let Some(panel) = self.workbench_git_panel.clone() {
            panel
        } else {
            let panel = self
                .workspace
                .upgrade()
                .and_then(|workspace| workspace.read(cx).panel::<GitPanel>(cx))
                .ok_or_else(|| anyhow!("the native Git surface is still loading"))?;
            self.workbench_git_panel = Some(panel.clone());
            self._workbench_git_panel_observation =
                Some(cx.observe_in(&panel, window, |this, _panel, window, cx| {
                    this.synchronize_git_surface_lifecycle_for_panel(cx);
                    this.sync_workbench_shell(window, cx);
                    cx.notify();
                }));
            self._workbench_git_panel_event_subscription = Some(cx.subscribe_in(
                &panel,
                window,
                |this, _panel, event: &PanelEvent, window, cx| {
                    this.handle_workbench_git_panel_event(event, window, cx);
                },
            ));
            panel
        };
        let binding = workbench_shell::NativeGitBinding {
            thread_id,
            repository,
            worktree_id,
            git_repository_id,
            generation,
        };
        let lifecycle = self.native_git_lifecycle(cx);
        if let Some(git_surface) = self.workbench_shell.git_surface_for_active_binding(cx) {
            git_surface.update(cx, |git_surface, cx| {
                git_surface.bind(binding.clone(), window, cx)?;
                git_surface.set_lifecycle(
                    binding.generation,
                    binding.git_repository_id,
                    lifecycle,
                    cx,
                );
                Ok::<(), anyhow::Error>(())
            })?;
            return Ok(git_surface);
        }
        let git_surface = cx.new(|cx| workbench_shell::NativeGitSurface::new(panel.clone(), cx));
        git_surface.update(cx, |git_surface, cx| {
            git_surface.bind(binding.clone(), window, cx)?;
            git_surface.set_lifecycle(binding.generation, binding.git_repository_id, lifecycle, cx);
            Ok::<(), anyhow::Error>(())
        })?;
        Ok(git_surface)
    }

    fn restore_workbench_selection_from_disk_if_needed(
        &mut self,
        thread_id: &str,
        _cx: &mut Context<Self>,
    ) {
        if self
            .workbench_shell
            .projection()
            .persisted_selection
            .is_some()
        {
            return;
        }
        match crate::workbench_surface_store::read_selection(paths::data_dir().as_path(), thread_id)
        {
            Ok(Some(selection)) => {
                let revision = selection.revision.max(1);
                let _ = self.workbench_shell.projection_mut().apply(
                    omega_workbench_state::ProjectionTransition::PersistSelection { revision },
                );
                // Re-apply the disk record after PersistSelection stamped the active
                // thread, so cold restore sees the saved surface request.
                self.workbench_shell.projection_mut().persisted_selection = Some(selection);
                let _ = self
                    .workbench_shell
                    .projection_mut()
                    .apply(omega_workbench_state::ProjectionTransition::ColdStart);
                let _ = self
                    .workbench_shell
                    .projection_mut()
                    .apply(omega_workbench_state::ProjectionTransition::RestoreSelection);
            }
            Ok(None) => {}
            Err(error) => {
                log::warn!("workbench surface disk restore failed: {error:#}");
            }
        }
    }

    fn persist_active_workbench_selection(&mut self, cx: &mut Context<Self>) {
        let Some(visible) = self.workbench_shell.projection().visible_projection() else {
            return;
        };
        let revision = self
            .workbench_shell
            .projection()
            .persistence_revision
            .saturating_add(1);
        if let Err(error) = self
            .workbench_shell
            .projection_mut()
            .apply(omega_workbench_state::ProjectionTransition::PersistSelection { revision })
        {
            log::warn!("workbench surface persist transition failed: {error:#}");
            return;
        }
        let Some(selection) = self
            .workbench_shell
            .projection()
            .persisted_selection
            .clone()
        else {
            return;
        };
        let data_dir = paths::data_dir().clone();
        cx.background_spawn(async move {
            if let Err(error) =
                crate::workbench_surface_store::write_selection(&data_dir, &selection)
            {
                log::warn!("workbench surface disk persist failed: {error:#}");
            }
        })
        .detach();
        let _ = visible;
    }

    fn prepare_plan_surface(
        &mut self,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Result<Entity<workbench_shell::NativePlanSurface>> {
        let visible = self
            .workbench_shell
            .projection()
            .visible_projection()
            .ok_or_else(|| anyhow!("open a thread before preparing native Plan"))?;
        let thread_id = visible.thread_id.clone();
        let acp_thread = self.active_agent_thread(cx);
        let binding = workbench_shell::NativePlanBinding {
            thread_id: thread_id.clone(),
            generation: visible.generation,
        };
        let plan_surface =
            if let Some(surface) = self.workbench_shell.plan_surface_for_active_binding(cx) {
                surface.update(cx, |surface, cx| {
                    surface.bind_thread(binding.clone(), acp_thread.clone(), cx);
                });
                surface
            } else {
                let mut navigation_handler = {
                    let panel = cx.entity().downgrade();
                    Some(Rc::new(
                        move |entry_index: usize, _window: &mut Window, cx: &mut App| {
                            panel
                                .update(cx, |panel, cx| {
                                    panel.navigate_to_plan_entry(&thread_id, entry_index, cx)
                                })
                                .unwrap_or(false)
                        },
                    )
                        as Rc<dyn Fn(usize, &mut Window, &mut App) -> bool>)
                };
                let surface = cx.new(|cx| {
                    let mut surface = workbench_shell::NativePlanSurface::new(binding.clone(), cx);
                    if let Some(handler) = navigation_handler.take() {
                        surface.set_navigation_handler(handler);
                    }
                    surface.bind_thread(binding, acp_thread, cx);
                    surface
                });
                surface
            };
        let generation = plan_surface.read(cx).binding().generation;
        let lifecycle = self.native_plan_lifecycle(cx);
        plan_surface.update(cx, |surface, cx| {
            surface.set_lifecycle(generation, lifecycle, cx);
        });
        Ok(plan_surface)
    }

    fn prepare_terminal_surface(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Result<Entity<workbench_shell::NativeTerminalSurface>> {
        if !self.workbench_terminal_has_authority(cx) {
            if let Some(surface) = self.workbench_terminal_surface.clone() {
                let generation = surface.read(cx).binding().generation;
                let owner_state = self.native_terminal_owner_state();
                surface.update(cx, |surface, cx| {
                    surface.set_owner_state(generation, owner_state, cx);
                });
                return Ok(surface);
            }
            anyhow::bail!("the native Terminal surface has no usable worktree authority");
        }
        let visible = self
            .workbench_shell
            .projection()
            .visible_projection()
            .ok_or_else(|| anyhow!("open a thread before preparing native Terminal"))?;
        let repository = visible
            .binding
            .clone()
            .ok_or_else(|| anyhow!("the active thread has no repository binding"))?;
        let worktree_id = self
            .active_workbench_worktree_id(cx)
            .ok_or_else(|| anyhow!("the active thread worktree is unavailable"))?;
        let worktree_abs_path = self
            .active_workbench_worktree_abs_path()
            .ok_or_else(|| anyhow!("the active thread worktree path is unavailable"))?;
        let binding = workbench_shell::NativeTerminalBinding {
            thread_id: visible.thread_id,
            repository,
            worktree_id,
            worktree_abs_path,
            generation: visible.generation,
        };
        let panel = if let Some(panel) = self.workbench_terminal_panel.clone() {
            panel
        } else {
            let panel = self
                .workspace
                .upgrade()
                .and_then(|workspace| workspace.read(cx).panel::<TerminalPanel>(cx))
                .ok_or_else(|| anyhow!("the native Terminal surface is still loading"))?;
            self.workbench_terminal_panel = Some(panel.clone());
            self._workbench_terminal_panel_observation =
                Some(cx.observe_in(&panel, window, |this, _panel, window, cx| {
                    this.sync_workbench_shell(window, cx);
                    cx.notify();
                }));
            self._workbench_terminal_panel_event_subscription = Some(cx.subscribe_in(
                &panel,
                window,
                |this, _panel, event: &PanelEvent, window, cx| {
                    this.handle_workbench_terminal_panel_event(event, window, cx);
                },
            ));
            panel
        };
        if !self.workbench_terminal_handlers_installed {
            let panel_owner = cx.entity().downgrade();
            let new_terminal_request_handler = Rc::new(move |window: &mut Window, cx: &mut App| {
                panel_owner
                    .update(cx, |panel, cx| {
                        panel.create_terminal_for_active_thread(window, cx);
                    })
                    .log_err();
            });
            let panel_owner = cx.entity().downgrade();
            let split_terminal_request_handler = Rc::new(
                move |direction: SplitDirection, window: &mut Window, cx: &mut App| {
                    panel_owner
                        .update(cx, |panel, cx| {
                            panel.create_terminal_split_for_active_thread(direction, window, cx);
                        })
                        .log_err();
                },
            );
            panel.update(cx, |panel, cx| {
                panel.set_workbench_request_handlers(
                    Some(new_terminal_request_handler),
                    Some(split_terminal_request_handler),
                    cx,
                );
            });
            self.workbench_terminal_handlers_installed = true;
        }
        if let Some(surface) = self.workbench_terminal_surface.clone() {
            surface.update(cx, |surface, cx| surface.bind(binding, cx));
            return Ok(surface);
        }
        let surface = cx.new(|cx| workbench_shell::NativeTerminalSurface::new(panel, binding, cx));
        self.workbench_terminal_surface = Some(surface.clone());
        Ok(surface)
    }

    fn native_git_lifecycle(&self, cx: &App) -> workbench_shell::NativeGitLifecycle {
        #[cfg(any(test, feature = "test-support"))]
        if let Some(lifecycle) = &self.workbench_git_lifecycle_override {
            return lifecycle.clone();
        }
        match self.workbench_shell.projection().connection {
            omega_workbench_state::ConnectionPhase::Offline => {
                return workbench_shell::NativeGitLifecycle::Offline;
            }
            omega_workbench_state::ConnectionPhase::Reconnecting
            | omega_workbench_state::ConnectionPhase::StaleProjection => {
                return workbench_shell::NativeGitLifecycle::Reconnecting;
            }
            omega_workbench_state::ConnectionPhase::Online => {}
        }
        let Some(identity) = self.workbench_shell.identity() else {
            return workbench_shell::NativeGitLifecycle::Loading;
        };
        match &identity.phase {
            IdentityPhase::NoProject | IdentityPhase::Missing => {
                return workbench_shell::NativeGitLifecycle::RepositoryRemoved;
            }
            IdentityPhase::Loading | IdentityPhase::Stale | IdentityPhase::Reconnecting => {
                return workbench_shell::NativeGitLifecycle::Loading;
            }
            IdentityPhase::Offline => {
                return workbench_shell::NativeGitLifecycle::Offline;
            }
            IdentityPhase::Error(error) | IdentityPhase::Inconsistent(error) => {
                return workbench_shell::NativeGitLifecycle::Error(error.clone());
            }
            IdentityPhase::Ready => {}
        }
        let Some(selected) = identity.selected.as_ref() else {
            return workbench_shell::NativeGitLifecycle::RepositoryRemoved;
        };
        let Some(repository_id) = selected.git_repository_id.map(RepositoryId) else {
            return workbench_shell::NativeGitLifecycle::RepositoryRemoved;
        };
        if self.workbench_git_panel_handed_off
            && let Some(panel) = self.workbench_git_panel.as_ref()
        {
            let panel = panel.read(cx);
            let Some(scope) = panel.repository_scope() else {
                return workbench_shell::NativeGitLifecycle::RepositoryRemoved;
            };
            if scope.repository_id != repository_id {
                return workbench_shell::NativeGitLifecycle::Loading;
            }
            if !panel.repository_scope_is_available(cx) {
                return workbench_shell::NativeGitLifecycle::RepositoryRemoved;
            }
        }
        let git_store = self.project.read(cx).git_store();
        let git_store = git_store.read(cx);
        let Some(repository) = git_store.repositories().get(&repository_id) else {
            return workbench_shell::NativeGitLifecycle::RepositoryRemoved;
        };
        if self.workbench_git_panel.as_ref().is_some_and(|panel| {
            let panel = panel.read(cx);
            panel
                .repository_scope()
                .is_some_and(|scope| scope.repository_id == repository_id)
                && panel.has_pending_operation(cx)
        }) {
            return workbench_shell::NativeGitLifecycle::OperationPending;
        }
        let pending = repository.read(cx).pending_ops_summary().item_summary;
        if pending.staging_count > 0 {
            return workbench_shell::NativeGitLifecycle::OperationPending;
        }
        if selected.git.conflicts > 0 {
            return workbench_shell::NativeGitLifecycle::Conflicted;
        }
        match &selected.branch {
            BranchIdentity::Detached(_) => return workbench_shell::NativeGitLifecycle::Detached,
            BranchIdentity::Unborn => return workbench_shell::NativeGitLifecycle::Unborn,
            BranchIdentity::Branch(_) | BranchIdentity::NoGit => {}
        }
        if selected.git.dirty_files > 0 {
            workbench_shell::NativeGitLifecycle::Dirty
        } else {
            workbench_shell::NativeGitLifecycle::Clean
        }
    }

    fn native_plan_lifecycle(&self, cx: &App) -> workbench_shell::NativePlanLifecycle {
        #[cfg(any(test, feature = "test-support"))]
        if let Some(lifecycle) = self.workbench_plan_lifecycle_override.as_ref() {
            return lifecycle.clone();
        }
        match self.workbench_shell.projection().connection {
            omega_workbench_state::ConnectionPhase::Offline
            | omega_workbench_state::ConnectionPhase::StaleProjection => {
                return workbench_shell::NativePlanLifecycle::Stale;
            }
            omega_workbench_state::ConnectionPhase::Reconnecting => {
                return workbench_shell::NativePlanLifecycle::Reconnecting;
            }
            omega_workbench_state::ConnectionPhase::Online => {}
        }
        if let Some(error) = self
            .active_agent_thread(cx)
            .and_then(|thread| thread.read(cx).plan_error().cloned())
        {
            return workbench_shell::NativePlanLifecycle::Malformed(error);
        }
        if let Some(interruption) = self
            .active_agent_thread(cx)
            .and_then(|thread| thread.read(cx).plan_interruption().cloned())
        {
            return workbench_shell::NativePlanLifecycle::Interrupted(interruption);
        }
        if self
            .active_agent_thread(cx)
            .is_some_and(|thread| thread.read(cx).had_error())
        {
            workbench_shell::NativePlanLifecycle::Interrupted("agent error".into())
        } else {
            workbench_shell::NativePlanLifecycle::Ready
        }
    }

    fn native_terminal_owner_state(&self) -> workbench_shell::NativeTerminalOwnerState {
        #[cfg(any(test, feature = "test-support"))]
        if let Some(owner_state) = self.workbench_terminal_owner_state_override.as_ref() {
            return owner_state.clone();
        }
        match self.workbench_shell.projection().connection {
            omega_workbench_state::ConnectionPhase::Offline => {
                return workbench_shell::NativeTerminalOwnerState::Offline;
            }
            omega_workbench_state::ConnectionPhase::Reconnecting
            | omega_workbench_state::ConnectionPhase::StaleProjection => {
                return workbench_shell::NativeTerminalOwnerState::Reconnecting;
            }
            omega_workbench_state::ConnectionPhase::Online => {}
        }
        let Some(identity) = self.workbench_shell.identity() else {
            return workbench_shell::NativeTerminalOwnerState::Error(
                "Terminal worktree identity is still loading".into(),
            );
        };
        match &identity.phase {
            IdentityPhase::Ready => workbench_shell::NativeTerminalOwnerState::Ready,
            IdentityPhase::NoProject | IdentityPhase::Missing => {
                workbench_shell::NativeTerminalOwnerState::WorktreeRemoved
            }
            IdentityPhase::Offline => workbench_shell::NativeTerminalOwnerState::Offline,
            IdentityPhase::Loading | IdentityPhase::Stale | IdentityPhase::Reconnecting => {
                workbench_shell::NativeTerminalOwnerState::Reconnecting
            }
            IdentityPhase::Error(error) | IdentityPhase::Inconsistent(error) => {
                workbench_shell::NativeTerminalOwnerState::Error(error.clone())
            }
        }
    }

    fn synchronize_git_surface_lifecycle_for_panel(&mut self, cx: &mut Context<Self>) {
        let scope = self
            .workbench_git_panel
            .as_ref()
            .and_then(|panel| panel.read(cx).repository_scope());
        if let Some(scope) = scope {
            let lifecycle = self.native_git_lifecycle(cx);
            self.workbench_shell
                .set_git_scope_lifecycle(scope, lifecycle, cx);
        }
    }

    fn synchronize_native_search_scope(
        search_view: &Entity<search::project_search::ProjectSearchView>,
        worktree_id: Option<WorktreeId>,
        unavailable: bool,
        cx: &mut Context<Self>,
    ) {
        let needs_update = search_view.read_with(cx, |search_view, cx| {
            let already_unavailable = matches!(
                search_view.lifecycle(cx),
                search::project_search::ProjectSearchLifecycle::Failed {
                    error: search::project_search::ProjectSearchError::WorktreeUnavailable,
                    ..
                }
            );
            search_view.worktree_scope(cx) != worktree_id || unavailable != already_unavailable
        });
        if !needs_update {
            return;
        }
        search_view.update(cx, |search_view, cx| {
            if unavailable {
                search_view.set_worktree_scope_unavailable(worktree_id, cx);
            } else {
                search_view.set_worktree_scope(worktree_id, cx);
            }
        });
    }

    fn detach_workspace_files_panel(
        &mut self,
        panel: &Entity<ProjectPanel>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Result<()> {
        let workspace = self
            .workspace
            .upgrade()
            .context("the workspace closed while opening Files")?;
        workspace.update(cx, |workspace, cx| {
            workspace.rehome_panel(panel, window, cx)
        })?;
        self.workbench_files_panel_handed_off = true;
        Ok(())
    }

    fn detach_workspace_git_panel(
        &mut self,
        panel: &Entity<GitPanel>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Result<()> {
        let workspace = self
            .workspace
            .upgrade()
            .context("the workspace closed while opening Git")?;
        workspace.update(cx, |workspace, cx| {
            workspace.rehome_panel(panel, window, cx)
        })?;
        self.workbench_git_panel_handed_off = true;
        Ok(())
    }

    fn detach_workspace_terminal_panel(
        &mut self,
        panel: &Entity<TerminalPanel>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Result<()> {
        let workspace = self
            .workspace
            .upgrade()
            .context("the workspace closed while opening Terminal")?;
        workspace.update(cx, |workspace, cx| {
            workspace.rehome_panel(panel, window, cx)
        })?;
        self.workbench_terminal_panel_handed_off = true;
        Ok(())
    }

    fn binding_epoch_matches(
        &self,
        expected_thread_id: &str,
        expected_binding: Option<&omega_workbench_state::RepositoryBinding>,
        expected_binding_generation: u64,
        require_active: bool,
        cx: &App,
    ) -> bool {
        let projection = self.workbench_shell.projection();
        (!require_active
            || (self
                .active_thread_id(cx)
                .is_some_and(|thread_id| thread_id.to_key_string() == expected_thread_id)
                && projection.active_thread_id.as_deref() == Some(expected_thread_id)))
            && projection
                .threads
                .get(expected_thread_id)
                .is_some_and(|thread| {
                    thread.generation == expected_binding_generation
                        && thread.binding.as_ref() == expected_binding
                })
    }

    fn record_thread_identity_operation_error(
        &mut self,
        thread_id: &str,
        source_binding: Option<&omega_workbench_state::RepositoryBinding>,
        attempted_binding: &omega_workbench_state::RepositoryBinding,
        binding_generation: u64,
        inconsistent: bool,
        message: impl Into<SharedString>,
    ) {
        self.thread_identity_operation_request =
            self.thread_identity_operation_request.saturating_add(1);
        let request_id = self.thread_identity_operation_request;
        self.thread_identity_operation_requests
            .insert(thread_id.to_string(), request_id);
        self.thread_identity_operation_errors.insert(
            thread_id.to_string(),
            ThreadIdentityOperationError {
                source_binding: source_binding.cloned(),
                attempted_binding: attempted_binding.clone(),
                binding_generation,
                request_id,
                inconsistent,
                message: message.into(),
            },
        );
        self.thread_identity_observation_revision =
            self.thread_identity_observation_revision.saturating_add(1);
    }

    fn select_thread_identity(
        &mut self,
        expected_thread_id: &str,
        expected_projection_binding: Option<&omega_workbench_state::RepositoryBinding>,
        expected_binding_generation: u64,
        binding: omega_workbench_state::RepositoryBinding,
        cx: &mut Context<Self>,
    ) {
        if !self.binding_epoch_matches(
            expected_thread_id,
            expected_projection_binding,
            expected_binding_generation,
            true,
            cx,
        ) {
            return;
        }
        if let Some(reason) = self.thread_identity_target_selection_unavailable_reason(cx) {
            self.record_thread_identity_operation_error(
                expected_thread_id,
                expected_projection_binding,
                &binding,
                expected_binding_generation,
                false,
                reason,
            );
            cx.notify();
            return;
        }
        let Some(observation_revision) = self
            .workbench_shell
            .identity()
            .map(|identity| identity.observation_revision)
        else {
            return;
        };
        let reconciling_inconsistent = self
            .workbench_shell
            .identity()
            .is_some_and(|identity| matches!(identity.phase, IdentityPhase::Inconsistent(_)));
        let active_thread_id = self.active_thread_id(cx);
        let worktree_path = self
            .workbench_shell
            .identity()
            .and_then(|identity| {
                identity
                    .candidates
                    .iter()
                    .find(|candidate| candidate.binding == binding)
            })
            .map(|candidate| candidate.worktree_abs_path.clone());
        let persisted_worktree_paths = worktree_path.as_ref().map(|selected_path| {
            let project_paths = self.project.read(cx).worktree_paths(cx);
            project_paths
                .ordered_pairs()
                .find(|(_, folder_path)| folder_path.as_path() == selected_path)
                .map(|(main_path, folder_path)| {
                    let mut paths = WorktreePaths::default();
                    paths.add_path(main_path, folder_path);
                    paths
                })
                .unwrap_or_else(|| {
                    WorktreePaths::from_folder_paths(&PathList::new(std::slice::from_ref(
                        selected_path,
                    )))
                })
        });
        let (Some(worktree_path), Some(persisted_worktree_paths)) =
            (worktree_path, persisted_worktree_paths)
        else {
            self.record_thread_identity_operation_error(
                expected_thread_id,
                expected_projection_binding,
                &binding,
                expected_binding_generation,
                false,
                "The selected worktree path is unavailable",
            );
            cx.notify();
            return;
        };
        let Some(conversation_view) = self.active_conversation_view().cloned() else {
            self.record_thread_identity_operation_error(
                expected_thread_id,
                expected_projection_binding,
                &binding,
                expected_binding_generation,
                false,
                "The active conversation is unavailable",
            );
            cx.notify();
            return;
        };
        let previous_work_dirs = conversation_view.read(cx).work_dirs().clone();
        let work_dirs = PathList::new(&[worktree_path]);
        if let Err(error) = conversation_view.update(cx, |conversation_view, cx| {
            if reconciling_inconsistent {
                conversation_view.reconcile_work_dirs(work_dirs, cx)
            } else {
                conversation_view.retarget_work_dirs(work_dirs, cx)
            }
        }) {
            let inconsistent = error
                .downcast_ref::<crate::conversation_view::InconsistentWorkDirsError>()
                .is_some();
            self.record_thread_identity_operation_error(
                expected_thread_id,
                expected_projection_binding,
                &binding,
                expected_binding_generation,
                inconsistent,
                error.to_string(),
            );
            cx.notify();
            return;
        }

        match self
            .workbench_shell
            .select_identity(observation_revision, &binding)
        {
            Ok(true) => {
                self.thread_identity_operation_requests
                    .remove(expected_thread_id);
                self.thread_identity_operation_errors
                    .remove(expected_thread_id);
                if let Some(thread_id) = active_thread_id {
                    ThreadMetadataStore::global(cx).update(cx, |store, cx| {
                        store.update_worktree_paths(&[thread_id], persisted_worktree_paths, cx);
                    });
                }
                self.thread_identity_observation_revision =
                    self.thread_identity_observation_revision.saturating_add(1);
                cx.notify();
            }
            Ok(false) => {
                if reconciling_inconsistent
                    && let Some(binding) = expected_projection_binding
                    && let Err(error) = self.workbench_shell.refresh_binding_epoch(
                        expected_thread_id,
                        binding,
                        expected_binding_generation,
                        cx,
                    )
                {
                    self.record_thread_identity_operation_error(
                        expected_thread_id,
                        expected_projection_binding,
                        &binding,
                        expected_binding_generation,
                        true,
                        error.to_string(),
                    );
                    cx.notify();
                    return;
                }
                if self
                    .thread_identity_operation_errors
                    .remove(expected_thread_id)
                    .is_some()
                {
                    self.thread_identity_observation_revision =
                        self.thread_identity_observation_revision.saturating_add(1);
                    cx.notify();
                }
            }
            Err(error) => {
                let rollback_result = conversation_view.update(cx, |conversation_view, cx| {
                    conversation_view.retarget_work_dirs(previous_work_dirs, cx)
                });
                if let Err(rollback_error) = rollback_result {
                    self.record_thread_identity_operation_error(
                        expected_thread_id,
                        expected_projection_binding,
                        &binding,
                        expected_binding_generation,
                        true,
                        format!(
                            "{error}; also failed to restore the previous worktree: {rollback_error}"
                        ),
                    );
                    cx.notify();
                    return;
                }
                log::warn!("failed to change thread repository identity: {error:#}");
                self.record_thread_identity_operation_error(
                    expected_thread_id,
                    expected_projection_binding,
                    &binding,
                    expected_binding_generation,
                    false,
                    error.to_string(),
                );
                cx.notify();
            }
        }
    }

    fn focus_thread_transcript(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.workbench_shell.return_to_transcript();
        if let Some(thread_view) = self.active_thread_view(cx) {
            thread_view
                .read(cx)
                .activation_focus_handle(cx)
                .focus(window, cx);
        } else {
            self.focus_handle.focus(window, cx);
        }
        cx.notify();
    }

    fn navigate_to_plan_entry(
        &mut self,
        expected_thread_id: &str,
        entry_index: usize,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some(visible) = self.workbench_shell.projection().visible_projection() else {
            return false;
        };
        if visible.thread_id != expected_thread_id {
            return false;
        }
        let Some(thread) = self.active_agent_thread(cx) else {
            return false;
        };
        if !matches!(
            thread.read(cx).entries().get(entry_index),
            Some(acp_thread::AgentThreadEntry::CompletedPlan(_))
        ) {
            return false;
        }
        let Some(thread_view) = self.active_thread_view(cx) else {
            return false;
        };
        let navigated = thread_view.update(cx, |thread_view, cx| {
            thread_view.scroll_to_entry_index(entry_index, cx)
        });
        #[cfg(any(test, feature = "test-support"))]
        if navigated {
            self.workbench_plan_navigation_target = Some(entry_index);
        }
        navigated
    }

    fn active_thread_for_outline_item(
        &self,
        item: &crate::thread_outline::OutlineItem,
        cx: &App,
    ) -> Option<Entity<AcpThread>> {
        let Some(visible) = self.workbench_shell.projection().visible_projection() else {
            return None;
        };
        if item.outline_binding.thread_id != visible.thread_id
            || item.outline_binding.repository != visible.binding
            || item.outline_binding.generation != visible.generation
        {
            return None;
        }
        if let Some(repository) = item.outline_binding.repository.as_ref() {
            let selected = self.workbench_shell.identity()?.selected.as_ref()?;
            if &selected.binding != repository
                || !item
                    .projection_binding
                    .work_dirs
                    .iter()
                    .any(|work_dir| work_dir == &selected.worktree_abs_path)
            {
                return None;
            }
        }
        let thread = self.active_agent_thread(cx)?;
        (thread.read(cx).projection(cx).binding == item.projection_binding).then_some(thread)
    }

    fn navigate_to_outline_entry(
        &mut self,
        item: &crate::thread_outline::OutlineItem,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some(entry_index) = item.entry_index else {
            return false;
        };
        self.navigate_to_outline_projection_entry(
            item,
            item.entry_id,
            Some(item.entry_revision),
            entry_index,
            window,
            cx,
        )
    }

    fn navigate_to_outline_projection_entry(
        &mut self,
        item: &crate::thread_outline::OutlineItem,
        entry_id: acp_thread::ThreadEntryId,
        entry_revision: Option<u64>,
        entry_index: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some(thread) = self.active_thread_for_outline_item(item, cx) else {
            return false;
        };
        let projection = thread.read(cx).projection(cx);
        if !projection.entries.iter().any(|entry| {
            entry.binding == projection.binding
                && entry.id == entry_id
                && entry_revision.is_none_or(|revision| entry.revision == revision)
                && entry.entry_index == Some(entry_index)
        }) {
            return false;
        }
        let Some(thread_view) = self.active_thread_view(cx) else {
            return false;
        };
        let navigated = thread_view.update(cx, |thread_view, cx| {
            thread_view.scroll_to_entry_index(entry_index, cx)
        });
        if navigated {
            #[cfg(any(test, feature = "test-support"))]
            {
                self.thread_outline_navigation_target = Some((entry_id, entry_index));
            }
            self.focus_thread_transcript(window, cx);
        }
        navigated
    }

    fn activate_outline_artifact(
        &mut self,
        item: crate::thread_outline::OutlineItem,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> OutlineActionOutcome {
        let crate::thread_outline::OutlineItemId::Artifact(artifact_id) = &item.id else {
            return OutlineActionOutcome::Unavailable("Select an artifact first".into());
        };
        let artifact_id = *artifact_id;
        let Some(thread) = self.active_thread_for_outline_item(&item, cx) else {
            return OutlineActionOutcome::Unavailable(
                "The artifact no longer belongs to the active thread or worktree".into(),
            );
        };
        let projection = thread.read(cx).projection(cx);
        if projection.binding != item.projection_binding {
            return OutlineActionOutcome::Unavailable(
                "The artifact belongs to a different thread or worktree".into(),
            );
        }
        let current_source_is_exact = projection.entries.iter().any(|entry| {
            entry.binding == projection.binding
                && entry.id == item.entry_id
                && entry.revision == item.entry_revision
                && entry.entry_index == item.entry_index
        });
        let current_artifact_is_exact = projection.artifacts.iter().any(|artifact| {
            artifact.binding == projection.binding
                && artifact.id == artifact_id
                && Some(artifact.revision) == item.artifact_revision
                && artifact.source_events == item.artifact_source_events
                && artifact.source_events.contains(&item.entry_id)
                && artifact.action_target == item.action
        });
        if !current_source_is_exact || !current_artifact_is_exact {
            return OutlineActionOutcome::Unavailable(
                "The artifact changed before it could be opened".into(),
            );
        }
        let primary_outcome = item.action.as_ref().map_or_else(
            || OutlineActionOutcome::Unavailable("No native artifact action is available".into()),
            |target| self.try_activate_outline_target(target, &item, window, cx),
        );
        if primary_outcome.succeeded() {
            return primary_outcome;
        }
        if self.navigate_to_outline_entry(&item, window, cx) {
            OutlineActionOutcome::SourceFallback
        } else {
            primary_outcome
        }
    }

    fn navigate_to_outline_artifact_source(
        &mut self,
        item: crate::thread_outline::OutlineItem,
        source_event: acp_thread::ThreadEntryId,
        entry_index: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let crate::thread_outline::OutlineItemId::Artifact(artifact_id) = &item.id else {
            return false;
        };
        let artifact_id = *artifact_id;
        let Some(thread) = self.active_thread_for_outline_item(&item, cx) else {
            return false;
        };
        let projection = thread.read(cx).projection(cx);
        if projection.binding != item.projection_binding {
            return false;
        }
        let source_is_exact = projection.entries.iter().any(|entry| {
            entry.binding == projection.binding
                && entry.id == source_event
                && entry.entry_index == Some(entry_index)
        });
        let artifact_is_exact = projection.artifacts.iter().any(|artifact| {
            artifact.binding == projection.binding
                && artifact.id == artifact_id
                && Some(artifact.revision) == item.artifact_revision
                && artifact.source_events == item.artifact_source_events
                && artifact.source_events.contains(&source_event)
                && artifact.action_target == item.action
        });
        if !source_is_exact || !artifact_is_exact {
            return false;
        }
        self.navigate_to_outline_projection_entry(
            &item,
            source_event,
            None,
            entry_index,
            window,
            cx,
        )
    }

    fn try_activate_outline_target(
        &mut self,
        target: &acp_thread::ThreadActionTarget,
        item: &crate::thread_outline::OutlineItem,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> OutlineActionOutcome {
        match target {
            acp_thread::ThreadActionTarget::Entry(entry_id) => {
                let Some(thread) = self.active_agent_thread(cx) else {
                    return OutlineActionOutcome::Unavailable(
                        "The active thread is unavailable".into(),
                    );
                };
                let projection = thread.read(cx).projection(cx);
                let Some(entry) = projection
                    .entries
                    .iter()
                    .find(|entry| entry.binding == projection.binding && entry.id == *entry_id)
                else {
                    return OutlineActionOutcome::Unavailable(
                        "The source event is unavailable".into(),
                    );
                };
                if entry.entry_index.is_some_and(|entry_index| {
                    self.navigate_to_outline_projection_entry(
                        item,
                        entry.id,
                        Some(entry.revision),
                        entry_index,
                        window,
                        cx,
                    )
                }) {
                    OutlineActionOutcome::Completed
                } else {
                    OutlineActionOutcome::Unavailable("The source event cannot be navigated".into())
                }
            }
            acp_thread::ThreadActionTarget::File { path, line } => {
                let Some(worktree_id) = self.active_workbench_worktree_id(cx) else {
                    return OutlineActionOutcome::Unavailable(
                        "The active worktree is unavailable".into(),
                    );
                };
                let Some(worktree_abs_path) = self.active_workbench_worktree_abs_path() else {
                    return OutlineActionOutcome::Unavailable(
                        "The active worktree path is unavailable".into(),
                    );
                };
                let relative_path = if path.is_absolute() {
                    let Ok(path) = path.strip_prefix(&worktree_abs_path) else {
                        return OutlineActionOutcome::Unavailable(
                            "The file is outside the artifact's active worktree".into(),
                        );
                    };
                    path
                } else {
                    path.as_path()
                };
                let Ok(relative_path) = RelPath::new(relative_path, PathStyle::local()) else {
                    return OutlineActionOutcome::Unavailable(
                        "The artifact path is invalid".into(),
                    );
                };
                let project_path = ProjectPath {
                    worktree_id,
                    path: relative_path.into_owned().into(),
                };
                let Some(workspace) = self.workspace.upgrade() else {
                    return OutlineActionOutcome::Unavailable(
                        "The workspace is unavailable".into(),
                    );
                };
                let expected_binding = item.outline_binding.clone();
                let crate::thread_outline::OutlineItemId::Artifact(artifact_id) = &item.id else {
                    return OutlineActionOutcome::Unavailable("Select an artifact first".into());
                };
                let artifact_id = *artifact_id;
                let Some(artifact_revision) = item.artifact_revision else {
                    return OutlineActionOutcome::Unavailable(
                        "The artifact revision is unavailable".into(),
                    );
                };
                let open_task = workspace.update(cx, |workspace, cx| {
                    workspace.open_path(project_path, None, true, window, cx)
                });
                let outline = self.thread_outline.downgrade();
                let line = *line;
                window
                    .spawn(cx, async move |cx| {
                        let result = async {
                            let opened_item = open_task.await?;
                            if let Some(line) = line
                                && let Some(editor) = opened_item.downcast::<Editor>()
                            {
                                editor.update_in(cx, |editor, window, cx| {
                                    let snapshot = editor.buffer().read(cx).snapshot(cx);
                                    let point =
                                        snapshot.clip_point(Point::new(line, 0), text::Bias::Left);
                                    editor.change_selections(
                                        Default::default(),
                                        window,
                                        cx,
                                        |selections| {
                                            selections.select_ranges([point..point]);
                                        },
                                    );
                                })?;
                            }
                            anyhow::Ok(())
                        }
                        .await;
                        outline.update(cx, |outline, cx| {
                            outline.report_artifact_action_result(
                                &expected_binding,
                                artifact_id,
                                artifact_revision,
                                result.map_err(|error| {
                                    SharedString::from(format!(
                                        "Failed to open artifact: {error:#}"
                                    ))
                                }),
                                cx,
                            );
                        })?;
                        anyhow::Ok(())
                    })
                    .detach_and_log_err(cx);
                OutlineActionOutcome::Pending
            }
            acp_thread::ThreadActionTarget::Uri(uri) => {
                let Ok(url) = url::Url::parse(&uri) else {
                    return OutlineActionOutcome::Unavailable("The artifact URL is invalid".into());
                };
                if !matches!(url.scheme(), "https" | "http") {
                    return OutlineActionOutcome::Unavailable(
                        "Only HTTP and HTTPS artifact links can be opened".into(),
                    );
                }
                cx.open_url(url.as_str());
                OutlineActionOutcome::Completed
            }
            acp_thread::ThreadActionTarget::ToolCall(_) => OutlineActionOutcome::Unavailable(
                "No native tool-call surface is available; opening the source event instead".into(),
            ),
            acp_thread::ThreadActionTarget::Terminal(_) => OutlineActionOutcome::Unavailable(
                "No retained native terminal is available; opening the source event instead".into(),
            ),
        }
    }

    fn select_work_surface(
        &mut self,
        surface: omega_workbench_state::WorkSurface,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let (files_panel, previous_scope, previous_scope_was_unavailable) =
            if surface == omega_workbench_state::WorkSurface::Files {
                match self.prepare_files_surface(window, cx) {
                    Ok((panel, previous_scope, previous_scope_was_unavailable)) => {
                        (Some(panel), previous_scope, previous_scope_was_unavailable)
                    }
                    Err(error) => {
                        log::warn!("could not prepare the Files work surface: {error:#}");
                        self.workbench_shell.record_error(error.to_string());
                        cx.notify();
                        return;
                    }
                }
            } else {
                (None, None, false)
            };
        let search_surface = if surface == omega_workbench_state::WorkSurface::Search {
            match self.prepare_search_surface(window, cx) {
                Ok(search_surface) => Some(search_surface),
                Err(error) => {
                    log::warn!("could not prepare the Search work surface: {error:#}");
                    self.workbench_shell.record_error(error.to_string());
                    cx.notify();
                    return;
                }
            }
        } else {
            None
        };
        let review_surface = if surface == omega_workbench_state::WorkSurface::Review {
            match self.prepare_review_surface(window, cx) {
                Ok(review_surface) => Some(review_surface),
                Err(error) => {
                    log::warn!("could not prepare the Review work surface: {error:#}");
                    self.workbench_shell.record_error(error.to_string());
                    cx.notify();
                    return;
                }
            }
        } else {
            None
        };
        let previous_git_scope = self
            .workbench_git_panel
            .as_ref()
            .and_then(|panel| panel.read(cx).repository_scope());
        let git_surface = if surface == omega_workbench_state::WorkSurface::Git {
            match self.prepare_git_surface(window, cx) {
                Ok(git_surface) => Some(git_surface),
                Err(error) => {
                    log::warn!("could not prepare the Git work surface: {error:#}");
                    self.workbench_shell.record_error(error.to_string());
                    cx.notify();
                    return;
                }
            }
        } else {
            None
        };
        let terminal_surface = if surface == omega_workbench_state::WorkSurface::Terminal {
            match self.prepare_terminal_surface(window, cx) {
                Ok(terminal_surface) => Some(terminal_surface),
                Err(error) => {
                    log::warn!("could not prepare the Terminal work surface: {error:#}");
                    self.workbench_shell.record_error(error.to_string());
                    cx.notify();
                    return;
                }
            }
        } else {
            None
        };
        let plan_surface = if surface == omega_workbench_state::WorkSurface::Plan {
            match self.prepare_plan_surface(window, cx) {
                Ok(plan_surface) => Some(plan_surface),
                Err(error) => {
                    log::warn!("could not prepare the Plan work surface: {error:#}");
                    self.workbench_shell.record_error(error.to_string());
                    cx.notify();
                    return;
                }
            }
        } else {
            None
        };
        let selection = if let Some(git_surface) = git_surface.clone() {
            self.workbench_shell.select_git_surface(git_surface, cx)
        } else if let Some(terminal_surface) = terminal_surface.clone() {
            self.workbench_shell
                .select_terminal_surface(terminal_surface, cx)
        } else if let Some(plan_surface) = plan_surface {
            self.workbench_shell.select_plan_surface(plan_surface, cx)
        } else {
            self.workbench_shell.select_surface(
                surface,
                files_panel.clone(),
                search_surface,
                review_surface,
                None,
                cx,
            )
        };
        match selection {
            Ok(workbench_shell::SurfaceSelection::Collapsed) => {
                self.persist_active_workbench_selection(cx);
                self.focus_thread_transcript(window, cx);
            }
            Ok(workbench_shell::SurfaceSelection::Opened(host)) => {
                self.persist_active_workbench_selection(cx);
                if let Some(panel) = files_panel.as_ref()
                    && let Err(error) = self.detach_workspace_files_panel(panel, window, cx)
                {
                    panel.update(cx, |panel, cx| {
                        if previous_scope_was_unavailable {
                            panel.set_worktree_scope_unavailable(previous_scope, window, cx);
                        } else {
                            panel.set_worktree_scope(previous_scope, window, cx);
                        }
                    });
                    if let Err(collapse_error) = self.workbench_shell.collapse_dock() {
                        log::warn!(
                            "failed to collapse Files after workspace handoff failed: \
                             {collapse_error:#}"
                        );
                    }
                    log::warn!(
                        "could not hand the native Files panel to the work surface: {error:#}"
                    );
                    self.workbench_shell.record_error(error.to_string());
                    cx.notify();
                    return;
                }
                let git_panel = git_surface
                    .as_ref()
                    .map(|git_surface| git_surface.read(cx).git_panel().clone());
                if let Some(git_panel) = git_panel.as_ref()
                    && !self.workbench_git_panel_handed_off
                    && let Err(error) = self.detach_workspace_git_panel(git_panel, window, cx)
                {
                    if let Some(panel) = self.workbench_git_panel.as_ref() {
                        panel
                            .update(cx, |panel, cx| {
                                panel.set_repository_scope(previous_git_scope, window, cx)
                            })
                            .log_err();
                    }
                    if let Err(collapse_error) = self.workbench_shell.collapse_dock() {
                        log::warn!(
                            "failed to collapse Git after workspace handoff failed: \
                             {collapse_error:#}"
                        );
                    }
                    log::warn!(
                        "could not hand the native Git panel to the work surface: {error:#}"
                    );
                    self.workbench_shell.record_error(error.to_string());
                    cx.notify();
                    return;
                }
                let terminal_panel = terminal_surface
                    .as_ref()
                    .map(|terminal_surface| terminal_surface.read(cx).terminal_panel().clone());
                if let Some(terminal_panel) = terminal_panel.as_ref()
                    && !self.workbench_terminal_panel_handed_off
                    && let Err(error) =
                        self.detach_workspace_terminal_panel(terminal_panel, window, cx)
                {
                    if let Err(collapse_error) = self.workbench_shell.collapse_dock() {
                        log::warn!(
                            "failed to collapse Terminal after workspace handoff failed: \
                             {collapse_error:#}"
                        );
                    }
                    log::warn!(
                        "could not hand the native Terminal panel to the work surface: {error:#}"
                    );
                    self.workbench_shell.record_error(error.to_string());
                    cx.notify();
                    return;
                }
                if let Some(terminal_surface) = terminal_surface.as_ref() {
                    terminal_surface.update(cx, |_terminal_surface, cx| cx.notify());
                    host.update(cx, |_host, cx| cx.notify());
                }
                host.focus_handle(cx).focus(window, cx);
                cx.notify();
            }
            Err(error) => {
                if let Some(panel) = files_panel {
                    panel.update(cx, |panel, cx| {
                        if previous_scope_was_unavailable {
                            panel.set_worktree_scope_unavailable(previous_scope, window, cx);
                        } else {
                            panel.set_worktree_scope(previous_scope, window, cx);
                        }
                    });
                }
                if git_surface.is_some()
                    && let Some(panel) = self.workbench_git_panel.as_ref()
                {
                    panel
                        .update(cx, |panel, cx| {
                            panel.set_repository_scope(previous_git_scope, window, cx)
                        })
                        .log_err();
                }
                log::warn!(
                    "could not select the {} work surface: {error:#}",
                    workbench_shell::WorkSurfaceExt::label(surface)
                );
                self.workbench_shell.record_error(error.to_string());
                cx.notify();
            }
        }
    }

    fn focus_activity_rail(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let surface = self.workbench_shell.focus_rail();
        if let Some(focus_handle) = self.workbench_shell.rail_focus_handle(surface).cloned() {
            focus_handle.focus(window, cx);
        }
        cx.notify();
    }

    fn move_activity_rail_focus(
        &mut self,
        movement: workbench_shell::RailFocusMovement,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let surface = self.workbench_shell.move_rail_focus(movement);
        if let Some(focus_handle) = self.workbench_shell.rail_focus_handle(surface).cloned() {
            focus_handle.focus(window, cx);
        }
        cx.notify();
    }

    fn activate_focused_work_surface(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let surface = self.workbench_shell.focused_rail_surface();
        self.select_work_surface(surface, window, cx);
    }

    fn collapse_work_surface_dock(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        match self.workbench_shell.collapse_dock() {
            Ok(true) => self.focus_thread_transcript(window, cx),
            Ok(false) => {}
            Err(error) => {
                log::warn!("failed to collapse work-surface dock: {error:#}");
                self.workbench_shell.record_error(error.to_string());
                cx.notify();
            }
        }
    }

    fn ensure_terminal_work_surface_open(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let terminal_is_open = self
            .workbench_shell
            .projection()
            .visible_projection()
            .is_some_and(|visible| {
                visible.dock_open
                    && visible.effective_surface
                        == Some(omega_workbench_state::WorkSurface::Terminal)
            });
        if !terminal_is_open {
            self.select_work_surface(omega_workbench_state::WorkSurface::Terminal, window, cx);
        }
        self.workbench_shell
            .projection()
            .visible_projection()
            .is_some_and(|visible| {
                visible.dock_open
                    && visible.effective_surface
                        == Some(omega_workbench_state::WorkSurface::Terminal)
            })
    }

    fn create_terminal_for_active_thread_at(
        &mut self,
        requested_working_directory: Option<PathBuf>,
        split_direction: Option<SplitDirection>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.workbench_terminal_has_authority(cx) {
            self.workbench_shell
                .record_error("The active thread worktree is unavailable");
            cx.notify();
            return;
        }
        let Some(surface) = self.workbench_terminal_surface.clone() else {
            self.workbench_shell
                .record_error("Open the Terminal work surface before creating a terminal");
            cx.notify();
            return;
        };
        let owner = surface.read(cx).binding().clone();
        let requested_working_directory =
            requested_working_directory.unwrap_or_else(|| owner.worktree_abs_path.clone());
        let working_directory = match util::paths::normalize_lexically(&requested_working_directory)
        {
            Ok(working_directory) => working_directory,
            Err(error) => {
                self.workbench_shell.record_error(format!(
                    "Terminal directory {} is invalid: {error}",
                    requested_working_directory.display()
                ));
                cx.notify();
                return;
            }
        };
        if !working_directory.starts_with(&owner.worktree_abs_path) {
            self.workbench_shell.record_error(format!(
                "Terminal directory {} is outside the active thread worktree {}",
                working_directory.display(),
                owner.worktree_abs_path.display()
            ));
            cx.notify();
            return;
        }
        if !surface.read(cx).owner_state().can_create() {
            self.workbench_shell
                .record_error("New terminals are unavailable for this worktree");
            cx.notify();
            return;
        }
        let panel = surface.read(cx).terminal_panel().clone();
        let spawn = panel.update(cx, |panel, cx| {
            panel.create_terminal_at_working_directory(Some(working_directory), window, cx)
        });
        cx.spawn_in(window, async move |this, cx| {
            match spawn.await {
                Ok(terminal) => {
                    let terminal = terminal
                        .upgrade()
                        .context("the created terminal closed before it could be registered")?;
                    let terminal_id = terminal.entity_id().as_u64();
                    this.update_in(cx, |this, window, cx| {
                        let owner_is_current = this.workbench_terminal_has_authority(cx)
                            && this
                                .workbench_terminal_surface
                                .as_ref()
                                .is_some_and(|surface| {
                                    let surface = surface.read(cx);
                                    surface.binding() == &owner
                                        && surface.owner_state().can_create()
                                });
                        if !owner_is_current {
                            panel.update(cx, |panel, cx| {
                                panel.remove_terminal(terminal_id, window, cx);
                            });
                            this.workbench_shell.record_error(
                                "Ignored a terminal created for a stale thread/worktree binding",
                            );
                            cx.notify();
                            return;
                        }
                        if split_direction.is_some_and(|direction| {
                            !panel.update(cx, |panel, cx| {
                                panel.move_terminal_to_split(terminal_id, direction, window, cx)
                            })
                        }) {
                            panel.update(cx, |panel, cx| {
                                panel.remove_terminal(terminal_id, window, cx);
                            });
                            this.workbench_shell.record_error(
                                "Could not place the new terminal in the requested split",
                            );
                            cx.notify();
                            return;
                        }
                        if let Some(surface) = this.workbench_terminal_surface.as_ref() {
                            surface.update(cx, |surface, cx| {
                                surface.record_terminal_owner(terminal_id, owner, cx);
                            });
                        }
                        cx.notify();
                    })?;
                }
                Err(error) => {
                    this.update(cx, |this, cx| {
                        this.workbench_shell.record_error(format!(
                            "Could not create terminal in the thread worktree: {error:#}"
                        ));
                        cx.notify();
                    })?;
                }
            }
            Ok::<(), anyhow::Error>(())
        })
        .detach_and_log_err(cx);
    }

    fn create_terminal_for_active_thread(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.create_terminal_for_active_thread_at(None, None, window, cx);
    }

    fn create_terminal_split_for_active_thread(
        &mut self,
        direction: SplitDirection,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.create_terminal_for_active_thread_at(None, Some(direction), window, cx);
    }

    fn activate_next_terminal_tab(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(panel) = self.workbench_terminal_panel.as_ref() {
            panel.update(cx, |panel, cx| {
                panel.activate_next_terminal_tab(window, cx);
            });
        }
    }

    fn activate_previous_terminal_tab(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(panel) = self.workbench_terminal_panel.as_ref() {
            panel.update(cx, |panel, cx| {
                panel.activate_previous_terminal_tab(window, cx);
            });
        }
    }

    fn close_active_terminal_tab(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(panel) = self.workbench_terminal_panel.as_ref() {
            panel.update(cx, |panel, cx| {
                panel.close_active_terminal_tab(window, cx);
            });
        }
    }

    fn render_surface_badge(
        surface: omega_workbench_state::WorkSurface,
        badge: &workbench_shell::SurfaceBadge,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let selector = format!(
            "omega.workbench.badge.{}",
            workbench_shell::WorkSurfaceExt::label(surface).to_ascii_lowercase()
        );
        let (tone, label, text) = match badge {
            workbench_shell::SurfaceBadge::Count { count, tone, label } => {
                (*tone, label.clone(), Some((*count).min(99).to_string()))
            }
            workbench_shell::SurfaceBadge::Attention { tone, label } => {
                (*tone, label.clone(), None)
            }
        };
        let background = match tone {
            workbench_shell::BadgeTone::Neutral => cx.theme().colors().element_selected,
            workbench_shell::BadgeTone::Accent => cx.theme().colors().icon_accent,
            workbench_shell::BadgeTone::Warning => cx.theme().status().warning,
            workbench_shell::BadgeTone::Error => cx.theme().status().error,
        };
        div()
            .id(SharedString::from(selector.clone()))
            .debug_selector(move || selector)
            .absolute()
            .top_0()
            .right_0()
            .min_w_2()
            .h_2()
            .px_px()
            .rounded_full()
            .flex()
            .items_center()
            .justify_center()
            .bg(background)
            .role(gpui::Role::Status)
            .aria_label(label)
            .when_some(text, |this, text| {
                this.min_w_3p5().h_3p5().text_xs().child(text)
            })
            .into_any_element()
    }

    fn render_activity_rail(&self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        use omega_workbench_state::WorkSurface;
        use workbench_shell::WorkSurfaceExt as _;

        let visible = self.workbench_shell.projection().visible_projection();
        let active_surface = visible
            .as_ref()
            .filter(|visible| visible.dock_open)
            .and_then(|visible| visible.effective_surface);
        let offline = self.workbench_shell.projection().connection
            != omega_workbench_state::ConnectionPhase::Online;

        let items = WorkSurface::FALLBACK_ORDER
            .into_iter()
            .map(|surface| {
                let capability = self.workbench_shell.capability(surface);
                let connection_unavailable = offline
                    && !matches!(
                        surface,
                        omega_workbench_state::WorkSurface::Terminal
                            | omega_workbench_state::WorkSurface::Plan
                    );
                let available = capability.is_some_and(|capability| {
                    capability.availability.is_available() && !connection_unavailable
                });
                let unavailable_reason = if connection_unavailable {
                    Some(SharedString::from("Work surfaces are unavailable offline"))
                } else {
                    capability
                        .and_then(|capability| capability.availability.reason())
                        .cloned()
                };
                let focus_handle = self.workbench_shell.rail_focus_handle(surface).cloned();
                let action = workbench_shell::select_action(surface);
                let key_binding = focus_handle.as_ref().map(|focus_handle| {
                    KeyBinding::for_action_in(action.as_ref(), focus_handle, cx)
                });
                let shortcut = key_binding
                    .as_ref()
                    .and_then(|key_binding| key_binding.keyboard_shortcut_text(window, cx));
                let label = surface.label();
                let active = active_surface == Some(surface);
                let tab_index = if self.workbench_shell.focused_rail_surface() == surface {
                    0isize
                } else {
                    -1isize
                };

                let mut button = IconButton::new(surface.rail_element_id(), surface.icon())
                    .debug_selector(move || surface.rail_element_id().into())
                    .shape(ui::IconButtonShape::Wide)
                    .width(px(28.))
                    .size(ButtonSize::Medium)
                    .icon_size(IconSize::Small)
                    .style(ButtonStyle::Subtle)
                    .selected_style(ButtonStyle::Tinted(ui::TintColor::Accent))
                    .toggle_state(active)
                    .aria_label(label)
                    .aria_expanded(active)
                    .disabled(!available)
                    .tab_index(tab_index);
                if let Some(focus_handle) = focus_handle.as_ref() {
                    button = button.track_focus(focus_handle);
                }
                if let Some(shortcut) = shortcut {
                    button = button.aria_keyshortcuts(shortcut);
                }
                if let Some(reason) = unavailable_reason {
                    button = button
                        .aria_description(reason.clone())
                        .tooltip(move |_, cx| Tooltip::with_meta(label, None, reason.clone(), cx));
                } else if let Some(focus_handle) = focus_handle.as_ref() {
                    button = button.tooltip(Tooltip::for_action_title_in(
                        label,
                        action.as_ref(),
                        focus_handle,
                    ));
                }
                let action = workbench_shell::select_action(surface);
                button = button.on_click(move |_, window, cx| {
                    window.dispatch_action(action.boxed_clone(), cx);
                });

                let badge = capability.and_then(|capability| capability.badge.as_ref());
                div()
                    .relative()
                    .size_8()
                    .flex()
                    .items_center()
                    .justify_center()
                    .child(button)
                    .children(badge.map(|badge| Self::render_surface_badge(surface, badge, cx)))
            })
            .collect::<Vec<_>>();

        v_flex()
            .id("omega.workbench.activity-rail")
            .debug_selector(|| "omega.workbench.activity-rail".into())
            .key_context("WorkbenchRail")
            .w(workbench_shell::ACTIVITY_RAIL_WIDTH)
            .h_full()
            .flex_shrink_0()
            .items_center()
            .gap_1()
            .pt_2()
            .bg(cx.theme().colors().panel_background)
            .border_r_1()
            .border_color(cx.theme().colors().border)
            .role(gpui::Role::Toolbar)
            .aria_label("Work surfaces")
            .aria_orientation(gpui::accesskit::Orientation::Vertical)
            .children(items)
            .child(div().flex_1())
            .children(self.workbench_shell.last_error().map(|error| {
                div()
                    .id("omega-workbench-rail-error")
                    .debug_selector(|| "omega-workbench-rail-error".into())
                    .mb_2()
                    .role(gpui::Role::Status)
                    .aria_label(error.clone())
                    .tooltip(Tooltip::text(error.clone()))
                    .child(
                        Icon::new(IconName::Warning)
                            .size(IconSize::Small)
                            .color(Color::Warning),
                    )
            }))
            .into_any_element()
    }

    fn render_work_surface_dock(
        &mut self,
        layout: workbench_shell::WorkbenchLayout,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        use workbench_shell::WorkSurfaceExt as _;

        if !layout.dock_visible {
            return None;
        }
        let visible = self.workbench_shell.projection().visible_projection()?;
        if !visible.dock_open {
            return None;
        }
        let surface = visible.effective_surface?;
        let host = self.workbench_shell.visible_host()?.clone();
        let resize_drag = workbench_shell::WorkbenchDockResizeDrag::new(layout.dock_width);
        let maximum_dock_width = workbench_shell::WorkbenchLayout::clamp_dock_width(
            window.viewport_size().width,
            workbench_shell::MAX_DOCK_WIDTH,
        )
        .unwrap_or(layout.dock_width);
        Some(
            v_flex()
                .id("omega-workbench-dock")
                .debug_selector(|| "omega.workbench.dock".into())
                .relative()
                .w(layout.dock_width)
                .h_full()
                .flex_shrink_0()
                .overflow_hidden()
                .bg(cx.theme().colors().panel_background)
                .border_r_1()
                .border_color(cx.theme().colors().border)
                .role(gpui::Role::Complementary)
                .aria_label(format!("{} work surface", surface.label()))
                .child(
                    h_flex()
                        .h(Tab::container_height(cx))
                        .w_full()
                        .flex_shrink_0()
                        .px_2()
                        .justify_between()
                        .border_b_1()
                        .border_color(cx.theme().colors().border)
                        .child(Label::new(surface.label()).size(LabelSize::Small))
                        .child(
                            IconButton::new(
                                "omega.workbench.control.dock.collapse",
                                IconName::Close,
                            )
                            .debug_selector(|| "omega.workbench.control.dock.collapse".into())
                            .icon_size(IconSize::Small)
                            .tab_index(0isize)
                            .aria_label("Collapse work surface")
                            .tooltip(|_, cx| {
                                Tooltip::for_action(
                                    "Collapse work surface",
                                    &workbench_shell::CollapseWorkSurfaceDock,
                                    cx,
                                )
                            })
                            .on_click(|_, window, cx| {
                                window.dispatch_action(
                                    workbench_shell::CollapseWorkSurfaceDock.boxed_clone(),
                                    cx,
                                );
                            }),
                        ),
                )
                .child(v_flex().flex_1().min_h_0().child(host))
                .child(
                    div()
                        .id("omega.workbench.control.dock.resize")
                        .debug_selector(|| "omega.workbench.control.dock.resize".into())
                        .absolute()
                        .right(px(0.))
                        .top(px(0.))
                        .h_full()
                        .w(workbench_shell::RESIZE_HANDLE_WIDTH)
                        .cursor_col_resize()
                        .block_mouse_except_scroll()
                        .role(gpui::Role::Splitter)
                        .aria_label("Resize work surface")
                        .aria_orientation(gpui::accesskit::Orientation::Vertical)
                        .aria_numeric_value(layout.dock_width.as_f32() as f64)
                        .aria_min_numeric_value(workbench_shell::MIN_DOCK_WIDTH.as_f32() as f64)
                        .aria_max_numeric_value(maximum_dock_width.as_f32() as f64)
                        .tooltip(Tooltip::text("Drag to resize; double-click to reset"))
                        .on_drag(resize_drag, |drag, _, window, cx| {
                            drag.begin(window.mouse_position().x);
                            cx.new(|_| gpui::Empty)
                        })
                        .on_drag_move::<workbench_shell::WorkbenchDockResizeDrag>(cx.listener(
                            |this,
                             event: &gpui::DragMoveEvent<
                                workbench_shell::WorkbenchDockResizeDrag,
                            >,
                             window,
                             cx| {
                                let requested_width =
                                    event.drag(cx).requested_width(event.event.position.x);
                                if this
                                    .workbench_shell
                                    .resize_dock(requested_width, window.viewport_size().width)
                                {
                                    cx.notify();
                                }
                            },
                        ))
                        .on_click(cx.listener(|this, event: &gpui::ClickEvent, window, cx| {
                            if event.click_count() >= 2
                                && this.workbench_shell.resize_dock(
                                    workbench_shell::DEFAULT_DOCK_WIDTH,
                                    window.viewport_size().width,
                                )
                            {
                                cx.notify();
                            }
                            cx.stop_propagation();
                        })),
                )
                .into_any_element(),
        )
    }
}

impl Render for AgentPanel {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.sync_workbench_shell(window, cx);

        // WARNING: Changes to this element hierarchy can have
        // non-obvious implications to the layout of children.
        //
        // If you need to change it, please confirm:
        // - The message editor expands (cmd-option-esc) correctly
        // - When expanded, the buttons at the bottom of the panel are displayed correctly
        // - Font size works as expected and can be changed with cmd-+/cmd-
        // - Scrolling in all views works as expected
        // - Files can be dropped into the panel
        let content = v_flex()
            .key_context(self.key_context())
            .relative()
            .size_full()
            .justify_between()
            .track_focus(&self.focus_handle)
            .bg(cx.theme().colors().panel_background)
            .on_action(cx.listener(|this, action: &NewThread, window, cx| {
                this.new_thread(action, window, cx);
            }))
            .on_action(cx.listener(|this, _: &NewTerminalThread, window, cx| {
                cx.stop_propagation();
                this.new_terminal(None, AgentThreadSource::AgentPanel, window, cx);
            }))
            .on_action(cx.listener(|this, _: &ToggleThreadsSidebar, _window, cx| {
                this.toggle_threads_sidebar(cx);
            }))
            .on_action(cx.listener(
                |this, _: &workbench_shell::ToggleRepositoryPicker, window, cx| {
                    if this
                        .thread_identity_target_selection_unavailable_reason(cx)
                        .is_none()
                    {
                        this.thread_repository_menu_handle.toggle(window, cx);
                    }
                },
            ))
            .on_action(cx.listener(
                |this, _: &workbench_shell::ToggleWorktreePicker, window, cx| {
                    if this
                        .thread_identity_target_selection_unavailable_reason(cx)
                        .is_none()
                    {
                        this.thread_worktree_menu_handle.toggle(window, cx);
                    }
                },
            ))
            .on_action(cx.listener(
                |this, _: &workbench_shell::ToggleBranchPicker, window, cx| {
                    if this
                        .thread_identity_branch_selection_unavailable_reason(cx)
                        .is_none()
                    {
                        this.thread_branch_menu_handle.toggle(window, cx);
                    }
                },
            ))
            .on_action(cx.listener(Self::open_active_thread_as_markdown))
            .on_action(cx.listener(Self::manage_skills))
            .on_action(cx.listener(Self::toggle_options_menu))
            .on_action(cx.listener(Self::increase_font_size))
            .on_action(cx.listener(Self::decrease_font_size))
            .on_action(cx.listener(Self::reset_font_size))
            .on_action(cx.listener(Self::toggle_zoom))
            .on_action(cx.listener(Self::toggle_terminal_thread_search))
            .on_action(cx.listener(|this, _: &ReauthenticateAgent, window, cx| {
                if let Some(conversation_view) = this.active_conversation_view() {
                    conversation_view.update(cx, |conversation_view, cx| {
                        conversation_view.reauthenticate(window, cx)
                    })
                }
            }))
            .on_action(cx.listener(|this, _: &LogoutAgent, window, cx| {
                if let Some(conversation_view) = this.active_conversation_view() {
                    conversation_view.update(cx, |conversation_view, cx| {
                        conversation_view.logout(window, cx)
                    })
                }
            }))
            .when(
                self.public_channels.selected_channel().is_none(),
                |parent| parent.child(self.render_toolbar(window, cx)),
            )
            .when(
                self.public_channels.selected_channel().is_none(),
                |parent| parent.children(self.render_new_user_onboarding(window, cx)),
            )
            .map(|parent| {
                if self.public_channels.selected_channel().is_some() {
                    return parent.child(self.render_selected_public_channel(cx));
                }
                // Full Auto is a surface of this panel, not a destination
                // beside it. `OMEGA-DELTA-0020`.
                if self.showing_full_auto
                    && let Some(full_auto) = self.full_auto.clone()
                {
                    return parent.child(full_auto);
                }
                match self.visible_surface() {
                    VisibleSurface::Uninitialized if !self.has_open_project(cx) => {
                        parent.child(self.render_no_project_state(cx))
                    }
                    VisibleSurface::Uninitialized => parent,
                    VisibleSurface::AgentThread(conversation_view) => parent
                        .child(
                            h_flex()
                                .size_full()
                                .items_stretch()
                                .child(
                                    v_flex()
                                        .id("omega-workbench-transcript")
                                        .debug_selector(|| "omega.workbench.transcript".into())
                                        .flex_1()
                                        .min_w_0()
                                        .min_h_0()
                                        .child(conversation_view.clone()),
                                )
                                .when(self.workbench_shell_enabled, |this| {
                                    this.child(self.thread_outline.clone())
                                }),
                        )
                        .child(self.render_drag_target(cx)),
                    VisibleSurface::Terminal(terminal_view) => {
                        let search_bar = self
                            .active_terminal_id()
                            .and_then(|terminal_id| self.terminals.get(&terminal_id))
                            .and_then(|terminal| terminal.search_bar.clone());
                        let terminal_content = v_flex()
                            .size_full()
                            .when_some(search_bar, |this, search_bar| {
                                this.when(!search_bar.read(cx).is_dismissed(), |this| {
                                    this.child(
                                        v_flex()
                                            .group("toolbar")
                                            .relative()
                                            .py(DynamicSpacing::Base06.rems(cx))
                                            .px(DynamicSpacing::Base08.rems(cx))
                                            .border_b_1()
                                            .border_color(cx.theme().colors().border_variant)
                                            .bg(cx.theme().colors().toolbar_background)
                                            .child(search_bar),
                                    )
                                })
                            })
                            .child(terminal_view.clone());

                        parent
                            .child(terminal_content)
                            .child(self.render_drag_target(cx))
                    }
                }
            });

        let content = if self.workbench_shell_enabled {
            let mut layout = workbench_shell::WorkbenchLayout::allocate(
                window.viewport_size().width,
                self.sidebar.open,
                self.workbench_shell
                    .projection()
                    .visible_projection()
                    .is_some_and(|visible| visible.dock_open),
                self.workbench_shell.dock_width(),
            );
            if !layout.dock_visible {
                match self.workbench_shell.collapse_for_layout(layout) {
                    Ok(true) => self.focus_thread_transcript(window, cx),
                    Ok(false) => {}
                    Err(error) => {
                        log::warn!(
                            "failed to collapse work-surface dock for narrow layout: {error:#}"
                        );
                        self.workbench_shell.record_error(error.to_string());
                    }
                }
            }
            layout = workbench_shell::WorkbenchLayout::allocate(
                window.viewport_size().width,
                self.sidebar.open,
                self.workbench_shell
                    .projection()
                    .visible_projection()
                    .is_some_and(|visible| visible.dock_open),
                self.workbench_shell.dock_width(),
            );

            h_flex()
                .id("omega-workbench-shell")
                .debug_selector(|| "omega.workbench.root".into())
                .key_context(self.key_context())
                .size_full()
                .items_stretch()
                .bg(cx.theme().colors().panel_background)
                .on_action(cx.listener(
                    |this, _: &workbench_shell::FocusActivityRail, window, cx| {
                        this.focus_activity_rail(window, cx);
                    },
                ))
                .on_action(
                    cx.listener(|this, _: &workbench_shell::SelectFiles, window, cx| {
                        this.select_work_surface(
                            omega_workbench_state::WorkSurface::Files,
                            window,
                            cx,
                        );
                    }),
                )
                .on_action(cx.listener(
                    |this, _: &zed_actions::project_panel::ToggleFocus, window, cx| {
                        if this.workbench_files_panel_handed_off {
                            cx.stop_propagation();
                            this.select_work_surface(
                                omega_workbench_state::WorkSurface::Files,
                                window,
                                cx,
                            );
                        }
                    },
                ))
                .on_action(cx.listener(|this, _: &ToggleGitFocus, window, cx| {
                    if this.workbench_git_panel_handed_off {
                        cx.stop_propagation();
                        this.select_work_surface(
                            omega_workbench_state::WorkSurface::Git,
                            window,
                            cx,
                        );
                    }
                }))
                .on_action(cx.listener(|this, _: &ToggleGitPanel, window, cx| {
                    if this.workbench_git_panel_handed_off {
                        cx.stop_propagation();
                        this.select_work_surface(
                            omega_workbench_state::WorkSurface::Git,
                            window,
                            cx,
                        );
                    }
                }))
                .on_action(cx.listener(|this, _: &CloseGitPanel, window, cx| {
                    if this.workbench_git_panel_handed_off
                        && this
                            .workbench_shell
                            .projection()
                            .visible_projection()
                            .is_some_and(|visible| {
                                visible.dock_open
                                    && visible.effective_surface
                                        == Some(omega_workbench_state::WorkSurface::Git)
                            })
                    {
                        cx.stop_propagation();
                        this.collapse_work_surface_dock(window, cx);
                    }
                }))
                .on_action(
                    cx.listener(|this, _: &workspace::CloseActiveDock, window, cx| {
                        let files_is_open = this
                            .workbench_shell
                            .projection()
                            .visible_projection()
                            .is_some_and(|visible| {
                                visible.dock_open
                                    && visible.effective_surface
                                        == Some(omega_workbench_state::WorkSurface::Files)
                            });
                        let files_tree_is_focused =
                            this.workbench_files_panel.as_ref().is_some_and(|panel| {
                                panel.focus_handle(cx).contains_focused(window, cx)
                            });
                        let files_host_is_focused = this
                            .workbench_shell
                            .visible_host()
                            .is_some_and(|host| host.focus_handle(cx).contains_focused(window, cx));
                        let git_is_open = this
                            .workbench_shell
                            .projection()
                            .visible_projection()
                            .is_some_and(|visible| {
                                visible.dock_open
                                    && visible.effective_surface
                                        == Some(omega_workbench_state::WorkSurface::Git)
                            });
                        let git_panel_is_focused =
                            this.workbench_git_panel.as_ref().is_some_and(|panel| {
                                panel.focus_handle(cx).contains_focused(window, cx)
                            });
                        let git_host_is_focused = this
                            .workbench_shell
                            .visible_host()
                            .is_some_and(|host| host.focus_handle(cx).contains_focused(window, cx));
                        let terminal_is_open = this
                            .workbench_shell
                            .projection()
                            .visible_projection()
                            .is_some_and(|visible| {
                                visible.dock_open
                                    && visible.effective_surface
                                        == Some(omega_workbench_state::WorkSurface::Terminal)
                            });
                        let terminal_panel_is_focused =
                            this.workbench_terminal_panel.as_ref().is_some_and(|panel| {
                                panel.focus_handle(cx).contains_focused(window, cx)
                            });
                        let terminal_host_is_focused = this
                            .workbench_shell
                            .visible_host()
                            .is_some_and(|host| host.focus_handle(cx).contains_focused(window, cx));
                        if (this.workbench_files_panel_handed_off
                            && files_is_open
                            && (files_tree_is_focused || files_host_is_focused))
                            || (this.workbench_git_panel_handed_off
                                && git_is_open
                                && (git_panel_is_focused || git_host_is_focused))
                            || (this.workbench_terminal_panel_handed_off
                                && terminal_is_open
                                && (terminal_panel_is_focused || terminal_host_is_focused))
                        {
                            cx.stop_propagation();
                            this.collapse_work_surface_dock(window, cx);
                        }
                    }),
                )
                .on_action(
                    cx.listener(|this, _: &workbench_shell::SelectSearch, window, cx| {
                        this.select_work_surface(
                            omega_workbench_state::WorkSurface::Search,
                            window,
                            cx,
                        );
                    }),
                )
                .on_action(
                    cx.listener(|this, _: &workbench_shell::SelectReview, window, cx| {
                        this.select_work_surface(
                            omega_workbench_state::WorkSurface::Review,
                            window,
                            cx,
                        );
                    }),
                )
                .on_action(
                    cx.listener(|this, _: &workbench_shell::SelectGit, window, cx| {
                        this.select_work_surface(
                            omega_workbench_state::WorkSurface::Git,
                            window,
                            cx,
                        );
                    }),
                )
                .on_action(
                    cx.listener(|this, _: &workbench_shell::SelectTerminal, window, cx| {
                        this.select_work_surface(
                            omega_workbench_state::WorkSurface::Terminal,
                            window,
                            cx,
                        );
                    }),
                )
                .on_action(cx.listener(|this, _: &workspace::NewTerminal, window, cx| {
                    if this.workbench_shell_enabled {
                        cx.stop_propagation();
                        if this.ensure_terminal_work_surface_open(window, cx) {
                            this.create_terminal_for_active_thread(window, cx);
                        }
                    }
                }))
                .on_action(cx.listener(
                    |this, action: &project_panel::OpenInThreadTerminal, window, cx| {
                        if this.workbench_shell_enabled {
                            cx.stop_propagation();
                            if this.ensure_terminal_work_surface_open(window, cx) {
                                this.create_terminal_for_active_thread_at(
                                    Some(action.working_directory.clone()),
                                    None,
                                    window,
                                    cx,
                                );
                            }
                        }
                    },
                ))
                .on_action(
                    cx.listener(|this, action: &workspace::OpenTerminal, window, cx| {
                        if this.workbench_shell_enabled {
                            cx.stop_propagation();
                            if action.local {
                                this.workbench_shell.record_error(
                                    "Local terminals are unavailable in the agent workbench",
                                );
                                cx.notify();
                            } else if this.ensure_terminal_work_surface_open(window, cx) {
                                this.create_terminal_for_active_thread_at(
                                    Some(action.working_directory.clone()),
                                    None,
                                    window,
                                    cx,
                                );
                            }
                        }
                    }),
                )
                .on_action(
                    cx.listener(|this, _: &workbench_shell::SelectPlan, window, cx| {
                        this.select_work_surface(
                            omega_workbench_state::WorkSurface::Plan,
                            window,
                            cx,
                        );
                    }),
                )
                .on_action(cx.listener(
                    |this, _: &workbench_shell::FocusNextSurface, window, cx| {
                        this.move_activity_rail_focus(
                            workbench_shell::RailFocusMovement::Next,
                            window,
                            cx,
                        );
                    },
                ))
                .on_action(cx.listener(
                    |this, _: &workbench_shell::FocusPreviousSurface, window, cx| {
                        this.move_activity_rail_focus(
                            workbench_shell::RailFocusMovement::Previous,
                            window,
                            cx,
                        );
                    },
                ))
                .on_action(cx.listener(
                    |this, _: &workbench_shell::FocusFirstSurface, window, cx| {
                        this.move_activity_rail_focus(
                            workbench_shell::RailFocusMovement::First,
                            window,
                            cx,
                        );
                    },
                ))
                .on_action(cx.listener(
                    |this, _: &workbench_shell::FocusLastSurface, window, cx| {
                        this.move_activity_rail_focus(
                            workbench_shell::RailFocusMovement::Last,
                            window,
                            cx,
                        );
                    },
                ))
                .on_action(cx.listener(
                    |this, _: &workbench_shell::ActivateFocusedSurface, window, cx| {
                        this.activate_focused_work_surface(window, cx);
                    },
                ))
                .on_action(cx.listener(
                    |this, _: &workbench_shell::CollapseWorkSurfaceDock, window, cx| {
                        this.collapse_work_surface_dock(window, cx);
                    },
                ))
                .on_action(cx.listener(
                    |this, _: &workbench_shell::FocusThreadTranscript, window, cx| {
                        this.focus_thread_transcript(window, cx);
                    },
                ))
                .on_action(cx.listener(
                    |this, _: &workbench_shell::NewTerminalForThread, window, cx| {
                        this.create_terminal_for_active_thread(window, cx);
                    },
                ))
                .on_action(cx.listener(
                    |this, _: &workbench_shell::ActivateNextTerminalTab, window, cx| {
                        this.activate_next_terminal_tab(window, cx);
                    },
                ))
                .on_action(cx.listener(
                    |this, _: &workbench_shell::ActivatePreviousTerminalTab, window, cx| {
                        this.activate_previous_terminal_tab(window, cx);
                    },
                ))
                .on_action(cx.listener(
                    |this, _: &workbench_shell::CloseActiveTerminalTab, window, cx| {
                        this.close_active_terminal_tab(window, cx);
                    },
                ))
                // The activity rail hugs the window's left edge, with the
                // threads sidebar beside it and the dock and content after —
                // an icon rail floating between two panels reads as broken.
                .child(self.render_activity_rail(window, cx))
                .children(self.render_sidebar(layout.sidebar, window, cx))
                .children(self.render_work_surface_dock(layout, window, cx))
                .child(v_flex().flex_1().min_w_0().h_full().child(content))
                .into_any_element()
        } else {
            content.into_any_element()
        };

        match self.visible_font_size() {
            WhichFontSize::AgentFont => {
                let theme_settings = ThemeSettings::get_global(cx);
                WithRemSize::new(theme_settings.agent_ui_font_size(cx))
                    .size_full()
                    .font_family(theme_settings.agent_ui_font_family().clone())
                    .child(content)
                    .into_any()
            }
            _ => content,
        }
    }
}

struct OnboardingUpsell;

impl Dismissable for OnboardingUpsell {
    const KEY: &'static str = "dismissed-trial-upsell";
}

struct TrialEndUpsell;

impl Dismissable for TrialEndUpsell {
    const KEY: &'static str = "dismissed-trial-end-upsell";
}

/// Test-only helper methods
#[cfg(any(test, feature = "test-support"))]
impl AgentPanel {
    pub fn test_new(workspace: &Workspace, window: &mut Window, cx: &mut Context<Self>) -> Self {
        Self::new(workspace, window, cx)
    }

    pub fn enable_workbench_shell_for_tests(&mut self, cx: &mut Context<Self>) {
        self.workbench_shell_enabled = true;
        cx.notify();
    }

    pub fn workbench_projection_for_tests(&self) -> &omega_workbench_state::WorkbenchProjection {
        self.workbench_shell.projection()
    }

    pub fn workbench_identity_for_tests(
        &self,
    ) -> Option<&crate::thread_identity::ThreadIdentityState> {
        self.workbench_shell.identity()
    }

    pub fn workbench_capability_for_tests(
        &self,
        surface: omega_workbench_state::WorkSurface,
    ) -> Option<&workbench_shell::SurfaceCapability> {
        self.workbench_shell.capability(surface)
    }

    pub fn workbench_repository_menu_for_tests(&self) -> Option<Entity<ContextMenu>> {
        self.thread_repository_menu_handle.deployed_menu()
    }

    pub fn workbench_identity_target_selection_ready_for_tests(&self, cx: &App) -> bool {
        self.thread_identity_target_selection_unavailable_reason(cx)
            .is_none()
    }

    pub fn workbench_branch_menu_for_tests(
        &self,
    ) -> Option<Entity<git_ui::branch_picker::BranchList>> {
        self.thread_branch_menu_handle.deployed_menu()
    }

    pub fn set_workbench_identity_phase_for_tests(
        &mut self,
        phase: Option<IdentityPhase>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.workbench_identity_phase_override = phase;
        self.thread_identity_observation_revision =
            self.thread_identity_observation_revision.saturating_add(1);
        self.sync_workbench_shell(window, cx);
        cx.notify();
    }

    pub fn set_workbench_identity_observation_for_tests(
        &mut self,
        observation: Option<ThreadIdentityObservation>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.workbench_identity_observation_override = observation;
        self.sync_workbench_shell(window, cx);
        cx.notify();
    }

    pub fn mark_workbench_identity_inconsistent_for_tests(
        &mut self,
        message: impl Into<SharedString>,
        cx: &mut Context<Self>,
    ) -> Result<()> {
        let visible = self
            .workbench_shell
            .projection()
            .visible_projection()
            .ok_or_else(|| anyhow!("workbench has no visible thread"))?;
        let binding = visible
            .binding
            .clone()
            .ok_or_else(|| anyhow!("workbench thread has no repository binding"))?;
        self.thread_identity_operation_request =
            self.thread_identity_operation_request.saturating_add(1);
        let request_id = self.thread_identity_operation_request;
        self.thread_identity_operation_requests
            .insert(visible.thread_id.clone(), request_id);
        self.thread_identity_operation_errors.insert(
            visible.thread_id,
            ThreadIdentityOperationError {
                source_binding: Some(binding.clone()),
                attempted_binding: binding,
                binding_generation: visible.generation,
                request_id,
                inconsistent: true,
                message: message.into(),
            },
        );
        self.thread_identity_observation_revision =
            self.thread_identity_observation_revision.saturating_add(1);
        cx.notify();
        Ok(())
    }

    pub fn workbench_focus_target_for_tests(&self) -> workbench_shell::WorkbenchFocusTarget {
        self.workbench_shell.focus_target()
    }

    pub fn workbench_host_entity_id_for_tests(
        &self,
        surface: omega_workbench_state::WorkSurface,
        cx: &App,
    ) -> Option<gpui::EntityId> {
        let host = self.workbench_shell.visible_host()?;
        (host.read(cx).key().surface == surface).then(|| host.entity_id())
    }

    pub fn visible_workbench_host_for_tests(
        &self,
    ) -> Option<Entity<workbench_shell::WorkSurfaceHost>> {
        self.workbench_shell.visible_host().cloned()
    }

    pub fn workbench_files_panel_for_tests(&self) -> Option<Entity<ProjectPanel>> {
        self.workbench_files_panel.clone()
    }

    pub fn workbench_search_surface_for_tests(
        &self,
        cx: &App,
    ) -> Option<Entity<workbench_shell::NativeSearchSurface>> {
        self.workbench_shell.search_surface_for_active_binding(cx)
    }

    pub fn workbench_review_surface_for_tests(
        &self,
        cx: &App,
    ) -> Option<Entity<workbench_shell::NativeReviewSurface>> {
        self.workbench_shell.review_surface_for_active_binding(cx)
    }

    pub fn workbench_git_surface_for_tests(
        &self,
        cx: &App,
    ) -> Option<Entity<workbench_shell::NativeGitSurface>> {
        self.workbench_shell.git_surface_for_active_binding(cx)
    }

    pub fn workbench_terminal_surface_for_tests(
        &self,
    ) -> Option<Entity<workbench_shell::NativeTerminalSurface>> {
        self.workbench_terminal_surface.clone()
    }

    pub fn workbench_plan_surface_for_tests(
        &self,
        cx: &App,
    ) -> Option<Entity<workbench_shell::NativePlanSurface>> {
        self.workbench_shell.plan_surface_for_active_binding(cx)
    }

    pub fn thread_outline_for_tests(&self) -> Entity<crate::thread_outline::ThreadOutline> {
        self.thread_outline.clone()
    }

    pub fn synchronize_thread_outline_for_tests(&mut self, cx: &mut Context<Self>) {
        self.synchronize_thread_outline(cx);
    }

    pub fn thread_outline_navigation_target_for_tests(
        &self,
    ) -> Option<(acp_thread::ThreadEntryId, usize)> {
        self.thread_outline_navigation_target
    }

    pub fn workbench_plan_navigation_target_for_tests(&self) -> Option<usize> {
        self.workbench_plan_navigation_target
    }

    pub fn workbench_terminal_panel_for_tests(&self) -> Option<Entity<TerminalPanel>> {
        self.workbench_terminal_panel.clone()
    }

    pub fn set_workbench_terminal_owner_state_for_tests(
        &mut self,
        owner_state: Option<workbench_shell::NativeTerminalOwnerState>,
        cx: &mut Context<Self>,
    ) {
        self.workbench_terminal_owner_state_override = owner_state;
        cx.notify();
    }

    pub fn set_workbench_plan_lifecycle_for_tests(
        &mut self,
        lifecycle: Option<workbench_shell::NativePlanLifecycle>,
        cx: &mut Context<Self>,
    ) {
        self.workbench_plan_lifecycle_override = lifecycle;
        if let Some(surface) = self.workbench_shell.plan_surface_for_active_binding(cx) {
            let generation = surface.read(cx).binding().generation;
            let lifecycle = self.native_plan_lifecycle(cx);
            surface.update(cx, |surface, cx| {
                surface.set_lifecycle(generation, lifecycle, cx);
            });
        }
        cx.notify();
    }

    pub fn set_workbench_terminal_badge_for_tests(
        &mut self,
        badge: Option<workbench_shell::SurfaceBadge>,
        cx: &mut Context<Self>,
    ) {
        self.workbench_terminal_badge_override = badge.clone();
        self.workbench_shell
            .set_badge(omega_workbench_state::WorkSurface::Terminal, badge);
        cx.notify();
    }

    pub fn set_workbench_git_lifecycle_for_tests(
        &mut self,
        lifecycle: Option<workbench_shell::NativeGitLifecycle>,
        cx: &mut Context<Self>,
    ) {
        self.workbench_git_lifecycle_override = lifecycle;
        self.synchronize_git_surface_lifecycle_for_panel(cx);
        cx.notify();
    }

    pub fn workbench_host_count_for_tests(&self) -> usize {
        self.workbench_shell.host_count()
    }

    pub fn set_workbench_badge_for_tests(
        &mut self,
        surface: omega_workbench_state::WorkSurface,
        badge: Option<workbench_shell::SurfaceBadge>,
        cx: &mut Context<Self>,
    ) {
        self.workbench_shell.set_badge(surface, badge);
        cx.notify();
    }

    pub fn fail_next_workbench_host_creation_for_tests(
        &mut self,
        surface: omega_workbench_state::WorkSurface,
    ) {
        self.workbench_shell.fail_next_host_creation(surface);
    }

    pub fn begin_workbench_surface_load_for_tests(
        &mut self,
        request_id: impl Into<String>,
        surface: omega_workbench_state::WorkSurface,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Result<workbench_shell::SurfaceLoadContext> {
        self.workbench_shell
            .begin_surface_load(request_id, surface, window, cx)
    }

    pub fn complete_workbench_surface_load_for_tests(
        &mut self,
        load: workbench_shell::SurfaceLoadContext,
        outcome: workbench_shell::SurfaceLoadOutcome,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Result<omega_workbench_state::TransitionEffect> {
        let effect = self
            .workbench_shell
            .complete_surface_load(load, outcome, window, cx)?;
        cx.notify();
        Ok(effect)
    }

    pub fn invalidate_workbench_surface_for_tests(
        &mut self,
        surface: omega_workbench_state::WorkSurface,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Result<omega_workbench_state::TransitionEffect> {
        let effect = self.workbench_shell.invalidate_surface(surface, cx)?;
        if self.workbench_shell.focus_target() == workbench_shell::WorkbenchFocusTarget::Transcript
        {
            self.focus_thread_transcript(window, cx);
        } else {
            cx.notify();
        }
        Ok(effect)
    }

    /// Drops a thread's `ConversationView` from `retained_threads` without
    /// deleting its metadata or kvp state. Simulates the post-restart
    pub fn test_unload_retained_thread(&mut self, id: ThreadId) -> bool {
        self.retained_threads.remove(&id).is_some()
    }

    /// Opens an external thread using an arbitrary AgentServer.
    ///
    /// This is a test-only helper that allows visual tests and integration tests
    /// to inject a stub server without modifying production code paths.
    /// Not compiled into production builds.
    pub fn open_external_thread_with_server(
        &mut self,
        server: Rc<dyn AgentServer>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let ext_agent = Agent::Custom {
            id: server.agent_id(),
        };

        let thread = self.create_agent_thread_with_server(
            ext_agent,
            Some(server),
            None,
            None,
            None,
            None,
            None,
            AgentThreadSource::AgentPanel,
            window,
            cx,
        );
        self.set_base_view(thread.into(), true, window, cx);
    }

    pub fn open_external_thread_with_server_and_work_dirs(
        &mut self,
        server: Rc<dyn AgentServer>,
        work_dirs: PathList,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let ext_agent = Agent::Custom {
            id: server.agent_id(),
        };
        let thread = self.create_agent_thread_with_server(
            ext_agent,
            Some(server),
            None,
            Some(work_dirs),
            None,
            None,
            None,
            AgentThreadSource::AgentPanel,
            window,
            cx,
        );
        self.set_base_view(thread.into(), true, window, cx);
    }

    /// Opens an external thread on an arbitrary `AgentServer` under a
    /// `ThreadId` a previous process wrote down.
    ///
    /// The restore path a real relaunch takes: `restore_new_draft` reads the
    /// persisted `ThreadId` and `agent_id` out of the metadata store and hands
    /// the id to `create_agent_thread_with_server`, which is the one argument
    /// that makes the reopened thread *the same thread* rather than a new one
    /// with the same content. A harness proving disclosure after a restart
    /// needs exactly that: the lane correlation on disk is keyed by `ThreadId`,
    /// so a thread reopened under a fresh id would silently stop being the
    /// thread the journal names, and the capture would show a lane run missing
    /// for the wrong reason.
    #[cfg(any(test, feature = "test-support"))]
    pub fn open_external_thread_with_server_under_id(
        &mut self,
        server: Rc<dyn AgentServer>,
        thread_id: ThreadId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let ext_agent = Agent::Custom {
            id: server.agent_id(),
        };

        let thread = self.create_agent_thread_with_server(
            ext_agent,
            Some(server),
            Some(thread_id),
            None,
            None,
            None,
            None,
            AgentThreadSource::AgentPanel,
            window,
            cx,
        );
        self.set_base_view(thread.into(), true, window, cx);
    }

    /// Opens a restored external thread with an arbitrary AgentServer and
    /// a specific `resume_session_id` — as if we just restored from the KVP.
    ///
    /// Test-only helper. Not compiled into production builds.
    pub fn open_restored_thread_with_server(
        &mut self,
        server: Rc<dyn AgentServer>,
        resume_session_id: acp::SessionId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let ext_agent = Agent::Custom {
            id: server.agent_id(),
        };

        // The panel addresses threads by `ThreadId` after the draft work;
        // map the test-provided `session_id` back through the metadata
        // store so this helper still resumes the right thread.
        let resume_thread_id = ThreadMetadataStore::try_global(cx).and_then(|store| {
            store
                .read(cx)
                .entry_by_session(&resume_session_id)
                .map(|m| m.thread_id)
        });

        let thread = self.create_agent_thread_with_server(
            ext_agent,
            Some(server),
            resume_thread_id,
            None,
            None,
            None,
            None,
            AgentThreadSource::AgentPanel,
            window,
            cx,
        );
        self.set_base_view(thread.into(), true, window, cx);
    }

    /// Returns the currently active thread view, if any.
    ///
    /// This is a test-only accessor that exposes the private `active_thread_view()`
    /// method for test assertions. Not compiled into production builds.
    pub fn active_thread_view_for_tests(&self) -> Option<&Entity<ConversationView>> {
        self.active_conversation_view()
    }

    /// Creates a draft thread using a stub server and sets it as the active view.
    #[cfg(any(test, feature = "test-support"))]
    pub fn open_draft_with_server(
        &mut self,
        server: Rc<dyn AgentServer>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let ext_agent = Agent::Custom {
            id: server.agent_id(),
        };
        let thread = self.create_agent_thread_with_server(
            ext_agent,
            Some(server),
            None,
            None,
            None,
            None,
            None,
            AgentThreadSource::AgentPanel,
            window,
            cx,
        );
        self.draft_thread = Some(thread.conversation_view.clone());
        self.set_base_view(thread.into(), true, window, cx);
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn insert_test_terminal(
        &mut self,
        title: impl Into<String>,
        focus: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Result<TerminalId> {
        let terminal_id = TerminalId::new();
        self.set_last_created_entry_kind_from_user_action(AgentPanelEntryKind::Terminal, cx);
        self.insert_display_only_terminal(
            terminal_id,
            None,
            Some(SharedString::from(title.into())),
            None,
            None,
            focus,
            focus,
            true,
            AgentThreadSource::AgentPanel,
            window,
            cx,
        )?;
        Ok(terminal_id)
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn restore_test_terminal(
        &mut self,
        metadata: TerminalThreadMetadata,
        focus: bool,
        source: AgentThreadSource,
        workspace: Option<&Workspace>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Result<()> {
        if self.has_terminal(metadata.terminal_id) {
            self.activate_terminal(metadata.terminal_id, focus, window, cx);
            return Ok(());
        }

        if !self.supports_terminal(cx) {
            return Ok(());
        }

        let working_directory = self.terminal_restore_working_directory(&metadata, workspace, cx);
        let initial_title = Self::terminal_restore_initial_title(&metadata);
        self.insert_display_only_terminal(
            metadata.terminal_id,
            working_directory,
            metadata.custom_title.clone(),
            initial_title,
            Some(metadata.created_at),
            true,
            focus,
            true,
            source,
            window,
            cx,
        )
    }

    #[cfg(any(test, feature = "test-support"))]
    fn insert_display_only_terminal(
        &mut self,
        terminal_id: TerminalId,
        working_directory: Option<PathBuf>,
        custom_title: Option<SharedString>,
        initial_title: Option<SharedString>,
        created_at: Option<DateTime<Utc>>,
        select: bool,
        focus: bool,
        run_init_command: bool,
        source: AgentThreadSource,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Result<()> {
        let init_command = Self::terminal_init_command(run_init_command, cx);
        let settings = TerminalSettings::get_global(cx).clone();
        let path_style = self.project.read(cx).path_style(cx);
        let builder = terminal::TerminalBuilder::new_display_only(
            settings.cursor_shape,
            settings.alternate_scroll,
            settings.max_scroll_history_lines,
            cx.entity_id().as_u64(),
            cx.background_executor(),
            path_style,
        );
        let terminal = cx.new(|cx| builder.subscribe(cx));
        let terminal_for_init_command = terminal.clone();
        let terminal_view = cx.new(|cx| {
            let mut view = TerminalView::new(
                terminal,
                self.workspace.clone(),
                self.workspace_id,
                self.project.downgrade(),
                window,
                cx,
            );
            view.set_show_workspace_actions(false, cx);
            view
        });
        self.insert_terminal(
            terminal_id,
            terminal_view,
            working_directory,
            custom_title,
            initial_title,
            created_at,
            select,
            focus,
            source,
            window,
            cx,
        );
        Self::write_terminal_init_command(&terminal_for_init_command, init_command, cx);
        Ok(())
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn emit_test_terminal_bell(&mut self, terminal_id: TerminalId, cx: &mut Context<Self>) {
        let Some(terminal_entity) = self
            .terminals
            .get(&terminal_id)
            .map(|terminal| terminal.view.read(cx).terminal().clone())
        else {
            return;
        };
        terminal_entity.update(cx, |_terminal, cx| {
            cx.emit(TerminalEvent::Bell);
        });
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn emit_test_terminal_close(&mut self, terminal_id: TerminalId, cx: &mut Context<Self>) {
        let Some(terminal_entity) = self
            .terminals
            .get(&terminal_id)
            .map(|terminal| terminal.view.read(cx).terminal().clone())
        else {
            return;
        };
        terminal_entity.update(cx, |_terminal, cx| {
            cx.emit(TerminalEvent::CloseTerminal);
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::NewWorktreeBranchTarget;
    use crate::conversation_view::tests::{StubAgentServer, init_test};
    use crate::test_support::{
        active_session_id, active_thread_id, open_thread_with_connection,
        open_thread_with_custom_connection, register_test_sidebar, send_message,
    };
    use acp_thread::{AgentConnection, StubAgentConnection, ThreadStatus};
    use action_log::ActionLog;
    use anyhow::{Result, anyhow};
    use feature_flags::FeatureFlagAppExt;
    use fs::FakeFs;
    use gpui::{App, Modifiers, TestAppContext, UpdateGlobal, VisualTestContext, px, size};
    use parking_lot::Mutex;
    use project::{Project, WorktreePaths};
    use settings::{SettingsStore, WorkingDirectory};
    use std::any::Any;

    use serde_json::json;
    use std::path::{Path, PathBuf};
    use std::sync::Arc;
    use std::time::Instant;

    struct PairingContentBudgetView {
        image: Arc<RenderImage>,
        image_size: Pixels,
    }

    impl Render for PairingContentBudgetView {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            render_ready_device_pairing_content(
                "omega-dev.example",
                4317,
                self.image.clone(),
                self.image_size,
            )
        }
    }

    #[test]
    fn test_is_known_terminal_agent_command() {
        assert!(is_known_terminal_agent_command("claude"));
        assert!(is_known_terminal_agent_command("codex"));
        assert!(!is_known_terminal_agent_command("cargo"));
        assert!(!is_known_terminal_agent_command("internal-agent"));
    }

    #[test]
    fn test_render_pairing_qr_creates_one_scaled_image() {
        let pairing_qr = omega_effectd::PairingQr {
            width: 2,
            modules: vec![true, false, false, true],
        };

        let (image, image_size) = render_pairing_qr(&pairing_qr).expect("valid QR should render");
        let bytes = image
            .as_bytes(0)
            .expect("rendered QR should have one frame");

        assert_eq!(image_size, px(6.));
        assert_eq!(bytes.len(), 6 * 6 * 4);
        assert_eq!(bytes.get(0..4), Some([0, 0, 0, 255].as_slice()));
        assert_eq!(bytes.get(12..16), Some([255, 255, 255, 255].as_slice()));
        assert_eq!(bytes.get(84..88), Some([0, 0, 0, 255].as_slice()));
    }

    #[test]
    fn test_render_pairing_qr_rejects_invalid_dimensions() {
        let pairing_qr = omega_effectd::PairingQr {
            width: 2,
            modules: vec![true],
        };

        assert!(render_pairing_qr(&pairing_qr).is_err());
    }

    #[gpui::test]
    fn test_pairing_content_stays_within_element_budget(cx: &mut TestAppContext) {
        init_test(cx);
        let module_width = 57;
        let pairing_qr = omega_effectd::PairingQr {
            width: module_width,
            modules: vec![false; module_width * module_width],
        };
        let (image, image_size) = render_pairing_qr(&pairing_qr).expect("valid QR should render");
        let window = cx.open_window(size(px(360.), px(480.)), move |_, _| {
            PairingContentBudgetView { image, image_size }
        });
        cx.run_until_parked();

        let snapshot = window
            .update(cx, |_, window, _| window.debug_render_snapshot())
            .expect("test window should remain open");
        assert!(
            snapshot.element_count() <= 40,
            "pairing content rendered {} elements; hotspots: {:?}",
            snapshot.element_count(),
            snapshot.element_hotspots()
        );
    }

    #[test]
    fn test_terminal_program_reports_known_agent_transitions() {
        let mut last_observed_program = None;

        assert_eq!(
            terminal_program_to_report(&mut last_observed_program, Some("codex".to_string())),
            Some("codex".to_string())
        );
        assert_eq!(
            terminal_program_to_report(&mut last_observed_program, Some("codex".to_string())),
            None
        );
        assert_eq!(
            terminal_program_to_report(&mut last_observed_program, Some("zsh".to_string())),
            None
        );
        assert_eq!(
            terminal_program_to_report(
                &mut last_observed_program,
                Some("customer-data-export".to_string())
            ),
            None
        );
        assert_eq!(
            terminal_program_to_report(&mut last_observed_program, Some("codex".to_string())),
            Some("codex".to_string())
        );
        assert_eq!(
            terminal_program_to_report(&mut last_observed_program, None),
            None
        );
        assert_eq!(
            terminal_program_to_report(&mut last_observed_program, Some("codex".to_string())),
            Some("codex".to_string())
        );
    }

    #[derive(Clone, Default)]
    struct SessionTrackingConnection {
        next_session_number: Arc<Mutex<usize>>,
        sessions: Arc<Mutex<HashSet<acp::SessionId>>>,
    }

    impl SessionTrackingConnection {
        fn new() -> Self {
            Self::default()
        }

        fn create_session(
            self: Rc<Self>,
            session_id: acp::SessionId,
            project: Entity<Project>,
            work_dirs: PathList,
            title: Option<SharedString>,
            cx: &mut App,
        ) -> Entity<AcpThread> {
            self.sessions.lock().insert(session_id.clone());

            let action_log = cx.new(|_| ActionLog::new(project.clone()));
            cx.new(|cx| {
                AcpThread::new(
                    None,
                    title,
                    Some(work_dirs),
                    self,
                    project,
                    action_log,
                    session_id,
                    watch::Receiver::constant(
                        acp::PromptCapabilities::new()
                            .image(true)
                            .audio(true)
                            .embedded_context(true),
                    ),
                    cx,
                )
            })
        }
    }

    impl AgentConnection for SessionTrackingConnection {
        fn agent_id(&self) -> AgentId {
            agent::OMEGA_AGENT_ID.clone()
        }

        fn telemetry_id(&self) -> SharedString {
            "session-tracking-test".into()
        }

        fn new_session(
            self: Rc<Self>,
            project: Entity<Project>,
            work_dirs: PathList,
            cx: &mut App,
        ) -> Task<Result<Entity<AcpThread>>> {
            let session_id = {
                let mut next_session_number = self.next_session_number.lock();
                let session_id = acp::SessionId::new(format!(
                    "session-tracking-session-{}",
                    *next_session_number
                ));
                *next_session_number += 1;
                session_id
            };
            let thread = self.create_session(session_id, project, work_dirs, None, cx);
            Task::ready(Ok(thread))
        }

        fn supports_load_session(&self) -> bool {
            true
        }

        fn load_session(
            self: Rc<Self>,
            session_id: acp::SessionId,
            project: Entity<Project>,
            work_dirs: PathList,
            title: Option<SharedString>,
            cx: &mut App,
        ) -> Task<Result<Entity<AcpThread>>> {
            let thread = self.create_session(session_id, project, work_dirs, title, cx);
            thread.update(cx, |thread, cx| {
                thread
                    .handle_session_update(
                        acp::SessionUpdate::UserMessageChunk(acp::ContentChunk::new(
                            "Restored user message".into(),
                        )),
                        cx,
                    )
                    .expect("restored user message should be applied");
                thread
                    .handle_session_update(
                        acp::SessionUpdate::AgentMessageChunk(acp::ContentChunk::new(
                            "Restored assistant message".into(),
                        )),
                        cx,
                    )
                    .expect("restored assistant message should be applied");
            });
            Task::ready(Ok(thread))
        }

        fn supports_close_session(&self) -> bool {
            true
        }

        fn close_session(
            self: Rc<Self>,
            session_id: &acp::SessionId,
            _cx: &mut App,
        ) -> Task<Result<()>> {
            self.sessions.lock().remove(session_id);
            Task::ready(Ok(()))
        }

        fn auth_methods(&self) -> &[acp::AuthMethod] {
            &[]
        }

        fn authenticate(&self, _method_id: acp::AuthMethodId, _cx: &mut App) -> Task<Result<()>> {
            Task::ready(Ok(()))
        }

        fn prompt(
            &self,
            params: acp::PromptRequest,
            _cx: &mut App,
        ) -> Task<Result<acp::PromptResponse>> {
            if !self.sessions.lock().contains(&params.session_id) {
                return Task::ready(Err(anyhow!("Session not found")));
            }

            Task::ready(Ok(acp::PromptResponse::new(acp::StopReason::EndTurn)))
        }

        fn cancel(&self, _session_id: &acp::SessionId, _cx: &mut App) {}

        fn into_any(self: Rc<Self>) -> Rc<dyn Any> {
            self
        }
    }

    #[gpui::test]
    async fn test_clicking_tool_call_output_keeps_agent_panel_focused_and_zoomed(
        cx: &mut TestAppContext,
    ) {
        init_test(cx);
        cx.update(|cx| {
            agent::ThreadStore::init_global(cx);
            language_model::LanguageModelRegistry::test(cx);
        });

        let fs = FakeFs::new(cx.executor());
        let project = Project::test(fs, [], cx).await;
        let multi_workspace =
            cx.add_window(|window, cx| MultiWorkspace::test_new(project.clone(), window, cx));
        let workspace = multi_workspace
            .read_with(cx, |multi_workspace, _cx| {
                multi_workspace.workspace().clone()
            })
            .unwrap();
        let cx = &mut VisualTestContext::from_window(multi_workspace.into(), cx);
        cx.simulate_resize(size(px(900.), px(700.)));

        let connection = StubAgentConnection::new();
        let panel = workspace.update_in(cx, |workspace, window, cx| {
            let panel = cx.new(|cx| AgentPanel::new(workspace, window, cx));
            workspace.add_panel(panel.clone(), window, cx);
            workspace.focus_panel::<AgentPanel>(window, cx);
            panel
        });
        open_thread_with_connection(&panel, connection.clone(), cx);

        let session_id = active_session_id(&panel, cx);
        let tool_call_id = acp::ToolCallId::new("tool-call-output-focus-regression");
        cx.update(|_window, cx| {
            connection.send_update(
                session_id.clone(),
                acp::SessionUpdate::ToolCall(
                    acp::ToolCall::new(tool_call_id.clone(), "Read file")
                        .kind(acp::ToolKind::Fetch)
                        .status(acp::ToolCallStatus::InProgress),
                ),
                cx,
            );
            connection.send_update(
                session_id,
                acp::SessionUpdate::ToolCallUpdate(acp::ToolCallUpdate::new(
                    tool_call_id.clone(),
                    acp::ToolCallUpdateFields::new()
                        .status(acp::ToolCallStatus::Completed)
                        .content(vec![acp::ToolCallContent::Content(acp::Content::new(
                            acp::ContentBlock::Text(acp::TextContent::new(
                                "tool output text".to_string(),
                            )),
                        ))]),
                )),
                cx,
            );
        });
        cx.run_until_parked();

        let thread_view = panel.read_with(cx, |panel, cx| panel.active_thread_view(cx).unwrap());
        thread_view.update(cx, |thread_view, cx| {
            thread_view.entry_view_state.update(cx, |state, _cx| {
                state.expand_tool_call(tool_call_id);
            });
            cx.notify();
        });

        panel.update_in(cx, |panel, window, cx| {
            panel.toggle_zoom(&ToggleZoom, window, cx);
        });

        // The thread receives only tool-call updates, so entry index 0 should remain stable.
        let output_bounds = cx
            .debug_bounds("tool-call-output-0-0")
            .expect("tool call output should be rendered");
        cx.simulate_click(output_bounds.center(), Modifiers::default());
        cx.run_until_parked();

        panel.update_in(cx, |panel, window, cx| {
            assert!(
                panel.focus_handle(cx).contains_focused(window, cx),
                "clicking tool call output should keep focus within the agent panel"
            );
            assert!(
                panel.is_zoomed(window, cx),
                "clicking tool call output should not close Zen Mode"
            );
        });

        let title_editor_focus_handle = panel.read_with(cx, |panel, cx| {
            panel
                .active_thread_view(cx)
                .expect("active thread view should be present")
                .read(cx)
                .title_editor
                .focus_handle(cx)
        });
        cx.update(|window, cx| {
            title_editor_focus_handle.focus(window, cx);
        });
        cx.run_until_parked();

        panel.update_in(cx, |panel, window, cx| {
            assert!(
                panel.focus_handle(cx).contains_focused(window, cx),
                "focusing the thread title editor should keep focus within the agent panel"
            );
            assert!(
                panel.is_zoomed(window, cx),
                "focusing the thread title editor should not close Zen Mode"
            );
        });
    }

    #[gpui::test]
    async fn test_active_thread_serialize_and_load_round_trip(cx: &mut TestAppContext) {
        init_test(cx);
        cx.update(|cx| {
            agent::ThreadStore::init_global(cx);
            language_model::LanguageModelRegistry::test(cx);
        });

        // Create a MultiWorkspace window with two workspaces.
        let fs = FakeFs::new(cx.executor());
        fs.insert_tree("/project_a", json!({ "file.txt": "" }))
            .await;
        let project_a = Project::test(fs.clone(), [Path::new("/project_a")], cx).await;
        let project_b = Project::test(fs, [], cx).await;

        let multi_workspace =
            cx.add_window(|window, cx| MultiWorkspace::test_new(project_a.clone(), window, cx));

        let workspace_a = multi_workspace
            .read_with(cx, |multi_workspace, _cx| {
                multi_workspace.workspace().clone()
            })
            .unwrap();

        let workspace_b = multi_workspace
            .update(cx, |multi_workspace, window, cx| {
                multi_workspace.test_add_workspace(project_b.clone(), window, cx)
            })
            .unwrap();

        workspace_a.update(cx, |workspace, _cx| {
            workspace.set_random_database_id();
        });
        workspace_b.update(cx, |workspace, _cx| {
            workspace.set_random_database_id();
        });

        let cx = &mut VisualTestContext::from_window(multi_workspace.into(), cx);

        // Set up workspace A: with an active thread.
        let panel_a = workspace_a.update_in(cx, |workspace, window, cx| {
            cx.new(|cx| AgentPanel::new(workspace, window, cx))
        });

        panel_a.update_in(cx, |panel, window, cx| {
            panel.open_external_thread_with_server(
                Rc::new(StubAgentServer::default_response()),
                window,
                cx,
            );
        });

        cx.run_until_parked();

        panel_a.read_with(cx, |panel, cx| {
            assert!(
                panel.active_agent_thread(cx).is_some(),
                "workspace A should have an active thread after connection"
            );
        });

        send_message(&panel_a, cx);

        let agent_type_a = panel_a.read_with(cx, |panel, _cx| panel.selected_agent.clone());

        // Set up workspace B: ClaudeCode, no active thread.
        let panel_b = workspace_b.update_in(cx, |workspace, window, cx| {
            cx.new(|cx| AgentPanel::new(workspace, window, cx))
        });

        panel_b.update(cx, |panel, _cx| {
            panel.selected_agent = Agent::Custom {
                id: "claude-acp".into(),
            };
        });

        // Serialize both panels.
        panel_a.update(cx, |panel, cx| panel.serialize(cx));
        panel_b.update(cx, |panel, cx| panel.serialize(cx));
        cx.run_until_parked();

        let workspace_a_id = workspace_a
            .read_with(cx, |workspace, _cx| workspace.database_id())
            .expect("workspace A should have a database id");
        let kvp = cx.update(|_window, cx| KeyValueStore::global(cx));
        let serialized_a: SerializedAgentPanel = cx
            .background_spawn(async move { read_serialized_panel(workspace_a_id, &kvp) })
            .await
            .expect("workspace A should serialize panel state");
        assert!(
            serialized_a.last_active_thread.is_some(),
            "active thread should be the thread restore target"
        );
        assert!(
            serialized_a.last_active_terminal_id.is_none(),
            "active thread serialization should not also include a terminal restore target"
        );

        cx.update(|_window, cx| {
            ThreadMetadataStore::init_global(cx);
        });

        // Load fresh panels for each workspace and verify independent state.
        let async_cx = cx.update(|window, cx| window.to_async(cx));
        let loaded_a = AgentPanel::load(workspace_a.downgrade(), async_cx)
            .await
            .expect("panel A load should succeed");
        cx.run_until_parked();

        let async_cx = cx.update(|window, cx| window.to_async(cx));
        let loaded_b = AgentPanel::load(workspace_b.downgrade(), async_cx)
            .await
            .expect("panel B load should succeed");
        cx.run_until_parked();

        // Workspace A should restore its thread and agent type
        loaded_a.read_with(cx, |panel, _cx| {
            assert_eq!(
                panel.selected_agent, agent_type_a,
                "workspace A agent type should be restored"
            );
            assert!(
                panel.active_conversation_view().is_some(),
                "workspace A should have its active thread restored"
            );
        });

        // Workspace B should restore its own agent type but have no active thread.
        loaded_b.read_with(cx, |panel, _cx| {
            assert_eq!(
                panel.selected_agent,
                Agent::Custom {
                    id: "claude-acp".into()
                },
                "workspace B agent type should be restored"
            );
            assert!(
                panel.active_conversation_view().is_none(),
                "workspace B should have no active thread when it had no prior conversation"
            );
        });
    }

    #[gpui::test]
    async fn test_active_terminal_serialize_and_load_round_trip(cx: &mut TestAppContext) {
        init_test(cx);
        cx.update(|cx| {
            agent::ThreadStore::init_global(cx);
            TerminalThreadMetadataStore::init_global(cx);
            language_model::LanguageModelRegistry::test(cx);
        });

        let fs = FakeFs::new(cx.executor());
        fs.insert_tree("/project", json!({ "file.txt": "" })).await;
        cx.update(|cx| <dyn fs::Fs>::set_global(fs.clone(), cx));
        let project = Project::test(fs.clone(), [Path::new("/project")], cx).await;

        let multi_workspace =
            cx.add_window(|window, cx| MultiWorkspace::test_new(project.clone(), window, cx));
        let workspace = multi_workspace
            .read_with(cx, |multi_workspace, _cx| {
                multi_workspace.workspace().clone()
            })
            .unwrap();
        workspace.update(cx, |workspace, _cx| {
            workspace.set_random_database_id();
        });

        let cx = &mut VisualTestContext::from_window(multi_workspace.into(), cx);
        let panel = workspace.update_in(cx, |workspace, window, cx| {
            cx.new(|cx| AgentPanel::new(workspace, window, cx))
        });

        panel.update_in(cx, |panel, window, cx| {
            panel.activate_new_thread(false, AgentThreadSource::AgentPanel, window, cx);
        });
        let terminal_id = panel
            .update_in(cx, |panel, window, cx| {
                panel.insert_test_terminal("Dev Server", true, window, cx)
            })
            .expect("test terminal should be inserted");
        panel.update(cx, |panel, cx| panel.serialize(cx));
        cx.run_until_parked();

        let workspace_id = workspace
            .read_with(cx, |workspace, _cx| workspace.database_id())
            .expect("workspace should have a database id");
        let kvp = cx.update(|_window, cx| KeyValueStore::global(cx));
        let serialized: SerializedAgentPanel = cx
            .background_spawn(async move { read_serialized_panel(workspace_id, &kvp) })
            .await
            .expect("workspace should serialize panel state");
        assert_eq!(
            serialized.last_active_terminal_id,
            Some(terminal_id.to_key_string())
        );
        assert!(
            serialized.last_active_thread.is_none(),
            "active terminal serialization should not also include a thread restore target"
        );

        cx.update(|_window, cx| {
            TerminalThreadMetadataStore::init_global(cx);
        });
        let async_cx = cx.update(|window, cx| window.to_async(cx));
        let loaded = AgentPanel::load(workspace.downgrade(), async_cx)
            .await
            .expect("panel load should succeed");
        for _ in 0..8 {
            cx.run_until_parked();
        }

        loaded.read_with(cx, |panel, cx| {
            assert_eq!(panel.active_terminal_id(), Some(terminal_id));
            assert!(
                panel.active_conversation_view().is_none(),
                "the restored terminal should remain active instead of falling back to a draft"
            );
            assert!(
                panel
                    .terminals(cx)
                    .into_iter()
                    .any(|terminal| terminal.id == terminal_id),
                "active terminal metadata should be restored into the loaded panel"
            );
        });
    }

    #[gpui::test]
    async fn test_terminal_restore_working_directory_does_not_read_leased_workspace(
        cx: &mut TestAppContext,
    ) {
        init_test(cx);
        cx.update(|cx| {
            agent::ThreadStore::init_global(cx);
            language_model::LanguageModelRegistry::test(cx);

            SettingsStore::update_global(cx, |store, cx| {
                store.update_user_settings(cx, |settings| {
                    settings
                        .terminal
                        .get_or_insert_default()
                        .project
                        .working_directory = Some(WorkingDirectory::AlwaysHome);
                });
            });
        });

        let fs = FakeFs::new(cx.executor());
        let project = Project::test(fs, [], cx).await;
        project.update(cx, |project, _cx| {
            project.mark_as_collab_for_testing();
        });
        project.read_with(cx, |project, _cx| {
            assert!(project.is_remote());
        });

        let multi_workspace =
            cx.add_window(|window, cx| MultiWorkspace::test_new(project.clone(), window, cx));
        let workspace = multi_workspace
            .read_with(cx, |multi_workspace, _cx| {
                multi_workspace.workspace().clone()
            })
            .expect("multi workspace should have an active workspace");
        let cx = &mut VisualTestContext::from_window(multi_workspace.into(), cx);
        let panel = workspace.update_in(cx, |workspace, window, cx| {
            cx.new(|cx| AgentPanel::new(workspace, window, cx))
        });

        assert_eq!(
            workspace.read_with(cx, |workspace, cx| {
                terminal_view::default_working_directory(workspace, cx)
            }),
            None
        );

        let metadata = TerminalThreadMetadata {
            terminal_id: TerminalId::new(),
            title: "Dev Server".into(),
            custom_title: None,
            created_at: Utc::now(),
            worktree_paths: project.read_with(cx, |project, cx| project.worktree_paths(cx)),
            remote_connection: None,
            working_directory: None,
        };
        assert_eq!(metadata.working_directory, None);

        let working_directory = workspace.update_in(cx, |workspace, _window, cx| {
            panel
                .read(cx)
                .terminal_restore_working_directory(&metadata, Some(workspace), cx)
        });

        assert_eq!(working_directory, None);
    }

    #[gpui::test]
    async fn test_pending_terminal_restore_prevents_initial_terminal_creation(
        cx: &mut TestAppContext,
    ) {
        let (panel, mut cx) = setup_panel(cx).await;

        panel.update_in(&mut cx, |panel, window, cx| {
            panel.last_created_entry_kind = AgentPanelEntryKind::Terminal;
            panel.pending_terminal_spawn = Some(TerminalId::new());
            panel.set_active(true, window, cx);
        });
        for _ in 0..4 {
            cx.run_until_parked();
        }

        panel.read_with(&cx, |panel, cx| {
            assert!(
                panel.terminals(cx).is_empty(),
                "activation while a terminal restore is pending should not create a second terminal"
            );
            assert!(
                panel.active_conversation_view().is_none(),
                "activation while a terminal restore is pending should not fall back to a draft"
            );
        });
    }

    #[gpui::test]
    async fn test_repeated_activation_only_creates_one_initial_terminal(cx: &mut TestAppContext) {
        let (panel, mut cx) = setup_panel(cx).await;

        panel.update_in(&mut cx, |panel, window, cx| {
            panel.last_created_entry_kind = AgentPanelEntryKind::Terminal;
            panel.set_active(true, window, cx);
            panel.set_active(true, window, cx);
        });
        for _ in 0..8 {
            cx.run_until_parked();
        }

        panel.read_with(&cx, |panel, cx| {
            assert_eq!(
                panel.terminals(cx).len(),
                1,
                "repeated activation should only enqueue one initial terminal"
            );
            assert!(
                panel.active_terminal_id().is_some(),
                "the single initial terminal should become active"
            );
        });
    }

    #[gpui::test]
    async fn test_restored_terminal_runs_init_command_once(cx: &mut TestAppContext) {
        let (panel, mut cx) = setup_panel(cx).await;
        cx.update(|_, cx| {
            let mut settings = AgentSettings::get_global(cx).clone();
            settings.terminal_init_command = Some(" claude --resume ".to_string());
            AgentSettings::override_global(settings, cx);
        });

        let metadata = TerminalThreadMetadata {
            terminal_id: TerminalId::new(),
            title: "Restored Terminal".into(),
            custom_title: None,
            created_at: Utc::now(),
            worktree_paths: WorktreePaths::from_folder_paths(&PathList::new(&[PathBuf::from(
                "/project",
            )])),
            remote_connection: None,
            working_directory: None,
        };
        let terminal_id = metadata.terminal_id;
        panel
            .update_in(&mut cx, |panel, window, cx| {
                panel.restore_test_terminal(
                    metadata.clone(),
                    true,
                    AgentThreadSource::AgentPanel,
                    None,
                    window,
                    cx,
                )
            })
            .expect("test terminal should be restored");
        cx.run_until_parked();

        let terminal = panel.read_with(&cx, |panel, cx| {
            panel
                .terminals
                .get(&terminal_id)
                .expect("terminal should exist")
                .view
                .read(cx)
                .terminal()
                .clone()
        });
        let input_log = terminal.update(&mut cx, |terminal, _| terminal.take_input_log());
        assert_eq!(input_log, vec![b" claude --resume \r".to_vec()]);
        assert!(
            !terminal.read_with(&cx, |terminal, _| terminal.keyboard_input_sent()),
            "writing the init command must not mark the terminal as having received \
             user keyboard input, otherwise a shell that fails to spawn would be \
             auto-closed before the user can see the error"
        );

        panel
            .update_in(&mut cx, |panel, window, cx| {
                panel.restore_test_terminal(
                    metadata,
                    true,
                    AgentThreadSource::AgentPanel,
                    None,
                    window,
                    cx,
                )
            })
            .expect("restoring an existing test terminal should succeed");
        cx.run_until_parked();

        let input_log = terminal.update(&mut cx, |terminal, _| terminal.take_input_log());
        assert!(
            input_log.is_empty(),
            "activating an already-restored terminal should not re-run the init command, got {input_log:?}"
        );
    }

    /// Exercises the real `spawn_terminal` path with a genuine shell PTY (not the
    /// display-only test terminal, where `write_to_pty` is a no-op) to verify the
    /// init command is actually delivered to the shell and executed.
    #[cfg(unix)]
    #[gpui::test]
    async fn test_spawn_terminal_runs_init_command_in_real_shell(cx: &mut TestAppContext) {
        let (panel, mut cx) = setup_panel(cx).await;
        cx.executor().allow_parking();
        cx.update(|_, cx| {
            let mut settings = AgentSettings::get_global(cx).clone();
            // `init_ran_42` is the command's output, not its echoed text, so finding
            // it proves the shell executed the command rather than just echoing it.
            settings.terminal_init_command = Some("printf 'init_ran_%s\\n' 42".to_string());
            AgentSettings::override_global(settings, cx);

            // Force a known POSIX shell so the test doesn't depend on the developer's login shell.
            let mut terminal_settings = TerminalSettings::get_global(cx).clone();
            terminal_settings.shell = task::Shell::Program("/bin/sh".to_string());
            TerminalSettings::override_global(terminal_settings, cx);
        });

        let terminal_id = TerminalId::new();
        panel.update_in(&mut cx, |panel, window, cx| {
            panel.spawn_terminal(
                terminal_id,
                // No working directory: the FakeFs project path doesn't exist on
                // the real filesystem the shell process runs against.
                None,
                None,
                None,
                None,
                true,
                true,
                true,
                AgentThreadSource::AgentPanel,
                window,
                cx,
            );
        });

        // The shell spawns on a background thread and produces output
        // asynchronously, so poll (with a deadline) rather than using a fixed
        // sleep, matching the real-PTY test in `acp_thread`.
        let deadline = Instant::now() + Duration::from_secs(10);
        let terminal = loop {
            cx.run_until_parked();
            let terminal = panel.read_with(&cx, |panel, cx| {
                panel
                    .terminals
                    .get(&terminal_id)
                    .map(|terminal| terminal.view.read(cx).terminal().clone())
            });
            if let Some(terminal) = &terminal
                && terminal
                    .read_with(&cx, |terminal, _| terminal.get_content())
                    .contains("init_ran_42")
            {
                break terminal.clone();
            }
            if Instant::now() >= deadline {
                let terminal_created = terminal.is_some();
                let (content, input_log) = if let Some(terminal) = terminal {
                    let content = terminal.read_with(&cx, |terminal, _| terminal.get_content());
                    let input_log =
                        terminal.update(&mut cx, |terminal, _| terminal.take_input_log());
                    (content, input_log)
                } else {
                    (String::new(), Vec::new())
                };
                panic!(
                    "init command output never appeared in the terminal; terminal_created={terminal_created}, content={content:?}, input_log={input_log:?}"
                );
            }
            cx.executor().timer(Duration::from_millis(50)).await;
        };

        let input_log = terminal.update(&mut cx, |terminal, _| terminal.take_input_log());
        assert_eq!(
            input_log,
            vec![b"printf 'init_ran_%s\\n' 42\r".to_vec()],
            "init command should be written only after terminal startup has settled"
        );
        assert!(
            !terminal.read_with(&cx, |terminal, _| terminal.keyboard_input_sent()),
            "writing the init command must not mark the terminal as having received \
             user keyboard input"
        );
    }

    #[gpui::test]
    async fn test_restored_terminal_does_not_update_global_entry_kind(cx: &mut TestAppContext) {
        let (panel, mut cx) = setup_panel(cx).await;
        cx.update(|_, cx| {
            TerminalThreadMetadataStore::init_global(cx);
        });

        panel.update_in(&mut cx, |panel, window, cx| {
            panel.activate_new_thread(false, AgentThreadSource::AgentPanel, window, cx);
        });
        cx.run_until_parked();
        cx.update(|_, cx| {
            assert_eq!(
                read_global_last_created_entry_kind(&KeyValueStore::global(cx)),
                Some(AgentPanelEntryKind::Thread)
            );
        });

        let metadata = TerminalThreadMetadata {
            terminal_id: TerminalId::new(),
            title: "Restored Terminal".into(),
            custom_title: None,
            created_at: Utc::now(),
            worktree_paths: WorktreePaths::from_folder_paths(&PathList::new(&[PathBuf::from(
                "/project",
            )])),
            remote_connection: None,
            working_directory: None,
        };
        panel
            .update_in(&mut cx, |panel, window, cx| {
                panel.restore_test_terminal(
                    metadata,
                    true,
                    AgentThreadSource::AgentPanel,
                    None,
                    window,
                    cx,
                )
            })
            .expect("test terminal should be restored");
        cx.run_until_parked();

        cx.update(|_, cx| {
            assert_eq!(
                read_global_last_created_entry_kind(&KeyValueStore::global(cx)),
                Some(AgentPanelEntryKind::Thread),
                "restoring a terminal should not change the global new-entry default"
            );
        });
    }

    #[gpui::test]
    async fn test_new_workspace_load_uses_global_terminal_entry_kind(cx: &mut TestAppContext) {
        init_test(cx);
        cx.update(|cx| {
            agent::ThreadStore::init_global(cx);
            TerminalThreadMetadataStore::init_global(cx);
            language_model::LanguageModelRegistry::test(cx);
        });

        let fs = FakeFs::new(cx.executor());
        fs.insert_tree("/project-a", json!({ "file.txt": "" }))
            .await;
        fs.insert_tree("/project-b", json!({ "file.txt": "" }))
            .await;
        cx.update(|cx| <dyn fs::Fs>::set_global(fs.clone(), cx));

        let project_a = Project::test(fs.clone(), [Path::new("/project-a")], cx).await;
        let project_b = Project::test(fs.clone(), [Path::new("/project-b")], cx).await;
        let multi_workspace =
            cx.add_window(|window, cx| MultiWorkspace::test_new(project_a.clone(), window, cx));
        let multi_workspace_entity = multi_workspace.root(cx).unwrap();
        let workspace_a = multi_workspace
            .read_with(cx, |multi_workspace, _cx| {
                multi_workspace.workspace().clone()
            })
            .unwrap();
        workspace_a.update(cx, |workspace, _cx| {
            workspace.set_random_database_id();
        });

        let cx = &mut VisualTestContext::from_window(multi_workspace.into(), cx);
        let panel_a = workspace_a.update_in(cx, |workspace, window, cx| {
            cx.new(|cx| AgentPanel::new(workspace, window, cx))
        });
        panel_a
            .update_in(cx, |panel, window, cx| {
                panel.insert_test_terminal("Dev Server", true, window, cx)
            })
            .expect("test terminal should be inserted");
        cx.run_until_parked();

        cx.update(|_window, cx| {
            assert_eq!(
                read_global_last_created_entry_kind(&KeyValueStore::global(cx)),
                Some(AgentPanelEntryKind::Terminal)
            );
        });

        let workspace_b = multi_workspace_entity.update_in(cx, |multi_workspace, window, cx| {
            multi_workspace.test_add_workspace(project_b.clone(), window, cx)
        });
        workspace_b.update(cx, |workspace, _cx| {
            workspace.set_random_database_id();
        });

        let async_cx = cx.update(|window, cx| window.to_async(cx));
        let loaded = AgentPanel::load(workspace_b.downgrade(), async_cx)
            .await
            .expect("panel load should succeed");
        workspace_b.update_in(cx, |workspace, window, cx| {
            workspace.add_panel(loaded.clone(), window, cx);
        });
        loaded.update_in(cx, |panel, window, cx| {
            panel.set_active(true, window, cx);
        });
        for _ in 0..8 {
            cx.run_until_parked();
        }

        loaded.read_with(cx, |panel, cx| {
            assert!(
                panel.active_terminal_id().is_some(),
                "new workspace should initialize to a terminal when terminal was the globally last used entry kind"
            );
            assert!(
                panel.active_conversation_view().is_none(),
                "new workspace should not initialize to a draft when terminal is the global entry kind"
            );
            assert!(panel.should_create_terminal_for_new_entry(cx));
        });
    }

    #[gpui::test]
    async fn test_non_native_thread_without_metadata_is_not_restored(cx: &mut TestAppContext) {
        init_test(cx);
        cx.update(|cx| {
            agent::ThreadStore::init_global(cx);
            language_model::LanguageModelRegistry::test(cx);
        });

        let fs = FakeFs::new(cx.executor());
        let project = Project::test(fs, [], cx).await;

        let multi_workspace =
            cx.add_window(|window, cx| MultiWorkspace::test_new(project.clone(), window, cx));

        let workspace = multi_workspace
            .read_with(cx, |multi_workspace, _cx| {
                multi_workspace.workspace().clone()
            })
            .unwrap();

        workspace.update(cx, |workspace, _cx| {
            workspace.set_random_database_id();
        });

        let cx = &mut VisualTestContext::from_window(multi_workspace.into(), cx);

        let panel = workspace.update_in(cx, |workspace, window, cx| {
            cx.new(|cx| AgentPanel::new(workspace, window, cx))
        });

        panel.update_in(cx, |panel, window, cx| {
            panel.open_external_thread_with_server(
                Rc::new(StubAgentServer::default_response()),
                window,
                cx,
            );
        });

        cx.run_until_parked();

        panel.read_with(cx, |panel, cx| {
            assert!(
                panel.active_agent_thread(cx).is_some(),
                "should have an active thread after connection"
            );
        });

        // Serialize without ever sending a message, so no thread metadata exists.
        panel.update(cx, |panel, cx| panel.serialize(cx));
        cx.run_until_parked();

        let async_cx = cx.update(|window, cx| window.to_async(cx));
        let loaded = AgentPanel::load(workspace.downgrade(), async_cx)
            .await
            .expect("panel load should succeed");
        cx.run_until_parked();

        loaded.read_with(cx, |panel, _cx| {
            assert!(
                panel.active_conversation_view().is_none(),
                "thread without metadata should not be restored; the panel should have no active thread"
            );
        });
    }

    #[gpui::test]
    async fn test_serialize_preserves_session_id_in_load_error(cx: &mut TestAppContext) {
        use crate::conversation_view::tests::FlakyAgentServer;
        use crate::thread_metadata_store::{ThreadId, ThreadMetadata};
        use chrono::Utc;
        use project::{AgentId as ProjectAgentId, WorktreePaths};

        init_test(cx);
        cx.update(|cx| {
            agent::ThreadStore::init_global(cx);
            language_model::LanguageModelRegistry::test(cx);
        });

        let fs = FakeFs::new(cx.executor());
        let project = Project::test(fs, [], cx).await;

        let multi_workspace =
            cx.add_window(|window, cx| MultiWorkspace::test_new(project.clone(), window, cx));
        let workspace = multi_workspace
            .read_with(cx, |mw, _cx| mw.workspace().clone())
            .unwrap();
        workspace.update(cx, |workspace, _cx| {
            workspace.set_random_database_id();
        });
        let workspace_id = workspace
            .read_with(cx, |workspace, _cx| workspace.database_id())
            .expect("workspace should have a database id");

        let cx = &mut VisualTestContext::from_window(multi_workspace.into(), cx);

        // Simulate a previous run that persisted metadata for this session.
        let resume_session_id = acp::SessionId::new("persistent-session");
        cx.update(|_window, cx| {
            ThreadMetadataStore::global(cx).update(cx, |store, cx| {
                store.save(
                    ThreadMetadata {
                        thread_id: ThreadId::new(),
                        session_id: Some(resume_session_id.clone()),
                        agent_id: ProjectAgentId::new("Flaky"),
                        title: Some("Persistent chat".into()),
                        title_override: None,
                        updated_at: Utc::now(),
                        created_at: Some(Utc::now()),
                        interacted_at: None,
                        worktree_paths: WorktreePaths::from_folder_paths(&PathList::default()),
                        remote_connection: None,
                        archived: false,
                    },
                    cx,
                );
            });
        });

        let panel = workspace.update_in(cx, |workspace, window, cx| {
            cx.new(|cx| AgentPanel::new(workspace, window, cx))
        });

        // Open a restored thread using a flaky server so the initial connect
        // fails and the view lands in LoadError — mirroring the cold-start
        // race against a custom agent over SSH.
        let (server, _fail) =
            FlakyAgentServer::new(StubAgentConnection::new().with_supports_load_session(true));
        panel.update_in(cx, |panel, window, cx| {
            panel.open_restored_thread_with_server(
                Rc::new(server),
                resume_session_id.clone(),
                window,
                cx,
            );
        });
        cx.run_until_parked();

        // Sanity: the view couldn't connect, so no live AcpThread exists.
        panel.read_with(cx, |panel, cx| {
            assert!(
                panel.active_agent_thread(cx).is_none(),
                "active_agent_thread should be None while the flaky server is failing"
            );
            let conversation_view = panel
                .active_conversation_view()
                .expect("panel should still have an active ConversationView");
            assert_eq!(
                conversation_view.read(cx).root_session_id.as_ref(),
                Some(&resume_session_id),
                "ConversationView should still hold the restored session id"
            );
        });

        // Serialize while in LoadError. Before the fix this wrote
        // `session_id=None` to the KVP and permanently lost the session.
        panel.update(cx, |panel, cx| panel.serialize(cx));
        cx.run_until_parked();

        let kvp = cx.update(|_window, cx| KeyValueStore::global(cx));
        let serialized: Option<SerializedAgentPanel> = cx
            .background_spawn(async move { read_serialized_panel(workspace_id, &kvp) })
            .await;
        let serialized_session_id = serialized
            .as_ref()
            .and_then(|p| p.last_active_thread.as_ref())
            .and_then(|t| t.session_id.clone());
        assert_eq!(
            serialized_session_id,
            Some(resume_session_id.0.to_string()),
            "serialize() must preserve the restored session id even while the \
             ConversationView is in LoadError; otherwise the bug survives a \
             restart because the KVP has been wiped"
        );
    }

    /// Extracts the text from a Text content block, panicking if it's not Text.
    fn expect_text_block(block: &acp::ContentBlock) -> &str {
        match block {
            acp::ContentBlock::Text(t) => t.text.as_str(),
            other => panic!("expected Text block, got {:?}", other),
        }
    }

    /// Extracts the (text_content, uri) from a Resource content block, panicking
    /// if it's not a TextResourceContents resource.
    fn expect_resource_block(block: &acp::ContentBlock) -> (&str, &str) {
        match block {
            acp::ContentBlock::Resource(r) => match &r.resource {
                acp::EmbeddedResourceResource::TextResourceContents(t) => {
                    (t.text.as_str(), t.uri.as_str())
                }
                other => panic!("expected TextResourceContents, got {:?}", other),
            },
            other => panic!("expected Resource block, got {:?}", other),
        }
    }

    #[gpui::test]
    async fn test_draft_prompt_blocks_use_current_editor_snapshot(cx: &mut TestAppContext) {
        init_test(cx);
        cx.update(|cx| {
            agent::ThreadStore::init_global(cx);
            language_model::LanguageModelRegistry::test(cx);
        });

        let fs = FakeFs::new(cx.executor());
        fs.insert_tree("/project", json!({ "file.txt": "" })).await;
        let project = Project::test(fs.clone(), [Path::new("/project")], cx).await;

        let multi_workspace =
            cx.add_window(|window, cx| MultiWorkspace::test_new(project.clone(), window, cx));
        let workspace = multi_workspace
            .read_with(cx, |multi_workspace, _cx| {
                multi_workspace.workspace().clone()
            })
            .unwrap();
        let cx = &mut VisualTestContext::from_window(multi_workspace.into(), cx);
        let panel = workspace.update_in(cx, |workspace, window, cx| {
            let panel = cx.new(|cx| AgentPanel::new(workspace, window, cx));
            workspace.add_panel(panel.clone(), window, cx);
            panel
        });

        let _stub_connection =
            crate::test_support::set_stub_agent_connection(StubAgentConnection::new());
        panel.update_in(cx, |panel, window, cx| {
            panel.selected_agent = Agent::Stub;
            panel.activate_draft(true, AgentThreadSource::AgentPanel, window, cx);
        });
        cx.run_until_parked();

        let thread_id = active_thread_id(&panel, cx);
        let thread = panel.read_with(cx, |panel, cx| {
            panel
                .active_agent_thread(cx)
                .expect("draft thread should be active")
        });
        let message_editor = panel.read_with(cx, |panel, cx| {
            panel
                .active_thread_view(cx)
                .expect("draft thread view should be active")
                .read(cx)
                .message_editor
                .clone()
        });

        thread.update(cx, |thread, cx| {
            thread.set_draft_prompt(
                Some(vec![acp::ContentBlock::Text(acp::TextContent::new(
                    "stale prompt",
                ))]),
                cx,
            );
        });
        message_editor.update_in(cx, |editor, window, cx| {
            editor.set_text("fresh prompt", window, cx);
        });
        let blocks = panel.read_with(cx, |panel, cx| {
            panel
                .draft_prompt_blocks_if_in_memory(thread_id, cx)
                .expect("draft should be in memory")
        });
        assert_eq!(blocks.len(), 1);
        assert_eq!(expect_text_block(&blocks[0]), "fresh prompt");

        thread.update(cx, |thread, cx| {
            thread.set_draft_prompt(
                Some(vec![acp::ContentBlock::Text(acp::TextContent::new(
                    "stale prompt after clear",
                ))]),
                cx,
            );
        });
        message_editor.update_in(cx, |editor, window, cx| {
            editor.set_text("", window, cx);
        });
        let blocks = panel.read_with(cx, |panel, cx| {
            panel
                .draft_prompt_blocks_if_in_memory(thread_id, cx)
                .expect("draft should be in memory")
        });
        assert!(
            blocks.is_empty(),
            "cleared editor snapshot should override stale saved draft prompt"
        );
    }

    #[gpui::test]
    async fn test_draft_has_user_content_checks_all_live_copies(cx: &mut TestAppContext) {
        init_test(cx);
        cx.update(|cx| {
            agent::ThreadStore::init_global(cx);
            language_model::LanguageModelRegistry::test(cx);
        });

        let fs = FakeFs::new(cx.executor());
        fs.insert_tree("/project_a", json!({ "file.txt": "" }))
            .await;
        fs.insert_tree("/project_b", json!({ "file.txt": "" }))
            .await;
        let project_a = Project::test(fs.clone(), [Path::new("/project_a")], cx).await;
        let project_b = Project::test(fs.clone(), [Path::new("/project_b")], cx).await;

        let multi_workspace =
            cx.add_window(|window, cx| MultiWorkspace::test_new(project_a.clone(), window, cx));
        let workspace_a = multi_workspace
            .read_with(cx, |multi_workspace, _cx| {
                multi_workspace.workspace().clone()
            })
            .unwrap();
        let workspace_b = multi_workspace
            .update(cx, |multi_workspace, window, cx| {
                multi_workspace.test_add_workspace(project_b.clone(), window, cx)
            })
            .unwrap();

        let cx = &mut VisualTestContext::from_window(multi_workspace.into(), cx);
        let panel_a = workspace_a.update_in(cx, |workspace, window, cx| {
            let panel = cx.new(|cx| AgentPanel::new(workspace, window, cx));
            workspace.add_panel(panel.clone(), window, cx);
            panel
        });
        let panel_b = workspace_b.update_in(cx, |workspace, window, cx| {
            let panel = cx.new(|cx| AgentPanel::new(workspace, window, cx));
            workspace.add_panel(panel.clone(), window, cx);
            panel
        });

        let _stub_connection =
            crate::test_support::set_stub_agent_connection(StubAgentConnection::new());
        panel_a.update_in(cx, |panel, window, cx| {
            panel.selected_agent = Agent::Stub;
            panel.activate_draft(true, AgentThreadSource::AgentPanel, window, cx);
        });
        cx.run_until_parked();
        let thread_id = active_thread_id(&panel_a, cx);

        panel_b.update_in(cx, |panel, window, cx| {
            panel.load_agent_thread(
                Agent::Stub,
                thread_id,
                Some(PathList::new(&[PathBuf::from("/project_b")])),
                None,
                false,
                AgentThreadSource::AgentPanel,
                window,
                cx,
            );
        });
        cx.run_until_parked();

        crate::test_support::type_draft_prompt(&panel_b, "content in second panel", cx);
        let panel_a_blocks = panel_a.read_with(cx, |panel, cx| {
            panel
                .draft_prompt_blocks_if_in_memory(thread_id, cx)
                .expect("draft should be live in first panel")
        });
        assert!(
            panel_a_blocks.is_empty(),
            "first live draft copy should be empty"
        );

        let has_user_content = cx.update(|_, cx| {
            crate::draft_prompt_store::draft_has_user_content(
                thread_id,
                [&workspace_a, &workspace_b],
                cx,
            )
        });
        assert!(
            has_user_content,
            "a later live draft copy with content should keep the draft"
        );
    }

    #[test]
    fn test_build_conflict_resolution_prompt_single_conflict() {
        let conflicts = vec![ConflictContent {
            file_path: "src/main.rs".to_string(),
            conflict_text: "<<<<<<< HEAD\nlet x = 1;\n=======\nlet x = 2;\n>>>>>>> feature"
                .to_string(),
            ours_branch_name: "HEAD".to_string(),
            theirs_branch_name: "feature".to_string(),
        }];

        let blocks = build_conflict_resolution_prompt(&conflicts);
        // 2 Text blocks + 1 ResourceLink + 1 Resource for the conflict
        assert_eq!(
            blocks.len(),
            4,
            "expected 2 text + 1 resource link + 1 resource block"
        );

        let intro_text = expect_text_block(&blocks[0]);
        assert!(
            intro_text.contains("Please resolve the following merge conflict in"),
            "prompt should include single-conflict intro text"
        );

        match &blocks[1] {
            acp::ContentBlock::ResourceLink(link) => {
                assert!(
                    link.uri.contains("file://"),
                    "resource link URI should use file scheme"
                );
                assert!(
                    link.uri.contains("main.rs"),
                    "resource link URI should reference file path"
                );
            }
            other => panic!("expected ResourceLink block, got {:?}", other),
        }

        let body_text = expect_text_block(&blocks[2]);
        assert!(
            body_text.contains("`HEAD` (ours)"),
            "prompt should mention ours branch"
        );
        assert!(
            body_text.contains("`feature` (theirs)"),
            "prompt should mention theirs branch"
        );
        assert!(
            body_text.contains("editing the file directly"),
            "prompt should instruct the agent to edit the file"
        );

        let (resource_text, resource_uri) = expect_resource_block(&blocks[3]);
        assert!(
            resource_text.contains("<<<<<<< HEAD"),
            "resource should contain the conflict text"
        );
        assert!(
            resource_uri.contains("merge-conflict"),
            "resource URI should use the merge-conflict scheme"
        );
        assert!(
            resource_uri.contains("main.rs"),
            "resource URI should reference the file path"
        );
    }

    #[test]
    fn test_build_conflict_resolution_prompt_multiple_conflicts_same_file() {
        let conflicts = vec![
            ConflictContent {
                file_path: "src/lib.rs".to_string(),
                conflict_text: "<<<<<<< main\nfn a() {}\n=======\nfn a_v2() {}\n>>>>>>> dev"
                    .to_string(),
                ours_branch_name: "main".to_string(),
                theirs_branch_name: "dev".to_string(),
            },
            ConflictContent {
                file_path: "src/lib.rs".to_string(),
                conflict_text: "<<<<<<< main\nfn b() {}\n=======\nfn b_v2() {}\n>>>>>>> dev"
                    .to_string(),
                ours_branch_name: "main".to_string(),
                theirs_branch_name: "dev".to_string(),
            },
        ];

        let blocks = build_conflict_resolution_prompt(&conflicts);
        // 1 Text instruction + 2 Resource blocks
        assert_eq!(blocks.len(), 3, "expected 1 text + 2 resource blocks");

        let text = expect_text_block(&blocks[0]);
        assert!(
            text.contains("all 2 merge conflicts"),
            "prompt should mention the total count"
        );
        assert!(
            text.contains("`main` (ours)"),
            "prompt should mention ours branch"
        );
        assert!(
            text.contains("`dev` (theirs)"),
            "prompt should mention theirs branch"
        );
        // Single file, so "file" not "files"
        assert!(
            text.contains("file directly"),
            "single file should use singular 'file'"
        );

        let (resource_a, _) = expect_resource_block(&blocks[1]);
        let (resource_b, _) = expect_resource_block(&blocks[2]);
        assert!(
            resource_a.contains("fn a()"),
            "first resource should contain first conflict"
        );
        assert!(
            resource_b.contains("fn b()"),
            "second resource should contain second conflict"
        );
    }

    #[test]
    fn test_build_conflict_resolution_prompt_multiple_conflicts_different_files() {
        let conflicts = vec![
            ConflictContent {
                file_path: "src/a.rs".to_string(),
                conflict_text: "<<<<<<< main\nA\n=======\nB\n>>>>>>> dev".to_string(),
                ours_branch_name: "main".to_string(),
                theirs_branch_name: "dev".to_string(),
            },
            ConflictContent {
                file_path: "src/b.rs".to_string(),
                conflict_text: "<<<<<<< main\nC\n=======\nD\n>>>>>>> dev".to_string(),
                ours_branch_name: "main".to_string(),
                theirs_branch_name: "dev".to_string(),
            },
        ];

        let blocks = build_conflict_resolution_prompt(&conflicts);
        // 1 Text instruction + 2 Resource blocks
        assert_eq!(blocks.len(), 3, "expected 1 text + 2 resource blocks");

        let text = expect_text_block(&blocks[0]);
        assert!(
            text.contains("files directly"),
            "multiple files should use plural 'files'"
        );

        let (_, uri_a) = expect_resource_block(&blocks[1]);
        let (_, uri_b) = expect_resource_block(&blocks[2]);
        assert!(
            uri_a.contains("a.rs"),
            "first resource URI should reference a.rs"
        );
        assert!(
            uri_b.contains("b.rs"),
            "second resource URI should reference b.rs"
        );
    }

    #[test]
    fn test_build_conflicted_files_resolution_prompt_file_paths_only() {
        let file_paths = vec![
            "src/main.rs".to_string(),
            "src/lib.rs".to_string(),
            "tests/integration.rs".to_string(),
        ];

        let blocks = build_conflicted_files_resolution_prompt(&file_paths);
        // 1 instruction Text block + (ResourceLink + newline Text) per file
        assert_eq!(
            blocks.len(),
            1 + (file_paths.len() * 2),
            "expected instruction text plus resource links and separators"
        );

        let text = expect_text_block(&blocks[0]);
        assert!(
            text.contains("unresolved merge conflicts"),
            "prompt should describe the task"
        );
        assert!(
            text.contains("conflict markers"),
            "prompt should mention conflict markers"
        );

        for (index, path) in file_paths.iter().enumerate() {
            let link_index = 1 + (index * 2);
            let newline_index = link_index + 1;

            match &blocks[link_index] {
                acp::ContentBlock::ResourceLink(link) => {
                    assert!(
                        link.uri.contains("file://"),
                        "resource link URI should use file scheme"
                    );
                    assert!(
                        link.uri.contains(path),
                        "resource link URI should reference file path: {path}"
                    );
                }
                other => panic!(
                    "expected ResourceLink block at index {}, got {:?}",
                    link_index, other
                ),
            }

            let separator = expect_text_block(&blocks[newline_index]);
            assert_eq!(
                separator, "\n",
                "expected newline separator after each file"
            );
        }
    }

    #[test]
    fn test_build_conflict_resolution_prompt_empty_conflicts() {
        let blocks = build_conflict_resolution_prompt(&[]);
        assert!(
            blocks.is_empty(),
            "empty conflicts should produce no blocks, got {} blocks",
            blocks.len()
        );
    }

    #[test]
    fn test_build_conflicted_files_resolution_prompt_empty_paths() {
        let blocks = build_conflicted_files_resolution_prompt(&[]);
        assert!(
            blocks.is_empty(),
            "empty paths should produce no blocks, got {} blocks",
            blocks.len()
        );
    }

    #[test]
    fn test_conflict_resource_block_structure() {
        let conflict = ConflictContent {
            file_path: "src/utils.rs".to_string(),
            conflict_text: "<<<<<<< HEAD\nold code\n=======\nnew code\n>>>>>>> branch".to_string(),
            ours_branch_name: "HEAD".to_string(),
            theirs_branch_name: "branch".to_string(),
        };

        let block = conflict_resource_block(&conflict);
        let (text, uri) = expect_resource_block(&block);

        assert_eq!(
            text, conflict.conflict_text,
            "resource text should be the raw conflict"
        );
        assert!(
            uri.starts_with("zed:///agent/merge-conflict"),
            "URI should use the zed merge-conflict scheme, got: {uri}"
        );
        assert!(uri.contains("utils.rs"), "URI should encode the file path");
    }

    fn open_generating_thread_with_loadable_connection(
        panel: &Entity<AgentPanel>,
        connection: &StubAgentConnection,
        cx: &mut VisualTestContext,
    ) -> (acp::SessionId, ThreadId) {
        open_thread_with_custom_connection(panel, connection.clone(), cx);
        let session_id = active_session_id(panel, cx);
        let thread_id = active_thread_id(panel, cx);
        send_message(panel, cx);
        cx.update(|_, cx| {
            connection.send_update(
                session_id.clone(),
                acp::SessionUpdate::AgentMessageChunk(acp::ContentChunk::new("done".into())),
                cx,
            );
        });
        cx.run_until_parked();
        (session_id, thread_id)
    }

    fn open_idle_thread_with_non_loadable_connection(
        panel: &Entity<AgentPanel>,
        connection: &StubAgentConnection,
        cx: &mut VisualTestContext,
    ) -> (acp::SessionId, ThreadId) {
        open_thread_with_custom_connection(panel, connection.clone(), cx);
        let session_id = active_session_id(panel, cx);
        let thread_id = active_thread_id(panel, cx);

        connection.set_next_prompt_updates(vec![acp::SessionUpdate::AgentMessageChunk(
            acp::ContentChunk::new("done".into()),
        )]);
        send_message(panel, cx);

        (session_id, thread_id)
    }

    #[gpui::test]
    async fn test_draft_promotion_creates_metadata_and_new_session_on_reload(
        cx: &mut TestAppContext,
    ) {
        init_test(cx);
        cx.update(|cx| {
            agent::ThreadStore::init_global(cx);
            language_model::LanguageModelRegistry::test(cx);
        });

        let fs = FakeFs::new(cx.executor());
        fs.insert_tree("/project", json!({ "file.txt": "" })).await;
        let project = Project::test(fs.clone(), [Path::new("/project")], cx).await;

        let multi_workspace =
            cx.add_window(|window, cx| MultiWorkspace::test_new(project.clone(), window, cx));

        let workspace = multi_workspace
            .read_with(cx, |mw, _cx| mw.workspace().clone())
            .unwrap();

        workspace.update(cx, |workspace, _cx| {
            workspace.set_random_database_id();
        });

        let cx = &mut VisualTestContext::from_window(multi_workspace.into(), cx);

        let panel = workspace.update_in(cx, |workspace, window, cx| {
            let panel = cx.new(|cx| AgentPanel::new(workspace, window, cx));
            workspace.add_panel(panel.clone(), window, cx);
            panel
        });

        // Register a shared stub connection and use Agent::Stub so the draft
        // (and any reloaded draft) uses it.
        let stub_connection =
            crate::test_support::set_stub_agent_connection(StubAgentConnection::new());
        stub_connection.set_next_prompt_updates(vec![acp::SessionUpdate::AgentMessageChunk(
            acp::ContentChunk::new("Response".into()),
        )]);
        panel.update_in(cx, |panel, window, cx| {
            panel.selected_agent = Agent::Stub;
            panel.activate_draft(true, AgentThreadSource::AgentPanel, window, cx);
        });
        cx.run_until_parked();

        // Verify the thread is considered a draft.
        panel.read_with(cx, |panel, cx| {
            assert!(
                panel.active_thread_is_draft(cx),
                "thread should be a draft before any message is sent"
            );
            assert!(
                panel.draft_thread.is_some(),
                "draft_thread field should be set"
            );
        });
        let draft_session_id = active_session_id(&panel, cx);
        let thread_id = active_thread_id(&panel, cx);

        // A draft thread is persisted with session_id: None.
        cx.update(|_window, cx| {
            let store = ThreadMetadataStore::global(cx).read(cx);
            let entry = store
                .entry(thread_id)
                .expect("draft thread should have a metadata row");
            assert!(
                entry.is_draft(),
                "draft thread metadata should have session_id=None, got {:?}",
                entry.session_id,
            );
        });

        // Type into the message editor; the editor observer pushes the text
        // into `AcpThread.draft_prompt`, which emits `PromptUpdated` and
        // persists the prompt to the kvp store.
        crate::test_support::type_draft_prompt(&panel, "Hello from draft", cx);
        panel.update(cx, |panel, cx| panel.serialize(cx));
        cx.run_until_parked();

        let async_cx = cx.update(|window, cx| window.to_async(cx));
        let reloaded_panel = AgentPanel::load(workspace.downgrade(), async_cx)
            .await
            .expect("panel load with draft should succeed");
        cx.run_until_parked();

        reloaded_panel.read_with(cx, |panel, cx| {
            assert!(
                panel.active_thread_is_draft(cx),
                "reloaded panel should still show the draft as active"
            );
            assert!(
                panel.active_view_is_new_draft(cx),
                "reloaded draft should still occupy the new-draft slot: \
                 what's in the new-draft slot stays there across restarts, \
                 regardless of whether it's also the active view"
            );
            let active_entity = panel.active_conversation_view().map(|v| v.entity_id());
            let draft_entity = panel.draft_thread.as_ref().map(|v| v.entity_id());
            assert!(
                active_entity.is_some() && active_entity == draft_entity,
                "active view and draft slot should share a single ConversationView entity \
                 (active={active_entity:?}, draft={draft_entity:?})"
            );
        });

        // Thread identity is stable across reload — the metadata row we wrote
        // pre-reload maps back to the same ConversationView.
        let reloaded_thread_id = active_thread_id(&reloaded_panel, cx);
        assert_eq!(
            reloaded_thread_id, thread_id,
            "reloaded draft should preserve its ThreadId"
        );

        // ACP session_id is NOT preserved: drafts don't persist a session id,
        // so the reloaded ConversationView opens a fresh ACP session.
        let reloaded_session_id = active_session_id(&reloaded_panel, cx);
        assert_ne!(
            reloaded_session_id, draft_session_id,
            "reloaded draft should have a fresh ACP session ID"
        );

        let restored_text =
            reloaded_panel.read_with(cx, |panel, cx| panel.editor_text(reloaded_thread_id, cx));
        assert_eq!(
            restored_text.as_deref(),
            Some("Hello from draft"),
            "draft prompt text should be restored from the draft-prompt kvp store"
        );

        // Send a message on the reloaded panel — this promotes the draft to a
        // real thread. `ThreadId` stays the same; `session_id` is populated.
        let panel = reloaded_panel;
        let promoted_session_id = reloaded_session_id;
        send_message(&panel, cx);

        panel.read_with(cx, |panel, cx| {
            assert!(
                !panel.active_thread_is_draft(cx),
                "thread should no longer be a draft after sending a message"
            );
            assert!(
                panel.draft_thread.is_none(),
                "draft_thread should be None after promotion"
            );
            assert_eq!(
                panel.active_thread_id(cx),
                Some(thread_id),
                "same ThreadId should remain active after promotion"
            );
        });

        cx.update(|_window, cx| {
            let store = ThreadMetadataStore::global(cx).read(cx);
            let metadata = store
                .entry(thread_id)
                .expect("promoted thread should have metadata");
            assert!(
                !metadata.is_draft(),
                "promoted thread metadata should no longer be a draft"
            );
            assert_eq!(
                metadata.session_id.as_ref(),
                Some(&promoted_session_id),
                "metadata session_id should match the thread's ACP session"
            );
        });

        // Serialize the panel, then reload it again.
        panel.update(cx, |panel, cx| panel.serialize(cx));
        cx.run_until_parked();

        let async_cx = cx.update(|window, cx| window.to_async(cx));
        let loaded_panel = AgentPanel::load(workspace.downgrade(), async_cx)
            .await
            .expect("panel load should succeed");
        cx.run_until_parked();

        // The second load should restore the promoted real thread, keyed by
        // its session_id.
        loaded_panel.read_with(cx, |panel, cx| {
            let active_id = panel.active_thread_id(cx);
            assert_eq!(
                active_id,
                Some(thread_id),
                "loaded panel should restore the promoted thread"
            );
            assert!(
                !panel.active_thread_is_draft(cx),
                "restored thread should not be a draft"
            );
        });
    }

    #[gpui::test]
    async fn test_new_draft_survives_reload_when_real_thread_is_active(cx: &mut TestAppContext) {
        init_test(cx);
        cx.update(|cx| {
            agent::ThreadStore::init_global(cx);
            language_model::LanguageModelRegistry::test(cx);
        });

        let fs = FakeFs::new(cx.executor());
        fs.insert_tree("/project", json!({ "file.txt": "" })).await;
        let project = Project::test(fs.clone(), [Path::new("/project")], cx).await;

        let multi_workspace =
            cx.add_window(|window, cx| MultiWorkspace::test_new(project.clone(), window, cx));
        let workspace = multi_workspace
            .read_with(cx, |mw, _cx| mw.workspace().clone())
            .unwrap();
        workspace.update(cx, |workspace, _cx| workspace.set_random_database_id());

        let cx = &mut VisualTestContext::from_window(multi_workspace.into(), cx);
        let panel = workspace.update_in(cx, |workspace, window, cx| {
            let panel = cx.new(|cx| AgentPanel::new(workspace, window, cx));
            workspace.add_panel(panel.clone(), window, cx);
            panel
        });

        // Register a shared stub connection under `Agent::Stub` so every
        // ConversationView the panel creates in this test (including any
        // post-reload rehydrations) reaches Connected synchronously.
        let stub_connection =
            crate::test_support::set_stub_agent_connection(StubAgentConnection::new());
        stub_connection.set_next_prompt_updates(vec![acp::SessionUpdate::AgentMessageChunk(
            acp::ContentChunk::new("ok".into()),
        )]);

        // 1. Create a real thread by sending a message.
        panel.update_in(cx, |panel, window, cx| {
            panel.selected_agent = Agent::Stub;
            panel.activate_draft(true, AgentThreadSource::AgentPanel, window, cx);
        });
        cx.run_until_parked();
        crate::test_support::send_message(&panel, cx);
        let real_thread_id = crate::test_support::active_thread_id(&panel, cx);
        let real_session_id = crate::test_support::active_session_id(&panel, cx);
        cx.run_until_parked();

        // 2. Open a draft, type into it, then press Cmd-N again to
        //    park it into retained_threads as a *retained* draft.
        panel.update_in(cx, |panel, window, cx| {
            panel.activate_draft(true, AgentThreadSource::AgentPanel, window, cx);
        });
        cx.run_until_parked();
        let retained_draft_id = crate::test_support::active_thread_id(&panel, cx);
        crate::test_support::type_draft_prompt(&panel, "retained draft text", cx);

        panel.update_in(cx, |panel, window, cx| {
            panel.new_thread(&NewThread, window, cx);
        });
        cx.run_until_parked();

        // The pre-existing draft is now in retained_threads (parked),
        // and a fresh empty ephemeral new-draft is active.
        panel.read_with(cx, |panel, cx| {
            assert!(
                panel.retained_threads.contains_key(&retained_draft_id),
                "first draft with content should be parked into retained_threads"
            );
            assert_ne!(
                panel.active_thread_id(cx),
                Some(retained_draft_id),
                "active view should be a fresh ephemeral draft, not the retained one"
            );
        });

        // 3. Type into the new ephemeral draft.
        let draft_thread_id = crate::test_support::active_thread_id(&panel, cx);
        crate::test_support::type_draft_prompt(&panel, "in-flight draft text", cx);

        // Sanity-check: both drafts' text has been persisted to the kvp
        // store via the editor observer / PromptUpdated chain.
        let (ephemeral_kvp, retained_kvp) = cx.update(|_, cx| {
            (
                crate::draft_prompt_store::read(draft_thread_id, cx),
                crate::draft_prompt_store::read(retained_draft_id, cx),
            )
        });
        assert!(
            ephemeral_kvp.is_some(),
            "ephemeral draft's prompt should be in the kvp store"
        );
        assert!(
            retained_kvp.is_some(),
            "retained draft's prompt should be in the kvp store"
        );

        assert_ne!(real_thread_id, draft_thread_id);
        assert_ne!(retained_draft_id, draft_thread_id);
        panel.read_with(cx, |panel, cx| {
            assert!(
                panel.active_view_is_new_draft(cx),
                "draft should currently occupy the new-draft slot"
            );
        });

        // 4. Switch the active view back to the real thread. The ephemeral
        //    draft has content, so it gets parked into `retained_threads`
        //    immediately (the `draft_thread` slot is cleared).
        panel.update_in(cx, |panel, window, cx| {
            panel.load_agent_thread(
                Agent::Stub,
                real_thread_id,
                None,
                None,
                false,
                AgentThreadSource::AgentPanel,
                window,
                cx,
            );
        });
        cx.run_until_parked();

        panel.read_with(cx, |panel, cx| {
            assert_eq!(panel.active_thread_id(cx), Some(real_thread_id));
            assert!(!panel.active_view_is_new_draft(cx));
        });

        // 5. Serialize + reload.
        panel.update(cx, |panel, cx| panel.serialize(cx));
        cx.run_until_parked();
        let async_cx = cx.update(|window, cx| window.to_async(cx));
        let loaded_panel = AgentPanel::load(workspace.downgrade(), async_cx)
            .await
            .expect("panel load should succeed");
        cx.run_until_parked();

        // 6. The real thread is the active view on reload. The draft
        //    was parked when the user navigated away, so the draft_thread
        //    slot is empty.
        loaded_panel.read_with(cx, |panel, cx| {
            assert_eq!(
                panel.active_thread_id(cx),
                Some(real_thread_id),
                "real thread should be the active view after reload"
            );
            assert!(
                !panel.active_thread_is_draft(cx),
                "real thread is not a draft"
            );
            assert!(
                panel.draft_thread.is_none(),
                "draft_thread slot should be empty since the draft was parked on navigate-away"
            );
        });

        // 7. All three threads' metadata rows survive the reload.
        cx.update(|_window, cx| {
            let store = ThreadMetadataStore::global(cx).read(cx);
            let ephemeral_row = store
                .entry(draft_thread_id)
                .expect("ephemeral draft metadata row should survive reload");
            assert!(
                ephemeral_row.is_draft(),
                "ephemeral draft row should still be a draft"
            );
            let retained_row = store
                .entry(retained_draft_id)
                .expect("retained draft metadata row should survive reload");
            assert!(
                retained_row.is_draft(),
                "retained draft row should still be a draft"
            );
            let real_row = store
                .entry(real_thread_id)
                .expect("real thread metadata row should survive reload");
            assert_eq!(real_row.session_id.as_ref(), Some(&real_session_id));
        });

        // 8. Opening the parked draft via load_agent_thread activates
        //    a fresh ConversationView and exposes its kvp-seeded prompt
        //    text in the editor.
        loaded_panel.update_in(cx, |panel, window, cx| {
            panel.load_agent_thread(
                Agent::Stub,
                draft_thread_id,
                None,
                None,
                false,
                AgentThreadSource::AgentPanel,
                window,
                cx,
            );
        });
        cx.run_until_parked();

        let restored_ephemeral_text =
            loaded_panel.read_with(cx, |panel, cx| panel.editor_text(draft_thread_id, cx));
        assert_eq!(
            restored_ephemeral_text.as_deref(),
            Some("in-flight draft text"),
            "ephemeral draft prompt text should be restored from the kvp store"
        );

        // 9. Opening the retained draft via load_agent_thread builds a
        //    fresh ConversationView (since retained_threads was not
        //    carried across the reload) and seeds its editor from the
        //    kvp store.
        loaded_panel.update_in(cx, |panel, window, cx| {
            panel.load_agent_thread(
                Agent::Stub,
                retained_draft_id,
                None,
                None,
                false,
                AgentThreadSource::AgentPanel,
                window,
                cx,
            );
        });
        cx.run_until_parked();

        let restored_retained_text =
            loaded_panel.read_with(cx, |panel, cx| panel.editor_text(retained_draft_id, cx));
        assert_eq!(
            restored_retained_text.as_deref(),
            Some("retained draft text"),
            "retained draft prompt text should be restored from the kvp store"
        );
    }

    #[gpui::test]
    async fn test_reloaded_ephemeral_draft_preserves_original_agent(cx: &mut TestAppContext) {
        init_test(cx);
        cx.update(|cx| {
            agent::ThreadStore::init_global(cx);
            language_model::LanguageModelRegistry::test(cx);
        });

        let fs = FakeFs::new(cx.executor());
        fs.insert_tree("/project", json!({ "file.txt": "" })).await;
        let project = Project::test(fs.clone(), [Path::new("/project")], cx).await;

        let multi_workspace =
            cx.add_window(|window, cx| MultiWorkspace::test_new(project.clone(), window, cx));
        let workspace = multi_workspace
            .read_with(cx, |mw, _cx| mw.workspace().clone())
            .unwrap();
        workspace.update(cx, |workspace, _cx| workspace.set_random_database_id());

        let cx = &mut VisualTestContext::from_window(multi_workspace.into(), cx);
        let panel = workspace.update_in(cx, |workspace, window, cx| {
            let panel = cx.new(|cx| AgentPanel::new(workspace, window, cx));
            workspace.add_panel(panel.clone(), window, cx);
            panel
        });

        let _stub_connection =
            crate::test_support::set_stub_agent_connection(StubAgentConnection::new());
        panel.update_in(cx, |panel, window, cx| {
            panel.selected_agent = Agent::Stub;
            panel.activate_draft(true, AgentThreadSource::AgentPanel, window, cx);
        });
        cx.run_until_parked();

        let draft_thread_id = crate::test_support::active_thread_id(&panel, cx);
        crate::test_support::type_draft_prompt(&panel, "pinned to stub", cx);

        // Diverge `selected_agent` from the draft's bound agent before
        // serialize.
        let other_agent = Agent::Custom {
            id: "other-agent".into(),
        };
        panel.update(cx, |panel, _cx| {
            panel.selected_agent = other_agent.clone();
        });
        panel.update(cx, |panel, cx| panel.serialize(cx));
        cx.run_until_parked();

        // Sanity-check: the draft's metadata row has agent_id="stub",
        // not "other-agent".
        cx.update(|_, cx| {
            let store = ThreadMetadataStore::global(cx).read(cx);
            let row = store
                .entry(draft_thread_id)
                .expect("draft metadata row should exist");
            assert_eq!(
                row.agent_id.as_ref(),
                "stub",
                "draft metadata should retain its original agent binding"
            );
        });

        let async_cx = cx.update(|window, cx| window.to_async(cx));
        let reloaded_panel = AgentPanel::load(workspace.downgrade(), async_cx)
            .await
            .expect("panel load should succeed");
        cx.run_until_parked();

        reloaded_panel.read_with(cx, |panel, cx| {
            let draft_view = panel
                .draft_thread
                .as_ref()
                .expect("draft slot should be repopulated");
            assert_eq!(
                draft_view.read(cx).thread_id,
                draft_thread_id,
                "restored draft should have the same ThreadId"
            );
            assert_eq!(
                draft_view.read(cx).agent_key(),
                &Agent::Stub,
                "restored draft should still be bound to its original Agent::Stub, \
                 not the panel's current `selected_agent`"
            );
        });
    }

    /// `OMEGA-DELTA-0034`. An empty workspace opens the front door — and still
    /// refuses the things that genuinely need a worktree.
    ///
    /// This test used to assert the opposite, because it encoded upstream's
    /// policy: `agent_ui: Require an open project for agent panel` (#56577) put
    /// a `has_open_project` guard in front of every panel entry, and this test
    /// pinned it. Omega's front door *is* the agent, and a window with nothing
    /// to restore is by definition a window with no project, so that policy
    /// refused a composer to every new user — omega#76's exit, failing.
    ///
    /// The clauses that were still true are kept and still asserted: an
    /// external ACP agent and a terminal both need a working directory, and
    /// neither is created here.
    #[gpui::test]
    async fn test_empty_workspace_opens_the_front_door(cx: &mut TestAppContext) {
        init_test(cx);
        cx.update(|cx| {
            agent::ThreadStore::init_global(cx);
            language_model::LanguageModelRegistry::test(cx);
        });

        let fs = FakeFs::new(cx.executor());
        // The global filesystem is what a *created* thread reaches for. This
        // test used to assert that no thread was created, so it never needed
        // one; now it asserts the opposite.
        cx.update(|cx| <dyn fs::Fs>::set_global(fs.clone(), cx));
        // Still no worktree: this is the fresh-install state under test.
        let project = Project::test(fs.clone(), [], cx).await;
        let multi_workspace =
            cx.add_window(|window, cx| MultiWorkspace::test_new(project.clone(), window, cx));
        let workspace = multi_workspace
            .read_with(cx, |multi_workspace, _cx| {
                multi_workspace.workspace().clone()
            })
            .unwrap();
        let cx = &mut VisualTestContext::from_window(multi_workspace.into(), cx);

        let panel = workspace.update_in(cx, |workspace, window, cx| {
            let panel = cx.new(|cx| AgentPanel::new(workspace, window, cx));
            workspace.add_panel(panel.clone(), window, cx);
            panel
        });

        panel.read_with(cx, |panel, cx| {
            assert_eq!(
                panel.project.read(cx).visible_worktrees(cx).count(),
                0,
                "this test is about the projectless case; a worktree here \
                 would make every assertion below prove something else"
            );
            assert_ne!(
                panel
                    .connection_store()
                    .read(cx)
                    .connection_status(&Agent::NativeAgent, cx),
                crate::agent_connection_store::AgentConnectionStatus::Disconnected,
                "an empty workspace must start the native agent connection; \
                 NativeAgentServer::connect never reads the project, and \
                 refusing here is what left omega#76's front door with no \
                 composer to type into"
            );
        });

        panel.update_in(cx, |panel, window, cx| {
            panel.new_thread(&NewThread, window, cx);
        });
        cx.run_until_parked();

        panel.read_with(cx, |panel, _cx| {
            assert!(
                panel.active_conversation_view().is_some(),
                "an empty workspace must reach a thread; this is omega#76's \
                 exit — a fresh launch lands on the agent and typing starts a \
                 real thread"
            );
            assert!(
                panel.draft_thread.is_some(),
                "an empty workspace must have a draft to type into"
            );
        });

        // The parts that genuinely need a worktree still refuse. Removing
        // *these* guards would not be project-optional threads; it would be
        // threads that fail later and less legibly.
        let before_external = panel.read_with(cx, |panel, _cx| panel.selected_agent.clone());
        panel.update_in(cx, |panel, window, cx| {
            panel.new_external_agent_thread(
                &NewExternalAgentThread {
                    agent: AgentId::new("external-agent"),
                },
                window,
                cx,
            );
        });
        cx.run_until_parked();
        panel.read_with(cx, |panel, _cx| {
            assert_eq!(
                panel.selected_agent, before_external,
                "an empty workspace must not start an external ACP agent, \
                 which has no working directory to run in"
            );
        });

        cx.update(|_, cx| {
            cx.update_flags(true, vec!["agent-panel-terminal".to_string()]);
        });
        panel.update_in(cx, |panel, window, cx| {
            panel.new_terminal(None, AgentThreadSource::AgentPanel, window, cx);
        });
        cx.run_until_parked();

        panel.read_with(cx, |panel, cx| {
            assert!(
                panel.terminals(cx).is_empty(),
                "an empty workspace must not create a terminal; a terminal \
                 needs a working directory"
            );
            assert!(
                !panel.should_create_terminal_for_new_entry(cx),
                "with no project, a new entry must be a thread and not a \
                 terminal — project.supports_terminal() is true for any local \
                 project, worktree or not, so this has to be checked on the \
                 panel's own wrapper"
            );
        });
    }

    #[gpui::test]
    async fn test_add_selection_to_terminal_thread_pastes_mention(cx: &mut TestAppContext) {
        init_test(cx);
        cx.update(|cx| {
            agent::ThreadStore::init_global(cx);
            language_model::LanguageModelRegistry::test(cx);
        });

        let fs = FakeFs::new(cx.executor());
        fs.insert_tree(
            "/project",
            json!({ "file.rs": "line one\nline two\nline three\n" }),
        )
        .await;
        let project = Project::test(fs.clone(), [Path::new("/project")], cx).await;

        let multi_workspace =
            cx.add_window(|window, cx| MultiWorkspace::test_new(project.clone(), window, cx));
        let workspace = multi_workspace
            .read_with(cx, |mw, _cx| mw.workspace().clone())
            .unwrap();
        let mut cx = VisualTestContext::from_window(multi_workspace.into(), cx);

        let panel = workspace.update_in(&mut cx, |workspace, window, cx| {
            let panel = cx.new(|cx| AgentPanel::new(workspace, window, cx));
            workspace.add_panel(panel.clone(), window, cx);
            panel
        });

        // Make a terminal thread the active conversation. A display-only terminal
        // avoids spawning a real shell; its working directory is supplied directly
        // so the mention resolves relative to it. No agent is started inside it.
        let terminal_id = TerminalId::new();
        panel
            .update_in(&mut cx, |panel, window, cx| {
                panel.insert_display_only_terminal(
                    terminal_id,
                    Some(PathBuf::from("/project")),
                    Some("Terminal".into()),
                    None,
                    None,
                    true,
                    true,
                    false,
                    AgentThreadSource::AgentPanel,
                    window,
                    cx,
                )
            })
            .expect("display-only terminal should be inserted");
        cx.run_until_parked();

        panel.read_with(&cx, |panel, _cx| {
            assert_eq!(panel.active_terminal_id(), Some(terminal_id));
            assert!(panel.active_conversation_view().is_none());
        });

        // Open the file in the center pane so the selection comes from a
        // worktree-backed editor (with a project path).
        workspace
            .update_in(&mut cx, |workspace, window, cx| {
                workspace.open_paths(
                    vec![PathBuf::from("/project/file.rs")],
                    workspace::OpenOptions::default(),
                    None,
                    window,
                    cx,
                )
            })
            .await;
        cx.run_until_parked();

        let editor = workspace.update(&mut cx, |workspace, cx| {
            workspace
                .active_item(cx)
                .and_then(|item| item.act_as::<Editor>(cx))
                .expect("opened file should be an editor")
        });

        cx.focus(&editor);
        cx.run_until_parked();

        let terminal = panel.read_with(&cx, |panel, cx| {
            panel
                .terminals
                .get(&terminal_id)
                .expect("terminal should exist")
                .view
                .read(cx)
                .terminal()
                .clone()
        });
        // Drop any input the terminal may have received during setup.
        terminal.update(&mut cx, |terminal, _| {
            terminal.take_input_log();
        });

        // With only a cursor and nothing highlighted, the action is a no-op and
        // must not paste anything into the terminal.
        workspace.update_in(&mut cx, |_, window, cx| {
            window.dispatch_action(AddSelectionToThread.boxed_clone(), cx);
        });
        cx.run_until_parked();
        let pasted_without_selection =
            terminal.update(&mut cx, |terminal, _| terminal.take_input_log());
        assert!(
            pasted_without_selection.is_empty(),
            "no selection should paste nothing, got {pasted_without_selection:?}"
        );

        // Now highlight a portion of the file: from the start of line 2 into line 3.
        editor.update_in(&mut cx, |editor, window, cx| {
            editor.change_selections(Default::default(), window, cx, |selections| {
                selections.select_ranges([text::Point::new(1, 0)..text::Point::new(2, 4)]);
            });
        });
        cx.run_until_parked();

        workspace.update_in(&mut cx, |_, window, cx| {
            window.dispatch_action(AddSelectionToThread.boxed_clone(), cx);
        });
        cx.run_until_parked();

        let pasted: String = terminal
            .update(&mut cx, |terminal, _| terminal.take_input_log())
            .into_iter()
            .map(|bytes| String::from_utf8(bytes).expect("pasted bytes should be valid UTF-8"))
            .collect();

        // Lines are 1-based and inclusive; the path is presented as
        // `<rel-path>:<start>-<end>`, with a trailing space.
        assert_eq!(pasted, "file.rs:2-3 ");
    }

    async fn setup_panel(cx: &mut TestAppContext) -> (Entity<AgentPanel>, VisualTestContext) {
        init_test(cx);
        cx.update(|cx| {
            agent::ThreadStore::init_global(cx);
            language_model::LanguageModelRegistry::test(cx);
        });

        let fs = FakeFs::new(cx.executor());
        cx.update(|cx| <dyn fs::Fs>::set_global(fs.clone(), cx));
        fs.insert_tree("/project", json!({ "file.txt": "" })).await;
        let project = Project::test(fs.clone(), [Path::new("/project")], cx).await;

        let multi_workspace =
            cx.add_window(|window, cx| MultiWorkspace::test_new(project.clone(), window, cx));

        let workspace = multi_workspace
            .read_with(cx, |mw, _cx| mw.workspace().clone())
            .unwrap();

        let mut cx = VisualTestContext::from_window(multi_workspace.into(), cx);

        let panel = workspace.update_in(&mut cx, |workspace, window, cx| {
            cx.new(|cx| AgentPanel::new(workspace, window, cx))
        });

        (panel, cx)
    }

    async fn setup_visible_panel(
        cx: &mut TestAppContext,
    ) -> (Entity<AgentPanel>, VisualTestContext) {
        setup_visible_panel_with_sidebar(cx, true).await
    }

    async fn setup_visible_panel_with_sidebar(
        cx: &mut TestAppContext,
        threads_list_active: bool,
    ) -> (Entity<AgentPanel>, VisualTestContext) {
        init_test(cx);
        cx.update(|cx| {
            agent::ThreadStore::init_global(cx);
            language_model::LanguageModelRegistry::test(cx);
            AgentSettings::override_global(
                AgentSettings {
                    notify_when_agent_waiting: NotifyWhenAgentWaiting::PrimaryScreen,
                    ..AgentSettings::get_global(cx).clone()
                },
                cx,
            );
        });

        let fs = FakeFs::new(cx.executor());
        cx.update(|cx| <dyn fs::Fs>::set_global(fs.clone(), cx));
        fs.insert_tree("/project", json!({ "file.txt": "" })).await;
        let project = Project::test(fs.clone(), [Path::new("/project")], cx).await;

        let multi_workspace =
            cx.add_window(|window, cx| MultiWorkspace::test_new(project.clone(), window, cx));

        let workspace = multi_workspace
            .read_with(cx, |multi_workspace, _cx| {
                multi_workspace.workspace().clone()
            })
            .unwrap();

        let mut cx = VisualTestContext::from_window(multi_workspace.into(), cx);
        register_test_sidebar(threads_list_active, &mut cx);

        let panel = workspace.update_in(&mut cx, |workspace, window, cx| {
            let panel = cx.new(|cx| AgentPanel::new(workspace, window, cx));
            workspace.add_panel(panel.clone(), window, cx);
            workspace.focus_panel::<AgentPanel>(window, cx);
            panel
        });

        (panel, cx)
    }

    fn expected_terminal_drop_text(paths: &[PathBuf]) -> String {
        let mut text = String::new();
        for path in paths {
            text.push(' ');
            text.push_str(&shlex::try_quote(path.to_str().unwrap()).unwrap());
        }
        text.push(' ');
        text
    }

    #[gpui::test]
    async fn test_terminal_external_image_drop_writes_path(cx: &mut TestAppContext) {
        let (panel, mut cx) = setup_panel(cx).await;
        cx.update(|_, cx| {
            cx.update_flags(true, vec!["agent-panel-terminal".to_string()]);
        });

        let terminal_id = panel
            .update_in(&mut cx, |panel, window, cx| {
                panel.insert_test_terminal("Image Upload", true, window, cx)
            })
            .expect("test terminal should be inserted");
        cx.run_until_parked();

        let terminal = panel.read_with(&cx, |panel, cx| {
            panel
                .terminals
                .get(&terminal_id)
                .expect("terminal should remain in the panel")
                .view
                .read(cx)
                .terminal()
                .clone()
        });
        terminal.update(&mut cx, |terminal, _cx| terminal.take_input_log());

        let image_path = PathBuf::from("/tmp/dropped-image.png");
        panel.update_in(&mut cx, |panel, window, cx| {
            let external_paths = ExternalPaths(vec![image_path.clone()].into());
            panel.paste_external_paths_into_active_terminal(&external_paths, window, cx);
        });

        let mut input_log = terminal.update(&mut cx, |terminal, _cx| terminal.take_input_log());
        assert_eq!(input_log.len(), 1, "expected one write to the terminal");
        let written =
            String::from_utf8(input_log.remove(0)).expect("terminal write should be valid UTF-8");
        assert_eq!(
            written,
            expected_terminal_drop_text(std::slice::from_ref(&image_path))
        );
    }

    #[gpui::test]
    async fn test_terminal_external_paths_drop_handler_writes_image_path(cx: &mut TestAppContext) {
        let (panel, mut cx) = setup_panel(cx).await;
        cx.update(|_, cx| {
            cx.update_flags(true, vec!["agent-panel-terminal".to_string()]);
        });

        let terminal_id = panel
            .update_in(&mut cx, |panel, window, cx| {
                panel.insert_test_terminal("Image Upload", true, window, cx)
            })
            .expect("test terminal should be inserted");
        cx.run_until_parked();

        let terminal = panel.read_with(&cx, |panel, cx| {
            panel
                .terminals
                .get(&terminal_id)
                .expect("terminal should remain in the panel")
                .view
                .read(cx)
                .terminal()
                .clone()
        });
        terminal.update(&mut cx, |terminal, _cx| terminal.take_input_log());

        let image_path = PathBuf::from("/tmp/dropped-image.png");
        panel.update_in(&mut cx, |panel, window, cx| {
            let external_paths = ExternalPaths(vec![image_path.clone()].into());
            panel.handle_external_paths_drop(&external_paths, window, cx);
        });

        let mut input_log = terminal.update(&mut cx, |terminal, _cx| terminal.take_input_log());
        assert_eq!(input_log.len(), 1, "expected one write to the terminal");
        let written =
            String::from_utf8(input_log.remove(0)).expect("terminal write should be valid UTF-8");
        assert_eq!(
            written,
            expected_terminal_drop_text(std::slice::from_ref(&image_path))
        );
    }

    #[gpui::test]
    async fn test_external_file_drop_on_thread_does_not_paste_into_later_terminal(
        cx: &mut TestAppContext,
    ) {
        init_test(cx);
        cx.update(|cx| {
            agent::ThreadStore::init_global(cx);
            language_model::LanguageModelRegistry::test(cx);
            cx.update_flags(true, vec!["agent-panel-terminal".to_string()]);
        });

        let fs = FakeFs::new(cx.executor());
        cx.update(|cx| <dyn fs::Fs>::set_global(fs.clone(), cx));
        fs.insert_tree("/project", json!({ "file.txt": "content" }))
            .await;
        let project = Project::test(fs.clone(), [Path::new("/project")], cx).await;

        let multi_workspace =
            cx.add_window(|window, cx| MultiWorkspace::test_new(project.clone(), window, cx));
        let workspace = multi_workspace
            .read_with(cx, |multi_workspace, _cx| {
                multi_workspace.workspace().clone()
            })
            .unwrap();
        let mut cx = VisualTestContext::from_window(multi_workspace.into(), cx);

        let panel = workspace.update_in(&mut cx, |workspace, window, cx| {
            let panel = cx.new(|cx| AgentPanel::new(workspace, window, cx));
            workspace.add_panel(panel.clone(), window, cx);
            panel
        });
        open_thread_with_connection(&panel, StubAgentConnection::new(), &mut cx);
        let thread_id = active_thread_id(&panel, &cx);

        let file_path = PathBuf::from("/project/file.txt");
        panel.update_in(&mut cx, |panel, window, cx| {
            let external_paths = ExternalPaths(vec![file_path.clone()].into());
            panel.handle_external_paths_drop(&external_paths, window, cx);
        });

        let terminal_id = panel
            .update_in(&mut cx, |panel, window, cx| {
                panel.insert_test_terminal("Drop Target", true, window, cx)
            })
            .expect("test terminal should be inserted");
        let terminal = panel.read_with(&cx, |panel, cx| {
            panel
                .terminals
                .get(&terminal_id)
                .expect("terminal should remain in the panel")
                .view
                .read(cx)
                .terminal()
                .clone()
        });
        terminal.update(&mut cx, |terminal, _cx| terminal.take_input_log());

        cx.run_until_parked();

        let input_log = terminal.update(&mut cx, |terminal, _cx| terminal.take_input_log());
        assert!(
            input_log.is_empty(),
            "thread drop completion should not write to the active terminal"
        );

        let expected_uri = MentionUri::File {
            abs_path: file_path,
        }
        .to_uri()
        .to_string();
        let expected_text = format!("[@file.txt]({expected_uri}) ");
        let actual_text = panel.read_with(&cx, |panel, cx| panel.editor_text(thread_id, cx));
        assert_eq!(actual_text.as_deref(), Some(expected_text.as_str()));
    }

    #[gpui::test]
    async fn test_terminal_entry_kind_controls_new_entry(cx: &mut TestAppContext) {
        let (panel, mut cx) = setup_panel(cx).await;
        panel.read_with(&cx, |panel, cx| {
            assert!(panel.project.read(cx).supports_terminal(cx));
            assert!(!panel.should_create_terminal_for_new_entry(cx));
        });

        let terminal_id = panel
            .update_in(&mut cx, |panel, window, cx| {
                panel.insert_test_terminal("Dev Server", true, window, cx)
            })
            .expect("test terminal should be inserted");
        cx.run_until_parked();

        panel.read_with(&cx, |panel, cx| {
            assert_eq!(panel.active_terminal_id(), Some(terminal_id));
            assert!(panel.has_terminal(terminal_id));
            assert!(panel.should_create_terminal_for_new_entry(cx));
            let terminals = panel.terminals(cx);
            assert_eq!(terminals.len(), 1);
            assert_eq!(terminals[0].title.as_ref(), "Dev Server");
        });

        panel.update_in(&mut cx, |panel, window, cx| {
            panel.activate_new_thread(false, AgentThreadSource::AgentPanel, window, cx);
        });
        cx.run_until_parked();

        panel.read_with(&cx, |panel, cx| {
            assert_eq!(panel.active_terminal_id(), None);
            assert!(panel.has_terminal(terminal_id));
            assert!(!panel.should_create_terminal_for_new_entry(cx));
        });
    }

    #[gpui::test]
    async fn test_skills_menu_entry_shows_manage_skills_shortcut(cx: &mut TestAppContext) {
        init_test(cx);
        cx.update(|cx| {
            let default_key_bindings = settings::KeymapFile::load_asset_allow_partial_failure(
                "keymaps/default-macos.json",
                cx,
            )
            .unwrap();
            cx.bind_keys(default_key_bindings);
            agent::ThreadStore::init_global(cx);
            language_model::LanguageModelRegistry::test(cx);
        });

        let fs = FakeFs::new(cx.executor());
        cx.update(|cx| <dyn fs::Fs>::set_global(fs.clone(), cx));
        fs.insert_tree("/project", json!({ "file.txt": "" })).await;
        let project = Project::test(fs.clone(), [Path::new("/project")], cx).await;

        let multi_workspace =
            cx.add_window(|window, cx| MultiWorkspace::test_new(project.clone(), window, cx));
        let workspace = multi_workspace
            .read_with(cx, |multi_workspace, _cx| {
                multi_workspace.workspace().clone()
            })
            .unwrap();
        let mut cx = VisualTestContext::from_window(multi_workspace.into(), cx);

        let panel = workspace.update_in(&mut cx, |workspace, window, cx| {
            let panel = cx.new(|cx| AgentPanel::new(workspace, window, cx));
            workspace.add_panel(panel.clone(), window, cx);
            panel
        });
        open_thread_with_connection(&panel, StubAgentConnection::new(), &mut cx);
        workspace.update_in(&mut cx, |workspace, window, cx| {
            workspace.focus_panel::<AgentPanel>(window, cx);
        });
        cx.run_until_parked();

        panel.update_in(&mut cx, |panel, window, cx| {
            panel.toggle_options_menu(&ToggleOptionsMenu, window, cx);
        });
        cx.run_until_parked();

        assert!(
            cx.debug_bounds("MENU_ITEM-Skills").is_some(),
            "Skills menu item should be visible"
        );
        assert!(
            cx.debug_bounds("KEY_BINDING-l").is_some(),
            "Skills menu item should show the ManageSkills shortcut"
        );
    }

    #[gpui::test]
    async fn test_terminal_title_omits_placeholder_title(cx: &mut TestAppContext) {
        let (panel, mut cx) = setup_panel(cx).await;
        let terminal_id = panel
            .update_in(&mut cx, |panel, window, cx| {
                panel.insert_test_terminal("", true, window, cx)
            })
            .expect("test terminal should be inserted");
        cx.run_until_parked();

        panel.read_with(&cx, |panel, cx| {
            let terminals = panel.terminals(cx);
            assert_eq!(terminals.len(), 1);
            assert_eq!(terminals[0].title.as_ref(), "");
            let terminal = panel
                .terminals
                .get(&terminal_id)
                .expect("terminal should remain in the panel");
            assert_eq!(terminal.title(cx).as_ref(), "");
        });

        let terminal_view = panel.read_with(&cx, |panel, _cx| {
            panel
                .terminals
                .get(&terminal_id)
                .expect("terminal should remain in the panel")
                .view
                .clone()
        });
        let terminal_entity =
            terminal_view.read_with(&cx, |terminal_view, _cx| terminal_view.terminal().clone());
        terminal_entity.update(&mut cx, |_terminal, cx| {
            cx.emit(TerminalEvent::TitleChanged);
        });
        cx.run_until_parked();

        panel.read_with(&cx, |panel, cx| {
            let terminals = panel.terminals(cx);
            assert_eq!(terminals.len(), 1);
            assert_eq!(terminals[0].title.as_ref(), "");
            let terminal = panel
                .terminals
                .get(&terminal_id)
                .expect("terminal should remain in the panel");
            assert_eq!(terminal.title(cx).as_ref(), "");
        });

        terminal_entity.update(&mut cx, |terminal, cx| {
            terminal.breadcrumb_text = "Shell Breadcrumb".to_string();
            cx.emit(TerminalEvent::BreadcrumbsChanged);
        });
        cx.run_until_parked();

        panel.read_with(&cx, |panel, cx| {
            let terminals = panel.terminals(cx);
            assert_eq!(terminals.len(), 1);
            assert_eq!(terminals[0].title.as_ref(), "Shell Breadcrumb");
            let terminal = panel
                .terminals
                .get(&terminal_id)
                .expect("terminal should remain in the panel");
            assert_eq!(terminal.title(cx).as_ref(), "Shell Breadcrumb");
        });
    }

    #[gpui::test]
    async fn test_title_edit_affordance_matches_threads_and_terminals(cx: &mut TestAppContext) {
        let (panel, mut cx) = setup_panel(cx).await;

        panel.update_in(&mut cx, |panel, window, cx| {
            panel.activate_draft(false, AgentThreadSource::AgentPanel, window, cx);
        });
        cx.run_until_parked();

        panel.update_in(&mut cx, |panel, window, cx| {
            assert!(matches!(
                panel.visible_surface(),
                VisibleSurface::AgentThread(_)
            ));
            assert!(panel.should_show_title_edit(window, cx));
        });

        let terminal_id = panel
            .update_in(&mut cx, |panel, window, cx| {
                panel.insert_test_terminal("Dev Server", true, window, cx)
            })
            .expect("test terminal should be inserted");
        cx.run_until_parked();

        panel.update_in(&mut cx, |panel, window, cx| {
            assert!(matches!(
                panel.visible_surface(),
                VisibleSurface::Terminal(_)
            ));
            assert!(panel.should_show_title_edit(window, cx));

            panel.edit_terminal_title(terminal_id, window, cx);
            assert!(!panel.should_show_title_edit(window, cx));
        });
    }

    #[gpui::test]
    async fn test_restored_terminal_uses_metadata_title_until_shell_title_arrives(
        cx: &mut TestAppContext,
    ) {
        let (panel, mut cx) = setup_panel(cx).await;
        let terminal_id = TerminalId::new();
        let now = Utc::now();
        let metadata = TerminalThreadMetadata {
            terminal_id,
            title: "Persisted Shell Title".into(),
            custom_title: None,
            created_at: now,
            worktree_paths: WorktreePaths::from_folder_paths(&PathList::new(&[PathBuf::from(
                "/project",
            )])),
            remote_connection: None,
            working_directory: None,
        };

        panel.update_in(&mut cx, |panel, window, cx| {
            panel
                .restore_test_terminal(metadata, true, AgentThreadSource::Sidebar, None, window, cx)
                .expect("test terminal should be restored");
        });
        cx.run_until_parked();

        let terminal_view = panel.read_with(&cx, |panel, cx| {
            let terminals = panel.terminals(cx);
            assert_eq!(terminals.len(), 1);
            assert_eq!(terminals[0].title.as_ref(), "Persisted Shell Title");
            panel
                .terminals
                .get(&terminal_id)
                .expect("terminal should be restored")
                .view
                .clone()
        });

        let terminal_entity =
            terminal_view.read_with(&cx, |terminal_view, _cx| terminal_view.terminal().clone());
        terminal_entity.update(&mut cx, |terminal, cx| {
            terminal.breadcrumb_text = "Fresh Shell Title".to_string();
            cx.emit(TerminalEvent::BreadcrumbsChanged);
        });
        cx.run_until_parked();

        panel.read_with(&cx, |panel, cx| {
            let terminals = panel.terminals(cx);
            assert_eq!(terminals.len(), 1);
            assert_eq!(terminals[0].title.as_ref(), "Fresh Shell Title");
        });
    }

    #[gpui::test]
    async fn test_restored_terminal_selects_without_focusing(cx: &mut TestAppContext) {
        let (panel, mut cx) = setup_panel(cx).await;
        let terminal_id = TerminalId::new();
        let now = Utc::now();
        let metadata = TerminalThreadMetadata {
            terminal_id,
            title: "Persisted Shell Title".into(),
            custom_title: None,
            created_at: now,
            worktree_paths: WorktreePaths::from_folder_paths(&PathList::new(&[PathBuf::from(
                "/project",
            )])),
            remote_connection: None,
            working_directory: None,
        };

        panel.update_in(&mut cx, |panel, window, cx| {
            panel
                .restore_test_terminal(
                    metadata,
                    false,
                    AgentThreadSource::Sidebar,
                    None,
                    window,
                    cx,
                )
                .expect("test terminal should be restored");
        });
        cx.run_until_parked();

        panel.read_with(&cx, |panel, _cx| {
            assert_eq!(panel.active_terminal_id(), Some(terminal_id));
        });
    }

    #[gpui::test]
    async fn test_terminal_working_directory_uses_active_workspace_while_workspace_is_updating(
        cx: &mut TestAppContext,
    ) {
        let (workspace, panel, mut cx) = setup_workspace_panel(cx).await;
        panel
            .update_in(&mut cx, |panel, window, cx| {
                panel.insert_test_terminal("Dev Server", false, window, cx)
            })
            .expect("test terminal should be inserted");
        cx.run_until_parked();

        panel.read_with(&cx, |panel, cx| {
            assert_eq!(panel.last_created_entry_kind, AgentPanelEntryKind::Terminal);
            assert!(panel.should_create_terminal_for_new_entry(cx));
        });

        workspace.update_in(&mut cx, |workspace, window, cx| {
            let panel = workspace
                .panel::<AgentPanel>(cx)
                .expect("agent panel should be registered in workspace");
            panel.read_with(cx, |panel, cx| {
                panel.terminal_working_directory(Some(workspace), cx);
            });
            workspace.focus_panel::<AgentPanel>(window, cx);
        });

        panel.read_with(&cx, |panel, cx| {
            assert_eq!(panel.last_created_entry_kind, AgentPanelEntryKind::Terminal);
            assert!(panel.should_create_terminal_for_new_entry(cx));
        });
    }

    #[gpui::test]
    async fn test_terminal_title_editor_is_created_only_while_editing(cx: &mut TestAppContext) {
        let (panel, mut cx) = setup_panel(cx).await;
        let terminal_id = panel
            .update_in(&mut cx, |panel, window, cx| {
                panel.insert_test_terminal("Dev Server", true, window, cx)
            })
            .expect("test terminal should be inserted");
        cx.run_until_parked();

        panel.read_with(&cx, |panel, _cx| {
            let terminal = panel
                .terminals
                .get(&terminal_id)
                .expect("terminal should remain in the panel");
            assert!(terminal.title_editor.is_none());
        });

        panel.update(&mut cx, |panel, cx| {
            panel.refresh_terminal_metadata(terminal_id, cx);
        });
        cx.run_until_parked();

        panel.read_with(&cx, |panel, _cx| {
            let terminal = panel
                .terminals
                .get(&terminal_id)
                .expect("terminal should remain in the panel");
            assert!(terminal.title_editor.is_none());
        });

        panel.update_in(&mut cx, |panel, window, cx| {
            panel.edit_terminal_title(terminal_id, window, cx);
        });
        cx.run_until_parked();

        panel.read_with(&cx, |panel, cx| {
            let terminal = panel
                .terminals
                .get(&terminal_id)
                .expect("terminal should remain in the panel");
            let title_editor = terminal
                .title_editor
                .as_ref()
                .expect("terminal title editor should be active while editing");
            assert_eq!(title_editor.read(cx).text(cx), "Dev Server");
        });

        panel.update_in(&mut cx, |panel, window, cx| {
            panel.stop_editing_terminal_title(terminal_id, false, window, cx);
        });
        cx.run_until_parked();

        panel.read_with(&cx, |panel, _cx| {
            let terminal = panel
                .terminals
                .get(&terminal_id)
                .expect("terminal should remain in the panel");
            assert!(terminal.title_editor.is_none());
        });
    }

    #[gpui::test]
    async fn test_terminal_title_editor_does_not_set_custom_title_when_unchanged(
        cx: &mut TestAppContext,
    ) {
        let (panel, mut cx) = setup_panel(cx).await;
        let terminal_id = panel
            .update_in(&mut cx, |panel, window, cx| {
                panel.insert_test_terminal("Initial Custom Title", true, window, cx)
            })
            .expect("test terminal should be inserted");
        cx.run_until_parked();

        let terminal_view = panel.read_with(&cx, |panel, _cx| {
            panel
                .terminals
                .get(&terminal_id)
                .expect("terminal should remain in the panel")
                .view
                .clone()
        });
        terminal_view.update(&mut cx, |terminal_view, cx| {
            terminal_view.set_custom_title(None, cx);
        });
        let terminal_entity =
            terminal_view.read_with(&cx, |terminal_view, _cx| terminal_view.terminal().clone());
        terminal_entity.update(&mut cx, |terminal, cx| {
            terminal.breadcrumb_text = "Shell Breadcrumb".to_string();
            cx.emit(TerminalEvent::BreadcrumbsChanged);
        });
        cx.run_until_parked();

        panel.read_with(&cx, |panel, cx| {
            let terminals = panel.terminals(cx);
            assert_eq!(terminals.len(), 1);
            assert_eq!(terminals[0].title.as_ref(), "Shell Breadcrumb");
        });

        panel.update_in(&mut cx, |panel, window, cx| {
            panel.edit_terminal_title(terminal_id, window, cx);
        });
        cx.run_until_parked();

        let title_editor = panel.read_with(&cx, |panel, cx| {
            let terminal = panel
                .terminals
                .get(&terminal_id)
                .expect("terminal should remain in the panel");
            let title_editor = terminal
                .title_editor
                .as_ref()
                .expect("terminal title editor should be active while editing")
                .clone();
            assert_eq!(title_editor.read(cx).text(cx), "Shell Breadcrumb");
            title_editor
        });

        panel.update_in(&mut cx, |panel, window, cx| {
            panel.handle_terminal_title_editor_event(
                terminal_id,
                &title_editor,
                &editor::EditorEvent::BufferEdited,
                window,
                cx,
            );
        });
        cx.run_until_parked();

        terminal_view.read_with(&cx, |terminal_view, _cx| {
            assert!(terminal_view.custom_title().is_none());
        });

        panel.update_in(&mut cx, |panel, window, cx| {
            panel.stop_editing_terminal_title(terminal_id, false, window, cx);
        });
        terminal_entity.update(&mut cx, |terminal, cx| {
            terminal.breadcrumb_text = "Updated Shell Breadcrumb".to_string();
            cx.emit(TerminalEvent::BreadcrumbsChanged);
        });
        cx.run_until_parked();

        panel.read_with(&cx, |panel, cx| {
            let terminals = panel.terminals(cx);
            assert_eq!(terminals.len(), 1);
            assert_eq!(terminals[0].title.as_ref(), "Updated Shell Breadcrumb");
        });
    }

    #[gpui::test]
    async fn test_terminal_custom_title_recomposes_with_live_spinner(cx: &mut TestAppContext) {
        let (panel, mut cx) = setup_panel(cx).await;
        let terminal_id = panel
            .update_in(&mut cx, |panel, window, cx| {
                panel.insert_test_terminal("Fix bug", true, window, cx)
            })
            .expect("test terminal should be inserted");
        cx.run_until_parked();

        let terminal_entity = panel.read_with(&cx, |panel, _cx| {
            panel
                .terminals
                .get(&terminal_id)
                .expect("terminal should remain in the panel")
                .view
                .clone()
        });
        let terminal_entity =
            terminal_entity.read_with(&cx, |terminal_view, _cx| terminal_view.terminal().clone());

        terminal_entity.update(&mut cx, |terminal, cx| {
            terminal.breadcrumb_text = "⠋ Thinking".to_string();
            cx.emit(TerminalEvent::BreadcrumbsChanged);
        });
        cx.run_until_parked();

        panel.read_with(&cx, |panel, cx| {
            let terminals = panel.terminals(cx);
            assert_eq!(terminals.len(), 1);
            assert_eq!(terminals[0].title.as_ref(), "⠋ Fix bug");
            let metadata = panel
                .terminal_metadata(terminal_id, cx)
                .expect("terminal metadata should be available");
            assert_eq!(metadata.title.as_ref(), "⠋ Thinking");
            assert_eq!(
                metadata.custom_title.as_ref().map(|title| title.as_ref()),
                Some("Fix bug")
            );
            assert_eq!(metadata.display_title().as_ref(), "⠋ Fix bug");
        });

        terminal_entity.update(&mut cx, |terminal, cx| {
            terminal.breadcrumb_text = "⠙ Thinking".to_string();
            cx.emit(TerminalEvent::BreadcrumbsChanged);
        });
        cx.run_until_parked();

        panel.read_with(&cx, |panel, cx| {
            let terminals = panel.terminals(cx);
            assert_eq!(terminals.len(), 1);
            assert_eq!(terminals[0].title.as_ref(), "⠙ Fix bug");
            let metadata = panel
                .terminal_metadata(terminal_id, cx)
                .expect("terminal metadata should be available");
            assert_eq!(metadata.title.as_ref(), "⠙ Thinking");
            assert_eq!(metadata.display_title().as_ref(), "⠙ Fix bug");
        });

        terminal_entity.update(&mut cx, |terminal, cx| {
            terminal.breadcrumb_text = "Thinking".to_string();
            cx.emit(TerminalEvent::BreadcrumbsChanged);
        });
        cx.run_until_parked();

        panel.read_with(&cx, |panel, cx| {
            let terminals = panel.terminals(cx);
            assert_eq!(terminals.len(), 1);
            assert_eq!(terminals[0].title.as_ref(), "Fix bug");
            let metadata = panel
                .terminal_metadata(terminal_id, cx)
                .expect("terminal metadata should be available");
            assert_eq!(metadata.title.as_ref(), "Thinking");
            assert_eq!(metadata.display_title().as_ref(), "Fix bug");
        });
    }

    #[gpui::test]
    async fn test_terminal_title_editor_excludes_spinner_prefix(cx: &mut TestAppContext) {
        let (panel, mut cx) = setup_panel(cx).await;
        let terminal_id = panel
            .update_in(&mut cx, |panel, window, cx| {
                panel.insert_test_terminal("Initial Custom Title", true, window, cx)
            })
            .expect("test terminal should be inserted");
        cx.run_until_parked();

        let terminal_view = panel.read_with(&cx, |panel, _cx| {
            panel
                .terminals
                .get(&terminal_id)
                .expect("terminal should remain in the panel")
                .view
                .clone()
        });
        terminal_view.update(&mut cx, |terminal_view, cx| {
            terminal_view.set_custom_title(None, cx);
        });
        let terminal_entity =
            terminal_view.read_with(&cx, |terminal_view, _cx| terminal_view.terminal().clone());
        terminal_entity.update(&mut cx, |terminal, cx| {
            terminal.breadcrumb_text = "⠋ Thinking".to_string();
            cx.emit(TerminalEvent::BreadcrumbsChanged);
        });
        cx.run_until_parked();

        panel.update_in(&mut cx, |panel, window, cx| {
            panel.edit_terminal_title(terminal_id, window, cx);
        });
        cx.run_until_parked();

        let title_editor = panel.read_with(&cx, |panel, cx| {
            let terminal = panel
                .terminals
                .get(&terminal_id)
                .expect("terminal should remain in the panel");
            let title_editor = terminal
                .title_editor
                .as_ref()
                .expect("terminal title editor should be active while editing")
                .clone();
            assert_eq!(title_editor.read(cx).text(cx), "Thinking");
            title_editor
        });

        title_editor.update_in(&mut cx, |editor, window, cx| {
            editor.set_text("Fix bug", window, cx);
            editor.focus_handle(cx).focus(window, cx);
        });
        panel.update_in(&mut cx, |panel, window, cx| {
            panel.handle_terminal_title_editor_event(
                terminal_id,
                &title_editor,
                &editor::EditorEvent::BufferEdited,
                window,
                cx,
            );
        });
        cx.run_until_parked();

        terminal_view.read_with(&cx, |terminal_view, _cx| {
            assert_eq!(terminal_view.custom_title(), Some("Fix bug"));
        });
        panel.read_with(&cx, |panel, cx| {
            let terminals = panel.terminals(cx);
            assert_eq!(terminals.len(), 1);
            assert_eq!(terminals[0].title.as_ref(), "⠋ Fix bug");
            let metadata = panel
                .terminal_metadata(terminal_id, cx)
                .expect("terminal metadata should be available");
            assert_eq!(metadata.title.as_ref(), "⠋ Thinking");
            assert_eq!(
                metadata.custom_title.as_ref().map(|title| title.as_ref()),
                Some("Fix bug")
            );
        });

        panel.update_in(&mut cx, |panel, window, cx| {
            panel.stop_editing_terminal_title(terminal_id, false, window, cx);
            panel.edit_terminal_title(terminal_id, window, cx);
        });
        cx.run_until_parked();

        panel.read_with(&cx, |panel, cx| {
            let terminal = panel
                .terminals
                .get(&terminal_id)
                .expect("terminal should remain in the panel");
            let title_editor = terminal
                .title_editor
                .as_ref()
                .expect("terminal title editor should be active while editing");
            assert_eq!(title_editor.read(cx).text(cx), "Fix bug");
        });
    }

    #[gpui::test]
    async fn test_terminal_bell_marks_and_activation_clears_notification(cx: &mut TestAppContext) {
        let (panel, mut cx) = setup_panel(cx).await;
        let first_terminal_id = panel
            .update_in(&mut cx, |panel, window, cx| {
                panel.insert_test_terminal("Build", true, window, cx)
            })
            .expect("first test terminal should be inserted");
        let second_terminal_id = panel
            .update_in(&mut cx, |panel, window, cx| {
                panel.insert_test_terminal("Server", true, window, cx)
            })
            .expect("second test terminal should be inserted");
        cx.run_until_parked();

        panel.read_with(&cx, |panel, _cx| {
            assert_eq!(panel.active_terminal_id(), Some(second_terminal_id));
        });

        panel.update(&mut cx, |panel, cx| {
            panel.emit_test_terminal_bell(first_terminal_id, cx);
        });
        cx.run_until_parked();

        panel.read_with(&cx, |panel, cx| {
            let first_terminal = panel
                .terminals(cx)
                .into_iter()
                .find(|terminal| terminal.id == first_terminal_id)
                .expect("first terminal should remain in the panel");
            assert!(first_terminal.has_notification);
        });

        panel.update_in(&mut cx, |panel, window, cx| {
            panel.activate_terminal(first_terminal_id, true, window, cx);
        });
        cx.run_until_parked();

        panel.read_with(&cx, |panel, cx| {
            let first_terminal = panel
                .terminals(cx)
                .into_iter()
                .find(|terminal| terminal.id == first_terminal_id)
                .expect("first terminal should remain in the panel");
            assert!(!first_terminal.has_notification);
        });
    }

    #[gpui::test]
    async fn test_visible_terminal_bell_is_suppressed(cx: &mut TestAppContext) {
        let (panel, mut cx) = setup_visible_panel(cx).await;
        let terminal_id = panel
            .update_in(&mut cx, |panel, window, cx| {
                panel.insert_test_terminal("Claude", true, window, cx)
            })
            .expect("test terminal should be inserted");
        cx.run_until_parked();

        cx.update(|window, cx| {
            assert!(window.is_window_active());
            assert!(panel.read(cx).focus_handle(cx).contains_focused(window, cx));
        });

        panel.update(&mut cx, |panel, cx| {
            panel.emit_test_terminal_bell(terminal_id, cx);
        });
        cx.run_until_parked();

        panel.read_with(&cx, |panel, cx| {
            let terminal = panel
                .terminals(cx)
                .into_iter()
                .find(|terminal| terminal.id == terminal_id)
                .expect("terminal should remain in the panel");
            assert!(!terminal.has_notification);
        });
        assert!(
            cx.windows()
                .iter()
                .all(|window| window.downcast::<AgentNotification>().is_none())
        );
    }

    #[gpui::test]
    async fn test_visible_terminal_bell_is_suppressed_without_focus(cx: &mut TestAppContext) {
        let (panel, mut cx) = setup_visible_panel(cx).await;
        let terminal_id = panel
            .update_in(&mut cx, |panel, window, cx| {
                panel.insert_test_terminal("Claude", true, window, cx)
            })
            .expect("test terminal should be inserted");
        cx.run_until_parked();

        let workspace = cx.update(|window, cx| {
            window
                .root::<MultiWorkspace>()
                .flatten()
                .expect("test window should have a MultiWorkspace root")
                .read(cx)
                .workspace()
                .clone()
        });
        workspace.update_in(&mut cx, |workspace, window, cx| {
            workspace.focus_handle(cx).focus(window, cx);
        });
        cx.update(|window, cx| {
            assert!(window.is_window_active());
            assert!(workspace.read(cx).focus_handle(cx).is_focused(window));
            assert!(!panel.read(cx).focus_handle(cx).contains_focused(window, cx));
        });

        panel.update(&mut cx, |panel, cx| {
            panel.emit_test_terminal_bell(terminal_id, cx);
        });
        cx.run_until_parked();

        panel.read_with(&cx, |panel, cx| {
            let terminal = panel
                .terminals(cx)
                .into_iter()
                .find(|terminal| terminal.id == terminal_id)
                .expect("terminal should remain in the panel");
            assert!(!terminal.has_notification);
        });
        assert!(
            cx.windows()
                .iter()
                .all(|window| window.downcast::<AgentNotification>().is_none())
        );
    }

    #[gpui::test]
    async fn test_terminal_bell_marks_without_popup_when_sidebar_open(cx: &mut TestAppContext) {
        let (panel, mut cx) = setup_visible_panel(cx).await;
        let first_terminal_id = panel
            .update_in(&mut cx, |panel, window, cx| {
                panel.insert_test_terminal("Build", true, window, cx)
            })
            .expect("first test terminal should be inserted");
        let second_terminal_id = panel
            .update_in(&mut cx, |panel, window, cx| {
                panel.insert_test_terminal("Server", true, window, cx)
            })
            .expect("second test terminal should be inserted");
        cx.run_until_parked();

        panel.read_with(&cx, |panel, _cx| {
            assert_eq!(panel.active_terminal_id(), Some(second_terminal_id));
        });
        cx.update(|window, cx| {
            let multi_workspace = window
                .root::<MultiWorkspace>()
                .flatten()
                .expect("test window should have a MultiWorkspace root");
            multi_workspace.update(cx, |multi_workspace, cx| {
                multi_workspace.open_sidebar(cx);
            });
        });
        cx.run_until_parked();

        panel.update(&mut cx, |panel, cx| {
            panel.emit_test_terminal_bell(first_terminal_id, cx);
        });
        cx.run_until_parked();

        panel.read_with(&cx, |panel, cx| {
            let first_terminal = panel
                .terminals(cx)
                .into_iter()
                .find(|terminal| terminal.id == first_terminal_id)
                .expect("first terminal should remain in the panel");
            assert!(first_terminal.has_notification);
        });
        assert!(
            cx.windows()
                .iter()
                .all(|window| window.downcast::<AgentNotification>().is_none())
        );
    }

    #[gpui::test]
    async fn test_terminal_bell_notifies_when_sidebar_history_open(cx: &mut TestAppContext) {
        let (panel, mut cx) = setup_visible_panel_with_sidebar(cx, false).await;
        let first_terminal_id = panel
            .update_in(&mut cx, |panel, window, cx| {
                panel.insert_test_terminal("Build", true, window, cx)
            })
            .expect("first test terminal should be inserted");
        let second_terminal_id = panel
            .update_in(&mut cx, |panel, window, cx| {
                panel.insert_test_terminal("Server", true, window, cx)
            })
            .expect("second test terminal should be inserted");
        cx.run_until_parked();

        panel.read_with(&cx, |panel, _cx| {
            assert_eq!(panel.active_terminal_id(), Some(second_terminal_id));
        });
        cx.update(|window, cx| {
            let multi_workspace = window
                .root::<MultiWorkspace>()
                .flatten()
                .expect("test window should have a MultiWorkspace root");
            multi_workspace.update(cx, |multi_workspace, cx| {
                multi_workspace.open_sidebar(cx);
            });
        });
        cx.run_until_parked();

        panel.update(&mut cx, |panel, cx| {
            panel.emit_test_terminal_bell(first_terminal_id, cx);
        });
        cx.run_until_parked();

        panel.read_with(&cx, |panel, cx| {
            let first_terminal = panel
                .terminals(cx)
                .into_iter()
                .find(|terminal| terminal.id == first_terminal_id)
                .expect("first terminal should remain in the panel");
            assert!(first_terminal.has_notification);
        });
        cx.windows()
            .iter()
            .find_map(|window| window.downcast::<AgentNotification>())
            .expect("terminal bell should notify when the sidebar thread list is hidden");
    }

    #[gpui::test]
    async fn test_terminal_notification_dismissed_when_sidebar_opens(cx: &mut TestAppContext) {
        let (panel, mut cx) = setup_visible_panel(cx).await;
        let first_terminal_id = panel
            .update_in(&mut cx, |panel, window, cx| {
                panel.insert_test_terminal("Build", true, window, cx)
            })
            .expect("first test terminal should be inserted");
        let second_terminal_id = panel
            .update_in(&mut cx, |panel, window, cx| {
                panel.insert_test_terminal("Server", true, window, cx)
            })
            .expect("second test terminal should be inserted");
        cx.run_until_parked();

        panel.read_with(&cx, |panel, _cx| {
            assert_eq!(panel.active_terminal_id(), Some(second_terminal_id));
        });
        panel.update(&mut cx, |panel, cx| {
            panel.emit_test_terminal_bell(first_terminal_id, cx);
        });
        cx.run_until_parked();

        cx.windows()
            .iter()
            .find_map(|window| window.downcast::<AgentNotification>())
            .expect("inactive terminal bell should show a notification");

        cx.update(|window, cx| {
            let multi_workspace = window
                .root::<MultiWorkspace>()
                .flatten()
                .expect("test window should have a MultiWorkspace root");
            multi_workspace.update(cx, |multi_workspace, cx| {
                multi_workspace.open_sidebar(cx);
            });
        });
        cx.run_until_parked();

        panel.read_with(&cx, |panel, cx| {
            let first_terminal = panel
                .terminals(cx)
                .into_iter()
                .find(|terminal| terminal.id == first_terminal_id)
                .expect("first terminal should remain in the panel");
            assert!(first_terminal.has_notification);
        });
        assert!(
            cx.windows()
                .iter()
                .all(|window| window.downcast::<AgentNotification>().is_none())
        );
    }

    #[gpui::test]
    async fn test_focused_terminal_bell_notifies_when_window_inactive(cx: &mut TestAppContext) {
        let (panel, mut cx) = setup_visible_panel(cx).await;
        let terminal_id = panel
            .update_in(&mut cx, |panel, window, cx| {
                panel.insert_test_terminal("Claude", true, window, cx)
            })
            .expect("test terminal should be inserted");
        cx.run_until_parked();

        cx.update(|window, cx| {
            assert!(window.is_window_active());
            assert!(panel.read(cx).focus_handle(cx).contains_focused(window, cx));
        });
        cx.deactivate_window();
        cx.update(|window, _cx| {
            assert!(!window.is_window_active());
        });

        panel.update(&mut cx, |panel, cx| {
            panel.emit_test_terminal_bell(terminal_id, cx);
        });
        cx.run_until_parked();

        panel.read_with(&cx, |panel, cx| {
            let terminal = panel
                .terminals(cx)
                .into_iter()
                .find(|terminal| terminal.id == terminal_id)
                .expect("terminal should remain in the panel");
            assert!(terminal.has_notification);
        });
        cx.windows()
            .iter()
            .find_map(|window| window.downcast::<AgentNotification>())
            .expect("background terminal bell should show a notification");
    }

    #[gpui::test]
    async fn test_active_terminal_notification_clears_when_window_reactivates(
        cx: &mut TestAppContext,
    ) {
        let (panel, mut cx) = setup_visible_panel(cx).await;
        let terminal_id = panel
            .update_in(&mut cx, |panel, window, cx| {
                panel.insert_test_terminal("Claude", true, window, cx)
            })
            .expect("test terminal should be inserted");
        cx.run_until_parked();

        cx.deactivate_window();
        panel.update(&mut cx, |panel, cx| {
            panel.emit_test_terminal_bell(terminal_id, cx);
        });
        cx.run_until_parked();

        panel.read_with(&cx, |panel, cx| {
            let terminal = panel
                .terminals(cx)
                .into_iter()
                .find(|terminal| terminal.id == terminal_id)
                .expect("terminal should remain in the panel");
            assert!(terminal.has_notification);
        });
        cx.windows()
            .iter()
            .find_map(|window| window.downcast::<AgentNotification>())
            .expect("background terminal bell should show a notification");

        cx.update(|window, _cx| {
            window.activate_window();
        });
        cx.run_until_parked();

        panel.read_with(&cx, |panel, cx| {
            let terminal = panel
                .terminals(cx)
                .into_iter()
                .find(|terminal| terminal.id == terminal_id)
                .expect("terminal should remain in the panel");
            assert!(!terminal.has_notification);
        });
        assert!(
            cx.windows()
                .iter()
                .all(|window| window.downcast::<AgentNotification>().is_none())
        );
    }

    #[gpui::test]
    async fn test_terminal_notification_dismissed_when_active_terminal_becomes_visible(
        cx: &mut TestAppContext,
    ) {
        let (panel, mut cx) = setup_panel(cx).await;
        cx.update(|_window, cx| {
            AgentSettings::override_global(
                AgentSettings {
                    notify_when_agent_waiting: NotifyWhenAgentWaiting::PrimaryScreen,
                    ..AgentSettings::get_global(cx).clone()
                },
                cx,
            );
        });
        let terminal_id = panel
            .update_in(&mut cx, |panel, window, cx| {
                panel.insert_test_terminal("Claude", true, window, cx)
            })
            .expect("test terminal should be inserted");
        cx.run_until_parked();

        panel.update(&mut cx, |panel, cx| {
            panel.emit_test_terminal_bell(terminal_id, cx);
        });
        cx.run_until_parked();

        panel.read_with(&cx, |panel, cx| {
            let terminal = panel
                .terminals(cx)
                .into_iter()
                .find(|terminal| terminal.id == terminal_id)
                .expect("terminal should remain in the panel");
            assert!(terminal.has_notification);
        });
        cx.windows()
            .iter()
            .find_map(|window| window.downcast::<AgentNotification>())
            .expect("hidden terminal bell should show a notification");

        let workspace = cx.update(|window, cx| {
            window
                .root::<MultiWorkspace>()
                .flatten()
                .expect("test window should have a MultiWorkspace root")
                .read(cx)
                .workspace()
                .clone()
        });
        workspace.update_in(&mut cx, |workspace, window, cx| {
            workspace.add_panel(panel.clone(), window, cx);
            workspace.focus_panel::<AgentPanel>(window, cx);
        });
        cx.run_until_parked();

        panel.read_with(&cx, |panel, cx| {
            let terminal = panel
                .terminals(cx)
                .into_iter()
                .find(|terminal| terminal.id == terminal_id)
                .expect("terminal should remain in the panel");
            assert!(!terminal.has_notification);
        });
        assert!(
            cx.windows()
                .iter()
                .all(|window| window.downcast::<AgentNotification>().is_none())
        );
    }

    #[gpui::test]
    async fn test_terminal_notification_closed_when_panel_dropped(cx: &mut TestAppContext) {
        let (panel, mut cx) = setup_panel(cx).await;
        cx.update(|_window, cx| {
            AgentSettings::override_global(
                AgentSettings {
                    notify_when_agent_waiting: NotifyWhenAgentWaiting::PrimaryScreen,
                    ..AgentSettings::get_global(cx).clone()
                },
                cx,
            );
        });
        let terminal_id = panel
            .update_in(&mut cx, |panel, window, cx| {
                panel.insert_test_terminal("Claude", true, window, cx)
            })
            .expect("test terminal should be inserted");
        let weak_panel = panel.downgrade();
        cx.run_until_parked();

        panel.update(&mut cx, |panel, cx| {
            panel.emit_test_terminal_bell(terminal_id, cx);
        });
        cx.run_until_parked();

        cx.windows()
            .iter()
            .find_map(|window| window.downcast::<AgentNotification>())
            .expect("hidden terminal bell should show a notification");

        drop(panel);
        cx.update(|_window, _cx| {});
        cx.run_until_parked();

        assert!(
            !weak_panel.is_upgradable(),
            "agent panel should be released after dropping the last handle"
        );
        assert!(
            cx.windows()
                .iter()
                .all(|window| window.downcast::<AgentNotification>().is_none())
        );
    }

    #[gpui::test]
    async fn test_terminal_notification_view_activates_terminal_workspace(cx: &mut TestAppContext) {
        init_test(cx);
        cx.update(|cx| {
            agent::ThreadStore::init_global(cx);
            language_model::LanguageModelRegistry::test(cx);
            cx.update_flags(true, vec!["agent-panel-terminal".to_string()]);
            AgentSettings::override_global(
                AgentSettings {
                    notify_when_agent_waiting: NotifyWhenAgentWaiting::PrimaryScreen,
                    ..AgentSettings::get_global(cx).clone()
                },
                cx,
            );
        });

        let fs = FakeFs::new(cx.executor());
        fs.insert_tree("/project_a", json!({ "file.txt": "" }))
            .await;
        fs.insert_tree("/project_b", json!({ "file.txt": "" }))
            .await;
        let project_a = Project::test(fs.clone(), [Path::new("/project_a")], cx).await;
        let project_b = Project::test(fs, [Path::new("/project_b")], cx).await;

        let multi_workspace =
            cx.add_window(|window, cx| MultiWorkspace::test_new(project_a.clone(), window, cx));
        let workspace_a = multi_workspace
            .read_with(cx, |multi_workspace, _cx| {
                multi_workspace.workspace().clone()
            })
            .unwrap();
        let workspace_b = multi_workspace
            .update(cx, |multi_workspace, window, cx| {
                multi_workspace.test_add_workspace(project_b.clone(), window, cx)
            })
            .unwrap();

        let cx = &mut VisualTestContext::from_window(multi_workspace.into(), cx);
        let panel_a = workspace_a.update_in(cx, |workspace, window, cx| {
            let panel = cx.new(|cx| AgentPanel::new(workspace, window, cx));
            workspace.add_panel(panel.clone(), window, cx);
            panel
        });

        let first_terminal_id = panel_a
            .update_in(cx, |panel, window, cx| {
                panel.insert_test_terminal("Build", true, window, cx)
            })
            .expect("first test terminal should be inserted");
        let second_terminal_id = panel_a
            .update_in(cx, |panel, window, cx| {
                panel.insert_test_terminal("Server", true, window, cx)
            })
            .expect("second test terminal should be inserted");
        cx.run_until_parked();

        multi_workspace
            .read_with(cx, |multi_workspace, _cx| {
                assert_eq!(multi_workspace.workspace(), &workspace_b);
            })
            .unwrap();
        panel_a.read_with(cx, |panel, _cx| {
            assert_eq!(panel.active_terminal_id(), Some(second_terminal_id));
        });

        panel_a.update(cx, |panel, cx| {
            panel.emit_test_terminal_bell(first_terminal_id, cx);
        });
        cx.run_until_parked();

        let notification = cx
            .windows()
            .iter()
            .find_map(|window| window.downcast::<AgentNotification>())
            .expect("terminal bell should show a notification");
        notification
            .update(cx, |notification, _window, cx| notification.accept(cx))
            .unwrap();
        assert!(
            cx.windows()
                .iter()
                .all(|window| window.downcast::<AgentNotification>().is_none())
        );
        cx.run_until_parked();

        multi_workspace
            .read_with(cx, |multi_workspace, _cx| {
                assert_eq!(multi_workspace.workspace(), &workspace_a);
            })
            .unwrap();
        panel_a.read_with(cx, |panel, cx| {
            assert_eq!(panel.active_terminal_id(), Some(first_terminal_id));
            let first_terminal = panel
                .terminals(cx)
                .into_iter()
                .find(|terminal| terminal.id == first_terminal_id)
                .expect("first terminal should remain in the panel");
            assert!(!first_terminal.has_notification);
        });
    }

    #[gpui::test]
    async fn test_running_thread_retained_when_navigating_away(cx: &mut TestAppContext) {
        let (panel, mut cx) = setup_panel(cx).await;

        let connection_a = StubAgentConnection::new();
        open_thread_with_connection(&panel, connection_a.clone(), &mut cx);
        send_message(&panel, &mut cx);

        let session_id_a = active_session_id(&panel, &cx);
        let thread_id_a = active_thread_id(&panel, &cx);

        // Send a chunk to keep thread A generating (don't end the turn).
        cx.update(|_, cx| {
            connection_a.send_update(
                session_id_a.clone(),
                acp::SessionUpdate::AgentMessageChunk(acp::ContentChunk::new("chunk".into())),
                cx,
            );
        });
        cx.run_until_parked();

        // Verify thread A is generating.
        panel.read_with(&cx, |panel, cx| {
            let thread = panel.active_agent_thread(cx).unwrap();
            assert_eq!(thread.read(cx).status(), ThreadStatus::Generating);
            assert!(panel.retained_threads.is_empty());
        });

        // Open a new thread B — thread A should be retained in background.
        let connection_b = StubAgentConnection::new();
        open_thread_with_connection(&panel, connection_b, &mut cx);

        panel.read_with(&cx, |panel, _cx| {
            assert_eq!(
                panel.retained_threads.len(),
                1,
                "Running thread A should be retained in retained_threads"
            );
            assert!(
                panel.retained_threads.contains_key(&thread_id_a),
                "Retained thread should be keyed by thread A's thread ID"
            );
        });
    }

    #[gpui::test]
    async fn test_idle_non_loadable_thread_retained_when_navigating_away(cx: &mut TestAppContext) {
        let (panel, mut cx) = setup_panel(cx).await;

        let connection_a = StubAgentConnection::new();
        connection_a.set_next_prompt_updates(vec![acp::SessionUpdate::AgentMessageChunk(
            acp::ContentChunk::new("Response".into()),
        )]);
        open_thread_with_connection(&panel, connection_a, &mut cx);
        send_message(&panel, &mut cx);

        let weak_view_a = panel.read_with(&cx, |panel, _cx| {
            panel.active_conversation_view().unwrap().downgrade()
        });
        let thread_id_a = active_thread_id(&panel, &cx);

        // Thread A should be idle (auto-completed via set_next_prompt_updates).
        panel.read_with(&cx, |panel, cx| {
            let thread = panel.active_agent_thread(cx).unwrap();
            assert_eq!(thread.read(cx).status(), ThreadStatus::Idle);
        });

        // Open a new thread B — thread A should be retained because it is not loadable.
        let connection_b = StubAgentConnection::new();
        open_thread_with_connection(&panel, connection_b, &mut cx);

        panel.read_with(&cx, |panel, _cx| {
            assert_eq!(
                panel.retained_threads.len(),
                1,
                "Idle non-loadable thread A should be retained in retained_threads"
            );
            assert!(
                panel.retained_threads.contains_key(&thread_id_a),
                "Retained thread should be keyed by thread A's thread ID"
            );
        });

        assert!(
            weak_view_a.upgrade().is_some(),
            "Idle non-loadable ConnectionView should still be retained"
        );
    }

    #[gpui::test]
    async fn test_background_thread_promoted_via_load(cx: &mut TestAppContext) {
        let (panel, mut cx) = setup_panel(cx).await;

        let connection_a = StubAgentConnection::new();
        open_thread_with_connection(&panel, connection_a.clone(), &mut cx);
        send_message(&panel, &mut cx);

        let session_id_a = active_session_id(&panel, &cx);
        let thread_id_a = active_thread_id(&panel, &cx);

        // Keep thread A generating.
        cx.update(|_, cx| {
            connection_a.send_update(
                session_id_a.clone(),
                acp::SessionUpdate::AgentMessageChunk(acp::ContentChunk::new("chunk".into())),
                cx,
            );
        });
        cx.run_until_parked();

        // Open thread B — thread A goes to background.
        let connection_b = StubAgentConnection::new();
        open_thread_with_connection(&panel, connection_b, &mut cx);
        send_message(&panel, &mut cx);

        let thread_id_b = active_thread_id(&panel, &cx);

        panel.read_with(&cx, |panel, _cx| {
            assert_eq!(panel.retained_threads.len(), 1);
            assert!(panel.retained_threads.contains_key(&thread_id_a));
        });

        // Load thread A back via load_agent_thread — should promote from background.
        panel.update_in(&mut cx, |panel, window, cx| {
            panel.load_agent_thread(
                panel.selected_agent(cx),
                thread_id_a,
                None,
                None,
                true,
                AgentThreadSource::AgentPanel,
                window,
                cx,
            );
        });

        // Thread A should now be the active view, promoted from background.
        let active_session = active_session_id(&panel, &cx);
        assert_eq!(
            active_session, session_id_a,
            "Thread A should be the active thread after promotion"
        );

        panel.read_with(&cx, |panel, _cx| {
            assert!(
                !panel.retained_threads.contains_key(&thread_id_a),
                "Promoted thread A should no longer be in retained_threads"
            );
            assert!(
                panel.retained_threads.contains_key(&thread_id_b),
                "Thread B (idle, non-loadable) should remain retained in retained_threads"
            );
        });
    }

    #[gpui::test]
    async fn test_reopening_visible_thread_keeps_thread_usable(cx: &mut TestAppContext) {
        let (panel, mut cx) = setup_panel(cx).await;
        cx.run_until_parked();

        panel.update(&mut cx, |panel, cx| {
            panel.connection_store.update(cx, |store, cx| {
                store.restart_connection(
                    Agent::NativeAgent,
                    Rc::new(StubAgentServer::new(SessionTrackingConnection::new())),
                    cx,
                );
            });
        });
        cx.run_until_parked();

        panel.update_in(&mut cx, |panel, window, cx| {
            panel.external_thread(
                Some(Agent::NativeAgent),
                None,
                None,
                None,
                None,
                true,
                AgentThreadSource::AgentPanel,
                window,
                cx,
            );
        });
        cx.run_until_parked();
        send_message(&panel, &mut cx);

        let session_id = active_session_id(&panel, &cx);

        panel.update_in(&mut cx, |panel, window, cx| {
            panel.open_thread(session_id.clone(), None, None, window, cx);
        });
        cx.run_until_parked();

        send_message(&panel, &mut cx);

        panel.read_with(&cx, |panel, cx| {
            let active_view = panel
                .active_conversation_view()
                .expect("visible conversation should remain open after reopening");
            let connected = active_view
                .read(cx)
                .as_connected()
                .expect("visible conversation should still be connected in the UI");
            assert!(
                !connected.has_thread_error(cx),
                "reopening an already-visible session should keep the thread usable"
            );
        });
    }

    #[gpui::test]
    async fn test_initial_content_for_thread_summary_uses_own_session_id(cx: &mut TestAppContext) {
        init_test(cx);
        cx.update(|cx| {
            agent::ThreadStore::init_global(cx);
            language_model::LanguageModelRegistry::test(cx);
        });

        let source_session_id = acp::SessionId::new("source-thread-session");
        let source_title: SharedString = "Source Thread Title".into();
        let db_thread = agent::DbThread {
            title: source_title.clone(),
            messages: Vec::new(),
            updated_at: Utc::now(),
            detailed_summary: None,
            initial_project_snapshot: None,
            cumulative_token_usage: Default::default(),
            request_token_usage: HashMap::default(),
            model: None,
            profile: None,
            subagent_context: None,
            speed: None,
            thinking_enabled: false,
            thinking_effort: None,
            draft_prompt: None,
            ui_scroll_position: None,
            sandboxed_terminal_temp_dir: None,
            sandbox_grants: Default::default(),
            thread_log: Default::default(),
            fork_origin: None,
        };

        let thread_store = cx.update(|cx| ThreadStore::global(cx));
        thread_store
            .update(cx, |store, cx| {
                store.save_thread(
                    source_session_id.clone(),
                    db_thread,
                    PathList::default(),
                    cx,
                )
            })
            .await
            .expect("saving source thread should succeed");
        cx.run_until_parked();

        thread_store.read_with(cx, |store, _cx| {
            let entry = store
                .thread_from_session_id(&source_session_id)
                .expect("saved thread should be listed in the store");
            assert!(
                entry.parent_session_id.is_none(),
                "saved thread is a root thread with no parent session"
            );
        });

        let content = cx
            .update(|cx| {
                AgentPanel::initial_content_for_thread_summary(source_session_id.clone(), cx)
            })
            .expect("initial content should be produced for a root thread");

        match content {
            AgentInitialContent::ThreadSummary { session_id, title } => {
                assert_eq!(
                    session_id, source_session_id,
                    "thread-summary mention should use the source thread's own session id"
                );
                assert_eq!(title, Some(source_title.clone()));
            }
            _ => panic!("expected AgentInitialContent::ThreadSummary"),
        }

        // Unknown session ids should still produce no content.
        let missing = cx.update(|cx| {
            AgentPanel::initial_content_for_thread_summary(
                acp::SessionId::new("does-not-exist"),
                cx,
            )
        });
        assert!(
            missing.is_none(),
            "unknown session ids should not produce initial content"
        );
    }

    #[gpui::test]
    async fn test_cleanup_retained_threads_keeps_five_most_recent_idle_loadable_threads(
        cx: &mut TestAppContext,
    ) {
        let (panel, mut cx) = setup_panel(cx).await;
        let connection = StubAgentConnection::new()
            .with_supports_load_session(true)
            .with_agent_id("loadable-stub".into())
            .with_telemetry_id("loadable-stub".into());
        let mut session_ids = Vec::new();
        let mut thread_ids = Vec::new();

        for _ in 0..7 {
            let (session_id, thread_id) =
                open_generating_thread_with_loadable_connection(&panel, &connection, &mut cx);
            session_ids.push(session_id);
            thread_ids.push(thread_id);
        }

        let base_time = Instant::now();

        for session_id in session_ids.iter().take(6) {
            connection.end_turn(session_id.clone(), acp::StopReason::EndTurn);
        }
        cx.run_until_parked();

        panel.update(&mut cx, |panel, cx| {
            for (index, thread_id) in thread_ids.iter().take(6).enumerate() {
                let conversation_view = panel
                    .retained_threads
                    .get(thread_id)
                    .expect("retained thread should exist")
                    .clone();
                conversation_view.update(cx, |view, cx| {
                    view.set_updated_at(base_time + Duration::from_secs(index as u64), cx);
                });
            }
            panel.cleanup_retained_threads(cx);
        });

        panel.read_with(&cx, |panel, _cx| {
            assert_eq!(
                panel.retained_threads.len(),
                5,
                "cleanup should keep at most five idle loadable retained threads"
            );
            assert!(
                !panel.retained_threads.contains_key(&thread_ids[0]),
                "oldest idle loadable retained thread should be removed"
            );
            for thread_id in &thread_ids[1..6] {
                assert!(
                    panel.retained_threads.contains_key(thread_id),
                    "more recent idle loadable retained threads should be retained"
                );
            }
            assert!(
                !panel.retained_threads.contains_key(&thread_ids[6]),
                "the active thread should not also be stored as a retained thread"
            );
        });
    }

    #[gpui::test]
    async fn test_cleanup_retained_threads_preserves_idle_non_loadable_threads(
        cx: &mut TestAppContext,
    ) {
        let (panel, mut cx) = setup_panel(cx).await;

        let non_loadable_connection = StubAgentConnection::new();
        let (_non_loadable_session_id, non_loadable_thread_id) =
            open_idle_thread_with_non_loadable_connection(
                &panel,
                &non_loadable_connection,
                &mut cx,
            );

        let loadable_connection = StubAgentConnection::new()
            .with_supports_load_session(true)
            .with_agent_id("loadable-stub".into())
            .with_telemetry_id("loadable-stub".into());
        let mut loadable_session_ids = Vec::new();
        let mut loadable_thread_ids = Vec::new();

        for _ in 0..7 {
            let (session_id, thread_id) = open_generating_thread_with_loadable_connection(
                &panel,
                &loadable_connection,
                &mut cx,
            );
            loadable_session_ids.push(session_id);
            loadable_thread_ids.push(thread_id);
        }

        let base_time = Instant::now();

        for session_id in loadable_session_ids.iter().take(6) {
            loadable_connection.end_turn(session_id.clone(), acp::StopReason::EndTurn);
        }
        cx.run_until_parked();

        panel.update(&mut cx, |panel, cx| {
            for (index, thread_id) in loadable_thread_ids.iter().take(6).enumerate() {
                let conversation_view = panel
                    .retained_threads
                    .get(thread_id)
                    .expect("retained thread should exist")
                    .clone();
                conversation_view.update(cx, |view, cx| {
                    view.set_updated_at(base_time + Duration::from_secs(index as u64), cx);
                });
            }
            panel.cleanup_retained_threads(cx);
        });

        panel.read_with(&cx, |panel, _cx| {
            assert_eq!(
                panel.retained_threads.len(),
                6,
                "cleanup should keep the non-loadable idle thread in addition to five loadable ones"
            );
            assert!(
                panel.retained_threads.contains_key(&non_loadable_thread_id),
                "idle non-loadable retained threads should not be cleanup candidates"
            );
            assert!(
                !panel.retained_threads.contains_key(&loadable_thread_ids[0]),
                "oldest idle loadable retained thread should still be removed"
            );
            for thread_id in &loadable_thread_ids[1..6] {
                assert!(
                    panel.retained_threads.contains_key(thread_id),
                    "more recent idle loadable retained threads should be retained"
                );
            }
            assert!(
                !panel.retained_threads.contains_key(&loadable_thread_ids[6]),
                "the active loadable thread should not also be stored as a retained thread"
            );
        });
    }

    #[test]
    fn test_deserialize_agent_variants() {
        // PascalCase (legacy AgentType format, persisted in panel state)
        assert_eq!(
            serde_json::from_str::<Agent>(r#""NativeAgent""#).unwrap(),
            Agent::NativeAgent,
        );
        assert_eq!(
            serde_json::from_str::<Agent>(r#"{"Custom":{"name":"my-agent"}}"#).unwrap(),
            Agent::Custom {
                id: "my-agent".into(),
            },
        );

        // Legacy TextThread variant deserializes to NativeAgent
        assert_eq!(
            serde_json::from_str::<Agent>(r#""TextThread""#).unwrap(),
            Agent::NativeAgent,
        );

        // snake_case (canonical format)
        assert_eq!(
            serde_json::from_str::<Agent>(r#""native_agent""#).unwrap(),
            Agent::NativeAgent,
        );
        assert_eq!(
            serde_json::from_str::<Agent>(r#"{"custom":{"name":"my-agent"}}"#).unwrap(),
            Agent::Custom {
                id: "my-agent".into(),
            },
        );

        // Serialization uses snake_case
        assert_eq!(
            serde_json::to_string(&Agent::NativeAgent).unwrap(),
            r#""native_agent""#,
        );
        assert_eq!(
            serde_json::to_string(&Agent::Custom {
                id: "my-agent".into()
            })
            .unwrap(),
            r#"{"custom":{"name":"my-agent"}}"#,
        );
    }

    #[gpui::test]
    fn test_resolve_worktree_branch_target() {
        let resolved = git_ui::worktree_service::resolve_worktree_branch_target(
            &NewWorktreeBranchTarget::ExistingBranch {
                name: "feature".to_string(),
            },
        );
        assert_eq!(resolved, Some("feature".to_string()));

        let resolved = git_ui::worktree_service::resolve_worktree_branch_target(
            &NewWorktreeBranchTarget::CurrentBranch,
        );
        assert_eq!(resolved, None);

        let resolved = git_ui::worktree_service::resolve_worktree_branch_target(
            &NewWorktreeBranchTarget::RemoteBranch {
                remote_name: "origin".to_string(),
                branch_name: "main".to_string(),
            },
        );
        assert_eq!(resolved, Some("refs/remotes/origin/main".to_string()));
    }

    #[gpui::test]
    async fn test_existing_thread_work_dirs_do_not_expand_when_worktrees_change(
        cx: &mut TestAppContext,
    ) {
        use crate::thread_metadata_store::ThreadMetadataStore;

        init_test(cx);
        cx.update(|cx| {
            agent::ThreadStore::init_global(cx);
            language_model::LanguageModelRegistry::test(cx);
        });

        // Set up a project with one worktree.
        let fs = FakeFs::new(cx.executor());
        fs.insert_tree("/project_a", json!({ "file.txt": "" }))
            .await;
        let project = Project::test(fs.clone(), [Path::new("/project_a")], cx).await;

        let multi_workspace =
            cx.add_window(|window, cx| MultiWorkspace::test_new(project.clone(), window, cx));
        let workspace = multi_workspace
            .read_with(cx, |mw, _cx| mw.workspace().clone())
            .unwrap();
        let mut cx = VisualTestContext::from_window(multi_workspace.into(), cx);

        let panel = workspace.update_in(&mut cx, |workspace, window, cx| {
            cx.new(|cx| AgentPanel::new(workspace, window, cx))
        });

        // Open thread A and send a message. With empty next_prompt_updates it
        // stays generating, so opening B will move A to retained_threads.
        let connection_a = StubAgentConnection::new().with_agent_id("agent-a".into());
        open_thread_with_custom_connection(&panel, connection_a.clone(), &mut cx);
        send_message(&panel, &mut cx);
        let session_id_a = active_session_id(&panel, &cx);
        let thread_id_a = active_thread_id(&panel, &cx);

        // Open thread C — thread A (generating) moves to background.
        // Thread C completes immediately (idle), then opening B moves C to background too.
        let connection_c = StubAgentConnection::new().with_agent_id("agent-c".into());
        connection_c.set_next_prompt_updates(vec![acp::SessionUpdate::AgentMessageChunk(
            acp::ContentChunk::new("done".into()),
        )]);
        open_thread_with_custom_connection(&panel, connection_c.clone(), &mut cx);
        send_message(&panel, &mut cx);
        let thread_id_c = active_thread_id(&panel, &cx);

        // Open thread B — thread C (idle, non-loadable) is retained in background.
        let connection_b = StubAgentConnection::new().with_agent_id("agent-b".into());
        open_thread_with_custom_connection(&panel, connection_b.clone(), &mut cx);
        send_message(&panel, &mut cx);
        let session_id_b = active_session_id(&panel, &cx);
        let _thread_id_b = active_thread_id(&panel, &cx);

        let metadata_store = cx.update(|_, cx| ThreadMetadataStore::global(cx));

        panel.read_with(&cx, |panel, _cx| {
            assert!(
                panel.retained_threads.contains_key(&thread_id_a),
                "Thread A should be in retained_threads"
            );
            assert!(
                panel.retained_threads.contains_key(&thread_id_c),
                "Thread C should be in retained_threads"
            );
        });

        // Verify initial work_dirs for thread B contain only /project_a.
        let initial_b_paths = panel.read_with(&cx, |panel, cx| {
            let thread = panel.active_agent_thread(cx).unwrap();
            thread.read(cx).work_dirs().cloned().unwrap()
        });
        assert_eq!(
            initial_b_paths.ordered_paths().collect::<Vec<_>>(),
            vec![&PathBuf::from("/project_a")],
            "Thread B should initially have only /project_a"
        );

        // Now add a second worktree to the project.
        fs.insert_tree("/project_b", json!({ "other.txt": "" }))
            .await;
        let (new_tree, _) = project
            .update(&mut cx, |project, cx| {
                project.find_or_create_worktree("/project_b", true, cx)
            })
            .await
            .unwrap();
        cx.read(|cx| new_tree.read(cx).as_local().unwrap().scan_complete())
            .await;
        cx.run_until_parked();

        // Existing thread targets remain exact when an unrelated worktree is
        // added to the project.
        let updated_b_paths = panel.read_with(&cx, |panel, cx| {
            let thread = panel.active_agent_thread(cx).unwrap();
            thread.read(cx).work_dirs().cloned().unwrap()
        });
        assert_eq!(
            updated_b_paths.ordered_paths().collect::<Vec<_>>(),
            vec![&PathBuf::from("/project_a")],
            "Thread B should retain its explicit target after adding /project_b"
        );

        // Retained threads keep their own explicit target as well.
        let updated_a_paths = panel.read_with(&cx, |panel, cx| {
            let bg_view = panel.retained_threads.get(&thread_id_a).unwrap();
            let root_thread = bg_view.read(cx).root_thread_view().unwrap();
            root_thread
                .read(cx)
                .thread
                .read(cx)
                .work_dirs()
                .cloned()
                .unwrap()
        });
        assert_eq!(
            updated_a_paths.ordered_paths().collect::<Vec<_>>(),
            vec![&PathBuf::from("/project_a")],
            "Thread A should retain its explicit target after adding /project_b"
        );

        // The same rule applies to an idle retained thread.
        let updated_c_paths = panel.read_with(&cx, |panel, cx| {
            let bg_view = panel.retained_threads.get(&thread_id_c).unwrap();
            let root_thread = bg_view.read(cx).root_thread_view().unwrap();
            root_thread
                .read(cx)
                .thread
                .read(cx)
                .work_dirs()
                .cloned()
                .unwrap()
        });
        assert_eq!(
            updated_c_paths.ordered_paths().collect::<Vec<_>>(),
            vec![&PathBuf::from("/project_a")],
            "Thread C should retain its explicit target after adding /project_b"
        );

        // Metadata must preserve the same exact target.
        cx.run_until_parked();
        for (label, session_id) in [("thread B", &session_id_b), ("thread A", &session_id_a)] {
            let metadata_paths = metadata_store.read_with(&cx, |store, _cx| {
                let metadata = store
                    .entry_by_session(session_id)
                    .unwrap_or_else(|| panic!("{label} thread metadata should exist"));
                metadata.folder_paths().clone()
            });
            assert_eq!(
                metadata_paths.ordered_paths().collect::<Vec<_>>(),
                vec![&PathBuf::from("/project_a")],
                "{label} metadata should retain its explicit target"
            );
        }

        // Removing the unrelated worktree does not perturb the target.
        let worktree_b_id = new_tree.read_with(&cx, |tree, _| tree.id());
        project.update(&mut cx, |project, cx| {
            project.remove_worktree(worktree_b_id, cx);
        });
        cx.run_until_parked();

        let after_remove_b = panel.read_with(&cx, |panel, cx| {
            let thread = panel.active_agent_thread(cx).unwrap();
            thread.read(cx).work_dirs().cloned().unwrap()
        });
        assert_eq!(
            after_remove_b.ordered_paths().collect::<Vec<_>>(),
            vec![&PathBuf::from("/project_a")],
            "Thread B work_dirs should revert to only /project_a after removing /project_b"
        );

        let after_remove_a = panel.read_with(&cx, |panel, cx| {
            let bg_view = panel.retained_threads.get(&thread_id_a).unwrap();
            let root_thread = bg_view.read(cx).root_thread_view().unwrap();
            root_thread
                .read(cx)
                .thread
                .read(cx)
                .work_dirs()
                .cloned()
                .unwrap()
        });
        assert_eq!(
            after_remove_a.ordered_paths().collect::<Vec<_>>(),
            vec![&PathBuf::from("/project_a")],
            "Thread A work_dirs should revert to only /project_a after removing /project_b"
        );
    }

    #[gpui::test]
    async fn test_new_workspace_inherits_global_last_used_agent(cx: &mut TestAppContext) {
        init_test(cx);
        cx.update(|cx| {
            agent::ThreadStore::init_global(cx);
            language_model::LanguageModelRegistry::test(cx);
            // Use an isolated DB so parallel tests can't overwrite our global key.
            cx.set_global(db::AppDatabase::test_new());
        });

        let custom_agent = Agent::Custom {
            id: "my-preferred-agent".into(),
        };

        // Write a known agent to the global KVP to simulate a user who has
        // previously used this agent in another workspace.
        let kvp = cx.update(|cx| KeyValueStore::global(cx));
        write_global_last_used_agent(kvp, custom_agent.clone()).await;

        let fs = FakeFs::new(cx.executor());
        let project = Project::test(fs.clone(), [], cx).await;

        let multi_workspace =
            cx.add_window(|window, cx| MultiWorkspace::test_new(project.clone(), window, cx));

        let workspace = multi_workspace
            .read_with(cx, |multi_workspace, _cx| {
                multi_workspace.workspace().clone()
            })
            .unwrap();

        workspace.update(cx, |workspace, _cx| {
            workspace.set_random_database_id();
        });

        let cx = &mut VisualTestContext::from_window(multi_workspace.into(), cx);

        // Load the panel via `load()`, which reads the global fallback
        // asynchronously when no per-workspace state exists.
        let async_cx = cx.update(|window, cx| window.to_async(cx));
        let panel = AgentPanel::load(workspace.downgrade(), async_cx)
            .await
            .expect("panel load should succeed");
        cx.run_until_parked();

        panel.read_with(cx, |panel, _cx| {
            assert_eq!(
                panel.selected_agent, custom_agent,
                "new workspace should inherit the global last-used agent"
            );
        });
    }

    #[gpui::test]
    async fn test_select_agent_action_updates_visible_draft(cx: &mut TestAppContext) {
        init_test(cx);
        let fs = FakeFs::new(cx.executor());
        cx.update(|cx| {
            agent::ThreadStore::init_global(cx);
            language_model::LanguageModelRegistry::test(cx);
            <dyn fs::Fs>::set_global(fs.clone(), cx);
        });

        fs.insert_tree("/project", json!({ "file.txt": "" })).await;
        let project = Project::test(fs.clone(), [Path::new("/project")], cx).await;
        let multi_workspace =
            cx.add_window(|window, cx| MultiWorkspace::test_new(project.clone(), window, cx));
        let workspace = multi_workspace
            .read_with(cx, |multi_workspace, _cx| {
                multi_workspace.workspace().clone()
            })
            .unwrap();
        let cx = &mut VisualTestContext::from_window(multi_workspace.into(), cx);

        let panel = workspace.update_in(cx, |workspace, window, cx| {
            let panel = cx.new(|cx| AgentPanel::new(workspace, window, cx));
            workspace.add_panel(panel.clone(), window, cx);
            panel
        });

        panel.update_in(cx, |panel, window, cx| {
            panel.activate_draft(false, AgentThreadSource::AgentPanel, window, cx);
        });

        cx.dispatch_action(SelectAgent {
            agent: "my-configured-agent".to_string(),
        });
        cx.run_until_parked();

        let expected_agent = Agent::Custom {
            id: "my-configured-agent".into(),
        };

        panel.read_with(cx, |panel, cx| {
            let draft = panel.draft_thread.as_ref().expect("draft should exist");
            assert_eq!(panel.selected_agent, expected_agent);
            assert_eq!(*draft.read(cx).agent_key(), expected_agent);
        });

        let kvp = cx.update(|_, cx| KeyValueStore::global(cx));
        assert_eq!(
            read_global_last_used_agent(&kvp),
            Some(expected_agent),
            "the selection should be persisted as the global last-used agent"
        );
    }

    #[gpui::test]
    async fn test_workspaces_maintain_independent_agent_selection(cx: &mut TestAppContext) {
        init_test(cx);
        cx.update(|cx| {
            agent::ThreadStore::init_global(cx);
            language_model::LanguageModelRegistry::test(cx);
        });

        let fs = FakeFs::new(cx.executor());
        let project_a = Project::test(fs.clone(), [], cx).await;
        let project_b = Project::test(fs, [], cx).await;

        let multi_workspace =
            cx.add_window(|window, cx| MultiWorkspace::test_new(project_a.clone(), window, cx));

        let workspace_a = multi_workspace
            .read_with(cx, |multi_workspace, _cx| {
                multi_workspace.workspace().clone()
            })
            .unwrap();

        let workspace_b = multi_workspace
            .update(cx, |multi_workspace, window, cx| {
                multi_workspace.test_add_workspace(project_b.clone(), window, cx)
            })
            .unwrap();

        workspace_a.update(cx, |workspace, _cx| {
            workspace.set_random_database_id();
        });
        workspace_b.update(cx, |workspace, _cx| {
            workspace.set_random_database_id();
        });

        let cx = &mut VisualTestContext::from_window(multi_workspace.into(), cx);

        let agent_a = Agent::Custom {
            id: "agent-alpha".into(),
        };
        let agent_b = Agent::Custom {
            id: "agent-beta".into(),
        };

        // Set up workspace A with agent_a
        let panel_a = workspace_a.update_in(cx, |workspace, window, cx| {
            cx.new(|cx| AgentPanel::new(workspace, window, cx))
        });
        panel_a.update(cx, |panel, _cx| {
            panel.selected_agent = agent_a.clone();
        });

        // Set up workspace B with agent_b
        let panel_b = workspace_b.update_in(cx, |workspace, window, cx| {
            cx.new(|cx| AgentPanel::new(workspace, window, cx))
        });
        panel_b.update(cx, |panel, _cx| {
            panel.selected_agent = agent_b.clone();
        });

        // Serialize both panels
        panel_a.update(cx, |panel, cx| panel.serialize(cx));
        panel_b.update(cx, |panel, cx| panel.serialize(cx));
        cx.run_until_parked();

        // Load fresh panels from serialized state and verify independence
        let async_cx = cx.update(|window, cx| window.to_async(cx));
        let loaded_a = AgentPanel::load(workspace_a.downgrade(), async_cx)
            .await
            .expect("panel A load should succeed");
        cx.run_until_parked();

        let async_cx = cx.update(|window, cx| window.to_async(cx));
        let loaded_b = AgentPanel::load(workspace_b.downgrade(), async_cx)
            .await
            .expect("panel B load should succeed");
        cx.run_until_parked();

        loaded_a.read_with(cx, |panel, _cx| {
            assert_eq!(
                panel.selected_agent, agent_a,
                "workspace A should restore agent-alpha, not agent-beta"
            );
        });

        loaded_b.read_with(cx, |panel, _cx| {
            assert_eq!(
                panel.selected_agent, agent_b,
                "workspace B should restore agent-beta, not agent-alpha"
            );
        });
    }

    #[gpui::test]
    async fn test_new_thread_uses_workspace_selected_agent(cx: &mut TestAppContext) {
        init_test(cx);
        cx.update(|cx| {
            agent::ThreadStore::init_global(cx);
            language_model::LanguageModelRegistry::test(cx);
        });

        let fs = FakeFs::new(cx.executor());
        fs.insert_tree("/project", json!({ "file.txt": "" })).await;
        let project = Project::test(fs.clone(), [Path::new("/project")], cx).await;

        let multi_workspace =
            cx.add_window(|window, cx| MultiWorkspace::test_new(project.clone(), window, cx));

        let workspace = multi_workspace
            .read_with(cx, |multi_workspace, _cx| {
                multi_workspace.workspace().clone()
            })
            .unwrap();

        workspace.update(cx, |workspace, _cx| {
            workspace.set_random_database_id();
        });

        let cx = &mut VisualTestContext::from_window(multi_workspace.into(), cx);

        let custom_agent = Agent::Custom {
            id: "my-custom-agent".into(),
        };

        let panel = workspace.update_in(cx, |workspace, window, cx| {
            let panel = cx.new(|cx| AgentPanel::new(workspace, window, cx));
            workspace.add_panel(panel.clone(), window, cx);
            panel
        });

        // Set selected_agent to a custom agent
        panel.update(cx, |panel, _cx| {
            panel.selected_agent = custom_agent.clone();
        });

        // Call new_thread, which internally calls external_thread(None, ...)
        // This resolves the agent from self.selected_agent
        panel.update_in(cx, |panel, window, cx| {
            panel.new_thread(&NewThread, window, cx);
        });

        panel.read_with(cx, |panel, _cx| {
            assert_eq!(
                panel.selected_agent, custom_agent,
                "selected_agent should remain the custom agent after new_thread"
            );
            assert!(
                panel.active_conversation_view().is_some(),
                "a thread should have been created"
            );
        });
    }

    #[gpui::test]
    async fn test_draft_replaced_when_selected_agent_changes(cx: &mut TestAppContext) {
        init_test(cx);
        let fs = FakeFs::new(cx.executor());
        cx.update(|cx| {
            agent::ThreadStore::init_global(cx);
            language_model::LanguageModelRegistry::test(cx);
            <dyn fs::Fs>::set_global(fs.clone(), cx);
        });

        fs.insert_tree("/project", json!({ "file.txt": "" })).await;
        let project = Project::test(fs.clone(), [Path::new("/project")], cx).await;

        let multi_workspace =
            cx.add_window(|window, cx| MultiWorkspace::test_new(project.clone(), window, cx));

        let workspace = multi_workspace
            .read_with(cx, |multi_workspace, _cx| {
                multi_workspace.workspace().clone()
            })
            .unwrap();

        workspace.update(cx, |workspace, _cx| {
            workspace.set_random_database_id();
        });

        let cx = &mut VisualTestContext::from_window(multi_workspace.into(), cx);

        let panel = workspace.update_in(cx, |workspace, window, cx| {
            let panel = cx.new(|cx| AgentPanel::new(workspace, window, cx));
            workspace.add_panel(panel.clone(), window, cx);
            panel
        });

        // Create a draft with the default NativeAgent.
        panel.update_in(cx, |panel, window, cx| {
            panel.activate_draft(true, AgentThreadSource::AgentPanel, window, cx);
        });

        let first_draft_id = panel.read_with(cx, |panel, cx| {
            assert!(panel.draft_thread.is_some());
            assert_eq!(panel.selected_agent, Agent::NativeAgent);
            let draft = panel.draft_thread.as_ref().unwrap();
            assert_eq!(*draft.read(cx).agent_key(), Agent::NativeAgent);
            draft.entity_id()
        });

        // Switch selected_agent to a custom agent, then activate_draft again.
        // The stale NativeAgent draft should be replaced.
        let custom_agent = Agent::Custom {
            id: "my-custom-agent".into(),
        };
        panel.update_in(cx, |panel, window, cx| {
            panel.selected_agent = custom_agent.clone();
            panel.activate_draft(true, AgentThreadSource::AgentPanel, window, cx);
        });

        panel.read_with(cx, |panel, cx| {
            let draft = panel.draft_thread.as_ref().expect("draft should exist");
            assert_ne!(
                draft.entity_id(),
                first_draft_id,
                "a new draft should have been created"
            );
            assert_eq!(
                *draft.read(cx).agent_key(),
                custom_agent,
                "the new draft should use the custom agent"
            );
        });

        // Calling activate_draft again with the same agent should return the
        // cached draft (no replacement).
        let second_draft_id = panel.read_with(cx, |panel, _cx| {
            panel.draft_thread.as_ref().unwrap().entity_id()
        });

        panel.update_in(cx, |panel, window, cx| {
            panel.activate_draft(true, AgentThreadSource::AgentPanel, window, cx);
        });

        panel.read_with(cx, |panel, _cx| {
            assert_eq!(
                panel.draft_thread.as_ref().unwrap().entity_id(),
                second_draft_id,
                "draft should be reused when the agent has not changed"
            );
        });
    }

    #[gpui::test]
    async fn test_activate_draft_preserves_typed_content(cx: &mut TestAppContext) {
        init_test(cx);
        let fs = FakeFs::new(cx.executor());
        cx.update(|cx| {
            agent::ThreadStore::init_global(cx);
            language_model::LanguageModelRegistry::test(cx);
            <dyn fs::Fs>::set_global(fs.clone(), cx);
        });

        fs.insert_tree("/project", json!({ "file.txt": "" })).await;
        let project = Project::test(fs.clone(), [Path::new("/project")], cx).await;

        let multi_workspace =
            cx.add_window(|window, cx| MultiWorkspace::test_new(project.clone(), window, cx));

        let workspace = multi_workspace
            .read_with(cx, |multi_workspace, _cx| {
                multi_workspace.workspace().clone()
            })
            .unwrap();

        workspace.update(cx, |workspace, _cx| {
            workspace.set_random_database_id();
        });

        let cx = &mut VisualTestContext::from_window(multi_workspace.into(), cx);

        let panel = workspace.update_in(cx, |workspace, window, cx| {
            let panel = cx.new(|cx| AgentPanel::new(workspace, window, cx));
            workspace.add_panel(panel.clone(), window, cx);
            panel
        });

        // Create a draft using the Stub agent, which connects synchronously.
        panel.update_in(cx, |panel, window, cx| {
            panel.selected_agent = Agent::Stub;
            panel.activate_draft(true, AgentThreadSource::AgentPanel, window, cx);
        });
        cx.run_until_parked();

        let initial_draft_id = panel.read_with(cx, |panel, _cx| {
            panel.draft_thread.as_ref().unwrap().entity_id()
        });
        let initial_thread_id =
            panel.read_with(cx, |panel, cx| panel.active_thread_id(cx).unwrap());

        // Type some text into the draft editor.
        let thread_view = panel.read_with(cx, |panel, cx| panel.active_thread_view(cx).unwrap());
        let message_editor = thread_view.read_with(cx, |view, _cx| view.message_editor.clone());
        message_editor.update_in(cx, |editor, window, cx| {
            editor.set_text("Don't lose me!", window, cx);
        });

        // Press cmd-n on a typed draft — the draft is parked into
        // `retained_threads` so the user can return to it from the
        // sidebar, and a fresh, *empty* ephemeral draft becomes active.
        // The parked draft retains the prompt; the new one is a blank
        // slate.
        cx.dispatch_action(NewThread);
        cx.run_until_parked();

        panel.read_with(cx, |panel, _cx| {
            assert!(
                panel.retained_threads.contains_key(&initial_thread_id),
                "typed draft should have been parked into retained_threads"
            );
            let active_draft_id = panel.draft_thread.as_ref().unwrap().entity_id();
            assert_ne!(
                active_draft_id, initial_draft_id,
                "cmd-n should produce a fresh ephemeral draft"
            );
        });

        // The parked draft still holds the typed prompt.
        let parked_text = panel.read_with(cx, |panel, cx| panel.editor_text(initial_thread_id, cx));
        assert_eq!(
            parked_text.as_deref(),
            Some("Don't lose me!"),
            "parked draft should retain the typed prompt"
        );

        // The new active draft starts empty — no carry-over.
        let active_thread_id = panel.read_with(cx, |panel, cx| panel.active_thread_id(cx).unwrap());
        let active_text = panel.read_with(cx, |panel, cx| panel.editor_text(active_thread_id, cx));
        assert_eq!(
            active_text, None,
            "fresh ephemeral draft should start empty, not carry the parked draft's prompt"
        );
    }

    /// When the user is viewing a *parked* draft (selected from the
    /// sidebar) and presses `+`, the panel should just focus the
    /// ephemeral new-draft slot — not park it and create yet another
    /// empty draft. `+` is "go to my new-thread slot", not "reset state".
    #[gpui::test]
    async fn test_plus_with_parked_draft_active_focuses_ephemeral(cx: &mut TestAppContext) {
        init_test(cx);
        let fs = FakeFs::new(cx.executor());
        cx.update(|cx| {
            agent::ThreadStore::init_global(cx);
            language_model::LanguageModelRegistry::test(cx);
            <dyn fs::Fs>::set_global(fs.clone(), cx);
        });

        fs.insert_tree("/project", json!({ "file.txt": "" })).await;
        let project = Project::test(fs.clone(), [Path::new("/project")], cx).await;
        let multi_workspace =
            cx.add_window(|window, cx| MultiWorkspace::test_new(project.clone(), window, cx));
        let workspace = multi_workspace
            .read_with(cx, |multi_workspace, _cx| {
                multi_workspace.workspace().clone()
            })
            .unwrap();
        workspace.update(cx, |workspace, _cx| workspace.set_random_database_id());
        let cx = &mut VisualTestContext::from_window(multi_workspace.into(), cx);
        let panel = workspace.update_in(cx, |workspace, window, cx| {
            let panel = cx.new(|cx| AgentPanel::new(workspace, window, cx));
            workspace.add_panel(panel.clone(), window, cx);
            panel
        });

        // Open an initial draft, type into it, then press `+` to park it
        // and create a fresh ephemeral. The fresh ephemeral is what we'll
        // expect to refocus later.
        panel.update_in(cx, |panel, window, cx| {
            panel.selected_agent = Agent::Stub;
            panel.activate_draft(true, AgentThreadSource::AgentPanel, window, cx);
        });
        cx.run_until_parked();
        let parked_thread_id = crate::test_support::active_thread_id(&panel, cx);
        crate::test_support::type_draft_prompt(&panel, "parked draft prompt", cx);
        panel.update_in(cx, |panel, window, cx| {
            panel.new_thread(&NewThread, window, cx);
        });
        cx.run_until_parked();

        let ephemeral_thread_id = crate::test_support::active_thread_id(&panel, cx);
        let ephemeral_entity_id = panel.read_with(cx, |panel, _cx| {
            panel.draft_thread.as_ref().unwrap().entity_id()
        });
        assert_ne!(
            ephemeral_thread_id, parked_thread_id,
            "sanity: parking should have produced a fresh ephemeral draft"
        );

        // Activate the parked draft (simulates clicking it in the sidebar).
        panel.update_in(cx, |panel, window, cx| {
            panel.load_agent_thread(
                Agent::Stub,
                parked_thread_id,
                None,
                None,
                true,
                AgentThreadSource::Sidebar,
                window,
                cx,
            );
        });
        cx.run_until_parked();
        assert_eq!(
            crate::test_support::active_thread_id(&panel, cx),
            parked_thread_id,
            "sanity: parked draft should be the active view after load_agent_thread"
        );
        // The parked draft has content, so it was NOT reclaimed as
        // ephemeral. The previous ephemeral draft should still be in
        // the draft_thread slot.
        panel.read_with(cx, |panel, _cx| {
            assert_eq!(
                panel.draft_thread.as_ref().unwrap().entity_id(),
                ephemeral_entity_id,
                "ephemeral draft slot should still hold the fresh draft"
            );
        });

        // Now press `+`. The ephemeral draft should become the active
        // view since it matches the selected agent.
        panel.update_in(cx, |panel, window, cx| {
            panel.new_thread(&NewThread, window, cx);
        });
        cx.run_until_parked();

        panel.read_with(cx, |panel, cx| {
            assert_eq!(
                panel.active_thread_id(cx),
                Some(ephemeral_thread_id),
                "`+` should have switched back to the existing ephemeral draft"
            );
            assert_eq!(
                panel.draft_thread.as_ref().unwrap().entity_id(),
                ephemeral_entity_id,
                "`+` should not have replaced the ephemeral draft"
            );
            assert!(
                panel.retained_threads.contains_key(&parked_thread_id),
                "parked draft should remain in `retained_threads`"
            );
        });
    }

    /// When viewing a parked draft (agent A) and selecting a different
    /// agent (B) from the dropdown menu, the panel should create a fresh
    /// draft for agent B — not reuse the existing ephemeral draft that
    /// was bound to agent A.
    #[gpui::test]
    async fn test_new_external_agent_replaces_mismatched_ephemeral_draft(cx: &mut TestAppContext) {
        init_test(cx);
        let fs = FakeFs::new(cx.executor());
        cx.update(|cx| {
            agent::ThreadStore::init_global(cx);
            language_model::LanguageModelRegistry::test(cx);
            <dyn fs::Fs>::set_global(fs.clone(), cx);
        });

        fs.insert_tree("/project", json!({ "file.txt": "" })).await;
        let project = Project::test(fs.clone(), [Path::new("/project")], cx).await;
        let multi_workspace =
            cx.add_window(|window, cx| MultiWorkspace::test_new(project.clone(), window, cx));
        let workspace = multi_workspace
            .read_with(cx, |multi_workspace, _cx| {
                multi_workspace.workspace().clone()
            })
            .unwrap();
        workspace.update(cx, |workspace, _cx| workspace.set_random_database_id());
        let cx = &mut VisualTestContext::from_window(multi_workspace.into(), cx);
        let panel = workspace.update_in(cx, |workspace, window, cx| {
            let panel = cx.new(|cx| AgentPanel::new(workspace, window, cx));
            workspace.add_panel(panel.clone(), window, cx);
            panel
        });

        // Create a draft with Stub agent, type into it, then press `+`
        // to park it — this also creates a fresh ephemeral draft (Stub).
        panel.update_in(cx, |panel, window, cx| {
            panel.selected_agent = Agent::Stub;
            panel.activate_draft(true, AgentThreadSource::AgentPanel, window, cx);
        });
        cx.run_until_parked();
        let parked_thread_id = crate::test_support::active_thread_id(&panel, cx);
        crate::test_support::type_draft_prompt(&panel, "parked prompt", cx);
        panel.update_in(cx, |panel, window, cx| {
            panel.new_thread(&NewThread, window, cx);
        });
        cx.run_until_parked();

        let ephemeral_thread_id = crate::test_support::active_thread_id(&panel, cx);
        assert_ne!(ephemeral_thread_id, parked_thread_id);
        panel.read_with(cx, |panel, cx| {
            assert_eq!(
                panel.draft_thread.as_ref().unwrap().read(cx).agent_key(),
                &Agent::Stub,
                "ephemeral draft should be Stub agent"
            );
        });

        // Navigate back to the parked draft (simulates sidebar click).
        panel.update_in(cx, |panel, window, cx| {
            panel.load_agent_thread(
                Agent::Stub,
                parked_thread_id,
                None,
                None,
                true,
                AgentThreadSource::Sidebar,
                window,
                cx,
            );
        });
        cx.run_until_parked();
        assert_eq!(
            crate::test_support::active_thread_id(&panel, cx),
            parked_thread_id,
        );

        // Now switch to NativeAgent (simulates selecting a different
        // agent from the toolbar dropdown). This should NOT reuse the
        // Stub ephemeral draft — it should replace it with one bound to
        // NativeAgent.
        panel.update_in(cx, |panel, window, cx| {
            panel.selected_agent = Agent::NativeAgent;
            panel.activate_new_thread(true, AgentThreadSource::AgentPanel, window, cx);
        });
        cx.run_until_parked();

        panel.read_with(cx, |panel, cx| {
            let draft = panel.draft_thread.as_ref().expect("draft should exist");
            assert_eq!(
                draft.read(cx).agent_key(),
                &Agent::NativeAgent,
                "ephemeral draft should be bound to NativeAgent, not Stub"
            );
            let active_id = panel.active_thread_id(cx).unwrap();
            assert_ne!(
                active_id, ephemeral_thread_id,
                "old Stub ephemeral draft should have been replaced"
            );
            assert!(
                panel.retained_threads.contains_key(&parked_thread_id),
                "parked draft should still be in retained_threads"
            );
        });
    }

    #[gpui::test]
    async fn test_typed_draft_is_parked_when_switching_agents(cx: &mut TestAppContext) {
        init_test(cx);
        let fs = FakeFs::new(cx.executor());
        cx.update(|cx| {
            agent::ThreadStore::init_global(cx);
            language_model::LanguageModelRegistry::test(cx);
            <dyn fs::Fs>::set_global(fs.clone(), cx);
        });

        fs.insert_tree("/project", json!({ "file.txt": "" })).await;
        let project = Project::test(fs.clone(), [Path::new("/project")], cx).await;

        let multi_workspace =
            cx.add_window(|window, cx| MultiWorkspace::test_new(project.clone(), window, cx));

        let workspace = multi_workspace
            .read_with(cx, |multi_workspace, _cx| {
                multi_workspace.workspace().clone()
            })
            .unwrap();

        workspace.update(cx, |workspace, _cx| {
            workspace.set_random_database_id();
        });

        let cx = &mut VisualTestContext::from_window(multi_workspace.into(), cx);

        let panel = workspace.update_in(cx, |workspace, window, cx| {
            let panel = cx.new(|cx| AgentPanel::new(workspace, window, cx));
            workspace.add_panel(panel.clone(), window, cx);
            panel
        });

        // Create a draft with a custom stub server that connects synchronously.
        panel.update_in(cx, |panel, window, cx| {
            panel.open_draft_with_server(
                Rc::new(StubAgentServer::new(StubAgentConnection::new())),
                window,
                cx,
            );
        });
        cx.run_until_parked();

        let initial_draft_id = panel.read_with(cx, |panel, _cx| {
            panel.draft_thread.as_ref().unwrap().entity_id()
        });
        let initial_thread_id =
            panel.read_with(cx, |panel, cx| panel.active_thread_id(cx).unwrap());

        // Type text into the first draft's editor.
        let thread_view = panel.read_with(cx, |panel, cx| panel.active_thread_view(cx).unwrap());
        let message_editor = thread_view.read_with(cx, |view, _cx| view.message_editor.clone());
        message_editor.update_in(cx, |editor, window, cx| {
            editor.set_text("saved prompt", window, cx);
        });

        // Switch to a different agent. The typed draft should be parked
        // into `retained_threads` (keeping the user's prompt accessible
        // from the sidebar) and a fresh empty draft on the new agent
        // should become active.
        cx.dispatch_action(NewExternalAgentThread {
            agent: Agent::Stub.id(),
        });
        cx.run_until_parked();

        // A new draft should have been created for the Stub agent.
        panel.read_with(cx, |panel, cx| {
            let draft = panel.draft_thread.as_ref().expect("draft should exist");
            assert_ne!(
                draft.entity_id(),
                initial_draft_id,
                "a new draft should have been created for the new agent"
            );
            assert_eq!(
                *draft.read(cx).agent_key(),
                Agent::Stub,
                "new draft should use the new agent"
            );
            assert!(
                panel.retained_threads.contains_key(&initial_thread_id),
                "typed draft should have been parked into retained_threads"
            );
        });

        // The parked draft retains the prompt.
        let parked_text = panel.read_with(cx, |panel, cx| panel.editor_text(initial_thread_id, cx));
        assert_eq!(
            parked_text.as_deref(),
            Some("saved prompt"),
            "parked draft should retain the user's prompt"
        );

        // The new draft on the new agent starts empty.
        let active_thread_id = panel.read_with(cx, |panel, cx| panel.active_thread_id(cx).unwrap());
        let active_text = panel.read_with(cx, |panel, cx| panel.editor_text(active_thread_id, cx));
        assert_eq!(
            active_text, None,
            "new draft on the new agent should start empty, not carry the parked draft's prompt"
        );
    }

    #[gpui::test]
    async fn test_rollback_all_succeed_returns_ok(cx: &mut TestAppContext) {
        init_test(cx);
        let fs = FakeFs::new(cx.executor());
        cx.update(|cx| {
            cx.update_flags(true, vec!["agent-v2".to_string()]);
            agent::ThreadStore::init_global(cx);
            language_model::LanguageModelRegistry::test(cx);
            <dyn fs::Fs>::set_global(fs.clone(), cx);
        });

        fs.insert_tree(
            "/project",
            json!({
                ".git": {},
                "src": { "main.rs": "fn main() {}" }
            }),
        )
        .await;

        let project = Project::test(fs.clone(), [Path::new("/project")], cx).await;
        cx.executor().run_until_parked();

        let repository = project.read_with(cx, |project, cx| {
            project.repositories(cx).values().next().unwrap().clone()
        });

        let multi_workspace =
            cx.add_window(|window, cx| MultiWorkspace::test_new(project.clone(), window, cx));

        let path_a = PathBuf::from("/worktrees/branch/project_a");
        let path_b = PathBuf::from("/worktrees/branch/project_b");

        let (sender_a, receiver_a) = futures::channel::oneshot::channel::<Result<()>>();
        let (sender_b, receiver_b) = futures::channel::oneshot::channel::<Result<()>>();
        sender_a.send(Ok(())).unwrap();
        sender_b.send(Ok(())).unwrap();

        let creation_infos = vec![
            (repository.clone(), path_a.clone(), receiver_a),
            (repository.clone(), path_b.clone(), receiver_b),
        ];

        let fs_clone = fs.clone();
        let result = multi_workspace
            .update(cx, |_, window, cx| {
                window.spawn(cx, async move |cx| {
                    git_ui::worktree_service::await_and_rollback_on_failure(
                        creation_infos,
                        fs_clone,
                        cx,
                    )
                    .await
                })
            })
            .unwrap()
            .await;

        let paths = result.expect("all succeed should return Ok");
        assert_eq!(paths, vec![path_a, path_b]);
    }

    #[gpui::test]
    async fn test_rollback_on_failure_attempts_all_worktrees(cx: &mut TestAppContext) {
        init_test(cx);
        let fs = FakeFs::new(cx.executor());
        cx.update(|cx| {
            cx.update_flags(true, vec!["agent-v2".to_string()]);
            agent::ThreadStore::init_global(cx);
            language_model::LanguageModelRegistry::test(cx);
            <dyn fs::Fs>::set_global(fs.clone(), cx);
        });

        fs.insert_tree(
            "/project",
            json!({
                ".git": {},
                "src": { "main.rs": "fn main() {}" }
            }),
        )
        .await;

        let project = Project::test(fs.clone(), [Path::new("/project")], cx).await;
        cx.executor().run_until_parked();

        let repository = project.read_with(cx, |project, cx| {
            project.repositories(cx).values().next().unwrap().clone()
        });

        // Actually create a worktree so it exists in FakeFs for rollback to find.
        let success_path = PathBuf::from("/worktrees/branch/project");
        cx.update(|cx| {
            repository.update(cx, |repo, _| {
                repo.create_worktree(
                    git::repository::CreateWorktreeTarget::NewBranch {
                        branch_name: "branch".to_string(),
                        base_sha: None,
                    },
                    success_path.clone(),
                )
            })
        })
        .await
        .unwrap()
        .unwrap();
        cx.executor().run_until_parked();

        // Verify the worktree directory exists before rollback.
        assert!(
            fs.is_dir(&success_path).await,
            "worktree directory should exist before rollback"
        );

        let multi_workspace =
            cx.add_window(|window, cx| MultiWorkspace::test_new(project.clone(), window, cx));

        // Build creation_infos: one success, one failure.
        let failed_path = PathBuf::from("/worktrees/branch/failed_project");

        let (sender_ok, receiver_ok) = futures::channel::oneshot::channel::<Result<()>>();
        let (sender_err, receiver_err) = futures::channel::oneshot::channel::<Result<()>>();
        sender_ok.send(Ok(())).unwrap();
        sender_err
            .send(Err(anyhow!("branch already exists")))
            .unwrap();

        let creation_infos = vec![
            (repository.clone(), success_path.clone(), receiver_ok),
            (repository.clone(), failed_path.clone(), receiver_err),
        ];

        let fs_clone = fs.clone();
        let result = multi_workspace
            .update(cx, |_, window, cx| {
                window.spawn(cx, async move |cx| {
                    git_ui::worktree_service::await_and_rollback_on_failure(
                        creation_infos,
                        fs_clone,
                        cx,
                    )
                    .await
                })
            })
            .unwrap()
            .await;

        assert!(
            result.is_err(),
            "should return error when any creation fails"
        );
        let err_msg = result.unwrap_err().to_string();
        assert!(
            err_msg.contains("branch already exists"),
            "error should mention the original failure: {err_msg}"
        );

        // The successful worktree should have been rolled back by git.
        cx.executor().run_until_parked();
        assert!(
            !fs.is_dir(&success_path).await,
            "successful worktree directory should be removed by rollback"
        );
    }

    #[gpui::test]
    async fn test_rollback_on_canceled_receiver(cx: &mut TestAppContext) {
        init_test(cx);
        let fs = FakeFs::new(cx.executor());
        cx.update(|cx| {
            cx.update_flags(true, vec!["agent-v2".to_string()]);
            agent::ThreadStore::init_global(cx);
            language_model::LanguageModelRegistry::test(cx);
            <dyn fs::Fs>::set_global(fs.clone(), cx);
        });

        fs.insert_tree(
            "/project",
            json!({
                ".git": {},
                "src": { "main.rs": "fn main() {}" }
            }),
        )
        .await;

        let project = Project::test(fs.clone(), [Path::new("/project")], cx).await;
        cx.executor().run_until_parked();

        let repository = project.read_with(cx, |project, cx| {
            project.repositories(cx).values().next().unwrap().clone()
        });

        let multi_workspace =
            cx.add_window(|window, cx| MultiWorkspace::test_new(project.clone(), window, cx));

        let path = PathBuf::from("/worktrees/branch/project");

        // Drop the sender to simulate a canceled receiver.
        let (_sender, receiver) = futures::channel::oneshot::channel::<Result<()>>();
        drop(_sender);

        let creation_infos = vec![(repository.clone(), path.clone(), receiver)];

        let fs_clone = fs.clone();
        let result = multi_workspace
            .update(cx, |_, window, cx| {
                window.spawn(cx, async move |cx| {
                    git_ui::worktree_service::await_and_rollback_on_failure(
                        creation_infos,
                        fs_clone,
                        cx,
                    )
                    .await
                })
            })
            .unwrap()
            .await;

        assert!(
            result.is_err(),
            "should return error when receiver is canceled"
        );
        let err_msg = result.unwrap_err().to_string();
        assert!(
            err_msg.contains("canceled"),
            "error should mention cancellation: {err_msg}"
        );
    }

    #[gpui::test]
    async fn test_rollback_cleans_up_orphan_directories(cx: &mut TestAppContext) {
        init_test(cx);
        let fs = FakeFs::new(cx.executor());
        cx.update(|cx| {
            cx.update_flags(true, vec!["agent-v2".to_string()]);
            agent::ThreadStore::init_global(cx);
            language_model::LanguageModelRegistry::test(cx);
            <dyn fs::Fs>::set_global(fs.clone(), cx);
        });

        fs.insert_tree(
            "/project",
            json!({
                ".git": {},
                "src": { "main.rs": "fn main() {}" }
            }),
        )
        .await;

        let project = Project::test(fs.clone(), [Path::new("/project")], cx).await;
        cx.executor().run_until_parked();

        let repository = project.read_with(cx, |project, cx| {
            project.repositories(cx).values().next().unwrap().clone()
        });

        let multi_workspace =
            cx.add_window(|window, cx| MultiWorkspace::test_new(project.clone(), window, cx));

        // Simulate the orphan state: create_dir_all was called but git
        // worktree add failed, leaving a directory with leftover files.
        let orphan_path = PathBuf::from("/worktrees/branch/orphan_project");
        fs.insert_tree(
            "/worktrees/branch/orphan_project",
            json!({ "leftover.txt": "junk" }),
        )
        .await;

        assert!(
            fs.is_dir(&orphan_path).await,
            "orphan dir should exist before rollback"
        );

        let (sender, receiver) = futures::channel::oneshot::channel::<Result<()>>();
        sender.send(Err(anyhow!("hook failed"))).unwrap();

        let creation_infos = vec![(repository.clone(), orphan_path.clone(), receiver)];

        let fs_clone = fs.clone();
        let result = multi_workspace
            .update(cx, |_, window, cx| {
                window.spawn(cx, async move |cx| {
                    git_ui::worktree_service::await_and_rollback_on_failure(
                        creation_infos,
                        fs_clone,
                        cx,
                    )
                    .await
                })
            })
            .unwrap()
            .await;

        cx.executor().run_until_parked();

        assert!(result.is_err());
        assert!(
            !fs.is_dir(&orphan_path).await,
            "orphan worktree directory should be removed by filesystem cleanup"
        );
    }

    #[gpui::test]
    async fn test_selected_agent_syncs_when_navigating_between_threads(cx: &mut TestAppContext) {
        let (panel, mut cx) = setup_panel(cx).await;

        let stub_agent = Agent::Custom { id: "Test".into() };

        // Open thread A and send a message so it is retained.
        let connection_a = StubAgentConnection::new();
        connection_a.set_next_prompt_updates(vec![acp::SessionUpdate::AgentMessageChunk(
            acp::ContentChunk::new("response a".into()),
        )]);
        open_thread_with_connection(&panel, connection_a, &mut cx);
        let _session_id_a = active_session_id(&panel, &cx);
        let thread_id_a = active_thread_id(&panel, &cx);
        send_message(&panel, &mut cx);
        cx.run_until_parked();

        panel.read_with(&cx, |panel, _cx| {
            assert_eq!(panel.selected_agent, stub_agent);
        });

        // Open thread B with a different agent — thread A goes to retained.
        let custom_agent = Agent::Custom {
            id: "my-custom-agent".into(),
        };
        let connection_b = StubAgentConnection::new()
            .with_agent_id("my-custom-agent".into())
            .with_telemetry_id("my-custom-agent".into());
        connection_b.set_next_prompt_updates(vec![acp::SessionUpdate::AgentMessageChunk(
            acp::ContentChunk::new("response b".into()),
        )]);
        open_thread_with_custom_connection(&panel, connection_b, &mut cx);
        send_message(&panel, &mut cx);
        cx.run_until_parked();

        panel.read_with(&cx, |panel, _cx| {
            assert_eq!(
                panel.selected_agent, custom_agent,
                "selected_agent should have changed to the custom agent"
            );
        });

        // Navigate back to thread A via load_agent_thread.
        panel.update_in(&mut cx, |panel, window, cx| {
            panel.load_agent_thread(
                stub_agent.clone(),
                thread_id_a,
                None,
                None,
                true,
                AgentThreadSource::AgentPanel,
                window,
                cx,
            );
        });

        panel.read_with(&cx, |panel, _cx| {
            assert_eq!(
                panel.selected_agent, stub_agent,
                "selected_agent should sync back to thread A's agent"
            );
        });
    }

    #[gpui::test]
    async fn test_classify_worktrees_skips_non_git_root_with_nested_repo(cx: &mut TestAppContext) {
        init_test(cx);
        cx.update(|cx| {
            agent::ThreadStore::init_global(cx);
            language_model::LanguageModelRegistry::test(cx);
        });

        let fs = FakeFs::new(cx.executor());
        fs.insert_tree(
            "/repo_a",
            json!({
                ".git": {},
                "src": { "main.rs": "" }
            }),
        )
        .await;
        fs.insert_tree(
            "/repo_b",
            json!({
                ".git": {},
                "src": { "lib.rs": "" }
            }),
        )
        .await;
        // `plain_dir` is NOT a git repo, but contains a nested git repo.
        fs.insert_tree(
            "/plain_dir",
            json!({
                "nested_repo": {
                    ".git": {},
                    "src": { "lib.rs": "" }
                }
            }),
        )
        .await;

        let project = Project::test(
            fs.clone(),
            [
                Path::new("/repo_a"),
                Path::new("/repo_b"),
                Path::new("/plain_dir"),
            ],
            cx,
        )
        .await;

        // Let the worktree scanner discover all `.git` directories.
        cx.executor().run_until_parked();

        let multi_workspace =
            cx.add_window(|window, cx| MultiWorkspace::test_new(project.clone(), window, cx));

        let workspace = multi_workspace
            .read_with(cx, |mw, _cx| mw.workspace().clone())
            .unwrap();

        let cx = &mut VisualTestContext::from_window(multi_workspace.into(), cx);

        let panel = workspace.update_in(cx, |workspace, window, cx| {
            cx.new(|cx| AgentPanel::new(workspace, window, cx))
        });

        cx.run_until_parked();

        panel.read_with(cx, |panel, cx| {
            let (git_repos, non_git_paths) =
                git_ui::worktree_service::classify_worktrees(panel.project.read(cx), cx);

            let git_work_dirs: Vec<PathBuf> = git_repos
                .iter()
                .map(|repo| repo.read(cx).work_directory_abs_path.to_path_buf())
                .collect();

            assert_eq!(
                git_repos.len(),
                2,
                "only repo_a and repo_b should be classified as git repos, \
                 but got: {git_work_dirs:?}"
            );
            assert!(
                git_work_dirs.contains(&PathBuf::from("/repo_a")),
                "repo_a should be in git_repos: {git_work_dirs:?}"
            );
            assert!(
                git_work_dirs.contains(&PathBuf::from("/repo_b")),
                "repo_b should be in git_repos: {git_work_dirs:?}"
            );

            assert_eq!(
                non_git_paths,
                vec![PathBuf::from("/plain_dir")],
                "plain_dir should be classified as a non-git path \
                 (not matched to nested_repo inside it)"
            );
        });
    }
    #[gpui::test]
    async fn test_vim_search_does_not_steal_focus_from_agent_panel(cx: &mut TestAppContext) {
        init_test(cx);
        cx.update(|cx| {
            agent::ThreadStore::init_global(cx);
            language_model::LanguageModelRegistry::test(cx);
            vim::init(cx);
            search::init(cx);

            // Enable vim mode
            settings::SettingsStore::update_global(cx, |store, cx| {
                store.update_user_settings(cx, |s| s.vim_mode = Some(true));
            });

            // Load vim keybindings
            let mut vim_key_bindings =
                settings::KeymapFile::load_asset_allow_partial_failure("keymaps/vim.json", cx)
                    .unwrap();
            for key_binding in &mut vim_key_bindings {
                key_binding.set_meta(settings::KeybindSource::Vim.meta());
            }
            cx.bind_keys(vim_key_bindings);
        });

        // Create a project with a file so we have a buffer in the center pane.
        let fs = FakeFs::new(cx.executor());
        fs.insert_tree("/project", json!({ "file.txt": "hello world" }))
            .await;
        let project = Project::test(fs.clone(), [Path::new("/project")], cx).await;

        let multi_workspace =
            cx.add_window(|window, cx| MultiWorkspace::test_new(project.clone(), window, cx));
        let workspace = multi_workspace
            .read_with(cx, |mw, _cx| mw.workspace().clone())
            .unwrap();
        let mut cx = VisualTestContext::from_window(multi_workspace.into(), cx);

        // Open a file in the center pane.
        workspace
            .update_in(&mut cx, |workspace, window, cx| {
                workspace.open_paths(
                    vec![PathBuf::from("/project/file.txt")],
                    workspace::OpenOptions::default(),
                    None,
                    window,
                    cx,
                )
            })
            .await;
        cx.run_until_parked();

        // Add a BufferSearchBar to the center pane's toolbar, as a real
        // workspace would have.
        workspace.update_in(&mut cx, |workspace, window, cx| {
            workspace.active_pane().update(cx, |pane, cx| {
                pane.toolbar().update(cx, |toolbar, cx| {
                    let search_bar = cx.new(|cx| search::BufferSearchBar::new(None, window, cx));
                    toolbar.add_item(search_bar, window, cx);
                });
            });
        });

        // Create the agent panel and add it to the workspace.
        let panel = workspace.update_in(&mut cx, |workspace, window, cx| {
            let panel = cx.new(|cx| AgentPanel::new(workspace, window, cx));
            workspace.add_panel(panel.clone(), window, cx);
            panel
        });

        // Open a thread so the panel has an active editor.
        open_thread_with_connection(&panel, StubAgentConnection::new(), &mut cx);

        // Focus the agent panel.
        workspace.update_in(&mut cx, |workspace, window, cx| {
            workspace.focus_panel::<AgentPanel>(window, cx);
        });
        cx.run_until_parked();

        // Verify the agent panel has focus.
        workspace.update_in(&mut cx, |_, window, cx| {
            assert!(
                panel.read(cx).focus_handle(cx).contains_focused(window, cx),
                "Agent panel should be focused before pressing '/'"
            );
        });

        // Press '/' — the vim search keybinding.
        cx.simulate_keystrokes("/");

        // Focus should remain on the agent panel.
        workspace.update_in(&mut cx, |_, window, cx| {
            assert!(
                panel.read(cx).focus_handle(cx).contains_focused(window, cx),
                "Focus should remain on the agent panel after pressing '/'"
            );
        });
    }

    /// Connection that tracks closed sessions and detects prompts against
    /// sessions that no longer exist, used to reproduce session disassociation.
    #[derive(Clone, Default)]
    struct DisassociationTrackingConnection {
        next_session_number: Arc<Mutex<usize>>,
        sessions: Arc<Mutex<HashSet<acp::SessionId>>>,
        closed_sessions: Arc<Mutex<Vec<acp::SessionId>>>,
        missing_prompt_sessions: Arc<Mutex<Vec<acp::SessionId>>>,
    }

    impl DisassociationTrackingConnection {
        fn new() -> Self {
            Self::default()
        }

        fn create_session(
            self: Rc<Self>,
            session_id: acp::SessionId,
            project: Entity<Project>,
            work_dirs: PathList,
            title: Option<SharedString>,
            cx: &mut App,
        ) -> Entity<AcpThread> {
            self.sessions.lock().insert(session_id.clone());

            let action_log = cx.new(|_| ActionLog::new(project.clone()));
            cx.new(|cx| {
                AcpThread::new(
                    None,
                    title,
                    Some(work_dirs),
                    self,
                    project,
                    action_log,
                    session_id,
                    watch::Receiver::constant(
                        acp::PromptCapabilities::new()
                            .image(true)
                            .audio(true)
                            .embedded_context(true),
                    ),
                    cx,
                )
            })
        }
    }

    impl AgentConnection for DisassociationTrackingConnection {
        fn agent_id(&self) -> AgentId {
            agent::OMEGA_AGENT_ID.clone()
        }

        fn telemetry_id(&self) -> SharedString {
            "disassociation-tracking-test".into()
        }

        fn new_session(
            self: Rc<Self>,
            project: Entity<Project>,
            work_dirs: PathList,
            cx: &mut App,
        ) -> Task<Result<Entity<AcpThread>>> {
            let session_id = {
                let mut next_session_number = self.next_session_number.lock();
                let session_id = acp::SessionId::new(format!(
                    "disassociation-tracking-session-{}",
                    *next_session_number
                ));
                *next_session_number += 1;
                session_id
            };
            let thread = self.create_session(session_id, project, work_dirs, None, cx);
            Task::ready(Ok(thread))
        }

        fn supports_load_session(&self) -> bool {
            true
        }

        fn load_session(
            self: Rc<Self>,
            session_id: acp::SessionId,
            project: Entity<Project>,
            work_dirs: PathList,
            title: Option<SharedString>,
            cx: &mut App,
        ) -> Task<Result<Entity<AcpThread>>> {
            let thread = self.create_session(session_id, project, work_dirs, title, cx);
            thread.update(cx, |thread, cx| {
                thread
                    .handle_session_update(
                        acp::SessionUpdate::UserMessageChunk(acp::ContentChunk::new(
                            "Restored user message".into(),
                        )),
                        cx,
                    )
                    .expect("restored user message should be applied");
                thread
                    .handle_session_update(
                        acp::SessionUpdate::AgentMessageChunk(acp::ContentChunk::new(
                            "Restored assistant message".into(),
                        )),
                        cx,
                    )
                    .expect("restored assistant message should be applied");
            });
            Task::ready(Ok(thread))
        }

        fn supports_close_session(&self) -> bool {
            true
        }

        fn close_session(
            self: Rc<Self>,
            session_id: &acp::SessionId,
            _cx: &mut App,
        ) -> Task<Result<()>> {
            self.sessions.lock().remove(session_id);
            self.closed_sessions.lock().push(session_id.clone());
            Task::ready(Ok(()))
        }

        fn auth_methods(&self) -> &[acp::AuthMethod] {
            &[]
        }

        fn authenticate(&self, _method_id: acp::AuthMethodId, _cx: &mut App) -> Task<Result<()>> {
            Task::ready(Ok(()))
        }

        fn prompt(
            &self,
            params: acp::PromptRequest,
            _cx: &mut App,
        ) -> Task<Result<acp::PromptResponse>> {
            if !self.sessions.lock().contains(&params.session_id) {
                self.missing_prompt_sessions.lock().push(params.session_id);
                return Task::ready(Err(anyhow!("Session not found")));
            }

            Task::ready(Ok(acp::PromptResponse::new(acp::StopReason::EndTurn)))
        }

        fn cancel(&self, _session_id: &acp::SessionId, _cx: &mut App) {}

        fn into_any(self: Rc<Self>) -> Rc<dyn Any> {
            self
        }
    }

    async fn setup_workspace_panel(
        cx: &mut TestAppContext,
    ) -> (Entity<Workspace>, Entity<AgentPanel>, VisualTestContext) {
        init_test(cx);
        cx.update(|cx| {
            agent::ThreadStore::init_global(cx);
            language_model::LanguageModelRegistry::test(cx);
        });

        let fs = FakeFs::new(cx.executor());
        fs.insert_tree("/project", json!({ "file.txt": "" })).await;
        cx.update(|cx| <dyn fs::Fs>::set_global(fs.clone(), cx));
        let project = Project::test(fs.clone(), [Path::new("/project")], cx).await;

        let multi_workspace =
            cx.add_window(|window, cx| MultiWorkspace::test_new(project.clone(), window, cx));

        let workspace = multi_workspace
            .read_with(cx, |mw, _cx| mw.workspace().clone())
            .unwrap();

        let mut cx = VisualTestContext::from_window(multi_workspace.into(), cx);

        let panel = workspace.update_in(&mut cx, |workspace, window, cx| {
            let panel = cx.new(|cx| AgentPanel::new(workspace, window, cx));
            workspace.add_panel(panel.clone(), window, cx);
            panel
        });

        (workspace, panel, cx)
    }

    /// Reproduces the retained-thread reset race:
    ///
    /// 1. Thread A is active and Connected.
    /// 2. User switches to thread B → A goes to retained_threads.
    /// 3. A thread_error is set on retained A's thread view.
    /// 4. AgentServersUpdated fires → retained A's handle_agent_servers_updated
    ///    sees has_thread_error=true → calls reset() → close_all_sessions →
    ///    session X removed, state = Loading.
    /// 5. User reopens thread X via open_thread → load_agent_thread checks
    ///    retained A's has_session → returns false (state is Loading) →
    ///    creates new ConversationView C.
    /// 6. Both A's reload task and C's load task complete → both call
    ///    load_session(X) → both get Connected with session X.
    /// 7. A is eventually cleaned up → on_release → close_all_sessions →
    ///    removes session X.
    /// 8. C sends → "Session not found".
    #[gpui::test]
    async fn test_retained_thread_reset_race_disassociates_session(cx: &mut TestAppContext) {
        let (_workspace, panel, mut cx) = setup_workspace_panel(cx).await;
        cx.run_until_parked();

        let connection = DisassociationTrackingConnection::new();
        panel.update(&mut cx, |panel, cx| {
            panel.connection_store.update(cx, |store, cx| {
                store.restart_connection(
                    Agent::Stub,
                    Rc::new(StubAgentServer::new(connection.clone())),
                    cx,
                );
            });
        });
        cx.run_until_parked();

        // Step 1: Open thread A and send a message.
        panel.update_in(&mut cx, |panel, window, cx| {
            panel.external_thread(
                Some(Agent::Stub),
                None,
                None,
                None,
                None,
                true,
                AgentThreadSource::AgentPanel,
                window,
                cx,
            );
        });
        cx.run_until_parked();
        send_message(&panel, &mut cx);

        let session_id_a = active_session_id(&panel, &cx);
        let _thread_id_a = active_thread_id(&panel, &cx);

        // Step 2: Open thread B → A goes to retained_threads.
        panel.update_in(&mut cx, |panel, window, cx| {
            panel.external_thread(
                Some(Agent::Stub),
                None,
                None,
                None,
                None,
                true,
                AgentThreadSource::AgentPanel,
                window,
                cx,
            );
        });
        cx.run_until_parked();
        send_message(&panel, &mut cx);

        // Confirm A is retained.
        panel.read_with(&cx, |panel, _cx| {
            assert!(
                panel.retained_threads.contains_key(&_thread_id_a),
                "thread A should be in retained_threads after switching to B"
            );
        });

        // Step 3: Set a thread_error on retained A's active thread view.
        // This simulates an API error that occurred before the user switched
        // away, or a transient failure.
        let retained_conversation_a = panel.read_with(&cx, |panel, _cx| {
            panel
                .retained_threads
                .get(&_thread_id_a)
                .expect("thread A should be retained")
                .clone()
        });
        retained_conversation_a.update(&mut cx, |conversation, cx| {
            if let Some(thread_view) = conversation.active_thread() {
                thread_view.update(cx, |view, cx| {
                    view.handle_thread_error(
                        crate::conversation_view::ThreadError::Other {
                            message: "simulated error".into(),
                            acp_error_code: None,
                        },
                        cx,
                    );
                });
            }
        });

        // Confirm the thread error is set.
        retained_conversation_a.read_with(&cx, |conversation, cx| {
            let connected = conversation.as_connected().expect("should be connected");
            assert!(
                connected.has_thread_error(cx),
                "retained A should have a thread error"
            );
        });

        // Step 4: Emit AgentServersUpdated → retained A's
        // handle_agent_servers_updated sees has_thread_error=true,
        // calls reset(), which closes session X and sets state=Loading.
        //
        // Critically, we do NOT call run_until_parked between the emit
        // and open_thread. The emit's synchronous effects (event delivery
        // → reset() → close_all_sessions → state=Loading) happen during
        // the update's flush_effects. But the async reload task spawned
        // by initial_state has NOT been polled yet.
        panel.update(&mut cx, |panel, cx| {
            panel.project.update(cx, |project, cx| {
                project
                    .agent_server_store()
                    .update(cx, |_store, cx| cx.emit(project::AgentServersUpdated));
            });
        });
        // After this update returns, the retained ConversationView is in
        // Loading state (reset ran synchronously), but its async reload
        // task hasn't executed yet.

        // Step 5: Immediately open thread X via open_thread, BEFORE
        // the retained view's async reload completes. load_agent_thread
        // checks retained A's has_session → returns false (state is
        // Loading) → creates a NEW ConversationView C for session X.
        panel.update_in(&mut cx, |panel, window, cx| {
            panel.open_thread(session_id_a.clone(), None, None, window, cx);
        });

        // NOW settle everything: both async tasks (A's reload and C's load)
        // complete, both register session X.
        cx.run_until_parked();

        // Verify session A is the active session via C.
        panel.read_with(&cx, |panel, cx| {
            let active_session = panel
                .active_agent_thread(cx)
                .map(|t| t.read(cx).session_id().clone());
            assert_eq!(
                active_session,
                Some(session_id_a.clone()),
                "session A should be the active session after open_thread"
            );
        });

        // Step 6: Force the retained ConversationView A to be dropped
        // while the active view (C) still has the same session.
        // We can't use remove_thread because C shares the same ThreadId
        // and remove_thread would kill the active view too. Instead,
        // directly remove from retained_threads and drop the handle
        // so on_release → close_all_sessions fires only on A.
        drop(retained_conversation_a);
        panel.update(&mut cx, |panel, _cx| {
            panel.retained_threads.remove(&_thread_id_a);
        });
        cx.run_until_parked();

        // The key assertion: sending messages on the ACTIVE view (C)
        // must succeed. If the session was disassociated by A's cleanup,
        // this will fail with "Session not found".
        send_message(&panel, &mut cx);
        send_message(&panel, &mut cx);

        let missing = connection.missing_prompt_sessions.lock().clone();
        assert!(
            missing.is_empty(),
            "session should not be disassociated after retained thread reset race, \
             got missing prompt sessions: {:?}",
            missing
        );

        panel.read_with(&cx, |panel, cx| {
            let active_view = panel
                .active_conversation_view()
                .expect("conversation should remain open");
            let connected = active_view
                .read(cx)
                .as_connected()
                .expect("conversation should be connected");
            assert!(
                !connected.has_thread_error(cx),
                "conversation should not have a thread error"
            );
        });
    }

    #[gpui::test]
    async fn test_initialize_from_source_transfers_draft_to_fresh_panel(cx: &mut TestAppContext) {
        init_test(cx);
        cx.update(|cx| {
            agent::ThreadStore::init_global(cx);
            language_model::LanguageModelRegistry::test(cx);
        });

        let fs = FakeFs::new(cx.executor());
        fs.insert_tree("/project_a", json!({ "file.txt": "" }))
            .await;
        fs.insert_tree("/project_b", json!({ "file.txt": "" }))
            .await;
        let project_a = Project::test(fs.clone(), [Path::new("/project_a")], cx).await;
        let project_b = Project::test(fs.clone(), [Path::new("/project_b")], cx).await;

        let multi_workspace =
            cx.add_window(|window, cx| MultiWorkspace::test_new(project_a.clone(), window, cx));

        let workspace_a = multi_workspace
            .read_with(cx, |mw, _cx| mw.workspace().clone())
            .unwrap();

        let workspace_b = multi_workspace
            .update(cx, |multi_workspace, window, cx| {
                multi_workspace.test_add_workspace(project_b.clone(), window, cx)
            })
            .unwrap();

        let cx = &mut VisualTestContext::from_window(multi_workspace.into(), cx);

        // Set up panel_a with an active thread and type draft text.
        let panel_a = workspace_a.update_in(cx, |workspace, window, cx| {
            let panel = cx.new(|cx| AgentPanel::new(workspace, window, cx));
            workspace.add_panel(panel.clone(), window, cx);
            panel
        });
        cx.run_until_parked();

        panel_a.update_in(cx, |panel, window, cx| {
            panel.open_external_thread_with_server(
                Rc::new(StubAgentServer::default_response()),
                window,
                cx,
            );
        });
        cx.run_until_parked();

        let thread_view_a =
            panel_a.read_with(cx, |panel, cx| panel.active_thread_view(cx).unwrap());
        let editor_a = thread_view_a.read_with(cx, |view, _cx| view.message_editor.clone());
        editor_a.update_in(cx, |editor, window, cx| {
            editor.set_text("Draft from workspace A", window, cx);
        });

        // Set up panel_b on workspace_b — starts as a fresh, empty panel.
        let panel_b = workspace_b.update_in(cx, |workspace, window, cx| {
            let panel = cx.new(|cx| AgentPanel::new(workspace, window, cx));
            workspace.add_panel(panel.clone(), window, cx);
            panel
        });
        cx.run_until_parked();

        // Initializing panel_b from workspace_a should transfer the draft,
        // even if panel_b already has an auto-created empty draft thread
        // (which set_active creates during add_panel).
        let transferred = panel_b.update_in(cx, |panel, window, cx| {
            panel.initialize_from_source_workspace_if_needed(workspace_a.downgrade(), window, cx)
        });
        assert!(
            transferred,
            "fresh destination panel should accept source content"
        );

        // Verify the panel was initialized: the base_view should now be an
        // AgentThread (not Uninitialized) and a draft_thread should be set.
        // We can't check the message editor text directly because the thread
        // needs a connected server session (not available in unit tests without
        // a stub server). The `transferred == true` return already proves that
        // source_panel_initialization read the content successfully.
        panel_b.read_with(cx, |panel, _cx| {
            assert!(
                panel.active_conversation_view().is_some(),
                "panel_b should have a conversation view after initialization"
            );
            assert!(
                panel.draft_thread.is_some(),
                "panel_b should have a draft_thread set after initialization"
            );
        });
    }

    #[gpui::test]
    async fn test_initialize_from_source_inherits_agent_without_draft_content(
        cx: &mut TestAppContext,
    ) {
        init_test(cx);
        cx.update(|cx| {
            agent::ThreadStore::init_global(cx);
            language_model::LanguageModelRegistry::test(cx);
        });

        let fs = FakeFs::new(cx.executor());
        fs.insert_tree("/project_a", json!({ "file.txt": "" }))
            .await;
        fs.insert_tree("/project_b", json!({ "file.txt": "" }))
            .await;
        let project_a = Project::test(fs.clone(), [Path::new("/project_a")], cx).await;
        let project_b = Project::test(fs.clone(), [Path::new("/project_b")], cx).await;

        let multi_workspace =
            cx.add_window(|window, cx| MultiWorkspace::test_new(project_a.clone(), window, cx));

        let workspace_a = multi_workspace
            .read_with(cx, |mw, _cx| mw.workspace().clone())
            .unwrap();

        let workspace_b = multi_workspace
            .update(cx, |multi_workspace, window, cx| {
                multi_workspace.test_add_workspace(project_b.clone(), window, cx)
            })
            .unwrap();

        let cx = &mut VisualTestContext::from_window(multi_workspace.into(), cx);

        let panel_a = workspace_a.update_in(cx, |workspace, window, cx| {
            let panel = cx.new(|cx| AgentPanel::new(workspace, window, cx));
            workspace.add_panel(panel.clone(), window, cx);
            panel
        });

        panel_a.update(cx, |panel, _cx| {
            panel.selected_agent = Agent::Stub;
        });

        let panel_b = workspace_b.update_in(cx, |workspace, window, cx| {
            let panel = cx.new(|cx| AgentPanel::new(workspace, window, cx));
            workspace.add_panel(panel.clone(), window, cx);
            panel
        });

        let initialized = panel_b.update_in(cx, |panel, window, cx| {
            panel.initialize_from_source_workspace_if_needed(workspace_a.downgrade(), window, cx)
        });
        assert!(
            initialized,
            "fresh destination panel should inherit the source agent"
        );

        panel_b.read_with(cx, |panel, _cx| {
            assert_eq!(
                panel.selected_agent,
                Agent::Stub,
                "destination panel should inherit the source panel's selected agent"
            );
            assert!(
                panel.active_conversation_view().is_none(),
                "agent-only initialization should not create a draft thread"
            );
        });
    }

    #[gpui::test]
    async fn test_initialize_from_source_retargets_empty_destination_draft_agent(
        cx: &mut TestAppContext,
    ) {
        init_test(cx);
        cx.update(|cx| {
            agent::ThreadStore::init_global(cx);
            language_model::LanguageModelRegistry::test(cx);
        });

        let fs = FakeFs::new(cx.executor());
        cx.update(|cx| <dyn fs::Fs>::set_global(fs.clone(), cx));
        fs.insert_tree("/project_a", json!({ "file.txt": "" }))
            .await;
        fs.insert_tree("/project_b", json!({ "file.txt": "" }))
            .await;
        let project_a = Project::test(fs.clone(), [Path::new("/project_a")], cx).await;
        let project_b = Project::test(fs.clone(), [Path::new("/project_b")], cx).await;

        let multi_workspace =
            cx.add_window(|window, cx| MultiWorkspace::test_new(project_a.clone(), window, cx));

        let workspace_a = multi_workspace
            .read_with(cx, |mw, _cx| mw.workspace().clone())
            .unwrap();

        let workspace_b = multi_workspace
            .update(cx, |multi_workspace, window, cx| {
                multi_workspace.test_add_workspace(project_b.clone(), window, cx)
            })
            .unwrap();

        let cx = &mut VisualTestContext::from_window(multi_workspace.into(), cx);

        let panel_a = workspace_a.update_in(cx, |workspace, window, cx| {
            let panel = cx.new(|cx| AgentPanel::new(workspace, window, cx));
            workspace.add_panel(panel.clone(), window, cx);
            panel
        });

        panel_a.update(cx, |panel, _cx| {
            panel.selected_agent = Agent::Stub;
        });

        let panel_b = workspace_b.update_in(cx, |workspace, window, cx| {
            let panel = cx.new(|cx| AgentPanel::new(workspace, window, cx));
            workspace.add_panel(panel.clone(), window, cx);
            panel
        });
        panel_b.update_in(cx, |panel, window, cx| {
            panel.activate_new_thread(false, AgentThreadSource::AgentPanel, window, cx);
        });

        let original_draft = panel_b.read_with(cx, |panel, cx| {
            let draft = panel.draft_thread.as_ref().expect("draft should exist");
            assert_eq!(
                *draft.read(cx).agent_key(),
                Agent::NativeAgent,
                "destination draft should start on the default agent"
            );
            draft.entity_id()
        });

        let initialized = panel_b.update_in(cx, |panel, window, cx| {
            panel.initialize_from_source_workspace_if_needed(workspace_a.downgrade(), window, cx)
        });
        assert!(
            initialized,
            "fresh destination draft should inherit the source agent"
        );

        panel_b.read_with(cx, |panel, cx| {
            let draft = panel.draft_thread.as_ref().expect("draft should exist");
            assert_ne!(
                draft.entity_id(),
                original_draft,
                "empty destination draft should be replaced when the inherited agent differs"
            );
            assert_eq!(
                *draft.read(cx).agent_key(),
                Agent::Stub,
                "empty destination draft should be rebound to the inherited agent"
            );
        });
    }

    #[gpui::test]
    async fn test_initialize_from_source_does_not_overwrite_existing_content(
        cx: &mut TestAppContext,
    ) {
        init_test(cx);
        cx.update(|cx| {
            agent::ThreadStore::init_global(cx);
            language_model::LanguageModelRegistry::test(cx);
        });

        let fs = FakeFs::new(cx.executor());
        fs.insert_tree("/project_a", json!({ "file.txt": "" }))
            .await;
        fs.insert_tree("/project_b", json!({ "file.txt": "" }))
            .await;
        let project_a = Project::test(fs.clone(), [Path::new("/project_a")], cx).await;
        let project_b = Project::test(fs.clone(), [Path::new("/project_b")], cx).await;

        let multi_workspace =
            cx.add_window(|window, cx| MultiWorkspace::test_new(project_a.clone(), window, cx));

        let workspace_a = multi_workspace
            .read_with(cx, |mw, _cx| mw.workspace().clone())
            .unwrap();

        let workspace_b = multi_workspace
            .update(cx, |multi_workspace, window, cx| {
                multi_workspace.test_add_workspace(project_b.clone(), window, cx)
            })
            .unwrap();

        let cx = &mut VisualTestContext::from_window(multi_workspace.into(), cx);

        // Set up panel_a with draft text.
        let panel_a = workspace_a.update_in(cx, |workspace, window, cx| {
            let panel = cx.new(|cx| AgentPanel::new(workspace, window, cx));
            workspace.add_panel(panel.clone(), window, cx);
            panel
        });
        cx.run_until_parked();

        panel_a.update_in(cx, |panel, window, cx| {
            panel.open_external_thread_with_server(
                Rc::new(StubAgentServer::default_response()),
                window,
                cx,
            );
        });
        cx.run_until_parked();

        let thread_view_a =
            panel_a.read_with(cx, |panel, cx| panel.active_thread_view(cx).unwrap());
        let editor_a = thread_view_a.read_with(cx, |view, _cx| view.message_editor.clone());
        editor_a.update_in(cx, |editor, window, cx| {
            editor.set_text("Draft from workspace A", window, cx);
        });

        // Set up panel_b with its OWN content — this is a non-fresh panel.
        let panel_b = workspace_b.update_in(cx, |workspace, window, cx| {
            let panel = cx.new(|cx| AgentPanel::new(workspace, window, cx));
            workspace.add_panel(panel.clone(), window, cx);
            panel
        });
        cx.run_until_parked();

        panel_b.update_in(cx, |panel, window, cx| {
            panel.open_external_thread_with_server(
                Rc::new(StubAgentServer::default_response()),
                window,
                cx,
            );
        });
        cx.run_until_parked();

        let thread_view_b =
            panel_b.read_with(cx, |panel, cx| panel.active_thread_view(cx).unwrap());
        let editor_b = thread_view_b.read_with(cx, |view, _cx| view.message_editor.clone());
        editor_b.update_in(cx, |editor, window, cx| {
            editor.set_text("Existing work in workspace B", window, cx);
        });

        // Attempting to initialize panel_b from workspace_a should be rejected
        // because panel_b already has meaningful content.
        let transferred = panel_b.update_in(cx, |panel, window, cx| {
            panel.initialize_from_source_workspace_if_needed(workspace_a.downgrade(), window, cx)
        });
        assert!(
            !transferred,
            "destination panel with existing content should not be overwritten"
        );

        // Verify panel_b still has its original content.
        panel_b.read_with(cx, |panel, cx| {
            let thread_view = panel
                .active_thread_view(cx)
                .expect("panel_b should still have its thread view");
            let text = thread_view.read(cx).message_editor.read(cx).text(cx);
            assert_eq!(
                text, "Existing work in workspace B",
                "destination panel's content should be preserved"
            );
        });
    }

    #[gpui::test]
    async fn test_create_thread_with_options_retains_thread_and_restores_agent(
        cx: &mut TestAppContext,
    ) {
        let (panel, mut cx) = setup_panel(cx).await;
        let _stub_connection =
            crate::test_support::set_stub_agent_connection(StubAgentConnection::new());

        // Baseline: panel's selected_agent is the stub.
        panel.update(&mut cx, |panel, _cx| {
            panel.selected_agent = Agent::Stub;
        });

        // Case 1: no agent override. The new thread should land in
        // `retained_threads` and `selected_agent` should be unchanged.
        let no_override_id = panel.update_in(&mut cx, |panel, window, cx| {
            panel.create_thread_with_options(
                CreateThreadOptions::default(),
                AgentThreadSource::AgentPanel,
                window,
                cx,
            )
        });

        panel.read_with(&cx, |panel, _cx| {
            assert!(
                panel.retained_threads.contains_key(&no_override_id),
                "thread created via create_thread_with_options should be retained"
            );
            assert_eq!(
                panel.selected_agent,
                Agent::Stub,
                "selected_agent should be unchanged when no agent override is requested"
            );
        });

        // Case 2: an explicit agent override that differs from the panel's
        // selection. `create_agent_thread_inner` updates `selected_agent` as a
        // side effect; `create_thread_with_options` must restore it so the
        // user's last-used agent isn't silently flipped by an agent-initiated
        // call.
        let override_agent = Agent::Custom {
            id: "override-agent".into(),
        };
        let override_id = panel.update_in(&mut cx, |panel, window, cx| {
            panel.create_thread_with_options(
                CreateThreadOptions {
                    agent: Some(override_agent.clone()),
                    ..CreateThreadOptions::default()
                },
                AgentThreadSource::AgentPanel,
                window,
                cx,
            )
        });

        panel.read_with(&cx, |panel, _cx| {
            assert!(
                panel.retained_threads.contains_key(&override_id),
                "thread created with an agent override should also be retained"
            );
            assert_ne!(
                no_override_id, override_id,
                "each call should produce a distinct ThreadId"
            );
            assert_eq!(
                panel.selected_agent,
                Agent::Stub,
                "selected_agent should be restored to the original after an agent override"
            );
        });
    }

    #[gpui::test]
    async fn test_create_omega_thread_with_message_preserves_background_view_and_can_reveal(
        cx: &mut TestAppContext,
    ) {
        let (panel, mut cx) = setup_panel(cx).await;
        let _stub_connection =
            crate::test_support::set_stub_agent_connection(StubAgentConnection::new());
        panel.update_in(&mut cx, |panel, window, cx| {
            panel.selected_agent = Agent::Stub;
            panel.activate_new_thread(false, AgentThreadSource::AgentPanel, window, cx);
        });
        let active_before = panel.read_with(&cx, |panel, cx| panel.active_thread_id(cx));

        let background_id = panel
            .update_in(&mut cx, |panel, window, cx| {
                panel.create_omega_thread_with_message(
                    "Run in the background".into(),
                    false,
                    window,
                    cx,
                )
            })
            .expect("project-backed thread should be created");

        panel.read_with(&cx, |panel, cx| {
            assert_eq!(panel.active_thread_id(cx), active_before);
            assert!(panel.retained_threads.contains_key(&background_id));
            assert_eq!(panel.selected_agent, Agent::Stub);
        });
        cx.run_until_parked();
        panel.read_with(&cx, |panel, cx| {
            let background = panel
                .retained_threads
                .get(&background_id)
                .expect("background thread should remain retained");
            assert!(background.read(cx).has_user_submitted_prompt(cx));
        });
        assert!(panel.update_in(&mut cx, |panel, window, cx| {
            panel.reveal_omega_thread(background_id, window, cx)
        }));
        panel.read_with(&cx, |panel, cx| {
            assert_eq!(panel.active_thread_id(cx), Some(background_id));
        });

        let foreground_id = panel
            .update_in(&mut cx, |panel, window, cx| {
                panel.create_omega_thread_with_message(
                    "Reveal this thread".into(),
                    true,
                    window,
                    cx,
                )
            })
            .expect("project-backed thread should be created");

        panel.read_with(&cx, |panel, cx| {
            assert_eq!(panel.active_thread_id(cx), Some(foreground_id));
            assert!(!panel.retained_threads.contains_key(&foreground_id));
            assert_eq!(panel.selected_agent, Agent::Stub);
        });
    }
}
