use gpui::{Action, App, Global, actions};
use omega_identity::HeldIdentityAction;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::{error::Error, fmt, path::PathBuf, sync::Arc};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IdentityActivationOutcome {
    Completed,
    Cancelled,
    Expired,
}

type IdentityActivationCallback = Box<dyn FnOnce(IdentityActivationOutcome, &mut App) + 'static>;

struct PendingIdentityActivation {
    intent: HeldIdentityAction,
    callback: IdentityActivationCallback,
}

/// Process-local ownership of the action that opened identity activation.
///
/// The durable identity service stores only an authenticated digest and
/// destination. The initiating surface keeps the actual payload in this
/// callback, so it is never projected into another subsystem or written to
/// disk. A process restart deliberately makes the durable intent an orphan;
/// onboarding must then cancel it instead of pretending it can resume it.
///
/// `Completed` is only a wake-up signal. The callback must consume the exact
/// held authorization from `omega_identity`, revalidate its live destination,
/// and resume at most once. The coordinator intentionally cannot authorize or
/// publish an action itself.
#[derive(Default)]
pub struct IdentityActivationEvents {
    pending: Option<PendingIdentityActivation>,
}

impl Global for IdentityActivationEvents {}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IdentityActivationEventError {
    IntentAlreadyOwned,
    DifferentIntentPending,
    IntentNotOwned,
    IntentExpired,
}

impl fmt::Display for IdentityActivationEventError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::IntentAlreadyOwned => {
                formatter.write_str("the identity activation action already has an owner")
            }
            Self::DifferentIntentPending => {
                formatter.write_str("a different identity activation action is already pending")
            }
            Self::IntentNotOwned => {
                formatter.write_str("the identity activation action is not owned by this process")
            }
            Self::IntentExpired => formatter.write_str("the identity activation action expired"),
        }
    }
}

impl Error for IdentityActivationEventError {}

impl IdentityActivationEvents {
    pub fn register(
        intent: HeldIdentityAction,
        now: u64,
        callback: impl FnOnce(IdentityActivationOutcome, &mut App) + 'static,
        cx: &mut App,
    ) -> Result<(), IdentityActivationEventError> {
        if intent.expires_at <= now {
            return Err(IdentityActivationEventError::IntentExpired);
        }
        let events = cx.default_global::<Self>();
        if let Some(pending) = &events.pending {
            if pending.intent == intent {
                return Err(IdentityActivationEventError::IntentAlreadyOwned);
            }
            return Err(IdentityActivationEventError::DifferentIntentPending);
        }
        events.pending = Some(PendingIdentityActivation {
            intent,
            callback: Box::new(callback),
        });
        Ok(())
    }

    pub fn owns(intent: &HeldIdentityAction, cx: &App) -> bool {
        cx.try_global::<Self>()
            .and_then(|events| events.pending.as_ref())
            .is_some_and(|pending| pending.intent == *intent)
    }

    pub fn complete(
        intent: &HeldIdentityAction,
        cx: &mut App,
    ) -> Result<(), IdentityActivationEventError> {
        Self::finish(intent, IdentityActivationOutcome::Completed, cx)
    }

    pub fn cancel(
        intent: &HeldIdentityAction,
        cx: &mut App,
    ) -> Result<(), IdentityActivationEventError> {
        Self::finish(intent, IdentityActivationOutcome::Cancelled, cx)
    }

    pub fn prune_expired(now: u64, cx: &mut App) -> bool {
        let callback = {
            let events = cx.default_global::<Self>();
            match events.pending.as_ref() {
                Some(pending) if pending.intent.expires_at <= now => {
                    events.pending.take().map(|pending| pending.callback)
                }
                _ => None,
            }
        };
        if let Some(callback) = callback {
            callback(IdentityActivationOutcome::Expired, cx);
            true
        } else {
            false
        }
    }

    fn finish(
        intent: &HeldIdentityAction,
        outcome: IdentityActivationOutcome,
        cx: &mut App,
    ) -> Result<(), IdentityActivationEventError> {
        let callback = {
            let events = cx.default_global::<Self>();
            let pending = events
                .pending
                .as_ref()
                .ok_or(IdentityActivationEventError::IntentNotOwned)?;
            if pending.intent != *intent {
                return Err(IdentityActivationEventError::IntentNotOwned);
            }
            events
                .pending
                .take()
                .map(|pending| pending.callback)
                .ok_or(IdentityActivationEventError::IntentNotOwned)?
        };
        callback(outcome, cx);
        Ok(())
    }
}

// If the zed binary doesn't use anything in this crate, it will be optimized away
// and the actions won't initialize. So we just provide an empty initialization function
// to be called from main.
//
// These may provide relevant context:
// https://github.com/rust-lang/rust/issues/47384
// https://github.com/mmastrac/rust-ctor/issues/280
pub fn init() {}

/// Opens a URL in the system's default web browser.
#[derive(Clone, PartialEq, Deserialize, JsonSchema, Action)]
#[action(namespace = omega, deprecated_aliases = ["zed::OpenBrowser"])]
#[serde(deny_unknown_fields)]
pub struct OpenBrowser {
    pub url: Arc<str>,
}

/// Opens an application URL — the Omega channel scheme, or the legacy
/// `zed://` scheme kept for compatibility with existing links.
#[derive(Clone, PartialEq, Deserialize, JsonSchema, Action)]
#[action(namespace = omega, deprecated_aliases = ["zed::OpenZedUrl"])]
#[serde(deny_unknown_fields)]
pub struct OpenAppUrl {
    pub url: Arc<str>,
}

/// Opens the keymap to either add a keybinding or change an existing one
#[derive(PartialEq, Clone, Default, Action, JsonSchema, Serialize, Deserialize)]
#[action(namespace = omega, no_json, no_register, deprecated_aliases = ["zed::ChangeKeybinding"])]
pub struct ChangeKeybinding {
    pub action: String,
}

actions!(
    omega,
    [
        /// Opens the settings editor.
        #[action(deprecated_aliases = ["zed::OpenSettings", "zed_actions::OpenSettingsEditor"])]
        OpenSettings,
        /// Opens Settings as a route inside a Comet-mode application window.
        OpenEmbeddedSettings,
        /// Closes an embedded settings route and returns to the prior app route.
        CloseEmbeddedSettings,
        /// Opens the full inherited settings editor.
        OpenLegacySettings,
        /// Opens the settings JSON file.
        #[action(deprecated_aliases = ["zed::OpenSettingsFile", "zed_actions::OpenSettings"])]
        OpenSettingsFile,
        /// Opens project-specific settings.
        #[action(deprecated_aliases = ["zed::OpenProjectSettings", "zed_actions::OpenProjectSettings"])]
        OpenProjectSettings,
        /// Opens the project tasks configuration.
        #[action(deprecated_aliases = ["zed::OpenProjectTasks"])]
        OpenProjectTasks,
        /// Opens the project tasks configuration with worktree setup guidance.
        #[action(deprecated_aliases = ["zed::OpenWorktreeSetupTasks"])]
        OpenWorktreeSetupTasks,
        /// Opens the default keymap file.
        #[action(deprecated_aliases = ["zed::OpenDefaultKeymap"])]
        OpenDefaultKeymap,
        /// Opens the user keymap file.
        #[action(deprecated_aliases = ["zed::OpenKeymapFile", "zed_actions::OpenKeymap"])]
        OpenKeymapFile,
        /// Opens the keymap editor.
        #[action(deprecated_aliases = ["zed::OpenKeymap", "zed_actions::OpenKeymapEditor"])]
        OpenKeymap,
        /// Opens account settings.
        #[action(deprecated_aliases = ["zed::OpenAccountSettings"])]
        OpenAccountSettings,
        /// Opens server settings.
        #[action(deprecated_aliases = ["zed::OpenServerSettings"])]
        OpenServerSettings,
        /// Quits the application.
        #[action(deprecated_aliases = ["zed::Quit"])]
        Quit,
        /// Shows information about Omega.
        #[action(deprecated_aliases = ["zed::About"])]
        About,
        /// Opens the documentation website.
        #[action(deprecated_aliases = ["zed::OpenDocs"])]
        OpenDocs,
        /// Views open source licenses.
        #[action(deprecated_aliases = ["zed::OpenLicenses"])]
        OpenLicenses,
        /// Opens the Omega status page.
        #[action(deprecated_aliases = ["zed::OpenStatusPage"])]
        OpenStatusPage,
        /// Opens the Omega merch store.
        #[action(deprecated_aliases = ["zed::GetMerch"])]
        GetMerch,
        /// Restarts the application.
        #[action(deprecated_aliases = ["zed::Restart"])]
        Restart,
        /// Opens the telemetry log.
        #[action(deprecated_aliases = ["zed::OpenTelemetryLog"])]
        OpenTelemetryLog,
        /// Opens the performance profiler.
        #[action(deprecated_aliases = ["zed::OpenPerformanceProfiler"])]
        OpenPerformanceProfiler,
        /// Shows the auto-update notification for testing.
        #[action(deprecated_aliases = ["zed::ShowUpdateNotification"])]
        ShowUpdateNotification,
    ]
);

actions!(
    omega,
    [
        /// Opens Editor Onboarding (theme, keymap, identity).
        ///
        /// Prefer the command palette (`cmd-shift-p` on macOS /
        /// `ctrl-shift-p` elsewhere), not the file picker (`cmd-p` /
        /// `ctrl-p`). Also available from Help → Editor Onboarding and
        /// Welcome → Return to Onboarding.
        #[action(deprecated_aliases = ["zed::OpenOnboarding"])]
        OpenOnboarding,
        /// Opens Editor Onboarding (same journey as OpenOnboarding).
        ///
        /// Prefer the command palette (`cmd-shift-p` on macOS /
        /// `ctrl-shift-p` elsewhere), not the file picker (`cmd-p` /
        /// `ctrl-p`).
        #[action(deprecated_aliases = ["zed::OpenEditorOnboarding"])]
        OpenEditorOnboarding,
        /// Opens the local Nostr account dashboard.
        OpenIdentityDashboard,
        /// Opens NIP-46 remote signer setup in the account dashboard.
        OpenRemoteSignerSetup,
    ]
);

#[derive(PartialEq, Clone, Copy, Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ExtensionCategoryFilter {
    Themes,
    IconThemes,
    Languages,
    Grammars,
    LanguageServers,
    ContextServers,
    Snippets,
    DebugAdapters,
}

/// Opens the extensions management interface.
#[derive(PartialEq, Clone, Default, Debug, Deserialize, JsonSchema, Action)]
#[action(namespace = omega, deprecated_aliases = ["zed::Extensions"])]
#[serde(deny_unknown_fields)]
pub struct Extensions {
    /// Filters the extensions page down to extensions that are in the specified category.
    #[serde(default)]
    pub category_filter: Option<ExtensionCategoryFilter>,
    /// Focuses just the extension with the specified ID.
    #[serde(default)]
    pub id: Option<String>,
}

/// Opens the ACP registry.
#[derive(PartialEq, Clone, Default, Debug, Deserialize, JsonSchema, Action)]
#[action(namespace = omega, deprecated_aliases = ["zed::AcpRegistry"])]
#[serde(deny_unknown_fields)]
pub struct AcpRegistry;

/// Show call diagnostics and connection quality statistics.
#[derive(PartialEq, Clone, Default, Debug, Deserialize, JsonSchema, Action)]
#[action(namespace = collab)]
#[serde(deny_unknown_fields)]
pub struct ShowCallStats;

/// Decreases the font size in the editor buffer.
#[derive(PartialEq, Clone, Default, Debug, Deserialize, JsonSchema, Action)]
#[action(namespace = omega, deprecated_aliases = ["zed::DecreaseBufferFontSize"])]
#[serde(deny_unknown_fields)]
pub struct DecreaseBufferFontSize {
    #[serde(default)]
    pub persist: bool,
}

/// Increases the font size in the editor buffer.
#[derive(PartialEq, Clone, Default, Debug, Deserialize, JsonSchema, Action)]
#[action(namespace = omega, deprecated_aliases = ["zed::IncreaseBufferFontSize"])]
#[serde(deny_unknown_fields)]
pub struct IncreaseBufferFontSize {
    #[serde(default)]
    pub persist: bool,
}

/// Opens the settings editor at a specific path.
#[derive(PartialEq, Clone, Debug, Deserialize, JsonSchema, Action)]
#[action(namespace = omega, deprecated_aliases = ["zed::OpenSettingsAt"])]
#[serde(deny_unknown_fields)]
pub struct OpenSettingsAt {
    /// A path to a specific setting (e.g. `theme.mode`)
    pub path: String,
    /// The settings file to select before opening `path`. When omitted, the
    /// existing settings file selection is preserved.
    #[serde(default)]
    pub target: Option<OpenSettingsAtTarget>,
}

#[derive(PartialEq, Clone, Debug, Deserialize, JsonSchema, Action)]
#[action(namespace = omega, deprecated_aliases = ["zed::OpenSettingsPage"])]
#[serde(deny_unknown_fields)]
pub struct OpenSettingsPage {
    /// A settings page title (e.g. `AI`).
    pub page: String,
    /// The settings file to select before opening `page`. When omitted, the
    /// existing settings file selection is preserved.
    #[serde(default)]
    pub target: Option<OpenSettingsAtTarget>,
}

/// `OpenSettingsAt` path of the agent skills page in the settings UI.
pub const AGENT_SKILLS_SETTINGS_PATH: &str = "agent.skills";

/// `OpenSettingsAt` path of the agent sandbox permissions page in the settings
/// UI.
pub const AGENT_SANDBOX_SETTINGS_PATH: &str = "agent.sandbox_permissions";

#[derive(PartialEq, Clone, Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum OpenSettingsAtTarget {
    User,
    Project { worktree_id: usize },
}

/// Resets the buffer font size to the default value.
#[derive(PartialEq, Clone, Default, Debug, Deserialize, JsonSchema, Action)]
#[action(namespace = omega, deprecated_aliases = ["zed::ResetBufferFontSize"])]
#[serde(deny_unknown_fields)]
pub struct ResetBufferFontSize {
    #[serde(default)]
    pub persist: bool,
}

/// Decreases the font size of the user interface.
#[derive(PartialEq, Clone, Default, Debug, Deserialize, JsonSchema, Action)]
#[action(namespace = omega, deprecated_aliases = ["zed::DecreaseUiFontSize"])]
#[serde(deny_unknown_fields)]
pub struct DecreaseUiFontSize {
    #[serde(default)]
    pub persist: bool,
}

/// Increases the font size of the user interface.
#[derive(PartialEq, Clone, Default, Debug, Deserialize, JsonSchema, Action)]
#[action(namespace = omega, deprecated_aliases = ["zed::IncreaseUiFontSize"])]
#[serde(deny_unknown_fields)]
pub struct IncreaseUiFontSize {
    #[serde(default)]
    pub persist: bool,
}

/// Resets the UI font size to the default value.
#[derive(PartialEq, Clone, Default, Debug, Deserialize, JsonSchema, Action)]
#[action(namespace = omega, deprecated_aliases = ["zed::ResetUiFontSize"])]
#[serde(deny_unknown_fields)]
pub struct ResetUiFontSize {
    #[serde(default)]
    pub persist: bool,
}

/// Resets all zoom levels (UI and buffer font sizes, including in the agent panel) to their default values.
#[derive(PartialEq, Clone, Default, Debug, Deserialize, JsonSchema, Action)]
#[action(namespace = omega, deprecated_aliases = ["zed::ResetAllZoom"])]
#[serde(deny_unknown_fields)]
pub struct ResetAllZoom {
    #[serde(default)]
    pub persist: bool,
}

pub mod editor {
    use gpui::actions;
    actions!(
        editor,
        [
            /// Moves cursor up.
            MoveUp,
            /// Moves cursor down.
            MoveDown,
            /// Reveals the current file in the system file manager.
            RevealInFileManager,
        ]
    );
}

pub mod dev {
    use gpui::actions;

    actions!(
        dev,
        [
            /// Clears Omega onboarding completion records (debug builds).
            ///
            /// Use the command palette (`cmd-shift-p` on macOS / `ctrl-shift-p`
            /// elsewhere), not the file picker (`cmd-p` / `ctrl-p`).
            ResetOnboarding,
        ]
    );
}

pub mod remote_debug {
    use gpui::actions;

    actions!(
        remote_debug,
        [
            /// Simulates a disconnection from the remote server for testing purposes.
            /// This will trigger the reconnection logic.
            SimulateDisconnect,
            /// Simulates a timeout/slow connection to the remote server for testing purposes.
            /// This will cause heartbeat failures and trigger reconnection.
            SimulateTimeout,
            /// Simulates a timeout/slow connection to the remote server for testing purposes.
            /// This will cause heartbeat failures and attempting a reconnection while having exhausted all attempts.
            SimulateTimeoutExhausted,
        ]
    );
}

pub mod workspace {
    use gpui::actions;

    actions!(
        workspace,
        [
            #[action(deprecated_aliases = ["editor::CopyPath", "outline_panel::CopyPath", "project_panel::CopyPath"])]
            CopyPath,
            #[action(deprecated_aliases = ["editor::CopyRelativePath", "outline_panel::CopyRelativePath", "project_panel::CopyRelativePath"])]
            CopyRelativePath,
            /// Opens the selected file with the system's default application.
            #[action(deprecated_aliases = ["project_panel::OpenWithSystem"])]
            OpenWithSystem,
        ]
    );
}

/// Describes which ref to base a new git worktree on. The worktree is
/// always created in a detached HEAD state; users can opt into creating
/// a branch afterwards from the worktree itself.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum NewWorktreeBranchTarget {
    /// Create a detached worktree from the current HEAD.
    #[default]
    CurrentBranch,
    /// Create a detached worktree at the tip of an existing branch.
    ExistingBranch { name: String },
    /// Create a detached worktree at the tip of a remote-tracking branch.
    RemoteBranch {
        remote_name: String,
        branch_name: String,
    },
}

/// Creates a new git worktree and switches the workspace to it.
/// Dispatched by the unified worktree picker when the user selects a "Create new worktree" entry.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema, Action)]
#[action(namespace = git)]
#[serde(deny_unknown_fields)]
pub struct CreateWorktree {
    /// When this is None, Omega will randomly generate a worktree name.
    pub worktree_name: Option<String>,
    pub branch_target: NewWorktreeBranchTarget,
}

/// Switches the workspace to an existing linked worktree.
/// Dispatched by the unified worktree picker when the user selects an existing worktree.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema, Action)]
#[action(namespace = git)]
#[serde(deny_unknown_fields)]
pub struct SwitchWorktree {
    pub path: PathBuf,
    pub display_name: String,
}

/// Opens an existing worktree in a new window.
/// Dispatched by the worktree picker's "Open in New Window" button.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema, Action)]
#[action(namespace = git)]
#[serde(deny_unknown_fields)]
pub struct OpenWorktreeInNewWindow {
    pub path: PathBuf,
}

pub mod git {
    use gpui::actions;

    actions!(
        git,
        [
            /// Checks out a different git branch.
            CheckoutBranch,
            /// Switches to a different git branch.
            Switch,
            /// Selects a different repository.
            SelectRepo,
            /// Filter remotes.
            FilterRemotes,
            /// Create a git remote.
            CreateRemote,
            /// Opens the git branch selector.
            #[action(deprecated_aliases = ["branches::OpenRecent"])]
            Branch,
            /// Shows uncommitted changes across the project.
            ViewUncommittedChanges,
            /// Shows unstaged changes across the project.
            ViewUnstagedChanges,
            /// Shows staged changes across the project.
            ViewStagedChanges,
            /// Opens the git stash selector.
            ViewStash,
            /// Opens the git worktree selector.
            Worktree,
            /// Creates a pull request for the current branch.
            CreatePullRequest
        ]
    );
}

pub mod toast {
    use gpui::actions;

    actions!(
        toast,
        [
            /// Runs the action associated with a toast notification.
            RunAction
        ]
    );
}

pub mod command_palette {
    use gpui::actions;

    actions!(
        command_palette,
        [
            /// Toggles the command palette.
            Toggle,
        ]
    );
}

pub mod text_finder {
    use gpui::actions;

    actions!(
        text_finder,
        [
            /// Opens the Project Search Picker.
            Toggle,
        ]
    );
}

pub mod project_panel {
    use gpui::actions;

    actions!(
        project_panel,
        [
            /// Toggles the project panel.
            Toggle,
            /// Toggles focus on the project panel.
            ToggleFocus
        ]
    );
}
pub mod theme {
    use gpui::actions;

    actions!(theme, [ToggleMode]);
}

pub mod search {
    use gpui::actions;
    actions!(
        search,
        [
            /// Toggles searching in ignored files.
            ToggleIncludeIgnored
        ]
    );
}
pub mod buffer_search {
    use gpui::{Action, actions};
    use schemars::JsonSchema;
    use serde::Deserialize;

    /// Opens the buffer search interface with the specified configuration.
    #[derive(PartialEq, Clone, Deserialize, JsonSchema, Action)]
    #[action(namespace = buffer_search)]
    #[serde(deny_unknown_fields)]
    pub struct Deploy {
        #[serde(default = "util::serde::default_true")]
        pub focus: bool,
        #[serde(default)]
        pub replace_enabled: bool,
        #[serde(default)]
        pub selection_search_enabled: bool,
    }

    impl Deploy {
        pub fn find() -> Self {
            Self {
                focus: true,
                replace_enabled: false,
                selection_search_enabled: false,
            }
        }

        pub fn replace() -> Self {
            Self {
                focus: true,
                replace_enabled: true,
                selection_search_enabled: false,
            }
        }
    }

    actions!(
        buffer_search,
        [
            /// Deploys the search and replace interface.
            DeployReplace,
            /// Dismisses the search bar.
            Dismiss,
            /// Focuses back on the editor.
            FocusEditor,
            /// Sets the search query from the selection or word under cursor.
            UseSelectionForFind,
        ]
    );
}
pub mod agent {
    use gpui::{Action, SharedString, actions};
    use schemars::JsonSchema;
    use serde::Deserialize;

    actions!(
        agent,
        [
            /// Opens the agent settings UI.
            #[action(deprecated_aliases = ["agent::OpenConfiguration"])]
            OpenSettings,
            /// Opens the agent onboarding modal.
            OpenOnboardingModal,
            /// Resets the agent onboarding state.
            ResetOnboarding,
            /// Starts a chat conversation with the agent.
            Chat,
            /// Toggles the language model selector dropdown.
            #[action(deprecated_aliases = ["assistant::ToggleModelSelector", "assistant2::ToggleModelSelector"])]
            ToggleModelSelector,
            /// Triggers re-authentication on Gemini
            ReauthenticateAgent,
            /// Logs out of the current external agent
            LogoutAgent,
            /// Add the current selection as context for threads in the agent panel.
            #[action(deprecated_aliases = ["assistant::QuoteSelection", "agent::QuoteSelection"])]
            AddSelectionToThread,
            /// Resets the agent panel zoom levels (agent UI and buffer font sizes).
            ResetAgentZoom,
            /// Pastes clipboard content without any formatting.
            PasteRaw,
        ]
    );

    /// Selects the agent used for new threads in the agent panel, without
    /// opening the panel. The selected agent is launched the next time the
    /// panel is opened.
    #[derive(Clone, PartialEq, Deserialize, JsonSchema, Action)]
    #[action(namespace = agent)]
    #[serde(deny_unknown_fields)]
    pub struct SelectAgent {
        /// The id of the agent to select.
        pub agent: String,
    }

    /// Opens a new agent thread with the provided branch diff for review.
    #[derive(Clone, PartialEq, Deserialize, JsonSchema, Action)]
    #[action(namespace = agent)]
    #[serde(deny_unknown_fields)]
    pub struct ReviewBranchDiff {
        /// The full text of the diff to review.
        pub diff_text: SharedString,
        /// The base ref that the diff was computed against (e.g. "main").
        pub base_ref: SharedString,
    }

    /// A single merge conflict region extracted from a file.
    #[derive(Clone, Debug, PartialEq, Deserialize, JsonSchema)]
    pub struct ConflictContent {
        pub file_path: String,
        pub conflict_text: String,
        pub ours_branch_name: String,
        pub theirs_branch_name: String,
    }

    /// Opens a new agent thread to resolve specific merge conflicts.
    #[derive(Clone, PartialEq, Deserialize, JsonSchema, Action)]
    #[action(namespace = agent)]
    #[serde(deny_unknown_fields)]
    pub struct ResolveConflictsWithAgent {
        /// Individual conflicts with their full text.
        pub conflicts: Vec<ConflictContent>,
    }

    /// Opens a new agent thread to resolve merge conflicts in the given file paths.
    #[derive(Clone, PartialEq, Deserialize, JsonSchema, Action)]
    #[action(namespace = agent)]
    #[serde(deny_unknown_fields)]
    pub struct ResolveConflictedFilesWithAgent {
        /// File paths with unresolved conflicts (for project-wide resolution).
        pub conflicted_file_paths: Vec<String>,
    }
}

pub mod assistant {
    use gpui::{Action, actions};
    use schemars::JsonSchema;
    use serde::Deserialize;

    actions!(
        agent,
        [
            /// Toggles the agent panel.
            Toggle,
            #[action(deprecated_aliases = ["assistant::ToggleFocus"])]
            ToggleFocus,
            FocusAgent,
            /// Opens the skill creator window for creating a new skill.
            OpenSkillCreator,
            /// Opens the skill creator window to import a skill from a GitHub URL.
            CreateSkillFromUrl,
            /// Opens the user-global AGENTS.md rules file.
            #[action(name = "OpenGlobalAGENTS.mdRules")]
            OpenGlobalAgentsMdRules,
            /// Opens the project AGENTS.md rules file.
            #[action(name = "OpenProjectAGENTS.mdRules")]
            OpenProjectAgentsMdRules,
            /// Opens the skills manager in the settings window.
            #[action(deprecated_aliases = ["agent::OpenRulesLibrary", "assistant::OpenRulesLibrary", "assistant::DeployPromptLibrary"])]
            ManageSkills,
        ]
    );

    /// Deploys the assistant interface with the specified configuration.
    #[derive(Clone, Default, Deserialize, PartialEq, JsonSchema, Action)]
    #[action(namespace = assistant)]
    #[serde(deny_unknown_fields)]
    pub struct InlineAssist {
        pub prompt: Option<String>,
    }
}

/// Opens the recent projects interface.
#[derive(PartialEq, Clone, Deserialize, Default, JsonSchema, Action)]
#[action(namespace = projects)]
#[serde(deny_unknown_fields)]
pub struct OpenRecent {
    #[serde(default)]
    pub create_new_window: Option<bool>,
}

/// Creates a project from a selected template.
#[derive(PartialEq, Clone, Deserialize, Default, JsonSchema, Action)]
#[action(namespace = projects)]
#[serde(deny_unknown_fields)]
pub struct OpenRemote {
    #[serde(default)]
    pub from_existing_connection: bool,
    #[serde(default)]
    pub create_new_window: Option<bool>,
}

/// Opens the dev container connection modal.
#[derive(PartialEq, Clone, Deserialize, Default, JsonSchema, Action)]
#[action(namespace = projects)]
#[serde(deny_unknown_fields)]
pub struct OpenDevContainer;

/// Where to spawn the task in the UI.
#[derive(Default, Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum RevealTarget {
    /// In the central pane group, "main" editor area.
    Center,
    /// In the terminal dock, "regular" terminal items' place.
    #[default]
    Dock,
}

/// Spawns a task with name or opens tasks modal.
#[derive(Debug, PartialEq, Clone, Deserialize, JsonSchema, Action)]
#[action(namespace = task)]
#[serde(untagged)]
pub enum Spawn {
    /// Spawns a task by the name given.
    ByName {
        task_name: String,
        #[serde(default)]
        reveal_target: Option<RevealTarget>,
    },
    /// Spawns a task by the tag given.
    ByTag {
        task_tag: String,
        #[serde(default)]
        reveal_target: Option<RevealTarget>,
    },
    /// Spawns a task via modal's selection.
    ViaModal {
        /// Selected task's `reveal_target` property override.
        #[serde(default)]
        reveal_target: Option<RevealTarget>,
    },
}

impl Spawn {
    pub fn modal() -> Self {
        Self::ViaModal {
            reveal_target: None,
        }
    }
}

/// Reruns the last task.
#[derive(PartialEq, Clone, Deserialize, Default, JsonSchema, Action)]
#[action(namespace = task)]
#[serde(deny_unknown_fields)]
pub struct Rerun {
    /// Controls whether the task context is reevaluated prior to execution of a task.
    /// If it is not, environment variables such as ZED_COLUMN, ZED_FILE are gonna be the same as in the last execution of a task
    /// If it is, these variables will be updated to reflect current state of editor at the time task::Rerun is executed.
    /// default: false
    #[serde(default)]
    pub reevaluate_context: bool,
    /// Overrides `allow_concurrent_runs` property of the task being reran.
    /// Default: null
    #[serde(default)]
    pub allow_concurrent_runs: Option<bool>,
    /// Overrides `use_new_terminal` property of the task being reran.
    /// Default: null
    #[serde(default)]
    pub use_new_terminal: Option<bool>,

    /// If present, rerun the task with this ID, otherwise rerun the last task.
    #[serde(skip)]
    pub task_id: Option<String>,
}

pub mod outline {
    use std::sync::OnceLock;

    use gpui::{AnyView, App, Window, actions};

    actions!(
        outline,
        [
            #[action(name = "Toggle")]
            ToggleOutline
        ]
    );
    /// A pointer to outline::toggle function, exposed here to sewer the breadcrumbs <-> outline dependency.
    pub static TOGGLE_OUTLINE: OnceLock<fn(AnyView, &mut Window, &mut App)> = OnceLock::new();
}

actions!(
    omega_predict_onboarding,
    [
        /// Opens the Omega Predict onboarding modal.
        #[action(deprecated_aliases = ["zed_predict_onboarding::OpenZedPredictOnboarding"])]
        OpenOmegaPredictOnboarding
    ]
);
actions!(
    git_onboarding,
    [
        /// Opens the git integration onboarding modal.
        OpenGitIntegrationOnboarding
    ]
);

pub mod debug_panel {
    use gpui::actions;
    actions!(
        debug_panel,
        [
            /// Toggles the debug panel.
            Toggle,
            /// Toggles focus on the debug panel.
            ToggleFocus
        ]
    );
}

pub mod full_auto_panel {
    use gpui::actions;
    actions!(
        full_auto_panel,
        [
            /// Toggles focus on the Full Auto panel.
            ToggleFocus,
            /// Opens the Full Auto launcher (never a composer toggle).
            OpenLauncher
        ]
    );
}

pub mod agent_computer {
    use gpui::actions;
    actions!(
        agent_computer,
        [
            /// Opens the Agent Computer panel.
            OpenPanel,
            /// Starts one Agent Computer cloud turn from the panel.
            StartTurn
        ]
    );
}

pub mod workroom {
    use gpui::actions;
    actions!(
        workroom,
        [
            /// Opens the Sarah workroom panel.
            OpenPanel,
            /// Focuses the Sarah workroom composer.
            FocusComposer,
            /// Sends the composer text as an owner message (pending until confirmed).
            SendMessage,
            /// Sends a typed interrupt intent for the active Sarah turn.
            InterruptTurn,
            /// Loads Sarah voice admission terms without opening audio or reserving credit.
            PrepareVoiceAdmission,
            /// Starts Sarah's managed Realtime voice session.
            StartVoice,
            /// Starts Sarah voice from the composer, loading admission terms
            /// first when they are not already current.
            ///
            /// `OMEGA-DELTA-0211`. The composer microphone never navigates.
            /// This is the one action that turns a composer click into audio
            /// without an interstitial page, and it reports its own refusal
            /// through the composer's voice notice.
            StartVoiceFromComposer,
            /// Mutes or unmutes Sarah's microphone capture.
            ToggleVoiceMute,
            /// Interrupts Sarah's current spoken response.
            InterruptVoice,
            /// Approves the currently projected Sarah command exactly once.
            ApproveSarahVoiceCommand,
            /// Declines the currently projected Sarah command.
            RejectSarahVoiceCommand,
            /// Ends Sarah's managed Realtime voice session.
            EndVoice,
            /// Retries Sarah's managed Realtime voice session after a failure.
            RetryVoice
        ]
    );
}

/// Actions for Sarah voice in the selected tester/community channel.
pub mod community_sarah {
    use gpui::actions;

    actions!(
        community_sarah,
        [
            /// Joins voice for the selected tester/community channel.
            JoinRoom,
            /// Leaves voice for the selected tester/community channel.
            LeaveRoom,
            /// Mutes or unmutes the local community-room microphone.
            ToggleMute,
            /// Summons Sarah into the selected community room.
            SummonSarah,
            /// Removes Sarah from the selected community room.
            RemoveSarah,
            /// Requests or transfers the bounded Sarah speaking floor.
            TalkToSarah,
            /// Ends Sarah's community-room presence with moderator authority.
            ModeratorStop
        ]
    );
}

actions!(
    debugger,
    [
        /// Toggles the enabled state of a breakpoint.
        ToggleEnableBreakpoint,
        /// Removes a breakpoint.
        UnsetBreakpoint,
        /// Opens the project debug tasks configuration.
        OpenProjectDebugTasks,
    ]
);

pub mod vim {
    use gpui::actions;

    actions!(
        vim,
        [
            /// Opens the default keymap file.
            OpenDefaultKeymap
        ]
    );
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct WslConnectionOptions {
    pub distro_name: String,
    pub user: Option<String>,
}

#[cfg(target_os = "windows")]
pub mod wsl_actions {
    use gpui::Action;
    use schemars::JsonSchema;
    use serde::Deserialize;

    /// Opens a folder inside Wsl.
    #[derive(PartialEq, Clone, Deserialize, Default, JsonSchema, Action)]
    #[action(namespace = projects)]
    #[serde(deny_unknown_fields)]
    pub struct OpenFolderInWsl {
        #[serde(default)]
        pub create_new_window: Option<bool>,
    }

    /// Open a wsl distro.
    #[derive(PartialEq, Clone, Deserialize, Default, JsonSchema, Action)]
    #[action(namespace = projects)]
    #[serde(deny_unknown_fields)]
    pub struct OpenWsl {
        #[serde(default)]
        pub create_new_window: Option<bool>,
    }
}

pub mod preview {
    pub mod markdown {
        use gpui::actions;

        actions!(
            markdown,
            [
                /// Opens a markdown preview for the current file.
                OpenPreview,
                /// Opens a markdown preview in a split pane.
                OpenPreviewToTheSide,
            ]
        );
    }

    pub mod svg {
        use gpui::actions;

        actions!(
            svg,
            [
                /// Opens an SVG preview for the current file.
                OpenPreview,
                /// Opens an SVG preview in a split pane.
                OpenPreviewToTheSide,
            ]
        );
    }
}

pub mod agents_sidebar {
    use gpui::{Action, actions};
    use schemars::JsonSchema;
    use serde::Deserialize;

    /// Toggles the thread switcher popup when the sidebar is focused.
    #[derive(PartialEq, Clone, Deserialize, JsonSchema, Default, Action)]
    #[action(namespace = agents_sidebar)]
    #[serde(deny_unknown_fields)]
    pub struct ToggleThreadSwitcher {
        #[serde(default)]
        pub select_last: bool,
    }

    actions!(
        agents_sidebar,
        [
            /// Moves focus to the sidebar's search/filter editor.
            FocusSidebarFilter,
        ]
    );
}

pub mod notebook {
    use gpui::actions;

    actions!(
        notebook,
        [
            /// Opens a Jupyter notebook file.
            OpenNotebook,
            /// Runs all cells in the notebook.
            RunAll,
            /// Runs the current cell and stays on it.
            Run,
            /// Runs the current cell and advances to the next cell.
            RunAndAdvance,
            /// Clears all cell outputs.
            ClearOutputs,
            /// Moves the current cell up.
            MoveCellUp,
            /// Moves the current cell down.
            MoveCellDown,
            /// Adds a new markdown cell.
            AddMarkdownBlock,
            /// Adds a new code cell.
            AddCodeBlock,
            /// Restarts the kernel.
            RestartKernel,
            /// Interrupts the current execution.
            InterruptKernel,
            /// Move down in cells.
            NotebookMoveDown,
            /// Move up in cells.
            NotebookMoveUp,
            /// Enters the current cell's editor (edit mode).
            EnterEditMode,
            /// Exits the cell editor and returns to cell command mode.
            EnterCommandMode,
        ]
    );
}

pub mod git_panel {
    use gpui::actions;

    actions!(
        git_panel,
        [
            /// Toggles focus on the git panel.
            ToggleFocus,
        ]
    );
}

#[cfg(test)]
mod tests {
    use std::{cell::Cell, rc::Rc};

    use super::{
        IdentityActivationEventError, IdentityActivationEvents, IdentityActivationOutcome,
        OpenEditorOnboarding, OpenIdentityDashboard, OpenOnboarding, dev::ResetOnboarding,
    };
    use gpui::{Action, TestAppContext};
    use omega_identity::{
        AccountRef, DurableIdentityActionKind, HeldIdentityAction, IdentityRef, ProofRef,
        ReceiptRef, ResourceRef,
    };

    fn held_intent(expires_at: u64) -> HeldIdentityAction {
        HeldIdentityAction {
            intent_ref: ReceiptRef::new("public-post-request").expect("valid intent ref"),
            account_ref: AccountRef::new("omega-account-test").expect("valid account ref"),
            account_generation: 1,
            identity_ref: IdentityRef::new(
                "omega-nostr-79be667ef9dcbbac55a06295ce870b07029bfcdb2dce28d959f2815b16f81798",
            )
            .expect("valid identity ref"),
            kind: DurableIdentityActionKind::PublicPost,
            destination_ref: ResourceRef::new("nip29-test-channel").expect("valid destination ref"),
            authorization_ref: ProofRef::new("activation-public-post-request")
                .expect("valid authorization ref"),
            payload_digest: "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
                .to_string(),
            issued_at: 10,
            expires_at,
        }
    }

    #[test]
    fn onboarding_actions_use_omega_product_namespace() {
        assert_eq!(OpenOnboarding.name(), "omega::OpenOnboarding");
        assert_eq!(OpenEditorOnboarding.name(), "omega::OpenEditorOnboarding");
        assert_eq!(OpenIdentityDashboard.name(), "omega::OpenIdentityDashboard");
        assert_eq!(ResetOnboarding.name(), "dev::ResetOnboarding");
    }

    #[gpui::test]
    fn activation_completion_resumes_the_exact_owner_once(cx: &mut TestAppContext) {
        let intent = held_intent(100);
        let observed = Rc::new(Cell::new(None));

        cx.update(|cx| {
            IdentityActivationEvents::register(
                intent.clone(),
                20,
                {
                    let observed = observed.clone();
                    move |outcome, _| observed.set(Some(outcome))
                },
                cx,
            )
            .expect("register exact owner");
            assert!(IdentityActivationEvents::owns(&intent, cx));

            IdentityActivationEvents::complete(&intent, cx).expect("complete exact owner");
            assert!(!IdentityActivationEvents::owns(&intent, cx));
            assert_eq!(
                IdentityActivationEvents::complete(&intent, cx),
                Err(IdentityActivationEventError::IntentNotOwned)
            );
        });

        assert_eq!(observed.get(), Some(IdentityActivationOutcome::Completed));
    }

    #[gpui::test]
    fn matching_intent_ref_cannot_resume_a_different_binding(cx: &mut TestAppContext) {
        let intent = held_intent(100);
        let mut wrong_destination = intent.clone();
        wrong_destination.destination_ref =
            ResourceRef::new("nip29-other-channel").expect("valid destination ref");
        let observed = Rc::new(Cell::new(None));

        cx.update(|cx| {
            IdentityActivationEvents::register(
                intent.clone(),
                20,
                {
                    let observed = observed.clone();
                    move |outcome, _| observed.set(Some(outcome))
                },
                cx,
            )
            .expect("register exact owner");

            assert!(!IdentityActivationEvents::owns(&wrong_destination, cx));
            assert_eq!(
                IdentityActivationEvents::complete(&wrong_destination, cx),
                Err(IdentityActivationEventError::IntentNotOwned)
            );
            assert!(IdentityActivationEvents::owns(&intent, cx));
        });

        assert_eq!(observed.get(), None);
    }

    #[gpui::test]
    fn expired_activation_is_pruned_and_notified(cx: &mut TestAppContext) {
        let intent = held_intent(100);
        let observed = Rc::new(Cell::new(None));

        cx.update(|cx| {
            IdentityActivationEvents::register(
                intent.clone(),
                20,
                {
                    let observed = observed.clone();
                    move |outcome, _| observed.set(Some(outcome))
                },
                cx,
            )
            .expect("register exact owner");
            assert!(!IdentityActivationEvents::prune_expired(99, cx));
            assert!(IdentityActivationEvents::prune_expired(100, cx));
            assert!(!IdentityActivationEvents::owns(&intent, cx));
        });

        assert_eq!(observed.get(), Some(IdentityActivationOutcome::Expired));
    }

    #[gpui::test]
    fn registration_never_replaces_an_existing_owner(cx: &mut TestAppContext) {
        let intent = held_intent(100);
        let first_observed = Rc::new(Cell::new(None));
        let second_observed = Rc::new(Cell::new(None));

        cx.update(|cx| {
            IdentityActivationEvents::register(
                intent.clone(),
                20,
                {
                    let first_observed = first_observed.clone();
                    move |outcome, _| first_observed.set(Some(outcome))
                },
                cx,
            )
            .expect("register first owner");
            assert_eq!(
                IdentityActivationEvents::register(
                    intent.clone(),
                    20,
                    {
                        let second_observed = second_observed.clone();
                        move |outcome, _| second_observed.set(Some(outcome))
                    },
                    cx,
                ),
                Err(IdentityActivationEventError::IntentAlreadyOwned)
            );
            IdentityActivationEvents::cancel(&intent, cx).expect("cancel first owner");
        });

        assert_eq!(
            first_observed.get(),
            Some(IdentityActivationOutcome::Cancelled)
        );
        assert_eq!(second_observed.get(), None);
    }
}
