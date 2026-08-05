use acp_thread::{
    AcpThread, AcpThreadEvent, AgentThreadEntry, AssistantMessage, AssistantMessageChunk,
    AuthRequired, ClientUserMessageId, ElicitationEntryId, ElicitationStatus, ElicitationStore,
    LoadError, MaxOutputTokensError, MentionUri, PermissionOptionChoice, PermissionOptions,
    PermissionPattern, RetryStatus, SelectedPermissionOutcome, ThreadStatus, ToolCall,
    ToolCallContent, ToolCallStatus,
};
use acp_thread::{AgentConnection, Plan};
use action_log::{ActionLog, ActionLogTelemetry, DiffStats};
use agent::{NoModelConfiguredError, ThreadStore};
use agent_client_protocol::schema::v1 as acp;
#[cfg(test)]
use agent_servers::AgentServerDelegate;
use agent_servers::{AgentServer, GEMINI_TERMINAL_AUTH_METHOD_ID};
use agent_settings::{AgentProfileId, AgentSettings};
use anyhow::{Result, anyhow};
#[cfg(feature = "audio")]
use audio::{Audio, Sound};
use buffer_diff::BufferDiff;
use client::zed_urls;
use collections::{HashMap, HashSet, IndexMap};
use editor::scroll::Autoscroll;
use editor::{
    Editor, EditorElement, EditorEvent, EditorMode, MultiBuffer, MultiBufferOffset, PathKey,
    SelectionEffects, SizingBehavior,
};
use file_icons::FileIcons;
use fs::Fs;
use futures::FutureExt as _;
use futures::future::Shared;
use gpui::{
    Action, Animation, AnimationExt, App, ClickEvent, ClipboardItem, CursorStyle, ElementId, Empty,
    Entity, EventEmitter, FocusHandle, Focusable, Hsla, ListOffset, ListState, ObjectFit,
    PlatformDisplay, ScrollHandle, SharedString, StyledText, Subscription, Task, TextRun,
    TextStyle, WeakEntity, Window, WindowHandle, div, ease_in_out, img, linear_color_stop,
    linear_gradient, list, pulsating_between,
};
use language::{Buffer, Language, Rope};
use language_model::LanguageModelCompletionError;
use markdown::{
    CodeBlockRenderer, CopyButtonVisibility, Markdown, MarkdownElement, MarkdownFont, MarkdownStyle,
};
use parking_lot::{Mutex, RwLock};
use project::{AgentId, AgentServerStore, Project, ProjectEntryId, ProjectPath};

use crate::conversation_view::elicitation::{
    ElicitationCard, ElicitationCardHandlers, ElicitationFormState, should_render_elicitation,
};
use crate::message_editor::SessionCapabilities;
use crate::plan_presentation::{PlanStatusKind, plan_label_markdown_style};
use crate::{AgentThreadSource, DEFAULT_THREAD_TITLE, resolve_agent_image};
use lru::LruCache;
use omega_actions::agent::{Chat, ToggleModelSelector};
use rope::Point;
use settings::{NotifyWhenAgentWaiting, Settings as _, SettingsStore};
use std::num::NonZeroUsize;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Instant;
use std::{rc::Rc, time::Duration};
use terminal_view::terminal_panel::TerminalPanel;
use text::Anchor;
use theme_settings::{AgentBufferFontSize, AgentUiFontSize};
use ui::{
    ButtonSize, Callout, CircularProgress, CommonAnimationExt, ContextMenu, ContextMenuEntry,
    CopyButton, DecoratedIcon, DiffStat, Disclosure, Divider, DividerColor, IconDecoration,
    IconDecorationKind, KeyBinding, Popover, PopoverMenu, PopoverMenuHandle, TintColor, Tooltip,
    WithScrollbar, prelude::*, right_click_menu,
};
use util::{
    ResultExt, debug_panic, defer,
    paths::{PathStyle, PathWithPosition},
    rel_path::RelPath,
    size::format_file_size,
    time::duration_alt_display,
};
use workspace::{
    CollaboratorId, MultiWorkspace, NewTerminal, PathList, Workspace, path_link::sanitize_path_text,
};

use super::config_options::ConfigOptionsView;
use super::entry_view_state::EntryViewState;
use crate::ModeSelector;
use crate::ModelSelectorPopover;
use crate::agent_connection_store::{
    AgentConnectedState, AgentConnectionEntryEvent, AgentConnectionStore,
};
use crate::agent_diff::AgentDiff;
use crate::completion_provider::{AgentContextSelection, AvailableSkill};
use crate::entry_view_state::{EntryViewEvent, ViewEvent};
use crate::message_editor::{InputAttempt, MessageEditor, MessageEditorEvent};
use crate::profile_selector::{ProfileProvider, ProfileSelector};

use crate::thread_metadata_store::{ThreadId, ThreadMetadataStore};
use crate::ui::{AgentNotification, AgentNotificationEvent};
use crate::{
    Agent, AgentDiffPane, AgentInitialContent, AgentPanel, AgentPanelEvent, AllowAlways, AllowOnce,
    AuthorizeToolCall, ClearMessageQueue, CycleFavoriteModels, CycleModeSelector,
    CycleThinkingEffort, EditFirstQueuedMessage, ExpandMessageEditor, Follow, KeepAll, NewThread,
    OpenAddContextMenu, OpenAgentDiff, RejectAll, RejectOnce, RemoveFirstQueuedMessage,
    ScrollOutputLineDown, ScrollOutputLineUp, ScrollOutputPageDown, ScrollOutputPageUp,
    ScrollOutputToBottom, ScrollOutputToNextMessage, ScrollOutputToPreviousMessage,
    ScrollOutputToTop, SendImmediately, SendNextQueuedMessage, ToggleFastMode,
    ToggleProfileSelector, ToggleSteerFirstQueuedMessage, ToggleThinkingEffortMenu,
    ToggleThinkingMode, UndoLastReject,
};

const STOPWATCH_THRESHOLD: Duration = Duration::from_secs(30);
const TOKEN_THRESHOLD: u64 = 250;

pub(crate) const DRAFT_PROMPT_PERSIST_DEBOUNCE: Duration = Duration::from_millis(250);

#[derive(Debug)]
pub(crate) struct InconsistentWorkDirsError {
    message: String,
}

impl InconsistentWorkDirsError {
    fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

impl std::fmt::Display for InconsistentWorkDirsError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for InconsistentWorkDirsError {}

fn apply_work_dir_transaction<T>(
    updates: &[T],
    mut update: impl FnMut(&T, bool) -> Result<()>,
) -> Result<()> {
    for (index, item) in updates.iter().enumerate() {
        if let Err(error) = update(item, false) {
            let rollback_errors = updates[..index]
                .iter()
                .rev()
                .filter_map(|updated| update(updated, true).err().map(|error| error.to_string()))
                .collect::<Vec<_>>();
            if !rollback_errors.is_empty() {
                return Err(InconsistentWorkDirsError::new(format!(
                    "{error}; thread working directories are inconsistent because rollback \
                     failed: {}. Reselect a repository target or reconnect this thread before \
                     continuing",
                    rollback_errors.join("; ")
                ))
                .into());
            }
            return Err(error);
        }
    }
    Ok(())
}
/// The composer shown while an executor connects looks like the real one.
///
/// omega#112. It used to say "Type while the executor connects — what you
/// write is kept". The owner: "putting that idiotic 'what u write is kept'
/// message in the input is not desired, remove it. it should already know what
/// the activ e executor is and load it like it actually will be."
///
/// He is right, and the reassurance was covering for the wrong thing. The
/// promise it made is one the field should simply keep — which it does — and
/// saying so out loud only advertises that there was a moment where it might
/// not have. The executor being connected is already known before the
/// connection exists, because it is the choice that *caused* the connect, so
/// this field can say what the real one will say and be the same field a
/// second early.
const COMPOSER_MAX_WIDTH: f32 = 768.;
const COMPOSER_COMPACT_HEIGHT: f32 = 49.;
const COMPOSER_EXPANDED_MIN_HEIGHT: f32 = 124.;
const COMPOSER_EXPANDED_MAX_HEIGHT: f32 = 308.;
const COMPOSER_ACTIONS_HEIGHT: f32 = 46.;
const COMPOSER_RADIUS: f32 = 26.;
const COMPOSER_TEXT_INSET: f32 = 16.;
const COMPOSER_SINGLE_LINE_CHARACTER_LIMIT: usize = 96;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ComposerLayout {
    Compact,
    Expanded,
    ManuallyExpanded,
}

fn composer_layout(manually_expanded: bool, text: &str) -> ComposerLayout {
    if manually_expanded {
        ComposerLayout::ManuallyExpanded
    } else if text.contains('\n') || text.chars().count() > COMPOSER_SINGLE_LINE_CHARACTER_LIMIT {
        ComposerLayout::Expanded
    } else {
        ComposerLayout::Compact
    }
}

fn composer_max_width(configured: Option<Pixels>) -> Pixels {
    px(configured
        .map(Pixels::as_f32)
        .unwrap_or(COMPOSER_MAX_WIDTH)
        .min(COMPOSER_MAX_WIDTH))
}

/// Wraps a composer's editor element in the accessibility node that screen
/// readers and voice control read as Omega's message field.
///
/// omega#217. There are two composers — the thread composer and the
/// pre-session composer drawn while the executor connects or while Omega's
/// router waits for a first send — and only the first one carried this node.
/// A launched window therefore published its model, voice, and send controls
/// to macOS with no text field between them, which read as the composer being
/// missing from the platform tree rather than as one of two composers being
/// unlabelled. Both call sites go through here so the pair cannot drift again.
fn accessible_composer_input(
    id: &'static str,
    text: String,
    focus_handle: &FocusHandle,
    set_value: impl Fn(String, &mut Window, &mut App) + 'static,
    editor: AnyElement,
) -> gpui::Stateful<Div> {
    let run_text = text.clone();
    let focus_for_action = focus_handle.clone();

    div()
        .id(id)
        .size_full()
        .min_w_0()
        .min_h_0()
        .track_focus(focus_handle)
        .role(gpui::Role::MultilineTextInput)
        .aria_label("Message composer")
        .aria_placeholder("Message Omega")
        .aria_value(text)
        .a11y_synthetic_children(move |builder| {
            let run_id = builder.synthetic_node_id(0);
            let mut run = gpui::accesskit::Node::new(gpui::Role::TextRun);
            run.set_text_direction(gpui::accesskit::TextDirection::LeftToRight);
            run.set_value(run_text.clone());
            run.set_character_lengths(
                run_text
                    .chars()
                    .map(|character| character.len_utf8() as u8)
                    .collect::<Vec<_>>(),
            );
            builder.push_child(run_id, run);
            let caret = gpui::accesskit::TextPosition {
                node: run_id,
                character_index: run_text.chars().count(),
            };
            builder
                .parent_node()
                .set_text_selection(gpui::accesskit::TextSelection {
                    anchor: caret,
                    focus: caret,
                });
        })
        .on_a11y_action(gpui::AccessibleAction::Focus, move |_, window, cx| {
            focus_for_action.focus(window, cx);
        })
        .on_a11y_action(gpui::AccessibleAction::SetValue, move |data, window, cx| {
            let Some(gpui::accesskit::ActionData::Value(value)) = data else {
                return;
            };
            set_value(value.to_string(), window, cx);
        })
        .child(editor)
}

pub(crate) mod elicitation;
mod message_queue;
mod thread_search_bar;
mod thread_view;
pub use message_queue::*;
pub use thread_view::*;

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
enum ThreadFeedback {
    Positive,
    Negative,
}

#[derive(Debug)]
pub(crate) enum ThreadError {
    PaymentRequired,
    DataRetentionConsentRequired,
    Refusal,
    AuthenticationRequired(SharedString),
    RateLimitExceeded {
        provider: SharedString,
    },
    ServerOverloaded {
        provider: SharedString,
    },
    PromptTooLarge,
    NoCredentials {
        provider: SharedString,
    },
    StreamError {
        provider: SharedString,
    },
    AuthenticationFailed {
        provider: SharedString,
    },
    PermissionDenied {
        provider: SharedString,
        message: Option<SharedString>,
    },
    RequestFailed,
    MaxOutputTokens,
    NoModelSelected,
    ApiError {
        provider: SharedString,
    },
    Other {
        message: SharedString,
        acp_error_code: Option<SharedString>,
    },
}

impl ThreadError {
    /// The public-safe sentence a phone should show for this error.
    ///
    /// Desktop already renders this in a callout the person can read. The
    /// device mirror used to project only `state: failed` with no body, so a
    /// phone showed "failed" and nothing else while the desktop held the
    /// whole reason. The text here is the same family of wording the desktop
    /// callout uses — never a stack trace, never a secret.
    pub(crate) fn device_mirror_text(&self, model_or_agent_name: &str) -> SharedString {
        match self {
            Self::PaymentRequired => {
                "You reached a usage limit. Hosted Omega AI plans are not available in this build."
                    .into()
            }
            Self::Refusal => format!(
                "{model_or_agent_name} refused to respond to this prompt. Rephrasing it can sometimes address the issue."
            )
            .into(),
            Self::DataRetentionConsentRequired => {
                format!("{model_or_agent_name} is not available with Zero Data Retention.").into()
            }
            Self::AuthenticationRequired(message) => message.clone(),
            Self::RateLimitExceeded { provider } => {
                format!("{provider}'s rate limit was reached.").into()
            }
            Self::ServerOverloaded { provider } => {
                format!("{provider}'s servers are temporarily unavailable.").into()
            }
            Self::PromptTooLarge => "Context too large for the model's context window.".into(),
            Self::NoCredentials { provider } => {
                format!("No credentials are configured for {provider}.").into()
            }
            Self::StreamError { provider } => {
                format!("The connection to {provider}'s API was interrupted.").into()
            }
            Self::AuthenticationFailed { provider } => {
                format!("Authentication with {provider} failed.").into()
            }
            Self::PermissionDenied { provider, message } => message.clone().unwrap_or_else(|| {
                format!(
                    "{provider}'s API rejected the request due to insufficient permissions."
                )
                .into()
            }),
            Self::RequestFailed => {
                "The request could not be completed after multiple attempts.".into()
            }
            Self::MaxOutputTokens => "The model reached its maximum output length.".into(),
            Self::NoModelSelected => "No model is selected.".into(),
            Self::ApiError { provider } => {
                format!("{provider}'s API returned an unexpected error.").into()
            }
            Self::Other { message, .. } => message.clone(),
        }
    }
}

impl From<anyhow::Error> for ThreadError {
    fn from(error: anyhow::Error) -> Self {
        if error.is::<MaxOutputTokensError>() {
            Self::MaxOutputTokens
        } else if error.is::<NoModelConfiguredError>() {
            Self::NoModelSelected
        } else if let Some(acp_error) = error.downcast_ref::<acp::Error>()
            && acp_error.code == acp::ErrorCode::AuthRequired
        {
            Self::AuthenticationRequired(acp_error.message.clone().into())
        } else if let Some(lm_error) = error.downcast_ref::<LanguageModelCompletionError>() {
            use LanguageModelCompletionError::*;
            match lm_error {
                RateLimitExceeded { provider, .. } => Self::RateLimitExceeded {
                    provider: provider.to_string().into(),
                },
                ServerOverloaded { provider, .. } | ApiInternalServerError { provider, .. } => {
                    Self::ServerOverloaded {
                        provider: provider.to_string().into(),
                    }
                }
                PromptTooLarge { .. } => Self::PromptTooLarge,
                PaymentRequired => Self::PaymentRequired,
                NoApiKey { provider } => Self::NoCredentials {
                    provider: provider.to_string().into(),
                },
                StreamEndedUnexpectedly { provider }
                | ApiReadResponseError { provider, .. }
                | DeserializeResponse { provider, .. }
                | HttpSend { provider, .. } => Self::StreamError {
                    provider: provider.to_string().into(),
                },
                AuthenticationError { provider, .. } => Self::AuthenticationFailed {
                    provider: provider.to_string().into(),
                },
                PermissionError { provider, message } => Self::PermissionDenied {
                    provider: provider.to_string().into(),
                    message: Some(message.clone().into()),
                },
                UpstreamProviderError { .. } => Self::RequestFailed,
                DataRetentionConsentRequired { .. } => Self::DataRetentionConsentRequired,
                BadRequestFormat { provider, .. }
                | HttpResponseError { provider, .. }
                | ApiEndpointNotFound { provider } => Self::ApiError {
                    provider: provider.to_string().into(),
                },
                _ => {
                    let message: SharedString = format!("{:#}", error).into();
                    Self::Other {
                        message,
                        acp_error_code: None,
                    }
                }
            }
        } else {
            let message: SharedString = format!("{:#}", error).into();

            // Extract ACP error code if available
            let acp_error_code = error
                .downcast_ref::<acp::Error>()
                .map(|acp_error| SharedString::from(acp_error.code.to_string()));

            Self::Other {
                message,
                acp_error_code,
            }
        }
    }
}

impl ProfileProvider for Entity<agent::Thread> {
    fn profile_id(&self, cx: &App) -> AgentProfileId {
        self.read(cx).profile().clone()
    }

    fn set_profile(&self, profile_id: AgentProfileId, cx: &mut App) {
        self.update(cx, |thread, cx| {
            // Apply the profile and let the thread swap to its default model.
            thread.set_profile(profile_id, cx);
        });
    }

    fn profiles_supported(&self, cx: &App) -> bool {
        self.read(cx)
            .model()
            .is_some_and(|model| model.supports_tools())
    }

    fn model_selected(&self, cx: &App) -> bool {
        self.read(cx).model().is_some()
    }
}

#[derive(Default)]
pub(crate) struct Conversation {
    threads: HashMap<acp::SessionId, Entity<AcpThread>>,
    permission_requests: IndexMap<acp::SessionId, Vec<acp::ToolCallId>>,
    elicitation_requests: IndexMap<acp::SessionId, Vec<ElicitationEntryId>>,
    subscriptions: Vec<Subscription>,
    updated_at: Option<Instant>,
}

impl Conversation {
    pub fn register_thread(&mut self, thread: Entity<AcpThread>, cx: &mut Context<Self>) {
        let session_id = thread.read(cx).session_id().clone();
        let subscription = cx.subscribe(&thread, {
            let session_id = session_id.clone();
            move |this, _thread, event, _cx| {
                this.updated_at = Some(Instant::now());
                match event {
                    AcpThreadEvent::ToolAuthorizationRequested(id) => {
                        this.permission_requests
                            .entry(session_id.clone())
                            .or_default()
                            .push(id.clone());
                    }
                    AcpThreadEvent::ToolAuthorizationReceived(id) => {
                        if let Some(tool_calls) = this.permission_requests.get_mut(&session_id) {
                            tool_calls.retain(|tool_call_id| tool_call_id != id);
                            if tool_calls.is_empty() {
                                this.permission_requests.shift_remove(&session_id);
                            }
                        }
                    }
                    AcpThreadEvent::ElicitationRequested(id) => {
                        this.elicitation_requests
                            .entry(session_id.clone())
                            .or_default()
                            .push(id.clone());
                    }
                    AcpThreadEvent::ElicitationResponded(id) => {
                        if let Some(elicitations) = this.elicitation_requests.get_mut(&session_id) {
                            elicitations.retain(|elicitation_id| elicitation_id != id);
                            if elicitations.is_empty() {
                                this.elicitation_requests.shift_remove(&session_id);
                            }
                        }
                    }
                    AcpThreadEvent::NewEntry
                    | AcpThreadEvent::StatusChanged
                    | AcpThreadEvent::TitleUpdated
                    | AcpThreadEvent::TokenUsageUpdated
                    | AcpThreadEvent::EntryUpdated(_)
                    | AcpThreadEvent::EntriesRemoved(_)
                    | AcpThreadEvent::Retry(_)
                    | AcpThreadEvent::SubagentSpawned(_)
                    | AcpThreadEvent::Stopped(_)
                    | AcpThreadEvent::Error
                    | AcpThreadEvent::LoadError(_)
                    | AcpThreadEvent::PromptCapabilitiesUpdated
                    | AcpThreadEvent::Refusal
                    | AcpThreadEvent::AvailableCommandsUpdated(_)
                    | AcpThreadEvent::ModeUpdated(_)
                    | AcpThreadEvent::ConfigOptionsUpdated(_)
                    | AcpThreadEvent::WorkingDirectoriesUpdated
                    | AcpThreadEvent::PlanUpdated(_)
                    | AcpThreadEvent::ProjectionUpdated(_)
                    | AcpThreadEvent::PromptUpdated => {}
                }
            }
        });
        self.subscriptions.push(subscription);
        self.threads.insert(session_id, thread);
    }

    pub fn permission_options_for_tool_call<'a>(
        &'a self,
        session_id: &acp::SessionId,
        tool_call_id: acp::ToolCallId,
        cx: &'a App,
    ) -> Option<&'a PermissionOptions> {
        let thread = self.threads.get(session_id)?;
        let (_, tool_call) = thread.read(cx).tool_call(&tool_call_id)?;
        let ToolCallStatus::WaitingForConfirmation { options, .. } = &tool_call.status else {
            return None;
        };
        Some(options)
    }

    pub fn pending_tool_call<'a>(
        &'a self,
        session_id: &acp::SessionId,
        cx: &'a App,
    ) -> Option<(acp::SessionId, acp::ToolCallId, &'a PermissionOptions)> {
        let thread = self.threads.get(session_id)?;
        let is_subagent = thread.read(cx).parent_session_id().is_some();
        let (result_session_id, thread, tool_id) = if is_subagent {
            let id = self.permission_requests.get(session_id)?.iter().next()?;
            (session_id.clone(), thread, id)
        } else {
            let (id, tool_calls) = self.permission_requests.first()?;
            let thread = self.threads.get(id)?;
            let tool_id = tool_calls.iter().next()?;
            (id.clone(), thread, tool_id)
        };
        let (_, tool_call) = thread.read(cx).tool_call(tool_id)?;

        let ToolCallStatus::WaitingForConfirmation { options, .. } = &tool_call.status else {
            return None;
        };
        Some((result_session_id, tool_id.clone(), options))
    }

    pub fn subagents_awaiting_permission(&self, cx: &App) -> Vec<(acp::SessionId, usize)> {
        self.permission_requests
            .iter()
            .filter_map(|(session_id, tool_call_ids)| {
                let thread = self.threads.get(session_id)?;
                if thread.read(cx).parent_session_id().is_some() && !tool_call_ids.is_empty() {
                    Some((session_id.clone(), tool_call_ids.len()))
                } else {
                    None
                }
            })
            .collect()
    }

    /// Returns the first pending tool call request for exactly `session_id`.
    /// Unlike `pending_tool_call`, this does not use the global FIFO pending
    /// request for non-subagent sessions.
    pub fn pending_tool_call_for_session(
        &self,
        session_id: &acp::SessionId,
        cx: &App,
    ) -> Option<acp::ToolCallId> {
        let thread = self.threads.get(session_id)?;
        let tool_call_id = self.permission_requests.get(session_id)?.iter().next()?;
        let (_, tool_call) = thread.read(cx).tool_call(tool_call_id)?;
        if !matches!(
            tool_call.status,
            ToolCallStatus::WaitingForConfirmation { .. }
        ) {
            return None;
        }
        Some(tool_call_id.clone())
    }

    pub fn pending_tool_call_count_for_session(&self, session_id: &acp::SessionId) -> usize {
        self.permission_requests
            .get(session_id)
            .map(|tool_call_ids| tool_call_ids.len())
            .unwrap_or(0)
    }

    pub fn respond_to_elicitation(
        &mut self,
        session_id: acp::SessionId,
        elicitation_id: ElicitationEntryId,
        response: acp::CreateElicitationResponse,
        cx: &mut Context<Self>,
    ) -> Option<()> {
        let thread = self.threads.get(&session_id)?.clone();
        thread.update(cx, |thread, cx| {
            thread.respond_to_elicitation(&elicitation_id, response, cx);
        });
        Some(())
    }

    pub fn authorize_pending_tool_call(
        &mut self,
        session_id: &acp::SessionId,
        kind: acp::PermissionOptionKind,
        cx: &mut Context<Self>,
    ) -> Option<()> {
        let (authorize_session_id, tool_call_id, options) =
            self.pending_tool_call(session_id, cx)?;
        let option = permission_option_for_action(options, kind)?;
        self.authorize_tool_call(
            authorize_session_id,
            tool_call_id,
            SelectedPermissionOutcome::new(option.option_id.clone(), option.kind),
            cx,
        );
        Some(())
    }

    pub fn authorize_with_granularity(
        &mut self,
        session_id: acp::SessionId,
        tool_call_id: acp::ToolCallId,
        selection: Option<&thread_view::PermissionSelection>,
        is_allow: bool,
        cx: &mut Context<Self>,
    ) -> Option<()> {
        let options =
            self.permission_options_for_tool_call(&session_id, tool_call_id.clone(), cx)?;
        let outcome = resolve_outcome_from_selection(options, selection, is_allow)?;
        self.authorize_tool_call(session_id, tool_call_id, outcome, cx);
        Some(())
    }

    pub fn authorize_tool_call(
        &mut self,
        session_id: acp::SessionId,
        tool_call_id: acp::ToolCallId,
        outcome: SelectedPermissionOutcome,
        cx: &mut Context<Self>,
    ) {
        let Some(thread) = self.threads.get(&session_id) else {
            return;
        };
        let agent_telemetry_id = thread.read(cx).connection().telemetry_id();
        let session_id = thread.read(cx).session_id().clone();

        telemetry::event!(
            "Agent Tool Call Authorized",
            agent = agent_telemetry_id,
            session = session_id,
            option = outcome.option_kind
        );

        thread.update(cx, |thread, cx| {
            thread.authorize_tool_call(tool_call_id, outcome, cx);
        });
        cx.notify();
    }

    fn set_work_dirs(&mut self, work_dirs: PathList, cx: &mut Context<Self>) {
        for thread in self.threads.values() {
            thread.update(cx, |thread, cx| {
                thread.set_work_dirs(work_dirs.clone(), cx);
            });
        }
    }
}

pub(crate) struct RootThreadUpdated;

impl EventEmitter<RootThreadUpdated> for ConversationView {}

fn permission_option_for_action(
    options: &PermissionOptions,
    kind: acp::PermissionOptionKind,
) -> Option<&acp::PermissionOption> {
    if kind == acp::PermissionOptionKind::AllowAlways
        && let PermissionOptions::Flat(options) = options
        && let Some(option) = options.iter().find(|option| {
            option.option_id.0.as_ref() == acp_thread::SandboxPermission::AllowAlways.as_id()
        })
    {
        return Some(option);
    }

    options.first_option_of_kind(kind)
}

pub struct StateChange;

impl EventEmitter<StateChange> for ConversationView {}

fn resolve_outcome_from_selection(
    options: &PermissionOptions,
    selection: Option<&thread_view::PermissionSelection>,
    is_allow: bool,
) -> Option<SelectedPermissionOutcome> {
    let choices = match options {
        PermissionOptions::Dropdown(choices) => choices.as_slice(),
        PermissionOptions::DropdownWithPatterns { choices, .. } => choices.as_slice(),
        PermissionOptions::Flat(_) => {
            let kind = if is_allow {
                acp::PermissionOptionKind::AllowOnce
            } else {
                acp::PermissionOptionKind::RejectOnce
            };
            let option = options.first_option_of_kind(kind)?;
            return Some(SelectedPermissionOutcome::new(
                option.option_id.clone(),
                option.kind,
            ));
        }
    };

    // When in per-command pattern mode, use the checked patterns.
    if let Some(thread_view::PermissionSelection::SelectedPatterns(checked)) = selection {
        if let Some(outcome) = options.build_outcome_for_checked_patterns(checked, is_allow) {
            return Some(outcome);
        }
    }

    // Use the selected granularity choice ("Always for terminal" or "Only this time").
    let selected_index = selection
        .and_then(|s| s.choice_index())
        .unwrap_or_else(|| choices.len().saturating_sub(1));
    let selected_choice = choices.get(selected_index).or(choices.last())?;
    Some(selected_choice.build_outcome(is_allow))
}

fn affects_thread_metadata(event: &AcpThreadEvent) -> bool {
    match event {
        AcpThreadEvent::NewEntry
        | AcpThreadEvent::TitleUpdated
        | AcpThreadEvent::ToolAuthorizationRequested(_)
        | AcpThreadEvent::ToolAuthorizationReceived(_)
        | AcpThreadEvent::ElicitationRequested(_)
        | AcpThreadEvent::ElicitationResponded(_)
        | AcpThreadEvent::Stopped(_)
        | AcpThreadEvent::Error
        | AcpThreadEvent::LoadError(_)
        | AcpThreadEvent::Refusal
        | AcpThreadEvent::StatusChanged
        | AcpThreadEvent::WorkingDirectoriesUpdated => true,
        // --
        AcpThreadEvent::EntryUpdated(_)
        | AcpThreadEvent::EntriesRemoved(_)
        | AcpThreadEvent::Retry(_)
        | AcpThreadEvent::TokenUsageUpdated
        | AcpThreadEvent::PromptCapabilitiesUpdated
        | AcpThreadEvent::AvailableCommandsUpdated(_)
        | AcpThreadEvent::ModeUpdated(_)
        | AcpThreadEvent::ConfigOptionsUpdated(_)
        | AcpThreadEvent::SubagentSpawned(_)
        | AcpThreadEvent::PlanUpdated(_)
        | AcpThreadEvent::ProjectionUpdated(_)
        | AcpThreadEvent::PromptUpdated => false,
    }
}

pub enum AcpServerViewEvent {
    ActiveThreadChanged,
}

impl EventEmitter<AcpServerViewEvent> for ConversationView {}

pub struct ConversationView {
    agent: Rc<dyn AgentServer>,
    connection_store: Entity<AgentConnectionStore>,
    connection_key: Agent,
    agent_server_store: Entity<AgentServerStore>,
    workspace: WeakEntity<Workspace>,
    project: Entity<Project>,
    thread_store: Option<Entity<ThreadStore>>,
    pub(crate) thread_id: ThreadId,
    pub(crate) root_session_id: Option<acp::SessionId>,
    desired_work_dirs: PathList,
    /// Working directories still being provisioned for this conversation.
    ///
    /// `OMEGA-DELTA-0214`. Isolation has to be settled before `new_session`,
    /// because every external ACP agent fixes its working directory when the
    /// session starts (`supports_live_work_dir_updates() == false`) and cannot
    /// be retargeted afterwards. Rather than delay the new-thread gesture on
    /// `git worktree add`, the view is created immediately and the session
    /// load awaits this. `None` — the ordinary case — costs nothing.
    pending_work_dirs: Option<Shared<Task<Option<PathList>>>>,
    server_state: ServerState,
    focus_handle: FocusHandle,
    notifications: Vec<WindowHandle<AgentNotification>>,
    notification_subscriptions: HashMap<WindowHandle<AgentNotification>, Vec<Subscription>>,
    auth_task: Option<Task<()>>,
    loading_status: Option<SharedString>,
    /// What a person types while the executor is still connecting.
    ///
    /// `OMEGA-DELTA-0122`. Handed to the real composer the moment one exists —
    /// see [`ConversationView::hand_loading_draft_over`] — and dropped there.
    loading_composer: Option<Entity<Editor>>,
    /// Files dropped while the pre-session composer is visible.
    ///
    /// That composer is intentionally a plain [`Editor`], so it cannot own
    /// resolved mentions. Keep the project paths alive until the session's
    /// [`MessageEditor`] exists and can resolve them with its capabilities.
    pending_dragged_files: Vec<(Vec<project::ProjectPath>, Vec<Entity<project::Worktree>>)>,
    /// Whether the pre-session composer is expanded to most of the window.
    ///
    /// `OMEGA-DELTA-0204`. `ThreadView` keeps the same bit as `editor_expanded`.
    /// It lives here as well rather than being shared because the two composers
    /// are different editors with different lifetimes — the expanded state does
    /// not survive the handover any more than the caret position does — and
    /// because a person who expands the pre-session field wants that field
    /// bigger, not a preference recorded for the thread that replaces it.
    loading_composer_expanded: bool,
    /// One readout shared by the loading composer and every connected thread.
    /// It follows whichever Vim editor in this window most recently focused.
    vim_mode_indicator: Entity<vim::ModeIndicator>,
    /// The rebuild waiting for the person to stop cycling executors.
    ///
    /// omega#117. Holding the task is what makes this a debounce: assigning a
    /// new one drops the old, and dropping a GPUI task cancels it.
    pending_executor_rebuild: Option<Task<()>>,
    /// The physical executor session created after Omega accepts its first turn.
    ///
    /// Holding the task keeps session creation alive while the logical router
    /// composer remains usable. Direct Agent conversations never use it.
    deferred_omega_session: Option<Task<()>>,
    /// The frozen first-route decision, recorded when the router chooses.
    ///
    /// Held as state — not composer copy. The owner's law (omega#160,
    /// `OMEGA-DELTA-0189`): routing mechanics are not explained in the
    /// composer bar. The durable receipt remains the disclosure surface.
    omega_route_summary: Option<SharedString>,
    /// Messages a person sent while the executor was still connecting.
    ///
    /// `OMEGA-DELTA-0170`. Enter always accepts: each press moves the
    /// composer's typed content here, in order, and the pending turns are drawn in the
    /// chat with a spinner. The whole list is dispatched — exactly once, via
    /// [`ConversationView::dispatch_pending_connect_messages`] — the moment a
    /// session exists. It deliberately lives on this view rather than on
    /// `ServerState::Loading`, so a connection that terminally fails carries
    /// the text into `LoadError` instead of dropping it with the state.
    pending_connect_messages: Vec<PendingConnectMessage>,
    /// Stays true after the pending vector is handed to the connected queue so
    /// sidebar visibility cannot flicker before thread metadata is promoted.
    first_message_submitted_while_connecting: bool,
    /// When settings change, use this to see if the theme has changed (which
    /// causes mermaid diagrams to re-render).
    last_theme_id: Option<String>,
    draft_prompt_persist_task: Option<Task<()>>,
    send_queue_journal: Rc<crate::omega_send_queue::SendQueueJournal>,
    /// Cache + worktree snapshot for resolving paths in markdown code spans.
    /// Shared with the child [`ThreadView`] when one is constructed.
    pub(crate) code_span_resolver: AgentCodeSpanResolver,
    request_elicitation_form_states: HashMap<ElicitationEntryId, ElicitationFormState>,
    _subscriptions: Vec<Subscription>,
}

impl ConversationView {
    pub fn has_auth_methods(&self) -> bool {
        self.as_connected().map_or(false, |connected| {
            !connected.connection.auth_methods().is_empty()
        })
    }

    pub fn supports_logout(&self) -> bool {
        self.as_connected().is_some_and(|connected| {
            connected.auth_state.is_ok() && connected.connection.supports_logout()
        })
    }

    pub fn active_thread(&self) -> Option<&Entity<ThreadView>> {
        match &self.server_state {
            ServerState::Connected(connected) => connected.active_view(),
            _ => None,
        }
    }

    pub fn pending_tool_call<'a>(
        &'a self,
        cx: &'a App,
    ) -> Option<(acp::SessionId, acp::ToolCallId, &'a PermissionOptions)> {
        let session_id = self.active_thread()?.read(cx).session_id.clone();
        self.as_connected()?
            .conversation
            .read(cx)
            .pending_tool_call(&session_id, cx)
    }

    pub fn root_thread_has_pending_tool_call(&self, cx: &App) -> bool {
        let Some(root_thread) = self.root_thread_view() else {
            return false;
        };
        let root_session_id = root_thread.read(cx).thread.read(cx).session_id().clone();
        self.as_connected().is_some_and(|connected| {
            connected
                .conversation
                .read(cx)
                .pending_tool_call(&root_session_id, cx)
                .is_some()
        })
    }

    pub(crate) fn root_thread(&self, cx: &App) -> Option<Entity<AcpThread>> {
        self.root_thread_view()
            .map(|view| view.read(cx).thread.clone())
    }

    pub fn root_thread_view(&self) -> Option<Entity<ThreadView>> {
        self.root_session_id
            .as_ref()
            .and_then(|id| self.thread_view(id))
    }

    pub fn thread_view(&self, session_id: &acp::SessionId) -> Option<Entity<ThreadView>> {
        let connected = self.as_connected()?;
        connected.threads.get(session_id).cloned()
    }

    pub fn as_connected(&self) -> Option<&ConnectedServerState> {
        match &self.server_state {
            ServerState::Connected(connected) => Some(connected),
            _ => None,
        }
    }

    pub fn as_connected_mut(&mut self) -> Option<&mut ConnectedServerState> {
        match &mut self.server_state {
            ServerState::Connected(connected) => Some(connected),
            _ => None,
        }
    }

    pub fn updated_at(&self, cx: &App) -> Option<Instant> {
        self.as_connected()
            .and_then(|connected| connected.conversation.read(cx).updated_at)
    }

    pub fn navigate_to_thread(
        &mut self,
        session_id: acp::SessionId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(connected) = self.as_connected_mut() else {
            return;
        };

        connected.navigate_to_thread(session_id);
        if let Some(view) = self.active_thread() {
            view.read(cx).activation_focus_handle(cx).focus(window, cx);
        }
        cx.emit(AcpServerViewEvent::ActiveThreadChanged);
        cx.notify();
    }

    pub fn open_subagent_in_right_pane(
        &mut self,
        session_id: acp::SessionId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(connected) = self.as_connected_mut() else {
            return;
        };
        let Some(view) = connected.threads.get(&session_id).cloned() else {
            return;
        };

        connected.right_pane_session_id = Some(session_id);
        view.read(cx).activation_focus_handle(cx).focus(window, cx);
        cx.notify();
    }

    pub fn close_right_pane(&mut self, cx: &mut Context<Self>) {
        let Some(connected) = self.as_connected_mut() else {
            return;
        };

        if connected.right_pane_session_id.take().is_some() {
            cx.notify();
        }
    }

    pub fn set_work_dirs(&mut self, work_dirs: PathList, cx: &mut Context<Self>) {
        self.desired_work_dirs = work_dirs.clone();
        if let Some(connected) = self.as_connected() {
            connected.conversation.update(cx, |conversation, cx| {
                conversation.set_work_dirs(work_dirs.clone(), cx);
            });
        }
    }

    pub fn work_dirs(&self) -> &PathList {
        &self.desired_work_dirs
    }

    /// Makes this conversation's session wait for `task` before it starts, and
    /// adopt the working directories it resolves to.
    ///
    /// `OMEGA-DELTA-0214`. The isolation decision is made synchronously when
    /// the thread is created; the `git worktree add` behind it is not. This is
    /// how the two are reconciled without either blocking the new-thread
    /// gesture or letting a session start in an occupied checkout. A task that
    /// resolves to `None` — provisioning failed, or there was nothing to
    /// isolate into — leaves the requested roots in place, and the send path
    /// discloses the collision instead.
    pub fn set_pending_work_dirs(&mut self, task: Shared<Task<Option<PathList>>>) {
        self.pending_work_dirs = Some(task);
    }

    pub(crate) fn project(&self) -> &Entity<Project> {
        &self.project
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn set_composer_text_for_tests(
        &mut self,
        text: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(thread_view) = self.root_thread_view() {
            let message_editor = thread_view.read(cx).message_editor.clone();
            message_editor.update(cx, |editor, cx| editor.set_text(text, window, cx));
        } else {
            let loading_composer = self.loading_composer(window, cx);
            loading_composer.update(cx, |editor, cx| editor.set_text(text, window, cx));
        }
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn send_for_tests(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(thread_view) = self.root_thread_view() {
            thread_view.update(cx, |thread_view, cx| thread_view.send(window, cx));
        } else {
            self.submit_before_session(window, cx);
        }
    }

    pub fn identity_mutation_unavailable_reason(&self, cx: &App) -> Option<SharedString> {
        match &self.server_state {
            ServerState::Loading { .. } => Some(
                "Wait for the agent session to finish loading before changing repository identity"
                    .into(),
            ),
            ServerState::LoadError { .. } => {
                Some("Reconnect the agent session before changing repository identity".into())
            }
            ServerState::Connected(connected) => {
                let threads = connected.conversation.read(cx).threads.values();
                if threads.len() == 0 {
                    return Some("The active agent session is unavailable".into());
                }
                if threads.clone().any(|thread| {
                    thread.read(cx).status() != acp_thread::ThreadStatus::Idle
                        || thread.read(cx).is_waiting_for_confirmation()
                }) {
                    return Some(
                        "Wait for the current agent turn or confirmation to finish before changing \
                         its target"
                            .into(),
                    );
                }
                None
            }
        }
    }

    pub fn work_dir_retarget_unavailable_reason(&self, cx: &App) -> Option<SharedString> {
        self.identity_mutation_unavailable_reason(cx)
            .or_else(|| {
                match &self.server_state {
                ServerState::Connected(connected)
                    if connected.conversation.read(cx).threads.values().any(
                        |thread| {
                            !thread
                                .read(cx)
                                .connection()
                                .supports_live_work_dir_updates()
                        },
                    ) =>
                {
                    Some(
                        "This agent fixes its working directory when a session starts; start a new \
                         thread to choose another target"
                            .into(),
                    )
                }
                _ => None,
            }
            })
    }

    pub fn retarget_work_dirs(
        &mut self,
        work_dirs: PathList,
        cx: &mut Context<Self>,
    ) -> Result<()> {
        self.retarget_work_dirs_impl(work_dirs, false, cx)
    }

    pub fn reconcile_work_dirs(
        &mut self,
        work_dirs: PathList,
        cx: &mut Context<Self>,
    ) -> Result<()> {
        self.retarget_work_dirs_impl(work_dirs, true, cx)
    }

    pub fn set_repository_mutation_pending(&mut self, pending: bool, cx: &mut Context<Self>) {
        let ServerState::Connected(connected) = &self.server_state else {
            return;
        };
        for thread_view in connected.threads.values() {
            thread_view.update(cx, |thread_view, cx| {
                thread_view.set_repository_mutation_pending(pending, cx);
            });
        }
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn register_acp_thread_for_tests(
        &mut self,
        thread: Entity<AcpThread>,
        cx: &mut Context<Self>,
    ) -> Result<()> {
        let ServerState::Connected(connected) = &self.server_state else {
            anyhow::bail!("conversation is not connected");
        };
        connected.conversation.update(cx, |conversation, cx| {
            conversation.register_thread(thread, cx);
        });
        Ok(())
    }

    fn retarget_work_dirs_impl(
        &mut self,
        work_dirs: PathList,
        force: bool,
        cx: &mut Context<Self>,
    ) -> Result<()> {
        if !force && self.desired_work_dirs == work_dirs {
            return Ok(());
        }
        match &self.server_state {
            ServerState::Loading { .. } => {
                anyhow::bail!(
                    "Wait for the agent session to finish loading before changing worktrees"
                );
            }
            ServerState::LoadError { .. } => {
                anyhow::bail!("The agent session must reconnect before changing worktrees");
            }
            ServerState::Connected(connected) => {
                let threads = connected
                    .conversation
                    .read(cx)
                    .threads
                    .values()
                    .cloned()
                    .collect::<Vec<_>>();
                let mut threads = threads;
                threads.sort_by(|left, right| {
                    left.read(cx)
                        .session_id()
                        .0
                        .cmp(&right.read(cx).session_id().0)
                });
                if threads.is_empty() {
                    anyhow::bail!("The active agent session is unavailable");
                }
                for thread in &threads {
                    let thread = thread.read(cx);
                    if !thread.connection().supports_live_work_dir_updates() {
                        anyhow::bail!(
                            "This agent cannot move an existing session to another worktree"
                        );
                    }
                    if thread.status() != acp_thread::ThreadStatus::Idle
                        || thread.is_waiting_for_confirmation()
                    {
                        anyhow::bail!(
                            "Wait for the current agent turn or confirmation to finish before \
                             changing worktrees"
                        );
                    }
                }
                let updates = threads
                    .iter()
                    .map(|thread| {
                        let thread = thread.read(cx);
                        let connection = thread.connection().clone();
                        let session_id = thread.session_id().clone();
                        let previous_work_dirs = thread
                            .work_dirs()
                            .cloned()
                            .unwrap_or_else(|| self.desired_work_dirs.clone());
                        (connection, session_id, previous_work_dirs)
                    })
                    .collect::<Vec<_>>();
                apply_work_dir_transaction(&updates, |update, rollback| {
                    let (connection, session_id, previous_work_dirs) = update;
                    let target = if rollback {
                        previous_work_dirs.clone()
                    } else {
                        work_dirs.clone()
                    };
                    connection.update_work_dirs(session_id, target, cx)
                })?;
                self.desired_work_dirs = work_dirs.clone();
                connected.conversation.update(cx, |conversation, cx| {
                    conversation.set_work_dirs(work_dirs, cx);
                });
            }
        }
        Ok(())
    }
}

enum ServerState {
    Loading {
        _loading: Entity<LoadingView>,
        connection: Option<Rc<dyn AgentConnection>>,
        _request_elicitation_subscription: Option<Subscription>,
    },
    LoadError {
        error: LoadError,
    },
    Connected(ConnectedServerState),
}

struct PendingConnectMessage {
    text: String,
    content: Vec<acp::ContentBlock>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum ConversationPreparation {
    Loading,
    RouterReady,
    Ready { session_id: String },
    SetupRequired { reason: SharedString },
}

pub(crate) fn visible_omega_route_decision(
    decision: &omega_front_door::RouteDecision,
) -> SharedString {
    use omega_front_door::router::{ExecutorOverride, RouteFallback};

    let route_override = decision
        .inputs
        .as_ref()
        .map_or("not recorded".to_owned(), |inputs| {
            match &inputs.executor_override {
                ExecutorOverride::Auto => "automatic".to_owned(),
                ExecutorOverride::Native => "Omega".to_owned(),
                ExecutorOverride::ExactExternal(agent_id) => format!("exact {agent_id}"),
            }
        });
    let fallback = match &decision.fallback {
        Some(RouteFallback::NativeForGeneralReasoning) => "Omega for general reasoning",
        Some(RouteFallback::NativeAfterExternalUnavailable) => {
            "Omega after external executor became unavailable"
        }
        None => "none",
    };
    format!(
        "Route: {} · override: {route_override} · fallback: {fallback}",
        decision.summary()
    )
    .into()
}

fn omega_task_requirements(
    initial_content: Option<&AgentInitialContent>,
) -> omega_front_door::router::TaskRequirements {
    match initial_content {
        Some(AgentInitialContent::ContentBlock { blocks, .. }) => {
            omega_task_requirements_for_blocks(blocks)
        }
        Some(AgentInitialContent::ThreadSummary { .. })
        | Some(AgentInitialContent::FromExternalSource(_))
        | None => omega_task_requirements_for_blocks(&[]),
    }
}

fn omega_task_requirements_for_blocks(
    blocks: &[acp::ContentBlock],
) -> omega_front_door::router::TaskRequirements {
    use omega_front_door::router::{TaskKind, TaskRequirements};

    TaskRequirements::new(
        if blocks.iter().any(|block| {
            matches!(
                block,
                acp::ContentBlock::Resource(_) | acp::ContentBlock::ResourceLink(_)
            )
        }) {
            TaskKind::RepositoryWork
        } else {
            TaskKind::GeneralReasoning
        },
    )
}

fn omega_initial_content_has_request(content: &AgentInitialContent) -> bool {
    match content {
        AgentInitialContent::ContentBlock { blocks, .. } => {
            blocks.iter().any(|block| match block {
                acp::ContentBlock::Text(text) => !text.text.trim().is_empty(),
                _ => true,
            })
        }
        AgentInitialContent::ThreadSummary { .. } | AgentInitialContent::FromExternalSource(_) => {
            true
        }
    }
}

// current -> Entity
// hashmap of threads, current becomes session_id
pub struct ConnectedServerState {
    auth_state: AuthState,
    active_id: Option<acp::SessionId>,
    right_pane_session_id: Option<acp::SessionId>,
    pub(crate) threads: HashMap<acp::SessionId, Entity<ThreadView>>,
    connection: Rc<dyn AgentConnection>,
    conversation: Entity<Conversation>,
    _connection_entry_subscription: Subscription,
    _request_elicitation_subscription: Option<Subscription>,
}

enum AuthState {
    Ok,
    Unauthenticated {
        description: Option<Entity<Markdown>>,
        pending_auth_method: Option<acp::AuthMethodId>,
    },
}

impl AuthState {
    pub fn is_ok(&self) -> bool {
        matches!(self, Self::Ok)
    }
}

struct LoadingView {
    _load_task: Task<()>,
}

impl ConnectedServerState {
    pub fn active_view(&self) -> Option<&Entity<ThreadView>> {
        self.active_id.as_ref().and_then(|id| self.threads.get(id))
    }

    pub fn has_thread_error(&self, cx: &App) -> bool {
        self.active_view()
            .map_or(false, |view| view.read(cx).thread_error.is_some())
    }

    pub fn navigate_to_thread(&mut self, session_id: acp::SessionId) {
        if self.threads.contains_key(&session_id) {
            self.active_id = Some(session_id);
        }
    }

    pub fn close_all_sessions(&self, cx: &mut App) -> Task<()> {
        let tasks = self.threads.values().filter_map(|view| {
            if self.connection.supports_close_session() {
                let session_id = view.read(cx).thread.read(cx).session_id().clone();
                Some(self.connection.clone().close_session(&session_id, cx))
            } else {
                None
            }
        });
        let task = futures::future::join_all(tasks);
        cx.background_spawn(async move {
            task.await;
        })
    }
}

impl ConversationView {
    pub fn new(
        agent: Rc<dyn AgentServer>,
        connection_store: Entity<AgentConnectionStore>,
        connection_key: Agent,
        resume_session_id: Option<acp::SessionId>,
        thread_id: Option<ThreadId>,
        work_dirs: Option<PathList>,
        title: Option<SharedString>,
        initial_content: Option<AgentInitialContent>,
        workspace: WeakEntity<Workspace>,
        project: Entity<Project>,
        thread_store: Option<Entity<ThreadStore>>,
        source: AgentThreadSource,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let vim_mode_indicator = cx.new(|cx| vim::ModeIndicator::new(window, cx));
        Self::new_with_vim_mode_indicator(
            agent,
            connection_store,
            connection_key,
            resume_session_id,
            thread_id,
            work_dirs,
            title,
            initial_content,
            workspace,
            project,
            thread_store,
            source,
            vim_mode_indicator,
            window,
            cx,
        )
    }

    pub(crate) fn new_with_vim_mode_indicator(
        agent: Rc<dyn AgentServer>,
        connection_store: Entity<AgentConnectionStore>,
        connection_key: Agent,
        resume_session_id: Option<acp::SessionId>,
        thread_id: Option<ThreadId>,
        work_dirs: Option<PathList>,
        title: Option<SharedString>,
        initial_content: Option<AgentInitialContent>,
        workspace: WeakEntity<Workspace>,
        project: Entity<Project>,
        thread_store: Option<Entity<ThreadStore>>,
        source: AgentThreadSource,
        vim_mode_indicator: Entity<vim::ModeIndicator>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let agent_server_store = project.read(cx).agent_server_store().clone();
        let send_queue_journal = crate::omega_send_queue::SendQueueJournal::global(cx);
        let code_span_resolver = AgentCodeSpanResolver::new(&project.downgrade(), cx);
        let mut subscriptions = vec![
            cx.observe_global_in::<SettingsStore>(window, Self::agent_ui_font_size_changed),
            cx.observe_global_in::<SettingsStore>(window, Self::invalidate_mermaid_caches),
            cx.observe_global_in::<AgentUiFontSize>(window, Self::agent_ui_font_size_changed),
            cx.observe_global_in::<AgentBufferFontSize>(window, Self::agent_ui_font_size_changed),
            cx.subscribe_in(
                &agent_server_store,
                window,
                Self::handle_agent_servers_updated,
            ),
        ];
        // `OMEGA-DELTA-0211`. The pre-session composer draws the same voice
        // control as the connected one, so it has to watch the same
        // workspace-owned notice. Without this the microphone click on a brand
        // new thread — the composer a person meets first — would change state
        // nothing repainted.
        {
            let composer_voice_notice =
                crate::composer_voice::composer_voice_notice(workspace.entity_id(), cx);
            subscriptions.push(cx.observe(&composer_voice_notice, |_, _, cx| cx.notify()));
        }
        subscriptions.push(cx.subscribe(&project, {
            let resolver = code_span_resolver.clone();
            move |_this: &mut Self, _project, event: &project::Event, cx| {
                if matches!(
                    event,
                    project::Event::WorktreeAdded(_)
                        | project::Event::WorktreeRemoved(_)
                        | project::Event::WorktreeUpdatedEntries(_, _)
                ) {
                    resolver.clear_cache();
                    cx.notify();
                }
            }
        }));

        cx.on_release(|this, cx| {
            if let Some(session_id) = this.root_session_id.as_ref() {
                crate::omega_agent_supervision::AgentSupervision::global(cx)
                    .remove_snapshot(session_id.0.as_ref());
            }
            this.request_elicitation_form_states.clear();
            if let Some(connected) = this.as_connected() {
                connected.close_all_sessions(cx).detach();
            }
            for window in this.notifications.drain(..) {
                window
                    .update(cx, |_, window, _| {
                        window.remove_window();
                    })
                    .ok();
            }
        })
        .detach();

        // `OMEGA-DELTA-0094`, omega#107. The audience is recorded where a
        // thread starts, and read where a thread is drawn.
        //
        // This is the only place that can tell the difference between a thread
        // that did not exist a moment ago and one being opened again, and the
        // difference decides whether the current selection applies. Both
        // signals are already here: `thread_id` is `Some` when reattaching to a
        // persisted record, and `resume_session_id` is `Some` when resuming a
        // session. At draw time neither is available and the substitute —
        // `AcpThread::is_draft_thread`, which is `entries().is_empty()` — is
        // also true of a resumed thread whose entries have not loaded, so
        // binding there would hand a community audience to somebody's old
        // private conversation on a slow disk.
        //
        // `record_thread_opening` binds once. A thread that already has an
        // audience keeps it.
        let reattached_to_a_persisted_record = thread_id.is_some();
        let thread_id = thread_id.unwrap_or_else(ThreadId::new);
        let desired_work_dirs = work_dirs.unwrap_or_else(|| project.read(cx).default_path_list(cx));
        crate::omega_audience_control::record_thread_opening(
            thread_id,
            if reattached_to_a_persisted_record || resume_session_id.is_some() {
                omega_audience::ThreadOpening::Resumed
            } else {
                omega_audience::ThreadOpening::Started
            },
            cx,
        );

        Self {
            agent: agent.clone(),
            connection_store: connection_store.clone(),
            connection_key: connection_key.clone(),
            agent_server_store,
            workspace,
            project: project.clone(),
            thread_store,
            thread_id,
            root_session_id: resume_session_id.clone(),
            desired_work_dirs: desired_work_dirs.clone(),
            pending_work_dirs: None,
            server_state: Self::initial_state(
                agent.clone(),
                connection_store,
                connection_key,
                resume_session_id,
                Some(desired_work_dirs),
                title,
                project,
                initial_content,
                source,
                window,
                cx,
            ),
            notifications: Vec::new(),
            notification_subscriptions: HashMap::default(),
            auth_task: None,
            loading_status: None,
            loading_composer: None,
            pending_dragged_files: Vec::new(),
            loading_composer_expanded: false,
            vim_mode_indicator,
            pending_executor_rebuild: None,
            deferred_omega_session: None,
            omega_route_summary: None,
            pending_connect_messages: Vec::new(),
            first_message_submitted_while_connecting: false,
            last_theme_id: Some(cx.theme().id.clone()),
            draft_prompt_persist_task: None,
            send_queue_journal,
            code_span_resolver,
            request_elicitation_form_states: HashMap::default(),
            _subscriptions: subscriptions,
            focus_handle: cx.focus_handle(),
        }
    }

    fn set_server_state(&mut self, state: ServerState, cx: &mut Context<Self>) {
        let previous_request_elicitation_connection = self.request_elicitation_connection();
        let next_request_elicitation_connection =
            Self::request_elicitation_connection_for_state(&state);

        if let Some(connected) = self.as_connected() {
            connected.close_all_sessions(cx).detach();
        }

        if let Some(connection) = previous_request_elicitation_connection
            && !next_request_elicitation_connection
                .as_ref()
                .is_some_and(|next_connection| Rc::ptr_eq(&connection, next_connection))
        {
            self.request_elicitation_form_states.clear();
        }

        self.server_state = state;
        cx.emit(StateChange);
        cx.emit(AcpServerViewEvent::ActiveThreadChanged);
        if matches!(&self.server_state, ServerState::Connected(_)) {
            cx.emit(RootThreadUpdated);
        }
        cx.notify();
    }

    fn request_elicitation_subscription(
        connection: &Rc<dyn AgentConnection>,
        cx: &mut Context<Self>,
    ) -> Option<Subscription> {
        let store = connection.request_elicitations()?;
        Some(cx.observe(&store, |this, _store, cx| {
            if let Some(active_thread) = this.active_thread().cloned() {
                active_thread.update(cx, |_thread, cx| cx.notify());
            }
            cx.notify();
        }))
    }

    fn request_elicitation_connection(&self) -> Option<Rc<dyn AgentConnection>> {
        Self::request_elicitation_connection_for_state(&self.server_state)
    }

    fn active_thread_renders_request_elicitations(&self) -> bool {
        match &self.server_state {
            ServerState::Connected(connected) => {
                connected.auth_state.is_ok() && connected.active_view().is_some()
            }
            _ => false,
        }
    }

    fn request_elicitation_connection_for_state(
        state: &ServerState,
    ) -> Option<Rc<dyn AgentConnection>> {
        match state {
            ServerState::Loading {
                connection: Some(connection),
                ..
            } => Some(connection.clone()),
            ServerState::Connected(connected) => Some(connected.connection.clone()),
            ServerState::Loading {
                connection: None, ..
            }
            | ServerState::LoadError { .. } => None,
        }
    }

    fn request_elicitation_store(&self) -> Option<Entity<ElicitationStore>> {
        self.request_elicitation_connection()?
            .request_elicitations()
    }

    /// The field a person types into while the executor is still connecting.
    ///
    /// `OMEGA-DELTA-0122`. The composer belongs to [`ThreadView`], and there is
    /// no thread view until a session exists, so for the whole of `Loading`
    /// there was no way to type. That was a defensible gap while connecting
    /// happened once at startup. `reset_onto_new_executor` made it a place a
    /// person goes on purpose: choosing an executor rebuilds the connection
    /// from nothing, and the owner, having just chosen one, is left looking at
    /// a window with no input in it.
    ///
    ///     "that loading thing is ok but you still dont show the input bar
    ///      while its fucking loading. i want to be able to type while shit is
    ///      loading."
    ///
    /// So this exists, and what is typed into it is moved into the real
    /// composer by [`Self::hand_loading_draft_over`] the moment one is built.
    ///
    /// # Why a bare `Editor` and not a `MessageEditor`
    ///
    /// `MessageEditor` is the real composer, and almost all of what makes it
    /// that — `@` mentions, `/` commands, skills, the queue — is a question
    /// asked of a session that does not exist yet. Its completions would have
    /// nothing to complete against, and a mention resolved here would be a
    /// crease this field can carry and the handover cannot: moving text across
    /// preserves what a person typed, not what an editor made of it. A plain
    /// field that loses nothing is better than a rich one that loses creases
    /// silently at the exact moment the connection lands.
    ///
    /// It wears `composer_editor_style` so the two fields are one field to look
    /// at, and the handover does not reflow a half-written sentence.
    fn loading_composer(&mut self, window: &mut Window, cx: &mut Context<Self>) -> Entity<Editor> {
        if let Some(editor) = self.loading_composer.clone() {
            return editor;
        }

        let settings = AgentSettings::get_global(cx);
        let min_lines = settings.message_editor_min_lines;
        let max_lines = settings.set_message_editor_max_lines();

        // The executor being connected is the one the person just chose, so the
        // field can name it now rather than after the handshake. Falls back to
        // the router's name when nothing has been chosen — a first launch,
        // where the answer genuinely is not known yet.
        let routed_executor = routed_executor_for_owner(
            &self.connection_key,
            crate::omega_executor_selector::selected(),
        );
        let loading_placeholder = routed_executor
            .map(|executor| placeholder_text(executor.name()))
            .unwrap_or_else(|| {
                let name = self
                    .agent_server_store
                    .read(cx)
                    .agent_display_name(&self.agent.agent_id())
                    .unwrap_or_else(|| self.agent.agent_id().0.to_string().into());
                placeholder_text(name.as_ref())
            });

        let editor = cx.new(|cx| {
            let mut editor = Editor::auto_height(min_lines, max_lines, window, cx);
            editor.set_placeholder_text(&loading_placeholder, window, cx);
            editor.set_show_indent_guides(false, cx);
            editor.set_soft_wrap();
            editor.disable_mouse_wheel_zoom();
            editor.set_use_modal_editing(true);
            editor
        });
        // Focused on creation, not only when something re-focuses the view.
        // A composer that looks ready and ignores the keyboard is worse than no
        // composer, because a person types a sentence into nothing.
        editor.read(cx).focus_handle(cx).focus(window, cx);

        self._subscriptions
            .push(cx.subscribe(&editor, |this, _editor, event, cx| {
                if matches!(event, EditorEvent::Edited { .. }) {
                    this.schedule_draft_prompt_persist(cx);
                }
            }));
        self.loading_composer = Some(editor.clone());
        editor
    }

    /// Move what was typed during `Loading` into the composer that now exists.
    ///
    /// `OMEGA-DELTA-0122`. This is the half of the loading composer that
    /// matters. Letting someone type into a field whose contents are thrown
    /// away when the thing they were waiting for arrives is worse than not
    /// letting them type at all: the second is a wait, the first is a lost
    /// sentence, and the sentence is the one that states the task.
    ///
    /// The caret goes back where it was, because a person who was mid-word when
    /// the connection landed is still mid-word.
    ///
    /// If the real composer already carries something — a restored draft, or
    /// content the panel was opened with — nothing is overwritten. The typed
    /// text goes on the end, after a blank line, and the caret follows it.
    /// Which is not tidy, and is the right trade: both texts are somebody's.
    fn hand_loading_draft_over(
        &mut self,
        thread_view: &Entity<ThreadView>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let message_editor = thread_view.read(cx).message_editor.clone();
        if let Some(loading_composer) = self.loading_composer.take() {
            let (text, cursor_offset) = loading_composer.update(cx, |editor, cx| {
                let snapshot = editor.display_snapshot(cx);
                let cursor = editor
                    .selections
                    .newest::<MultiBufferOffset>(&snapshot)
                    .head();
                (editor.text(cx), cursor.0)
            });
            if !text.is_empty() {
                message_editor.update(cx, |message_editor, cx| {
                    if message_editor.is_empty(cx) {
                        message_editor.insert_text(&text, window, cx);
                        message_editor.set_cursor_offset(cursor_offset, window, cx);
                    } else {
                        message_editor.set_cursor_offset(usize::MAX, window, cx);
                        message_editor.insert_text(&format!("\n\n{text}"), window, cx);
                    }
                });
            }
        }

        for (paths, added_worktrees) in self.pending_dragged_files.drain(..) {
            message_editor.update(cx, |message_editor, cx| {
                message_editor.insert_dragged_files(paths, added_worktrees, window, cx);
            });
        }
    }

    /// The executor the pending turns are waiting on, named the way the
    /// loading composer's placeholder names it: the one the person just chose,
    /// falling back to the router's display name on a first launch where
    /// nothing has been chosen yet.
    fn connecting_executor_name(&self, cx: &App) -> SharedString {
        let routed_executor = routed_executor_for_owner(
            &self.connection_key,
            crate::omega_executor_selector::selected(),
        );
        routed_executor
            .map(|executor| SharedString::from(executor.name()))
            .unwrap_or_else(|| {
                self.agent_server_store
                    .read(cx)
                    .agent_display_name(&self.agent.agent_id())
                    .unwrap_or_else(|| self.agent.agent_id().0.to_string().into())
            })
    }

    /// `Chat` while the executor is still connecting.
    ///
    /// `OMEGA-DELTA-0170`, superseding the refusal half of
    /// `OMEGA-DELTA-0122`. Enter always accepts: the composer's text moves to
    /// [`Self::pending_connect_messages`] and is drawn in the chat as a
    /// pending turn, and the composer clears so the next sentence — and the
    /// next Enter — behave exactly as they would in a connected thread. The
    /// owner, on the refusal this replaces: "never block user from hitting
    /// enter, if not connected just show a loading thing in the chat."
    fn submit_while_connecting(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        let loading_composer = self.loading_composer(window, cx);
        let text = loading_composer.read(cx).text(cx);
        if text.trim().is_empty() {
            return false;
        }
        let content = vec![acp::ContentBlock::Text(acp::TextContent::new(text.clone()))];
        loading_composer.update(cx, |editor, cx| {
            editor.set_text("", window, cx);
        });
        self.pending_connect_messages
            .push(PendingConnectMessage { text, content });
        self.first_message_submitted_while_connecting = true;
        cx.notify();
        true
    }

    fn submit_before_session(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if omega_zero_base::is_primary_interface()
            && self.project.read(cx).visible_worktrees(cx).next().is_none()
        {
            window.dispatch_action(
                Box::new(workspace::Open {
                    create_new_window: Some(false),
                }),
                cx,
            );
            return;
        }
        if !self.submit_while_connecting(window, cx) {
            return;
        }
        if matches!(
            self.preparation_state(cx),
            ConversationPreparation::RouterReady
        ) {
            self.start_deferred_omega_session(window, cx);
        }
    }

    fn start_deferred_omega_session(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.deferred_omega_session.is_some() || self.root_session_id.is_some() {
            return;
        }
        let Some(connected) = self.as_connected() else {
            return;
        };
        if !connected.auth_state.is_ok() || connected.active_view().is_some() {
            return;
        }

        let connection = connected.connection.clone();
        let conversation = connected.conversation.clone();
        let project = self.project.clone();
        let work_dirs = self.desired_work_dirs.clone();
        let task_requirements = self.pending_connect_messages.first().map_or_else(
            || omega_task_requirements(None),
            |message| omega_task_requirements_for_blocks(&message.content),
        );
        let Some(router) = connection
            .clone()
            .downcast::<crate::omega_router::OmegaAgentConnection>()
        else {
            self.handle_load_error(
                LoadError::Other(
                    "Omega's router disconnected before it could choose an executor".into(),
                ),
                window,
                cx,
            );
            return;
        };
        let decision = match router.prepare_next_session(
            task_requirements,
            omega_front_door::router::ExecutorOverride::Auto,
        ) {
            Ok(decision) => decision,
            Err(error) => {
                self.handle_load_error(
                    LoadError::Other(
                        format!("Omega could not prepare the first route: {error:#}").into(),
                    ),
                    window,
                    cx,
                );
                return;
            }
        };
        self.omega_route_summary = Some(visible_omega_route_decision(&decision));
        cx.notify();
        let session = connection
            .clone()
            .new_session(project, work_dirs.clone(), cx);

        self.deferred_omega_session = Some(cx.spawn_in(window, async move |this, cx| {
            let result = match session.await {
                Err(error) => match error.downcast::<AuthRequired>() {
                    Ok(error) => {
                        cx.update(|window, cx| {
                            Self::handle_auth_required(this.clone(), error, connection, window, cx)
                        })
                        .log_err();
                        return;
                    }
                    Err(error) => Err(error),
                },
                Ok(thread) => Ok(thread),
            };

            this.update_in(cx, |this, window, cx| match result {
                Ok(thread) => {
                    thread.update(cx, |thread, cx| {
                        thread.set_work_dirs(work_dirs, cx);
                    });
                    this.clear_resolved_request_elicitations_for_connection(&connection, cx);
                    let session_id = thread.read(cx).session_id().clone();
                    conversation.update(cx, |conversation, cx| {
                        conversation.register_thread(thread.clone(), cx);
                    });
                    let current =
                        this.new_thread_view(thread, conversation, false, None, window, cx);
                    current.update(cx, |thread_view, cx| {
                        thread_view.rehydrate_durable_queue(
                            this.send_queue_journal.clone(),
                            window,
                            cx,
                        );
                    });
                    let loading_composer_was_focused = this
                        .loading_composer
                        .as_ref()
                        .is_some_and(|editor| editor.focus_handle(cx).contains_focused(window, cx));
                    this.hand_loading_draft_over(&current, window, cx);
                    if loading_composer_was_focused {
                        current
                            .read(cx)
                            .message_editor
                            .focus_handle(cx)
                            .focus(window, cx);
                    }

                    this.root_session_id = Some(session_id.clone());
                    if let Some(connected) = this.as_connected_mut() {
                        connected.active_id = Some(session_id.clone());
                        connected.threads.insert(session_id, current.clone());
                    }
                    cx.emit(StateChange);
                    cx.emit(AcpServerViewEvent::ActiveThreadChanged);
                    cx.emit(RootThreadUpdated);
                    this.dispatch_pending_connect_messages(&current, window, cx);
                    cx.notify();
                }
                Err(error) => {
                    this.handle_load_error(LoadError::Other(error.to_string().into()), window, cx)
                }
            })
            .log_err();
        }));
    }

    /// Send everything a person submitted while the executor was connecting,
    /// in order, now that a thread exists.
    ///
    /// `OMEGA-DELTA-0170`. The list is taken — `std::mem::take` — before
    /// anything is enqueued, so a pending message can dispatch at most once no
    /// matter how the connection state churns afterwards. The messages ride
    /// the thread's ordinary `MessageQueue`: the first is fast-tracked the
    /// way Enter on an empty composer fast-tracks a queued message, and the
    /// rest auto-dispatch as each turn stops, which is the exact machinery —
    /// and the exact ordering promise — follow-ups typed during generation
    /// already have.
    ///
    /// Deferred, because this runs while the `ConversationView` is mutably
    /// borrowed and dispatching reaches back through the thread view
    /// (omega#116 is the crash class a second lease produces).
    fn dispatch_pending_connect_messages(
        &mut self,
        thread_view: &Entity<ThreadView>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.pending_connect_messages.is_empty() {
            return;
        }
        let pending = std::mem::take(&mut self.pending_connect_messages);
        let thread_view = thread_view.downgrade();
        window.defer(cx, move |window, cx| {
            thread_view
                .update(cx, |thread_view, cx| {
                    for message in pending {
                        let fallback_content = message.content.clone();
                        if let Err(error) =
                            thread_view.add_to_queue(message.content, Vec::new(), window, cx)
                        {
                            thread_view.message_editor.update(cx, |editor, cx| {
                                if editor.is_empty(cx) {
                                    editor.set_message(fallback_content, window, cx);
                                } else {
                                    editor.append_message(
                                        fallback_content,
                                        Some("\n\n"),
                                        window,
                                        cx,
                                    );
                                }
                            });
                            thread_view.handle_message_queue_error(error, cx);
                        }
                    }
                    let is_generating = thread_view.thread.read(cx).status() != ThreadStatus::Idle;
                    match thread_view.message_queue.try_fast_track(is_generating) {
                        Ok(Some(candidate)) => {
                            thread_view.dispatch_queued_candidate(candidate, window, cx)
                        }
                        Ok(None) => {}
                        Err(error) => thread_view.handle_message_queue_error(error, cx),
                    }
                })
                .ok();
        });
    }

    /// Rebuild the connection for a *different* executor.
    ///
    /// omega#112. `reset` is the wrong move for this and the difference is one
    /// argument: it passes `resume_session_id`, so the freshly built connection
    /// is immediately asked to continue the thread the previous executor owned.
    /// Codex answered exactly that way — `no rollout found for thread id ...`,
    /// a rollout being its session file — because the id named another agent's
    /// conversation.
    ///
    /// Session ids are not portable between executors, so switching starts a
    /// new one. Everything else about the rebuild is the same, and the rebuild
    /// is the point: the connection is where the choice is read, so nothing
    /// short of building a new one can attach a different executor.
    /// Rebuild onto a different executor, on the next turn of the loop.
    ///
    /// omega#116. The whole body is deferred, and that is the fix rather than
    /// a detail. Every step of it reads the `ThreadView` — for the work dirs,
    /// for the composer's focus handle — so calling it while a `ThreadView` is
    /// mutably borrowed takes a second lease and GPUI panics with
    /// `cannot read ThreadView while it is already being updated`.
    ///
    /// Its two callers do exactly that. The Shift-Tab handler runs inside
    /// `cx.listener` on the `ThreadView`; so does the menu path once a
    /// selection is made. The owner pressed Shift-Tab and the application
    /// vanished.
    ///
    /// Deferring means no caller has to know. Fixing the one read I noticed
    /// first was the wrong shape: it left the same trap for the next read
    /// anyone adds, and the second crash proved it — the app died at launch
    /// for the same reason, in a path I had not looked at.
    pub fn reset_onto_new_executor(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        // omega#117. Cycle now, connect once you stop.
        //
        // Shift-Tab used to rebuild on every press, so walking four executors
        // paid four full adapter starts — each one an `npm exec` and an ACP
        // handshake — and the owner could not cycle at anything like the speed
        // a keystroke implies. The selection is a *choice*; the connection is
        // what the choice eventually costs, and the two do not have to happen
        // in the same instant.
        //
        // The label, the placeholder and the disclosure all read the selection,
        // so they change on the keystroke and cycling stays instant. Only the
        // connect waits, and only until the presses stop.
        //
        // Holding the task is the debounce: assigning here drops the previous
        // one, and a dropped GPUI task is cancelled. `rebuild_onto_new_executor`
        // is still deferred internally, so the borrow rule that crashed this
        // twice is unchanged.
        const SETTLE: Duration = Duration::from_millis(450);

        let this = cx.entity();
        self.pending_executor_rebuild = Some(cx.spawn_in(window, async move |_, cx| {
            cx.background_executor().timer(SETTLE).await;
            this.update_in(cx, |this, window, cx| {
                this.pending_executor_rebuild = None;
                this.rebuild_onto_new_executor(window, cx);
            })
            .ok();
        }));
    }

    pub fn executor_switch_pending(&self) -> bool {
        self.pending_executor_rebuild.is_some()
    }

    fn rebuild_onto_new_executor(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.deferred_omega_session.take();
        self.omega_route_summary = None;
        // omega#117. Landing back where you started costs nothing.
        //
        // Cycling four executors and stopping on the one already attached is
        // the ordinary way to look at the list, and it used to tear down a
        // working connection and build the same one again. The comparison is
        // against the thread's *disclosure* — what is actually attached — not
        // against the last thing selected, because those disagree exactly when
        // a previous rebuild failed, and that is the case where a rebuild is
        // most wanted rather than least.
        if let Some(selected) = crate::omega_executor_selector::selected()
            && let Some(view) = self.root_thread_view()
        {
            let disclosure = view.read(cx).executor_disclosure(cx);
            let attached = crate::omega_executor_selector::SelectableExecutor::of(
                disclosure.class,
                disclosure.agent_id.as_ref(),
            );
            if attached == Some(selected) {
                return;
            }
        }

        self.clear_resolved_request_elicitations(cx);
        self.loading_status = None;

        // Drop the cached connection first, or nothing reconnects.
        //
        // omega#112. `AgentConnectionStore` keys connections by `Agent`, and
        // the key does not change when the executor does — Omega's router is
        // the agent either way. So `request_connection` handed back the *same*
        // live connection, with Codex still in its external-ACP slot, and the
        // rebuild that was supposed to read the new choice never called
        // `connect` at all. The window said "Loading" and came back to Codex.
        //
        // `restart_connection` removes the entry before requesting, which is
        // exactly the difference. It is already the move used when a
        // connection has to be genuinely re-established rather than reused.
        cx.update_entity(&self.connection_store.clone(), |store, cx| {
            store.restart_connection(self.connection_key.clone(), self.agent.clone(), cx);
        });

        // The working directory survives; the conversation does not. A person
        // switching executors is still working in the same place.
        let work_dirs = self
            .root_thread_view()
            .and_then(|thread_view| thread_view.read(cx).thread.read(cx).work_dirs().cloned());

        let state = Self::initial_state(
            self.agent.clone(),
            self.connection_store.clone(),
            self.connection_key.clone(),
            None,
            work_dirs,
            None,
            self.project.clone(),
            None,
            AgentThreadSource::AgentPanel,
            window,
            cx,
        );
        self.set_server_state(state, cx);

        if let Some(view) = self.root_thread_view() {
            view.update(cx, |this, cx| {
                this.message_editor.update(cx, |editor, cx| {
                    editor.set_session_capabilities(this.session_capabilities.clone(), cx);
                });
            });

            // omega#112. Put the cursor back in the composer — and omega#116:
            // *outside* the update above, which is not a style preference.
            //
            // `PopoverMenu` restores focus to whatever held it before the menu
            // opened, which is normally right; this rebuild replaced the whole
            // thread view, so the handle it restores to belongs to an editor
            // that no longer exists.
            //
            // Focusing dispatches focus handlers synchronously, and those read
            // the `ThreadView`. Done inside `view.update`, that is a second
            // lease on an entity already mutably borrowed, and GPUI panics —
            // `cannot read ThreadView while it is already being updated`, at
            // launch, before any window appears. Reading the handle out first
            // and focusing after the borrow has ended is what makes it safe.
            let composer = view.read(cx).message_editor.read(cx).focus_handle(cx);
            composer.focus(window, cx);
        } else {
            // `OMEGA-DELTA-0122`. Which is the ordinary case here, not the
            // fallback: the rebuild has just put this view back into `Loading`,
            // so there is no thread view yet and the composer on screen is the
            // loading one. It gets the caret for the reason above — a person
            // who has just chosen an executor should be able to type, and
            // typing is now something they can do before it arrives.
            let loading_composer = self.loading_composer(window, cx);
            loading_composer.focus_handle(cx).focus(window, cx);
        }
        cx.notify();
    }

    fn reset(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.deferred_omega_session.take();
        self.omega_route_summary = None;
        let (resume_session_id, work_dirs, title) = self
            .root_thread_view()
            .map(|thread_view| {
                let tv = thread_view.read(cx);
                let thread = tv.thread.read(cx);
                (
                    Some(thread.session_id().clone()),
                    thread.work_dirs().cloned(),
                    thread.title_or_first_user_message(cx),
                )
            })
            .unwrap_or_else(|| {
                let session_id = self.root_session_id.clone();
                let (work_dirs, title) = session_id
                    .as_ref()
                    .and_then(|id| {
                        let store = ThreadMetadataStore::try_global(cx)?;
                        let entry = store.read(cx).entry_by_session(id)?;
                        Some((Some(entry.folder_paths().clone()), entry.title()))
                    })
                    .unwrap_or((None, None));
                (session_id, work_dirs, title)
            });

        self.clear_resolved_request_elicitations(cx);
        self.loading_status = None;

        let state = Self::initial_state(
            self.agent.clone(),
            self.connection_store.clone(),
            self.connection_key.clone(),
            resume_session_id,
            work_dirs,
            title,
            self.project.clone(),
            None,
            AgentThreadSource::AgentPanel,
            window,
            cx,
        );
        self.set_server_state(state, cx);

        if let Some(view) = self.root_thread_view() {
            view.update(cx, |this, cx| {
                this.message_editor.update(cx, |editor, cx| {
                    editor.set_session_capabilities(this.session_capabilities.clone(), cx);
                });
            });
        }
        cx.notify();
    }

    fn initial_state(
        agent: Rc<dyn AgentServer>,
        connection_store: Entity<AgentConnectionStore>,
        connection_key: Agent,
        resume_session_id: Option<acp::SessionId>,
        work_dirs: Option<PathList>,
        title: Option<SharedString>,
        project: Entity<Project>,
        initial_content: Option<AgentInitialContent>,
        source: AgentThreadSource,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> ServerState {
        // OMEGA-DELTA-0035. The first-party agent is the router over the native
        // server now, so "is this the native agent?" cannot be a bare downcast:
        // a wrapped native agent would read as external and be refused in a
        // shared project.
        if project.read(cx).is_via_collab() && !crate::omega_router::is_native_agent_server(&agent)
        {
            return ServerState::LoadError {
                error: LoadError::Other(
                    "External agents are not yet supported in shared projects.".into(),
                ),
            };
        }
        let initial_work_dirs = work_dirs.unwrap_or_else(|| project.read(cx).default_path_list(cx));
        let has_initial_request = initial_content
            .as_ref()
            .is_some_and(omega_initial_content_has_request);
        let is_new_omega =
            resume_session_id.is_none() && matches!(&connection_key, Agent::NativeAgent);
        let defer_omega_session = is_new_omega && !has_initial_request;
        let prepare_initial_omega_route = is_new_omega && has_initial_request;

        let connection_entry = connection_store.update(cx, |store, cx| {
            store.request_connection(connection_key, agent.clone(), cx)
        });

        let connection_entry_subscription =
            cx.subscribe(&connection_entry, |this, _entry, event, cx| match event {
                AgentConnectionEntryEvent::NewVersionAvailable(version) => {
                    if let Some(thread) = this.root_thread_view() {
                        thread.update(cx, |thread, cx| {
                            thread.new_server_version_available = Some(version.clone());
                            cx.notify();
                        });
                    }
                }
                AgentConnectionEntryEvent::LoadingStatusChanged(status) => {
                    this.loading_status = status.clone();
                    cx.notify();
                }
            });

        let connect_result = connection_entry.read(cx).wait_for_connection();

        let side = crate::agent_sidebar_side(cx);
        let thread_location = "current_worktree";

        let load_task = cx.spawn_in(window, async move |this, cx| {
            let connection = match connect_result.await {
                Ok(AgentConnectedState { connection, .. }) => connection,
                Err(err) => {
                    this.update_in(cx, |this, window, cx| {
                        this.handle_load_error(err, window, cx);
                        cx.notify();
                    })
                    .log_err();
                    return;
                }
            };

            this.update_in(cx, |this, _window, cx| {
                let request_elicitation_subscription =
                    Self::request_elicitation_subscription(&connection, cx);
                if let ServerState::Loading {
                    connection: loading_connection,
                    _request_elicitation_subscription,
                    ..
                } = &mut this.server_state
                {
                    *loading_connection = Some(connection.clone());
                    *_request_elicitation_subscription = request_elicitation_subscription;
                    cx.notify();
                }
            })
            .log_err();

            telemetry::event!(
                "Agent Thread Started",
                agent = connection.telemetry_id(),
                source = source.as_str(),
                side = side,
                thread_location = thread_location
            );

            if defer_omega_session {
                this.update_in(cx, |this, window, cx| {
                    let request_elicitation_subscription =
                        Self::request_elicitation_subscription(&connection, cx);
                    this.set_server_state(
                        ServerState::Connected(ConnectedServerState {
                            connection,
                            auth_state: AuthState::Ok,
                            active_id: None,
                            right_pane_session_id: None,
                            threads: HashMap::default(),
                            conversation: cx.new(|_cx| Conversation::default()),
                            _connection_entry_subscription: connection_entry_subscription,
                            _request_elicitation_subscription: request_elicitation_subscription,
                        }),
                        cx,
                    );
                    if !this.pending_connect_messages.is_empty() {
                        this.start_deferred_omega_session(window, cx);
                    }
                })
                .log_err();
                return;
            }

            if prepare_initial_omega_route {
                let Some(router) = connection
                    .clone()
                    .downcast::<crate::omega_router::OmegaAgentConnection>()
                else {
                    this.update_in(cx, |this, window, cx| {
                        this.handle_load_error(
                            LoadError::Other(
                                "Omega's router disconnected before it could choose an executor"
                                    .into(),
                            ),
                            window,
                            cx,
                        );
                    })
                    .log_err();
                    return;
                };
                let decision = router.prepare_next_session(
                    omega_task_requirements(initial_content.as_ref()),
                    omega_front_door::router::ExecutorOverride::Auto,
                );
                match decision {
                    Ok(decision) => {
                        this.update(cx, |this, cx| {
                            this.omega_route_summary =
                                Some(visible_omega_route_decision(&decision));
                            cx.notify();
                        })
                        .log_err();
                    }
                    Err(error) => {
                        this.update_in(cx, |this, window, cx| {
                            this.handle_load_error(
                                LoadError::Other(
                                    format!("Omega could not prepare the first route: {error:#}")
                                        .into(),
                                ),
                                window,
                                cx,
                            );
                        })
                        .log_err();
                        return;
                    }
                }
            }

            // `OMEGA-DELTA-0214`. Isolation is settled here, before the
            // session exists. After `new_session` an external ACP agent's
            // working directory is fixed for the life of the session, so this
            // is the last honest moment to move the thread into its own
            // worktree.
            if let Ok(Some(pending)) =
                this.read_with(cx, |this, _cx| this.pending_work_dirs.clone())
            {
                if let Some(isolated) = pending.await {
                    this.update(cx, |this, cx| {
                        this.set_work_dirs(isolated, cx);
                    })
                    .log_err();
                }
                this.update(cx, |this, _cx| {
                    this.pending_work_dirs = None;
                })
                .log_err();
            }

            let session_work_dirs =
                match this.read_with(cx, |this, _cx| this.desired_work_dirs.clone()) {
                    Ok(work_dirs) => work_dirs,
                    Err(error) => {
                        log::warn!(
                            "conversation disappeared while resolving its working directory: \
                             {error:#}"
                        );
                        initial_work_dirs.clone()
                    }
                };
            let mut resumed_without_history = false;
            let result = if let Some(session_id) = resume_session_id.clone() {
                cx.update(|_, cx| {
                    if connection.supports_load_session() {
                        connection.clone().load_session(
                            session_id,
                            project.clone(),
                            session_work_dirs,
                            title,
                            cx,
                        )
                    } else if connection.supports_resume_session() {
                        resumed_without_history = true;
                        connection.clone().resume_session(
                            session_id,
                            project.clone(),
                            session_work_dirs,
                            title,
                            cx,
                        )
                    } else {
                        Task::ready(Err(anyhow!(LoadError::Other(
                            "Loading or resuming sessions is not supported by this agent.".into()
                        ))))
                    }
                })
                .log_err()
            } else {
                cx.update(|_, cx| {
                    connection
                        .clone()
                        .new_session(project.clone(), session_work_dirs, cx)
                })
                .log_err()
            };

            let Some(result) = result else {
                return;
            };

            let result = match result.await {
                Err(e) => match e.downcast::<acp_thread::AuthRequired>() {
                    Ok(err) => {
                        cx.update(|window, cx| {
                            Self::handle_auth_required(this, err, connection, window, cx)
                        })
                        .log_err();
                        return;
                    }
                    Err(err) => Err(err),
                },
                Ok(thread) => Ok(thread),
            };

            this.update_in(cx, |this, window, cx| {
                match result {
                    Ok(thread) => {
                        let desired_work_dirs = this.desired_work_dirs.clone();
                        let restored_lifecycle =
                            ThreadMetadataStore::try_global(cx).and_then(|store| {
                                store
                                    .read(cx)
                                    .entry(this.thread_id)
                                    .map(|metadata| metadata.lifecycle)
                            });
                        thread.update(cx, |thread, cx| {
                            thread.set_work_dirs(desired_work_dirs, cx);
                            if let Some(lifecycle) = restored_lifecycle {
                                thread.restore_terminal_status(lifecycle.terminal_status());
                            }
                        });
                        this.clear_resolved_request_elicitations_for_connection(&connection, cx);
                        let root_session_id = thread.read(cx).session_id().clone();

                        let conversation = cx.new(|cx| {
                            let mut conversation = Conversation::default();
                            conversation.register_thread(thread.clone(), cx);
                            conversation
                        });

                        let current = this.new_thread_view(
                            thread,
                            conversation.clone(),
                            resumed_without_history,
                            initial_content,
                            window,
                            cx,
                        );
                        current.update(cx, |thread_view, cx| {
                            thread_view.rehydrate_durable_queue(
                                this.send_queue_journal.clone(),
                                window,
                                cx,
                            );
                        });

                        let loading_composer_was_focused =
                            this.loading_composer.as_ref().is_some_and(|editor| {
                                editor.focus_handle(cx).contains_focused(window, cx)
                            });
                        this.hand_loading_draft_over(&current, window, cx);

                        if loading_composer_was_focused {
                            current
                                .read(cx)
                                .message_editor
                                .focus_handle(cx)
                                .focus(window, cx);
                        }

                        this.root_session_id = Some(root_session_id.clone());
                        let request_elicitation_subscription =
                            Self::request_elicitation_subscription(&connection, cx);
                        this.set_server_state(
                            ServerState::Connected(ConnectedServerState {
                                connection,
                                auth_state: AuthState::Ok,
                                active_id: Some(root_session_id.clone()),
                                right_pane_session_id: None,
                                threads: HashMap::from_iter([(root_session_id, current.clone())]),
                                conversation,
                                _connection_entry_subscription: connection_entry_subscription,
                                _request_elicitation_subscription: request_elicitation_subscription,
                            }),
                            cx,
                        );
                        this.dispatch_pending_connect_messages(&current, window, cx);
                    }
                    Err(err) => {
                        this.handle_load_error(
                            LoadError::Other(err.to_string().into()),
                            window,
                            cx,
                        );
                    }
                };
            })
            .log_err();
        });

        let loading_view = cx.new(|_cx| LoadingView {
            _load_task: load_task,
        });

        ServerState::Loading {
            _loading: loading_view,
            connection: None,
            _request_elicitation_subscription: None,
        }
    }

    fn new_thread_view(
        &self,
        thread: Entity<AcpThread>,
        conversation: Entity<Conversation>,
        resumed_without_history: bool,
        initial_content: Option<AgentInitialContent>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Entity<ThreadView> {
        let agent_id = self.agent.agent_id();
        let connection = thread.read(cx).connection().clone();
        let session_id = thread.read(cx).session_id().clone();
        let available_skills = connection
            .clone()
            .downcast::<agent::NativeAgentConnection>()
            .map(|native_connection| native_available_skills(&native_connection, &session_id, cx))
            .unwrap_or_default();
        let omega_steer_capability = if connection
            .clone()
            .downcast::<crate::omega_exo_connection::ExoHarnessConnection>()
            .is_some()
        {
            omega_front_door::SteerCapability::CannotSteer
        } else {
            omega_front_door::SteerCapability::Unknown
        };
        let session_capabilities = Arc::new(RwLock::new(
            SessionCapabilities::new(
                thread.read(cx).prompt_capabilities(),
                thread.read(cx).available_commands().to_vec(),
                available_skills,
            )
            .with_omega_steer_capability(omega_steer_capability),
        ));

        let action_log = thread.read(cx).action_log().clone();

        let entry_view_state = cx.new(|_| {
            EntryViewState::new(
                self.workspace.clone(),
                self.project.downgrade(),
                self.thread_store.clone(),
                session_capabilities.clone(),
                self.agent.agent_id(),
            )
        });

        let count = thread.read(cx).entries().len();
        let list_state = ListState::new(0, gpui::ListAlignment::Top, px(2048.0));
        list_state.set_follow_mode(gpui::FollowMode::Tail);

        entry_view_state.update(cx, |view_state, cx| {
            for ix in 0..count {
                view_state.sync_entry(ix, &thread, window, cx);
            }
            list_state.splice_focusable(
                0..0,
                (0..count).map(|ix| view_state.entry(ix)?.focus_handle(cx)),
            );
        });

        if let Some(scroll_position) = thread.read(cx).ui_scroll_position() {
            list_state.scroll_to(scroll_position);
        } else {
            list_state.scroll_to_end();
        }

        AgentDiff::set_active_thread(&self.workspace, thread.clone(), window, cx);

        let connection = thread.read(cx).connection().clone();
        let session_id = thread.read(cx).session_id().clone();

        // Check for config options first
        // Config options take precedence over legacy mode/model selectors
        let config_options_provider = connection.session_config_options(&session_id, cx);

        let config_options_view;
        let mode_selector;
        let model_selector;
        if let Some(config_options) = config_options_provider {
            // Use config options - don't create mode_selector or model_selector
            let agent_server = self.agent.clone();
            let fs = self.project.read(cx).fs().clone();
            config_options_view =
                Some(cx.new(|cx| {
                    ConfigOptionsView::new(config_options, agent_server, fs, window, cx)
                }));
            model_selector = None;
            mode_selector = None;
        } else {
            // Fall back to dedicated mode/model selectors
            config_options_view = None;
            model_selector = connection.model_selector(&session_id).map(|selector| {
                cx.new(|cx| {
                    ModelSelectorPopover::new(
                        selector,
                        PopoverMenuHandle::default(),
                        self.focus_handle(cx),
                        window,
                        cx,
                    )
                })
            });

            mode_selector = connection
                .session_modes(&session_id, cx)
                .map(|session_modes| {
                    let fs = self.project.read(cx).fs().clone();
                    cx.new(|_cx| ModeSelector::new(session_modes, self.agent.clone(), fs))
                });
        }

        let subscriptions = vec![
            cx.subscribe_in(&thread, window, Self::handle_thread_event),
            cx.observe(&action_log, |_, _, cx| cx.notify()),
        ];

        let subagent_sessions = thread
            .read(cx)
            .entries()
            .iter()
            .filter_map(|entry| match entry {
                AgentThreadEntry::ToolCall(call) => call
                    .subagent_session_info
                    .as_ref()
                    .map(|i| i.session_id.clone()),
                _ => None,
            })
            .collect::<Vec<_>>();

        if !subagent_sessions.is_empty() {
            let parent_session_id = thread.read(cx).session_id().clone();
            cx.spawn_in(window, async move |this, cx| {
                this.update_in(cx, |this, window, cx| {
                    for subagent_id in subagent_sessions {
                        this.load_subagent_session(
                            subagent_id,
                            parent_session_id.clone(),
                            window,
                            cx,
                        );
                    }
                })
            })
            .detach();
        }

        let profile_selector: Option<Rc<agent::NativeAgentConnection>> =
            connection.clone().downcast();
        let profile_selector = profile_selector
            .and_then(|native_connection| native_connection.thread(&session_id, cx))
            .map(|native_thread| {
                cx.new(|cx| {
                    ProfileSelector::new(
                        <dyn Fs>::global(cx),
                        Arc::new(native_thread),
                        self.focus_handle(cx),
                        cx,
                    )
                })
            });

        let agent_display_name = self
            .agent_server_store
            .read(cx)
            .agent_display_name(&agent_id.clone())
            .unwrap_or_else(|| agent_id.0.clone());

        let agent_icon = self.agent.logo();
        let agent_icon_from_external_svg = self
            .agent_server_store
            .read(cx)
            .agent_icon(&self.agent.agent_id())
            .or_else(|| {
                project::AgentRegistryStore::try_global(cx).and_then(|store| {
                    store
                        .read(cx)
                        .agent(&self.agent.agent_id())
                        .and_then(|a| a.icon_path().cloned())
                })
            });

        let weak = cx.weak_entity();
        cx.new(|cx| {
            ThreadView::new(
                self.thread_id,
                thread,
                conversation,
                weak,
                self.vim_mode_indicator.clone(),
                agent_icon,
                agent_icon_from_external_svg,
                agent_id,
                agent_display_name,
                self.workspace.clone(),
                entry_view_state,
                config_options_view,
                mode_selector,
                model_selector,
                profile_selector,
                list_state,
                session_capabilities,
                resumed_without_history,
                self.project.downgrade(),
                self.code_span_resolver.clone(),
                self.thread_store.clone(),
                initial_content,
                subscriptions,
                window,
                cx,
            )
        })
    }

    fn handle_auth_required(
        this: WeakEntity<Self>,
        err: AuthRequired,
        connection: Rc<dyn AgentConnection>,
        window: &mut Window,
        cx: &mut App,
    ) {
        this.update(cx, |this, cx| {
            let description = err
                .description
                .map(|desc| cx.new(|cx| Markdown::new(desc.into(), None, None, cx)));
            let auth_state = AuthState::Unauthenticated {
                pending_auth_method: None,
                description,
            };
            // omega#166. The auth card replaces whatever composer held the
            // keyboard — the connected thread's message editor, or the
            // loading composer a brand-new direct-agent draft focuses on
            // creation. If focus is left on an editor that the next frame no
            // longer paints, the window has no focused dispatch path at all:
            // the command palette, New Thread, and every other workspace
            // binding go dead, and a keyboard-only user is trapped on the
            // auth card with only pointer escapes. So whenever focus was
            // anywhere inside this conversation view, it moves to the view's
            // own root handle, which every state — including the auth card —
            // keeps in the tree via `track_focus`.
            let focus_was_inside = this.focus_handle.contains_focused(window, cx);
            if let Some(connected) = this.as_connected_mut() {
                connected.auth_state = auth_state;
                cx.emit(StateChange);
            } else {
                let request_elicitation_subscription =
                    Self::request_elicitation_subscription(&connection, cx);
                this.set_server_state(
                    ServerState::Connected(ConnectedServerState {
                        auth_state,
                        active_id: None,
                        right_pane_session_id: None,
                        threads: HashMap::default(),
                        connection,
                        conversation: cx.new(|_cx| Conversation::default()),
                        _connection_entry_subscription: Subscription::new(|| {}),
                        _request_elicitation_subscription: request_elicitation_subscription,
                    }),
                    cx,
                );
            }
            if focus_was_inside {
                this.focus_handle.focus(window, cx);
            }
            cx.notify();
        })
        .ok();
    }

    fn handle_load_error(&mut self, err: LoadError, window: &mut Window, cx: &mut Context<Self>) {
        // omega#166, same trap as `handle_auth_required`: the load-error card
        // also drops the composer, so focus left on a no-longer-painted
        // editor (message editor or the loading composer) would kill every
        // workspace keybinding. Reclaim it onto the view's root handle, which
        // the error state keeps in the tree.
        if self.focus_handle.contains_focused(window, cx) {
            self.focus_handle.focus(window, cx);
        }
        self.emit_load_error_telemetry(&err);
        self.set_server_state(ServerState::LoadError { error: err }, cx);
    }

    fn handle_agent_servers_updated(
        &mut self,
        _agent_server_store: &Entity<project::AgentServerStore>,
        _event: &project::AgentServersUpdated,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // If we're in a LoadError state OR have a thread_error set (which can happen
        // when agent.connect() fails during loading), retry loading the thread.
        // This handles the case where a thread is restored before authentication completes.
        let should_retry = match &self.server_state {
            ServerState::Loading { .. } => false,
            ServerState::LoadError { .. } => true,
            ServerState::Connected(connected) => {
                connected.auth_state.is_ok() && connected.has_thread_error(cx)
            }
        };

        if should_retry {
            if let Some(active) = self.root_thread_view() {
                active.update(cx, |active, cx| {
                    active.clear_thread_error(cx);
                });
            }
            self.reset(window, cx);
        }
    }

    pub fn agent_key(&self) -> &Agent {
        &self.connection_key
    }

    pub fn title(&self, cx: &App) -> SharedString {
        match &self.server_state {
            ServerState::Connected(view) => view
                .active_view()
                .and_then(|v| v.read(cx).thread.read(cx).title_or_first_user_message(cx))
                .unwrap_or_else(|| DEFAULT_THREAD_TITLE.into()),
            ServerState::Loading { .. } => self
                .loading_status
                .clone()
                .unwrap_or_else(|| "Loading…".into()),
            ServerState::LoadError { error, .. } => match error {
                LoadError::Unsupported { .. } => {
                    format!("Upgrade {}", self.agent.agent_id()).into()
                }
                LoadError::FailedToInstall(_) => {
                    format!("Failed to Install {}", self.agent.agent_id()).into()
                }
                LoadError::Exited { .. } => format!("{} Exited", self.agent.agent_id()).into(),
                // Deliberately not "Error Loading <agent>": the agent loaded.
                LoadError::SessionGone => "Conversation No Longer Available".into(),
                LoadError::Other(_) => format!("Error Loading {}", self.agent.agent_id()).into(),
            },
        }
    }

    pub fn cancel_generation(&mut self, cx: &mut Context<Self>) {
        if let Some(active) = self.active_thread() {
            active.update(cx, |active, cx| {
                active.cancel_generation(cx);
            });
        }
    }

    pub fn parent_id(&self) -> ThreadId {
        self.thread_id
    }

    pub fn is_loading(&self) -> bool {
        matches!(self.server_state, ServerState::Loading { .. })
    }

    pub(crate) fn preparation_state(&self, cx: &App) -> ConversationPreparation {
        match &self.server_state {
            ServerState::Loading { .. } => ConversationPreparation::Loading,
            ServerState::LoadError { error } => ConversationPreparation::SetupRequired {
                reason: error.to_string().into(),
            },
            ServerState::Connected(connected) if !connected.auth_state.is_ok() => {
                ConversationPreparation::SetupRequired {
                    reason: "This agent requires authentication before creating a session".into(),
                }
            }
            ServerState::Connected(connected)
                if connected.active_view().is_none()
                    && matches!(&self.connection_key, Agent::NativeAgent) =>
            {
                ConversationPreparation::RouterReady
            }
            ServerState::Connected(_) => {
                let Some(thread) = self.root_thread(cx) else {
                    return ConversationPreparation::SetupRequired {
                        reason: "The agent connected but did not create a session".into(),
                    };
                };
                ConversationPreparation::Ready {
                    session_id: thread.read(cx).session_id().to_string(),
                }
            }
        }
    }

    /// Whether this conversation already owns text that must survive a new-
    /// conversation gesture.
    ///
    /// Session creation has several handoff phases. Looking only at ACP
    /// entries misses text still in the loading editor, accepted turns waiting
    /// for a connection, and turns already handed to the connected queue.
    pub fn has_unsubmitted_or_pending_content(&self, cx: &App) -> bool {
        if !self.pending_connect_messages.is_empty()
            || !self.pending_dragged_files.is_empty()
            || self
                .loading_composer
                .as_ref()
                .is_some_and(|editor| !editor.read(cx).text(cx).trim().is_empty())
        {
            return true;
        }

        if let Some(thread_view) = self.root_thread_view() {
            let thread_view = thread_view.read(cx);
            if thread_view.has_queued_messages()
                || !thread_view
                    .message_editor
                    .read(cx)
                    .text(cx)
                    .trim()
                    .is_empty()
            {
                return true;
            }
        }

        self.root_thread(cx).is_some_and(|thread| {
            thread
                .read(cx)
                .draft_prompt()
                .is_some_and(|blocks| !blocks.is_empty())
        })
    }

    pub(crate) fn first_message_was_submitted_while_connecting(&self) -> bool {
        self.first_message_submitted_while_connecting
    }

    pub(crate) fn submitted_message_is_waiting_for_metadata(&self, cx: &App) -> bool {
        self.first_message_submitted_while_connecting
            && ThreadMetadataStore::try_global(cx)
                .and_then(|store| store.read(cx).entry(self.thread_id).cloned())
                .is_none_or(|metadata| metadata.is_draft())
    }

    fn handle_thread_event(
        &mut self,
        thread: &Entity<AcpThread>,
        event: &AcpThreadEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let session_id = thread.read(cx).session_id().clone();
        let has_thread = self
            .as_connected()
            .is_some_and(|connected| connected.threads.contains_key(&session_id));
        if !has_thread {
            return;
        };
        // Asked of the conversation, not of the thread. A subagent that does
        // not record a parent used to answer "I am the root" here, and an
        // external ACP subagent never records one: `new_session` opens it on
        // the other agent's connection, which has never heard of the thread
        // that asked for it. So a Codex or Claude delegation finishing read as
        // the root finishing, and the root's message queue was spent on it —
        // a follow-up the person had typed was dispatched mid-turn and left
        // the queue with nothing said. The conversation already knows which
        // session is its root; every other thread it holds is a subagent of
        // it, whatever the thread says about itself.
        let is_subagent = self.root_session_id.as_ref() != Some(&session_id);
        if !is_subagent {
            let snapshot = {
                let thread = thread.read(cx);
                crate::omega_agent_supervision::ThreadSupervisionSnapshot {
                    thread_key: session_id.0.to_string(),
                    title: thread
                        .title_or_first_user_message(cx)
                        .unwrap_or_else(|| DEFAULT_THREAD_TITLE.into()),
                    executor: {
                        // `OMEGA-DELTA-0208`. The chrome line: this snapshot is
                        // what the supervision surfaces show a person, and the
                        // `provider/model` wire pair is not chrome copy.
                        use crate::omega_executor_disclosure::ThreadExecutorDisclosure as _;
                        crate::omega_routed_model::chrome_line(
                            &thread.omega_executor_disclosure(cx),
                        )
                        .into()
                    },
                    lifecycle: crate::omega_agent_supervision::lifecycle_for_thread(&thread),
                }
            };
            let supervision = crate::omega_agent_supervision::AgentSupervision::global(cx);
            // `OMEGA-DELTA-0214`. Publish the roots alongside the lifecycle.
            // Occupancy is asked at thread creation, when the occupying thread
            // is between turns and holds no claim, so the binding has to be
            // maintained wherever the lifecycle is.
            let remote_connection = self.project.read(cx).remote_connection_options(cx);
            supervision.bind_roots(
                &snapshot.thread_key,
                self.desired_work_dirs.ordered_paths().cloned(),
                remote_connection.as_ref(),
            );
            supervision.set_snapshot(snapshot);
        }
        if !is_subagent && affects_thread_metadata(event) {
            cx.emit(RootThreadUpdated);
        }
        match event {
            AcpThreadEvent::StatusChanged => {
                if let Some(active) = self.thread_view(&session_id) {
                    active.update(cx, |active, cx| {
                        active.sync_generating_indicator(cx);
                    });
                }
            }
            AcpThreadEvent::NewEntry => {
                if let Some(active) = self.thread_view(&session_id) {
                    let entry_view_state = active.read(cx).entry_view_state.clone();
                    let list_state = active.read(cx).list_state.clone();
                    let missing_range = entry_view_state.update(cx, |view_state, cx| {
                        let missing_range = view_state.sync_missing_entries(thread, window, cx);
                        list_state.splice_focusable(
                            missing_range.start..missing_range.start,
                            missing_range
                                .clone()
                                .map(|index| view_state.entry(index)?.focus_handle(cx)),
                        );
                        missing_range
                    });
                    active.update(cx, |active, cx| {
                        for index in missing_range {
                            active.sync_elicitation_state_for_entry(index, window, cx);
                        }
                        active.sync_editor_mode(cx);
                        active.sync_generating_indicator(cx);
                    });
                }
            }
            AcpThreadEvent::EntryUpdated(index) => {
                if let Some(active) = self.thread_view(&session_id) {
                    let entry_view_state = active.read(cx).entry_view_state.clone();
                    let list_state = active.read(cx).list_state.clone();
                    entry_view_state.update(cx, |view_state, cx| {
                        view_state.sync_entry(*index, thread, window, cx);
                    });
                    list_state.remeasure_items(*index..*index + 1);
                    active.update(cx, |active, cx| {
                        active.sync_elicitation_state_for_entry(*index, window, cx);
                        active.auto_expand_streaming_thought(cx);
                        active.sync_generating_indicator(cx);
                    });
                }
            }
            AcpThreadEvent::EntriesRemoved(range) => {
                if let Some(active) = self.thread_view(&session_id) {
                    let entry_view_state = active.read(cx).entry_view_state.clone();
                    let list_state = active.read(cx).list_state.clone();
                    entry_view_state.update(cx, |view_state, _cx| view_state.remove(range.clone()));
                    list_state.splice(range.clone(), 0);
                    active.update(cx, |active, cx| {
                        active.sync_editor_mode(cx);
                    });
                }
            }
            AcpThreadEvent::SubagentSpawned(subagent_session_id) => {
                self.load_subagent_session(subagent_session_id.clone(), session_id, window, cx)
            }
            AcpThreadEvent::ToolAuthorizationRequested(_) => {
                self.notify_with_sound("Waiting for tool confirmation", IconName::Info, window, cx);
            }
            AcpThreadEvent::ToolAuthorizationReceived(_) => {}
            AcpThreadEvent::ElicitationRequested(_) => {
                self.notify_with_sound("Waiting for input", IconName::Info, window, cx);
            }
            AcpThreadEvent::ElicitationResponded(_) => {}
            AcpThreadEvent::Retry(retry) => {
                if let Some(active) = self.thread_view(&session_id) {
                    active.update(cx, |active, _cx| {
                        active.thread_retry_status = Some(retry.clone());
                    });
                }
            }
            AcpThreadEvent::Stopped(stop_reason) => {
                if let Some(active) = self.thread_view(&session_id) {
                    let is_generating =
                        matches!(thread.read(cx).status(), ThreadStatus::Generating);
                    active.update(cx, |active, cx| {
                        if !is_generating {
                            active.thread_retry_status.take();
                            active.clear_auto_expand_tracking(cx);
                            if active.list_state.is_following_tail() {
                                active.list_state.scroll_to_end();
                            }
                        }
                        active.sync_generating_indicator(cx);
                    });
                }
                if is_subagent {
                    if *stop_reason == acp::StopReason::EndTurn {
                        thread.update(cx, |thread, cx| {
                            thread.mark_as_subagent_output(cx);
                        });
                    }
                    return;
                }

                let sent_queued_message = if let Some(active) = self.root_thread_view() {
                    active.update(cx, |active, cx| {
                        // Don't auto-send while the user is editing the next message.
                        let is_first_editor_focused = active
                            .message_queue
                            .first()
                            .is_some_and(|entry| entry.editor.focus_handle(cx).is_focused(window));
                        match active
                            .message_queue
                            .on_generation_stopped(is_first_editor_focused)
                        {
                            Ok(Some(candidate)) => {
                                active.dispatch_queued_candidate(candidate, window, cx);
                                true
                            }
                            Ok(None) => false,
                            Err(error) => {
                                active.handle_message_queue_error(error, cx);
                                false
                            }
                        }
                    })
                } else {
                    false
                };

                // Skip notifying when a queued message was just auto-sent: the agent
                // is not actually idle and a notification here would fire just before the
                // next turn starts.
                if !sent_queued_message {
                    let used_tools = thread.read(cx).used_tools_since_last_user_message();
                    self.notify_with_sound(
                        if used_tools {
                            "Finished running tools"
                        } else {
                            "New message"
                        },
                        IconName::OmegaAssistant,
                        window,
                        cx,
                    );
                }
            }
            AcpThreadEvent::Refusal => {
                let error = ThreadError::Refusal;
                if let Some(active) = self.thread_view(&session_id) {
                    active.update(cx, |active, cx| {
                        active.handle_thread_error(error, cx);
                        active.thread_retry_status.take();
                    });
                }
                if !is_subagent {
                    let model_or_agent_name = self.current_model_name(cx);
                    let notification_message =
                        format!("{} refused to respond to this request", model_or_agent_name);
                    self.notify_with_sound(&notification_message, IconName::Warning, window, cx);
                }
            }
            AcpThreadEvent::Error => {
                if let Some(active) = self.thread_view(&session_id) {
                    let is_generating =
                        matches!(thread.read(cx).status(), ThreadStatus::Generating);
                    active.update(cx, |active, cx| {
                        if !is_generating {
                            active.thread_retry_status.take();
                            if active.list_state.is_following_tail() {
                                active.list_state.scroll_to_end();
                            }
                        }
                        active.sync_generating_indicator(cx);
                    });
                }
                if !is_subagent {
                    self.notify_with_sound(
                        "Agent stopped due to an error",
                        IconName::Warning,
                        window,
                        cx,
                    );
                }
            }
            AcpThreadEvent::LoadError(error) => {
                // omega#167. A load error on a live thread means the agent
                // server died under a conversation the person can already
                // see. Replacing the whole connected state with
                // `ServerState::LoadError` here wiped the streamed transcript
                // from view and left the sidebar row opening nothing, because
                // the row navigates into `ConnectedServerState::threads` and
                // that map had just been dropped. Keep the connected state:
                // the transcript stays, the sidebar row keeps opening it, and
                // the failure lands in the affected thread as an error card.
                // Load-time failures (no thread view yet) still take the
                // `LoadError` surface below.
                if let Some(view) = self.thread_view(&session_id) {
                    view.update(cx, |view, cx| {
                        view.handle_thread_error(
                            ThreadError::Other {
                                message: format!(
                                    "The agent server quit while this conversation was open: \
                                     {error}. The transcript above is preserved; start a new \
                                     conversation to relaunch the agent."
                                )
                                .into(),
                                acp_error_code: None,
                            },
                            cx,
                        );
                    });
                    cx.notify();
                    return;
                }
                self.handle_load_error(error.clone(), window, cx);
            }
            AcpThreadEvent::TitleUpdated => {
                let override_title = ThreadMetadataStore::try_global(cx).and_then(|store| {
                    store
                        .read(cx)
                        .entry(self.thread_id)
                        .and_then(|m| m.title_override.clone())
                });
                let title =
                    override_title.or_else(|| thread.read(cx).title_or_first_user_message(cx));
                if let Some(title) = title
                    && let Some(active_thread) = self.thread_view(&session_id)
                {
                    let title_editor = active_thread.read(cx).title_editor.clone();
                    title_editor.update(cx, |editor, cx| {
                        if editor.text(cx) != title {
                            editor.set_text(title, window, cx);
                        }
                    });
                }
                cx.notify();
            }
            AcpThreadEvent::PromptCapabilitiesUpdated => {
                if let Some(active) = self.thread_view(&session_id) {
                    active.update(cx, |active, _cx| {
                        active
                            .session_capabilities
                            .write()
                            .set_prompt_capabilities(thread.read(_cx).prompt_capabilities());
                    });
                }
            }
            AcpThreadEvent::TokenUsageUpdated => {
                if let Some(active) = self.thread_view(&session_id) {
                    active.update(cx, |active, cx| {
                        active.update_turn_tokens(cx);
                    });
                }
            }
            AcpThreadEvent::AvailableCommandsUpdated(available_commands) => {
                if let Some(thread_view) = self.thread_view(&session_id) {
                    let available_skills = thread
                        .read(cx)
                        .connection()
                        .clone()
                        .downcast::<agent::NativeAgentConnection>()
                        .map(|native_connection| {
                            native_available_skills(&native_connection, &session_id, cx)
                        })
                        .unwrap_or_default();
                    let agent_display_name = self
                        .agent_server_store
                        .read(cx)
                        .agent_display_name(&self.agent.agent_id())
                        .unwrap_or_else(|| self.agent.agent_id().0.to_string().into());

                    // omega#112. Name the executor that will actually read
                    // this, not the router that will hand it over.
                    //
                    // `agent_display_name` comes from `self.agent.agent_id()`,
                    // which is Omega's own router — so the composer said
                    // "Message the Omega Agent" while Codex was running the
                    // turn. The owner: "if im talking to Codex it should say
                    // message Codex".
                    //
                    // Read from the same disclosure record the executor
                    // selector reads, so the placeholder and the dropdown
                    // cannot disagree about who is listening. A disclosure that
                    // is not one of the four selectable names falls back to the
                    // router's name rather than inventing a fifth.
                    let executor_name = thread_view.read(cx).executor_disclosure(cx);
                    let executor_name = crate::omega_executor_selector::SelectableExecutor::of(
                        executor_name.class,
                        executor_name.agent_id.as_ref(),
                    )
                    .map(|executor| executor.name().to_owned());

                    let new_placeholder = placeholder_text(
                        executor_name
                            .as_deref()
                            .unwrap_or(agent_display_name.as_ref()),
                    );

                    thread_view.update(cx, |thread_view, cx| {
                        let mut session_capabilities = thread_view.session_capabilities.write();
                        session_capabilities.set_available_commands(available_commands.clone());
                        session_capabilities.set_available_skills(available_skills);
                        thread_view.message_editor.update(cx, |editor, cx| {
                            editor.set_placeholder_text(&new_placeholder, window, cx);
                        });
                    });
                }
            }
            AcpThreadEvent::ModeUpdated(_mode) => {
                // The connection keeps track of the mode
                cx.notify();
            }
            AcpThreadEvent::ConfigOptionsUpdated(_) => {
                // The watch task in ConfigOptionsView handles rebuilding selectors
                cx.notify();
            }
            AcpThreadEvent::WorkingDirectoriesUpdated => {
                cx.notify();
            }
            AcpThreadEvent::PlanUpdated(_) => {
                cx.notify();
            }
            AcpThreadEvent::ProjectionUpdated(_) => {}
            AcpThreadEvent::PromptUpdated => {
                if !is_subagent && thread.read(cx).is_draft_thread() {
                    self.schedule_draft_prompt_persist(cx);
                }
                cx.notify();
            }
        }
        cx.notify();
    }

    fn schedule_draft_prompt_persist(&mut self, cx: &mut Context<Self>) {
        let thread_id = self.thread_id;
        self.draft_prompt_persist_task = Some(cx.spawn(async move |this, cx| {
            cx.background_executor()
                .timer(DRAFT_PROMPT_PERSIST_DEBOUNCE)
                .await;
            let persist = this.update(cx, |this, cx| {
                let snapshot = if let Some(thread) = this.root_thread(cx) {
                    let thread = thread.read(cx);
                    if !thread.is_draft_thread() {
                        return None;
                    }
                    thread
                        .draft_prompt()
                        .map(|prompt| prompt.to_vec())
                        .unwrap_or_default()
                } else {
                    this.loading_composer
                        .as_ref()
                        .map(|editor| editor.read(cx).text(cx))
                        .filter(|text| !text.is_empty())
                        .map(|text| vec![acp::ContentBlock::Text(acp::TextContent::new(text))])
                        .unwrap_or_default()
                };
                Some(if snapshot.is_empty() {
                    crate::draft_prompt_store::delete(thread_id, cx)
                } else {
                    crate::draft_prompt_store::write(thread_id, &snapshot, cx)
                })
            });
            if let Ok(Some(persist)) = persist {
                persist.await.log_err();
            }
        }));
    }

    fn authenticate(
        &mut self,
        method: acp::AuthMethodId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(workspace) = self.workspace.upgrade() else {
            return;
        };
        let Some(connected) = self.as_connected_mut() else {
            return;
        };
        let connection = connected.connection.clone();

        let AuthState::Unauthenticated {
            pending_auth_method,
            ..
        } = &mut connected.auth_state
        else {
            return;
        };

        let agent_telemetry_id = connection.telemetry_id();

        if let Some(login_task) = connection.terminal_auth_task(&method, cx) {
            pending_auth_method.replace(method.clone());

            let project = self.project.clone();
            cx.emit(StateChange);
            cx.notify();
            self.auth_task = Some(cx.spawn_in(window, {
                async move |this, cx| {
                    let result = async {
                        let login = login_task.await?;
                        this.update_in(cx, |_this, window, cx| {
                            Self::spawn_external_agent_login(
                                login,
                                workspace,
                                project,
                                method.clone(),
                                false,
                                window,
                                cx,
                            )
                        })?
                        .await
                    }
                    .await;

                    match &result {
                        Ok(_) => telemetry::event!(
                            "Authenticate Agent Succeeded",
                            agent = agent_telemetry_id
                        ),
                        Err(_) => {
                            telemetry::event!(
                                "Authenticate Agent Failed",
                                agent = agent_telemetry_id,
                            )
                        }
                    }

                    this.update_in(cx, |this, window, cx| {
                        if let Err(err) = result {
                            this.cancel_request_elicitations(cx);
                            if let Some(ConnectedServerState {
                                auth_state:
                                    AuthState::Unauthenticated {
                                        pending_auth_method,
                                        ..
                                    },
                                ..
                            }) = this.as_connected_mut()
                            {
                                pending_auth_method.take();
                                cx.emit(StateChange);
                            }
                            if let Some(active) = this.root_thread_view() {
                                active.update(cx, |active, cx| {
                                    active.handle_thread_error(err, cx);
                                })
                            }
                        } else {
                            this.reset(window, cx);
                        }
                        this.auth_task.take()
                    })
                    .ok();
                }
            }));
            return;
        }

        pending_auth_method.replace(method.clone());

        let authenticate = connection.authenticate(method, cx);
        cx.emit(StateChange);
        cx.notify();
        self.auth_task = Some(cx.spawn_in(window, {
            async move |this, cx| {
                let result = authenticate.await;

                match &result {
                    Ok(_) => telemetry::event!(
                        "Authenticate Agent Succeeded",
                        agent = agent_telemetry_id
                    ),
                    Err(_) => {
                        telemetry::event!("Authenticate Agent Failed", agent = agent_telemetry_id,)
                    }
                }

                this.update_in(cx, |this, window, cx| {
                    if let Err(err) = result {
                        this.cancel_request_elicitations(cx);
                        if let Some(ConnectedServerState {
                            auth_state:
                                AuthState::Unauthenticated {
                                    pending_auth_method,
                                    ..
                                },
                            ..
                        }) = this.as_connected_mut()
                        {
                            pending_auth_method.take();
                            cx.emit(StateChange);
                        }
                        if let Some(active) = this.root_thread_view() {
                            active.update(cx, |active, cx| active.handle_thread_error(err, cx));
                        }
                    } else {
                        this.reset(window, cx);
                    }
                    this.auth_task.take()
                })
                .ok();
            }
        }));
    }

    fn load_subagent_session(
        &mut self,
        subagent_id: acp::SessionId,
        parent_session_id: acp::SessionId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(connected) = self.as_connected() else {
            return;
        };
        if connected.threads.contains_key(&subagent_id) {
            return;
        }

        // omega#109. A subagent spawned with an `executor` is an external ACP
        // agent — Codex, Claude Code — and its session belongs to that agent's
        // own server, not to this connection. `NativeAgent::sessions` has never
        // heard of it and `load_session` below cannot produce it, so the card
        // had a session id and nothing to resolve it to. It is resolved from
        // the registry the spawn writes instead.
        //
        // The reference taken here is what keeps the transcript afterwards. The
        // subagent's own lifetime ends with the tool call that spawned it: the
        // handle drops the connection, which drops the child process. Holding
        // the thread from here means the card still shows what the subagent did
        // once the agent server is gone, which is the state a reader is in for
        // all but the seconds the turn was running.
        if let Some(external_thread) = agent::external_subagent_thread(&subagent_id, cx) {
            self.register_subagent_thread_view(external_thread, window, cx);
            return;
        }

        if !connected.connection.supports_load_session() {
            return;
        }
        let Some(parent_thread) = connected.threads.get(&parent_session_id) else {
            return;
        };
        let work_dirs = parent_thread
            .read(cx)
            .thread
            .read(cx)
            .work_dirs()
            .cloned()
            .unwrap_or_else(|| self.project.read(cx).default_path_list(cx));

        let subagent_thread_task = connected.connection.clone().load_session(
            subagent_id,
            self.project.clone(),
            work_dirs,
            None,
            cx,
        );

        cx.spawn_in(window, async move |this, cx| {
            let subagent_thread = subagent_thread_task.await?;
            this.update_in(cx, |this, window, cx| {
                this.register_subagent_thread_view(subagent_thread, window, cx);
            })
        })
        .detach();
    }

    /// Give a subagent's thread a view, and put it where the card looks.
    ///
    /// Shared by both ways a subagent thread arrives — loaded from this
    /// connection, or resolved from the external-subagent registry — because
    /// what has to happen after is the same either way, and the half that is
    /// easy to forget is `register_thread`: without it the thread's entries
    /// never reach the view's `EntryViewState`, and the card renders an empty
    /// transcript while the subagent works.
    fn register_subagent_thread_view(
        &mut self,
        subagent_thread: Entity<AcpThread>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(conversation) = self
            .as_connected()
            .map(|connected| connected.conversation.clone())
        else {
            return;
        };
        let subagent_session_id = subagent_thread.read(cx).session_id().clone();
        conversation.update(cx, |conversation, cx| {
            conversation.register_thread(subagent_thread.clone(), cx);
        });
        let view = self.new_thread_view(subagent_thread, conversation, false, None, window, cx);
        let Some(connected) = self.as_connected_mut() else {
            return;
        };
        connected.threads.insert(subagent_session_id, view);
    }

    fn spawn_external_agent_login(
        login: task::SpawnInTerminal,
        workspace: Entity<Workspace>,
        project: Entity<Project>,
        method: acp::AuthMethodId,
        previous_attempt: bool,
        window: &mut Window,
        cx: &mut App,
    ) -> Task<Result<()>> {
        let Some(terminal_panel) = workspace.read(cx).panel::<TerminalPanel>(cx) else {
            return Task::ready(Err(anyhow!("Terminal panel is unavailable")));
        };

        window.spawn(cx, async move |cx| {
            let mut task = login.clone();
            if let Some(cmd) = &task.command {
                // Have "node" command use Zed's managed Node runtime by default
                if cmd == "node" {
                    let resolved_node_runtime = project.update(cx, |project, cx| {
                        let agent_server_store = project.agent_server_store().clone();
                        agent_server_store.update(cx, |store, cx| {
                            store.node_runtime().map(|node_runtime| {
                                cx.background_spawn(async move { node_runtime.binary_path().await })
                            })
                        })
                    });

                    if let Some(resolve_task) = resolved_node_runtime {
                        if let Ok(node_path) = resolve_task.await {
                            task.command = Some(node_path.to_string_lossy().to_string());
                        }
                    }
                }
            }
            task.shell = task::Shell::WithArguments {
                program: task.command.take().expect("login command should be set"),
                args: std::mem::take(&mut task.args),
                title_override: None,
            };

            let terminal = terminal_panel
                .update_in(cx, |terminal_panel, window, cx| {
                    terminal_panel.spawn_task(&task, window, cx)
                })?
                .await?;

            let success_patterns = match method.0.as_ref() {
                "claude-login" | GEMINI_TERMINAL_AUTH_METHOD_ID => vec![
                    "Login successful".to_string(),
                    "Type your message".to_string(),
                ],
                _ => Vec::new(),
            };
            if success_patterns.is_empty() {
                // No success patterns specified: wait for the process to exit and check exit code
                let exit_status = terminal
                    .read_with(cx, |terminal, cx| terminal.wait_for_completed_task(cx))?
                    .await;

                match exit_status {
                    Some(status) if status.success() => Ok(()),
                    Some(status) => Err(anyhow!(
                        "Login command failed with exit code: {:?}",
                        status.code()
                    )),
                    None => Err(anyhow!("Login command terminated without exit status")),
                }
            } else {
                // Look for specific output patterns to detect successful login
                let mut exit_status = terminal
                    .read_with(cx, |terminal, cx| terminal.wait_for_completed_task(cx))?
                    .fuse();

                let logged_in = cx
                    .spawn({
                        let terminal = terminal.clone();
                        async move |cx| {
                            loop {
                                cx.background_executor().timer(Duration::from_secs(1)).await;
                                let content =
                                    terminal.update(cx, |terminal, _cx| terminal.get_content())?;
                                if success_patterns
                                    .iter()
                                    .any(|pattern| content.contains(pattern))
                                {
                                    return anyhow::Ok(());
                                }
                            }
                        }
                    })
                    .fuse();
                futures::pin_mut!(logged_in);
                futures::select_biased! {
                    result = logged_in => {
                        if let Err(e) = result {
                            log::error!("{e}");
                            return Err(anyhow!("exited before logging in"));
                        }
                    }
                    _ = exit_status => {
                        if !previous_attempt
                            && project.read_with(cx, |project, _| project.is_via_remote_server())
                            && method.0.as_ref() == GEMINI_TERMINAL_AUTH_METHOD_ID
                        {
                            return cx
                                .update(|window, cx| {
                                    Self::spawn_external_agent_login(
                                        login,
                                        workspace,
                                        project.clone(),
                                        method,
                                        true,
                                        window,
                                        cx,
                                    )
                                })?
                                .await;
                        }
                        return Err(anyhow!("exited before logging in"));
                    }
                }
                terminal.update(cx, |terminal, _| terminal.kill_active_task())?;
                Ok(())
            }
        })
    }

    pub fn has_user_submitted_prompt(&self, cx: &App) -> bool {
        self.root_thread_view().is_some_and(|active| {
            active
                .read(cx)
                .thread
                .read(cx)
                .entries()
                .iter()
                .any(|entry| matches!(entry, AgentThreadEntry::UserMessage(_)))
        })
    }

    fn render_auth_required_state(
        &self,
        connection: &Rc<dyn AgentConnection>,
        description: Option<&Entity<Markdown>>,
        pending_auth_method: Option<&acp::AuthMethodId>,
        window: &mut Window,
        cx: &Context<Self>,
    ) -> impl IntoElement {
        let auth_methods = connection.auth_methods();

        let agent_display_name = self
            .agent_server_store
            .read(cx)
            .agent_display_name(&self.agent.agent_id())
            .unwrap_or_else(|| self.agent.agent_id().0);

        let show_fallback_description =
            auth_methods.len() > 1 && description.is_none() && pending_auth_method.is_none();

        let auth_buttons = || {
            h_flex().justify_end().flex_wrap().gap_1().children(
                connection
                    .auth_methods()
                    .iter()
                    .enumerate()
                    .rev()
                    .map(|(ix, method)| {
                        let (method_id, name) = (method.id().0.clone(), method.name().to_string());
                        let agent_telemetry_id = connection.telemetry_id();

                        Button::new(method_id.clone(), name)
                            .label_size(LabelSize::Small)
                            .map(|this| {
                                if ix == 0 {
                                    this.style(ButtonStyle::Tinted(TintColor::Accent))
                                } else {
                                    this.style(ButtonStyle::Outlined)
                                }
                            })
                            .when_some(method.description(), |this, description| {
                                this.tooltip(Tooltip::text(description.to_string()))
                            })
                            .on_click({
                                cx.listener(move |this, _, window, cx| {
                                    telemetry::event!(
                                        "Authenticate Agent Started",
                                        agent = agent_telemetry_id,
                                        method = method_id
                                    );

                                    this.authenticate(
                                        acp::AuthMethodId::new(method_id.clone()),
                                        window,
                                        cx,
                                    )
                                })
                            })
                    }),
            )
        };

        if pending_auth_method.is_some() {
            return Callout::new()
                .icon(IconName::Info)
                .title(format!("Authenticating to {}…", agent_display_name))
                .actions_slot(
                    Icon::new(IconName::ArrowCircle)
                        .size(IconSize::Small)
                        .color(Color::Muted)
                        .with_rotate_animation(2)
                        .into_any_element(),
                )
                .into_any_element();
        }

        Callout::new()
            .icon(IconName::Info)
            .title(format!("Authenticate to {}", agent_display_name))
            .when(auth_methods.len() == 1, |this| {
                this.actions_slot(auth_buttons())
            })
            .description_slot(
                v_flex()
                    .text_ui(cx)
                    .map(|this| {
                        if show_fallback_description {
                            this.child(
                                Label::new("Choose one of the following authentication options:")
                                    .size(LabelSize::Small)
                                    .color(Color::Muted),
                            )
                        } else {
                            this.children(description.map(|desc| {
                                self.render_markdown(
                                    desc.clone(),
                                    MarkdownStyle::themed(MarkdownFont::Agent, window, cx),
                                    cx,
                                )
                            }))
                        }
                    })
                    .when(auth_methods.len() > 1, |this| {
                        this.gap_1().child(auth_buttons())
                    }),
            )
            .into_any_element()
    }

    fn sync_request_elicitation_states(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(store) = self.request_elicitation_store() else {
            self.request_elicitation_form_states.clear();
            return;
        };

        let elicitations = store
            .read(cx)
            .elicitations()
            .iter()
            .map(|elicitation| {
                let is_pending = matches!(elicitation.status, ElicitationStatus::Pending { .. });
                let schema = match &elicitation.request.mode {
                    acp::ElicitationMode::Form(mode) => Some(mode.requested_schema.clone()),
                    _ => None,
                };
                (elicitation.id.clone(), is_pending, schema)
            })
            .collect::<Vec<_>>();

        let known_ids = elicitations
            .iter()
            .map(|(id, _, _)| id.clone())
            .collect::<HashSet<_>>();
        self.request_elicitation_form_states
            .retain(|id, _| known_ids.contains(id));

        for (id, is_pending, schema) in elicitations {
            if is_pending
                && let Some(schema) = schema
                && !self.request_elicitation_form_states.contains_key(&id)
            {
                self.request_elicitation_form_states
                    .insert(id, ElicitationFormState::new(&schema, window, cx));
            } else if !is_pending {
                self.request_elicitation_form_states.remove(&id);
            }
        }
    }

    fn render_request_elicitations(
        &self,
        connection: &Rc<dyn AgentConnection>,
        view: WeakEntity<Self>,
        cx: &App,
    ) -> Vec<AnyElement> {
        let Some(store) = connection.request_elicitations() else {
            return Vec::new();
        };

        let handlers = Self::request_elicitation_card_handlers(view);
        let agent_display_name = self
            .agent_server_store
            .read(cx)
            .agent_display_name(&self.agent.agent_id())
            .unwrap_or_else(|| self.agent.agent_id().0);

        store
            .read(cx)
            .elicitations()
            .iter()
            .enumerate()
            .filter(|(_, elicitation)| should_render_elicitation(elicitation))
            .map(|(ix, elicitation)| {
                ElicitationCard::new(
                    ix,
                    elicitation,
                    agent_display_name.clone(),
                    self.request_elicitation_form_states.get(&elicitation.id),
                    handlers.clone(),
                )
                .render(cx)
                .into_any_element()
            })
            .collect()
    }

    fn request_elicitation_card_handlers(view: WeakEntity<Self>) -> ElicitationCardHandlers {
        ElicitationCardHandlers::new(
            {
                let view = view.clone();
                move |elicitation_id, window, cx| {
                    view.update(cx, |this, cx| {
                        this.submit_request_elicitation(elicitation_id, window, cx);
                    })
                    .log_err();
                }
            },
            {
                let view = view.clone();
                move |elicitation_id, window, cx| {
                    view.update(cx, |this, cx| {
                        this.decline_request_elicitation(elicitation_id, window, cx);
                    })
                    .log_err();
                }
            },
            {
                let view = view.clone();
                move |elicitation_id, window, cx| {
                    view.update(cx, |this, cx| {
                        this.cancel_request_elicitation(elicitation_id, window, cx);
                    })
                    .log_err();
                }
            },
            {
                let view = view.clone();
                move |elicitation_id, _window, cx| {
                    view.update(cx, |this, cx| {
                        this.dismiss_request_url_elicitation(elicitation_id, cx);
                    })
                    .log_err();
                }
            },
            move |_elicitation_id, url, _window, cx| cx.open_url(&url),
            {
                let view = view.clone();
                move |elicitation_id, field_name, value, cx| {
                    view.update(cx, |this, cx| {
                        this.update_request_elicitation_form_state(
                            &elicitation_id,
                            |form| form.set_boolean(&field_name, value),
                            cx,
                        );
                    })
                    .log_err();
                }
            },
            {
                let view = view.clone();
                move |elicitation_id, field_name, value, cx| {
                    view.update(cx, |this, cx| {
                        this.update_request_elicitation_form_state(
                            &elicitation_id,
                            |form| form.set_single_select(&field_name, value),
                            cx,
                        );
                    })
                    .log_err();
                }
            },
            move |elicitation_id, field_name, value, selected, cx| {
                view.update(cx, |this, cx| {
                    this.update_request_elicitation_form_state(
                        &elicitation_id,
                        |form| form.set_multi_select(&field_name, value, selected),
                        cx,
                    );
                })
                .log_err();
            },
        )
    }

    fn update_request_elicitation_form_state(
        &mut self,
        elicitation_id: &ElicitationEntryId,
        update: impl FnOnce(&mut ElicitationFormState),
        cx: &mut Context<Self>,
    ) {
        if let Some(form) = self.request_elicitation_form_states.get_mut(elicitation_id) {
            update(form);
            self.notify_request_elicitation_renderers(cx);
        }
    }

    fn notify_request_elicitation_renderers(&self, cx: &mut Context<Self>) {
        if let Some(active_thread) = self.active_thread().cloned() {
            active_thread.update(cx, |_thread, cx| cx.notify());
        }
        cx.notify();
    }

    fn submit_request_elicitation(
        &mut self,
        elicitation_id: ElicitationEntryId,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(store) = self.request_elicitation_store() else {
            return;
        };

        let mode = store
            .read(cx)
            .elicitation(&elicitation_id)
            .map(|(_, elicitation)| elicitation.request.mode.clone());
        let Some(mode) = mode else {
            return;
        };

        match mode {
            acp::ElicitationMode::Form(mode) => {
                let Some(state) = self
                    .request_elicitation_form_states
                    .get_mut(&elicitation_id)
                else {
                    return;
                };
                let Some(submission) = state.begin_submission(cx) else {
                    return;
                };
                let schema = mode.requested_schema;
                let validation_task = cx.background_spawn(async move {
                    let result = submission.validate(&schema);
                    (submission, result)
                });
                self.notify_request_elicitation_renderers(cx);
                cx.spawn(async move |this, cx| {
                    let (submission, result) = validation_task.await;
                    this.update(cx, |this, cx| {
                        let is_current = this
                            .request_elicitation_form_states
                            .get_mut(&elicitation_id)
                            .is_some_and(|state| {
                                state.validation_matches_current_values(&submission, cx)
                            });
                        if !is_current {
                            this.notify_request_elicitation_renderers(cx);
                            return;
                        }
                        match result {
                            Ok(content) => {
                                this.respond_to_request_elicitation(
                                    elicitation_id,
                                    acp::CreateElicitationResponse::new(
                                        acp::ElicitationAction::Accept(
                                            acp::ElicitationAcceptAction::new().content(content),
                                        ),
                                    ),
                                    cx,
                                );
                            }
                            Err(errors) => {
                                this.update_request_elicitation_form_state(
                                    &elicitation_id,
                                    |state| state.set_errors(errors),
                                    cx,
                                );
                            }
                        }
                    })
                    .log_err();
                })
                .detach();
            }
            acp::ElicitationMode::Url(_) => {
                self.respond_to_request_elicitation(
                    elicitation_id,
                    acp::CreateElicitationResponse::new(acp::ElicitationAction::Accept(
                        acp::ElicitationAcceptAction::new(),
                    )),
                    cx,
                );
            }
            _ => {}
        }
    }

    fn decline_request_elicitation(
        &mut self,
        elicitation_id: ElicitationEntryId,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.respond_to_request_elicitation(
            elicitation_id,
            acp::CreateElicitationResponse::new(acp::ElicitationAction::Decline),
            cx,
        );
    }

    fn cancel_request_elicitation(
        &mut self,
        elicitation_id: ElicitationEntryId,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.respond_to_request_elicitation(
            elicitation_id,
            acp::CreateElicitationResponse::new(acp::ElicitationAction::Cancel),
            cx,
        );
    }

    fn dismiss_request_url_elicitation(
        &mut self,
        elicitation_id: ElicitationEntryId,
        cx: &mut Context<Self>,
    ) {
        self.request_elicitation_form_states.remove(&elicitation_id);
        if let Some(store) = self.request_elicitation_store() {
            store.update(cx, |store, cx| {
                store.cancel_elicitation(&elicitation_id, cx);
            });
        }
        self.notify_request_elicitation_renderers(cx);
    }

    fn respond_to_request_elicitation(
        &mut self,
        elicitation_id: ElicitationEntryId,
        response: acp::CreateElicitationResponse,
        cx: &mut Context<Self>,
    ) {
        self.request_elicitation_form_states.remove(&elicitation_id);
        if let Some(store) = self.request_elicitation_store() {
            store.update(cx, |store, cx| {
                store.respond_to_elicitation(&elicitation_id, response, cx);
            });
        }
        cx.notify();
    }

    fn cancel_request_elicitations(&mut self, cx: &mut App) {
        self.request_elicitation_form_states.clear();
        if let Some(store) = self.request_elicitation_store() {
            store.update(cx, |store, cx| store.clear(cx));
        }
    }

    fn clear_resolved_request_elicitations(&mut self, cx: &mut App) {
        if let Some(connection) = self.request_elicitation_connection() {
            self.clear_resolved_request_elicitations_for_connection(&connection, cx);
        }
    }

    fn clear_resolved_request_elicitations_for_connection(
        &mut self,
        connection: &Rc<dyn AgentConnection>,
        cx: &mut App,
    ) {
        let Some(store) = connection.request_elicitations() else {
            return;
        };
        let cleared_ids = store.update(cx, |store, cx| store.clear_resolved(cx));
        for id in cleared_ids {
            self.request_elicitation_form_states.remove(&id);
        }
    }

    fn emit_load_error_telemetry(&self, error: &LoadError) {
        let error_kind = match error {
            LoadError::Unsupported { .. } => "unsupported",
            LoadError::FailedToInstall(_) => "failed_to_install",
            LoadError::Exited { .. } => "exited",
            LoadError::SessionGone => "session_gone",
            LoadError::Other(_) => "other",
        };

        let agent_name = self.agent.agent_id();

        telemetry::event!(
            "Agent Panel Error Shown",
            agent = agent_name,
            kind = error_kind,
            message = error.to_string(),
        );
    }

    /// Whether this conversation's first Omega route is still unrecorded.
    ///
    /// True until the router freezes a decision or a physical session exists.
    /// This is the fact the executor dropdown reads to know a draft can still
    /// be re-homed; it is state, never composer copy.
    fn omega_route_not_yet_recorded(&self) -> bool {
        self.omega_route_summary.is_none()
            && self.deferred_omega_session.is_none()
            && self.root_session_id.is_none()
    }

    /// The name the composer executor dropdown shows for this conversation.
    ///
    /// `OMEGA-DELTA-0184`. Read from this view's own connection key — the
    /// conversation's owner — never from a separate selection store, so the
    /// face cannot disagree with the conversation it sits in.
    fn composer_executor_label(&self, cx: &App) -> SharedString {
        match self.agent_key() {
            Agent::NativeAgent => "Omega Agent".into(),
            Agent::Custom { id } => self
                .agent_server_store
                .read(cx)
                .agent_display_name(id)
                .unwrap_or_else(|| {
                    crate::omega_composer_executor_menu::named_direct_agent_label(id.as_ref())
                        .map_or_else(|| id.0.clone(), SharedString::from)
                }),
            #[cfg(any(test, feature = "test-support"))]
            Agent::Stub => "Stub".into(),
        }
    }

    /// The composer, while the executor is connecting.
    ///
    /// `OMEGA-DELTA-0122`, amended by `OMEGA-DELTA-0170`. Same box, same
    /// place, same type — see [`Self::loading_composer`] for why it exists and
    /// [`Self::hand_loading_draft_over`] for what becomes of a draft left in
    /// it.
    ///
    /// # Enter always accepts
    ///
    /// The owner, on the refusal this replaces: "never block user from
    /// hitting enter, if not connected just show a loading thing in the
    /// chat." A `Chat` here does what a `Chat` does in a connected thread: the
    /// message leaves the composer. It becomes a pending turn drawn above the
    /// box — [`Self::render_pending_connect_messages`] — and dispatches, in
    /// order, the moment the session exists. Send is the same press with a
    /// mouse, so it is live too; a dead button beside a live Enter would make
    /// the two claims about one state.
    ///
    /// # The indicator is in the bar, bottom left — and says nothing more
    ///
    /// The owner: "move the loading indicator to inside the input bar like
    /// bottom left". Which is also where it stops: bottom-left is empty once
    /// loaded and stays empty — `render_zero_base_executor_bar` puts the
    /// provider's controls on the right. `OMEGA-DELTA-0175` adds the Vim mode
    /// readout to the left in both states.
    ///
    /// `OMEGA-DELTA-0189`, omega#160: the indicator is "Connecting…" while
    /// something is actually connecting, and absent otherwise. It used to
    /// narrate router readiness and frozen route summaries beside a second
    /// Automatic/Omega routing dropdown. The owner rejected that as unclear
    /// exposition: the Omega Agent mode routes automatically — that is the
    /// whole point — so the control, its state, and every sentence explaining
    /// the router belong nowhere on the surface. Routing behavior itself
    /// (`OMEGA-DELTA-0179`) is unchanged: the first request routes with
    /// `ExecutorOverride::Auto` and the durable receipt records the decision.
    /// A person who wants one exact executor picks it in the executor
    /// dropdown (`OMEGA-DELTA-0184`).
    ///
    /// # The same controls as a connected thread
    ///
    /// `OMEGA-DELTA-0204`. The owner, on a screenshot of a new thread beside an
    /// old one: "new chat threads dont show the dropdowns and voice button that
    /// existing threads do ... it should be the exact fucking same."
    ///
    /// It was not the same. This bar carried the executor dropdown and Send;
    /// `render_zero_base_executor_bar` carries the executor dropdown, the
    /// Luna/Flash/Pro tier dropdown, the microphone and Send, and puts them on
    /// the right. The field had no expand control either. So the composer a
    /// person meets first — on the thread whose first message is the one most
    /// likely to be long, because it is the one that states the task — was the
    /// most reduced composer in the app, and every control returned a moment
    /// later once a session existed. Nothing about an unconnected session made
    /// any of them unavailable: voice is a Sarah session on the workspace, the
    /// tier is a settings default that a session reads when it is created, and
    /// expanding a text field is local to the field.
    ///
    /// The two bars are now the same set in the same order in the same places.
    /// What genuinely cannot exist before a session — the executor disclosure
    /// line, the routed model, the turn's phase dot, the Exo inspector — stays
    /// absent rather than being invented, and the connecting indicator takes
    /// the space they will occupy.
    fn render_loading_composer(
        &mut self,
        router_ready: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let editor = self.loading_composer(window, cx);
        let editor_text = editor.read(cx).text(cx);
        let layout = composer_layout(self.loading_composer_expanded, &editor_text);
        let compact = layout == ComposerLayout::Compact;
        let manually_expanded = layout == ComposerLayout::ManuallyExpanded;
        // omega#217. Built once, before the layout branches, because only one
        // of them draws and an element is consumed when it is drawn.
        let accessible_editor = {
            let composer_focus = editor.focus_handle(cx);
            let set_value_editor = editor.downgrade();
            accessible_composer_input(
                "omega-workbench-pre-session-composer-input",
                editor_text.clone(),
                &composer_focus,
                move |value, window, cx| {
                    let Some(editor) = set_value_editor.upgrade() else {
                        return;
                    };
                    editor.update(cx, |editor, cx| {
                        editor.set_text(value, window, cx);
                    });
                },
                EditorElement::new(&editor, crate::message_editor::composer_editor_style(cx))
                    .into_any_element(),
            )
        };
        let status: Option<SharedString> = if router_ready {
            // Logically ready and idle: nothing is connecting, so nothing
            // pulses. The deferred-session window between the first send and
            // its physical session is a real connection in progress.
            self.deferred_omega_session
                .is_some()
                .then(|| "Connecting…".into())
        } else {
            Some(
                self.loading_status
                    .clone()
                    .unwrap_or_else(|| "Connecting…".into()),
            )
        };
        let max_content_width = composer_max_width(AgentSettings::get_global(cx).max_content_width);
        let pending_turns = self.render_pending_connect_messages(false, cx);

        // The executor dropdown is live from the very first paint of a new
        // conversation, while its session is still connecting.
        // `OMEGA-DELTA-0184`, omega#165.
        let conversation_is_bound =
            !self.pending_connect_messages.is_empty() || !self.omega_route_not_yet_recorded();
        let model_picker = self.pre_session_model_picker(cx);
        let executor_menu = crate::omega_composer_executor_menu::render_composer_executor_menu(
            self.workspace.clone(),
            self.composer_executor_label(cx),
            conversation_is_bound,
            editor.focus_handle(cx),
            model_picker,
            window,
            cx,
        );

        let colors = cx.theme().colors();
        let pill_background = colors.text.opacity(0.03);
        let pill_border = colors.text.opacity(0.08);
        let primary_background = colors.text;
        let primary_foreground = colors.editor_background;
        let opaque_window =
            cx.theme().window_background_appearance() == gpui::WindowBackgroundAppearance::Opaque;
        let action_controls = h_flex()
            .min_w_0()
            .when(!compact, |this| this.flex_wrap())
            .gap_1()
            .children(executor_menu)
            .when(omega_zero_base::is_primary_interface(), |this| {
                this.child(crate::composer_voice::render_composer_voice_controls(
                    self.workspace.entity_id(),
                    cx,
                ))
            })
            .child(
                div()
                    .id("add-context-connecting")
                    .size(px(28.))
                    .flex()
                    .items_center()
                    .justify_center()
                    .child(
                        Icon::new(IconName::Paperclip)
                            .size(IconSize::Small)
                            .color(Color::Muted),
                    ),
            )
            .child(
                div()
                    .size(px(28.))
                    .rounded_full()
                    .overflow_hidden()
                    .bg(primary_background)
                    .hover(|style| style.opacity(0.85))
                    .child(
                        IconButton::new("send-message", IconName::ArrowUp)
                            .aria_label("Send Message")
                            .icon_size(IconSize::Small)
                            .icon_color(Color::Custom(primary_foreground))
                            .style(ButtonStyle::Transparent)
                            .size(ButtonSize::Medium)
                            .width(rems_from_px(28.))
                            .tooltip(Tooltip::text("Send"))
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.submit_before_session(window, cx);
                            })),
                    ),
            );

        v_flex()
            .key_context("AcpThread")
            .flex_1()
            .size_full()
            .justify_end()
            .on_action(cx.listener(|this, _: &Chat, window, cx| {
                this.submit_before_session(window, cx);
            }))
            .on_action(cx.listener(|this, _: &ExpandMessageEditor, _window, cx| {
                this.loading_composer_expanded = !this.loading_composer_expanded;
                cx.notify();
            }))
            .children(pending_turns)
            .child(
                h_flex().w_full().justify_center().child(
                    div().w_full().max_w(max_content_width).px_6().pb_6().child(
                        v_flex()
                            .debug_selector(|| "omega.workbench.composer".into())
                            .w_full()
                            .min_w_0()
                            .overflow_hidden()
                            .rounded(px(COMPOSER_RADIUS))
                            .bg(pill_background)
                            .border_1()
                            .border_color(pill_border)
                            .when(opaque_window, |this| {
                                this.shadow(vec![
                                    gpui::BoxShadow::new(
                                        px(0.),
                                        px(4.),
                                        gpui::black().opacity(0.12),
                                    )
                                    .blur_radius(px(8.)),
                                ])
                            })
                            .map(|this| {
                                if compact {
                                    this.h(px(COMPOSER_COMPACT_HEIGHT)).child(
                                        h_flex()
                                            .size_full()
                                            .min_w_0()
                                            .child(
                                                div()
                                                    .flex_1()
                                                    .min_w_0()
                                                    .h_full()
                                                    .overflow_hidden()
                                                    .pl(px(COMPOSER_TEXT_INSET))
                                                    .pr_2()
                                                    .py_3()
                                                    .child(accessible_editor),
                                            )
                                            .child(
                                                h_flex()
                                                    .flex_none()
                                                    .min_w_0()
                                                    .gap_1()
                                                    .pr_2()
                                                    .when(omega_zero_base::is_active(), |this| {
                                                        this.child(self.vim_mode_indicator.clone())
                                                    })
                                                    .child(action_controls),
                                            ),
                                    )
                                } else {
                                    this.min_h(px(COMPOSER_EXPANDED_MIN_HEIGHT))
                                        .max_h(px(COMPOSER_EXPANDED_MAX_HEIGHT))
                                        .when(manually_expanded, |this| {
                                            this.h(px(COMPOSER_EXPANDED_MAX_HEIGHT))
                                        })
                                        .child(
                                            v_flex()
                                                .relative()
                                                .w_full()
                                                .min_w_0()
                                                .min_h(px(COMPOSER_EXPANDED_MIN_HEIGHT
                                                    - COMPOSER_ACTIONS_HEIGHT))
                                                .flex_1()
                                                .overflow_hidden()
                                                .px(px(COMPOSER_TEXT_INSET))
                                                .pt_4()
                                                .pb_1()
                                                .child(accessible_editor),
                                        )
                                        .child(
                                            h_flex()
                                                .w_full()
                                                .min_w_0()
                                                .h(px(COMPOSER_ACTIONS_HEIGHT))
                                                .flex_none()
                                                .flex_wrap()
                                                .gap_1()
                                                .justify_between()
                                                .px_3()
                                                .pt_1()
                                                .pb(px(10.))
                                                .child(
                                                    h_flex()
                                                        .min_w_0()
                                                        .gap_1()
                                                        .when(
                                                            omega_zero_base::is_active(),
                                                            |this| {
                                                                this.child(
                                                                    self.vim_mode_indicator.clone(),
                                                                )
                                                            },
                                                        )
                                                        .children(status.map(|status| {
                                                            Label::new(status)
                                                                .size(LabelSize::Small)
                                                                .color(Color::Muted)
                                                                .with_animation(
                                                                    "loading-agent-label",
                                                                    Animation::new(
                                                                        Duration::from_secs(2),
                                                                    )
                                                                    .repeat()
                                                                    .with_easing(pulsating_between(
                                                                        0.3, 0.7,
                                                                    )),
                                                                    |label, delta| {
                                                                        label.alpha(delta)
                                                                    },
                                                                )
                                                        })),
                                                )
                                                .child(action_controls),
                                        )
                                }
                            }),
                    ),
                ),
            )
            .into_any()
    }

    /// The Luna / Flash / Pro dropdown, before a session exists.
    ///
    /// `OMEGA-DELTA-0204`. Nothing here names a model as *serving* a turn,
    /// because no turn has run.
    ///
    /// `OMEGA-DELTA-0207`. It names the model the next turn will start on, and
    /// it gets that from the registry rather than from the standing choice.
    /// The standing choice claimed to be "a truthful statement about what the
    /// next turn will start on" and was not one: it is a process-wide static
    /// that begins every launch at `Luna` and is never seeded from settings, so
    /// this composer said **Luna** on a thread whose send went to
    /// `openagents/gemini-3.6-flash`.
    fn pre_session_model_picker(
        &self,
        cx: &mut Context<Self>,
    ) -> crate::omega_composer_executor_menu::ComposerModelPicker {
        use crate::omega_model_tier::selected;
        use crate::omega_routed_model::face_for_next_turn;

        if matches!(self.agent_key(), Agent::NativeAgent) {
            let fs = self.project.read(cx).fs().clone();
            crate::omega_composer_executor_menu::ComposerModelPicker::omega(
                face_for_next_turn(None, selected(), cx),
                true,
                Rc::new(move |model_id, _window, cx| {
                    if let Some(tier) = crate::omega_model_tier::ModelTier::ALL
                        .iter()
                        .copied()
                        .find(|tier| tier.agent_model_id() == model_id.as_str())
                    {
                        crate::omega_model_tier::select(tier);
                    }
                    crate::omega_model_tier::select_model_before_session(
                        model_id.as_str(),
                        fs.clone(),
                        cx,
                    );
                }),
                cx,
            )
        } else {
            crate::omega_composer_executor_menu::ComposerModelPicker {
                label: "Loading models…".into(),
                current_model: None,
                models: Vec::new(),
                traits: Vec::new(),
                enabled: false,
                empty_message: "This agent's models appear when its session is ready.".into(),
                on_select: Rc::new(|_, _, _| {}),
            }
        }
    }

    /// The turns a person sent before the executor finished connecting.
    ///
    /// `OMEGA-DELTA-0170`. The owner: "if not connected just show a loading
    /// thing in the chat." Each pending message is drawn where the transcript
    /// will be, wearing the spinner the rest of the app uses for work in
    /// progress, and naming the executor it will go to — the visibility that
    /// makes auto-dispatch on connect honest for a person who reached this
    /// state by switching executor.
    ///
    /// With `connection_failed`, the same turns say plainly that they were not
    /// sent, keep the text on screen, and offer the one action that moves them
    /// forward: retrying the connection, which re-enters `Loading` with the
    /// pending list intact.
    fn render_pending_connect_messages(
        &self,
        connection_failed: bool,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        if self.pending_connect_messages.is_empty() {
            return None;
        }
        let executor_name = self.connecting_executor_name(cx);
        let max_content_width = AgentSettings::get_global(cx).max_content_width;

        Some(
            h_flex()
                .pb_2()
                .px_2()
                .justify_center()
                .child(
                    v_flex()
                        .when_some(max_content_width, |this, max_w| this.flex_basis(max_w))
                        .when(max_content_width.is_none(), |this| this.w_full())
                        .min_w_0()
                        .gap_2()
                        .children(self.pending_connect_messages.iter().map(|message| {
                            let text = SharedString::from(message.text.trim().to_string());
                            v_flex()
                                .w_full()
                                .min_w_0()
                                .p_2()
                                .gap_1()
                                .bg(cx.theme().colors().editor_background)
                                .border_1()
                                .border_color(cx.theme().colors().border)
                                .rounded_lg()
                                .child(div().w_full().min_w_0().child(Label::new(text)))
                                .child(if connection_failed {
                                    h_flex()
                                        .gap_1p5()
                                        .min_w_0()
                                        .child(
                                            Icon::new(IconName::XCircleFilled)
                                                .size(IconSize::Small)
                                                .color(Color::Error),
                                        )
                                        .child(
                                            Label::new(format!(
                                                "Not sent — {executor_name} failed to connect. \
                                                 Your message is kept."
                                            ))
                                            .size(LabelSize::Small)
                                            .color(Color::Error),
                                        )
                                } else {
                                    h_flex()
                                        .gap_1p5()
                                        .min_w_0()
                                        .child(
                                            Icon::new(IconName::TodoProgress)
                                                .size(IconSize::Small)
                                                .color(Color::Accent)
                                                .with_rotate_animation(2),
                                        )
                                        .child(
                                            Label::new(format!(
                                                "Sending to {executor_name} once it connects…"
                                            ))
                                            .size(LabelSize::Small)
                                            .color(Color::Muted),
                                        )
                                })
                        }))
                        .when(connection_failed, |this| {
                            this.child(
                                h_flex().justify_end().child(
                                    Button::new("retry-pending-connect", "Retry Connection")
                                        .style(ButtonStyle::Filled)
                                        .on_click(cx.listener(|this, _, window, cx| {
                                            this.reset(window, cx);
                                        })),
                                ),
                            )
                        }),
                )
                .into_any(),
        )
    }

    fn render_load_error(
        &self,
        e: &LoadError,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let (title, message, action_slot): (_, SharedString, Vec<AnyElement>) = match e {
            LoadError::Unsupported {
                command: path,
                current_version,
                minimum_version,
            } => {
                return self.render_unsupported(path, current_version, minimum_version, window, cx);
            }
            LoadError::FailedToInstall(msg) => (
                "Failed to Install",
                msg.into(),
                vec![self.create_copy_button(msg.to_string()).into_any_element()],
            ),
            LoadError::Exited { status, stderr } => {
                let mut message = format!("Server exited with status {status}");
                if let Some(stderr) = stderr {
                    message.push_str("\n");
                    message.push_str(stderr);
                };
                // omega#169. Exit 127 is the shell reporting that the
                // configured command does not exist, so the failure is a
                // setup problem and the card owes the reader the setup
                // action, not just an honest exit status.
                let setup_action = (status.code() == Some(127)).then(|| {
                    Button::new("load-error-add-more-agents", "Add More Agents")
                        .on_click(|_, window, cx| {
                            window.dispatch_action(Box::new(omega_actions::AcpRegistry), cx)
                        })
                        .into_any_element()
                });
                let copy_action = stderr
                    .is_some()
                    .then(|| self.create_copy_button(message.clone()).into_any_element());
                (
                    "Failed to Launch",
                    message.into(),
                    setup_action.into_iter().chain(copy_action).collect(),
                )
            }
            // Nothing failed to launch. The agent started fine and does not
            // have this conversation any more, which is ordinary once a thread
            // outlives the agent's own session store. Say that, and give the
            // reader the one action that moves them forward, instead of a
            // launch failure carrying a session id they cannot use.
            LoadError::SessionGone => {
                return Callout::new()
                    .severity(Severity::Warning)
                    .icon(IconName::Info)
                    .title("Conversation No Longer Available")
                    .description(
                        "The agent no longer has this conversation. Its history is still here to \
                         read, and a new thread will pick up where you left off.",
                    )
                    .actions_slot(
                        Button::new("session-gone-new-thread", "New Thread")
                            .on_click(|_, window, cx| {
                                window.dispatch_action(NewThread.boxed_clone(), cx)
                            })
                            .into_any_element(),
                    )
                    .into_any_element();
            }
            LoadError::Other(msg) => (
                "Failed to Launch",
                msg.into(),
                vec![self.create_copy_button(msg.to_string()).into_any_element()],
            ),
        };

        Callout::new()
            .severity(Severity::Error)
            .icon(IconName::XCircleFilled)
            .title(title)
            .description(message)
            .actions_slot(
                h_flex()
                    .gap_1()
                    .children(self.render_run_on_omegas_own_loop(cx))
                    .children(action_slot),
            )
            .into_any_element()
    }

    /// The first-party path out of an ACP adapter Omega could not reach.
    ///
    /// `OMEGA-DELTA-0114`, omega#106. `Agent::NativeAgent` *is* the router, so
    /// there is no picker entry that reaches the native loop: while a detected
    /// agent's adapter stays unreachable, this callout is the whole panel and
    /// the reader has nowhere to go. That was the residual cost
    /// `OMEGA-DELTA-0095` recorded when it kept the hard failure, and this is
    /// the payment.
    ///
    /// It is a **button and not a fallback**, and the difference is the entire
    /// argument of that delta. Omega deciding to run somewhere else produces a
    /// thread that reports one executor and runs another; a person reading what
    /// failed and choosing the native loop produces a thread that runs where
    /// they asked, and the disclosure line then says `native_loop` truthfully.
    ///
    /// Offered only when an attach actually failed, so it cannot appear beside
    /// an unrelated error and read as a general "give up on Codex" control.
    /// `omega_agent_attach::unreachable_adapter` is typed state rather than a
    /// read of the message, because the cause reaches this file as a
    /// `LoadError::Other(String)` and recovering it from prose would be a
    /// parser over a sentence written for a human.
    fn render_run_on_omegas_own_loop(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        if !matches!(self.connection_key, Agent::NativeAgent) {
            return None;
        }
        let adapter = crate::omega_agent_attach::unreachable_adapter()?;
        let view = cx.entity();
        Some(
            Button::new("omega-run-on-own-loop", "Run on Omega's Own Loop")
                .tooltip(Tooltip::text(format!(
                    "Run this thread on Omega's own agent loop instead of {}. \
                     Your {} is fine; {} is a separate npm download Omega could \
                     not resolve. Restarting Omega goes back to {}.",
                    adapter.adapter_id, adapter.agent_name, adapter.adapter_id, adapter.agent_name,
                )))
                .on_click(move |_, window, cx| {
                    crate::omega_agent_attach::run_on_omegas_own_loop();
                    view.update(cx, |this, cx| this.reset(window, cx));
                })
                .into_any_element(),
        )
    }

    fn render_unsupported(
        &self,
        path: &SharedString,
        version: &SharedString,
        minimum_version: &SharedString,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let (heading_label, description_label) = (
            format!("Upgrade {} to work with Omega", self.agent.agent_id()),
            if version.is_empty() {
                format!(
                    "Currently using {}, which does not report a valid --version",
                    path,
                )
            } else {
                format!(
                    "Currently using {}, which is only version {} (need at least {minimum_version})",
                    path, version
                )
            },
        );

        v_flex()
            .w_full()
            .p_3p5()
            .gap_2p5()
            .border_t_1()
            .border_color(cx.theme().colors().border)
            .bg(linear_gradient(
                180.,
                linear_color_stop(cx.theme().colors().editor_background.opacity(0.4), 4.),
                linear_color_stop(cx.theme().status().info_background.opacity(0.), 0.),
            ))
            .child(
                v_flex().gap_0p5().child(Label::new(heading_label)).child(
                    Label::new(description_label)
                        .size(LabelSize::Small)
                        .color(Color::Muted),
                ),
            )
            .into_any_element()
    }

    pub(crate) fn as_native_connection(
        &self,
        cx: &App,
    ) -> Option<Rc<agent::NativeAgentConnection>> {
        self.root_thread(cx)?
            .read(cx)
            .connection()
            .clone()
            .downcast()
    }

    pub fn as_native_thread(&self, cx: &App) -> Option<Entity<agent::Thread>> {
        self.as_native_connection(cx)?
            .thread(self.root_session_id.as_ref()?, cx)
    }

    fn render_markdown(
        &self,
        markdown: Entity<Markdown>,
        style: MarkdownStyle,
        cx: &App,
    ) -> MarkdownElement {
        render_agent_markdown(
            markdown,
            style,
            &self.workspace,
            &self.code_span_resolver,
            None,
            cx,
        )
    }

    fn notify_with_sound(
        &mut self,
        caption: impl Into<SharedString>,
        icon: IconName,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        #[cfg(feature = "audio")]
        self.play_notification_sound(window, cx);
        self.show_notification(caption, icon, window, cx);
    }

    fn is_visible(&self, multi_workspace: &Entity<MultiWorkspace>, cx: &Context<Self>) -> bool {
        let Some(workspace) = self.workspace.upgrade() else {
            return false;
        };

        let multi_workspace = multi_workspace.read(cx);
        multi_workspace.sidebar_open() && multi_workspace.is_threads_list_view_active(cx)
            || multi_workspace.workspace() == &workspace
                && self.is_visible_in_agent_panel(&workspace, cx)
    }

    fn is_visible_in_agent_panel(&self, workspace: &Entity<Workspace>, cx: &Context<Self>) -> bool {
        AgentPanel::is_visible(workspace, cx)
            && workspace
                .read(cx)
                .panel::<AgentPanel>(cx)
                .is_some_and(|panel| {
                    panel
                        .read(cx)
                        .visible_conversation_view()
                        .map(|conversation_view| conversation_view.entity_id())
                        == Some(cx.entity_id())
                })
    }

    fn agent_status_visible(&self, window: &Window, cx: &Context<Self>) -> bool {
        if !window.is_window_active() {
            return false;
        }

        if let Some(multi_workspace) = window.root::<MultiWorkspace>().flatten() {
            self.is_visible(&multi_workspace, cx)
        } else {
            self.workspace
                .upgrade()
                .is_some_and(|workspace| self.is_visible_in_agent_panel(&workspace, cx))
        }
    }

    #[cfg(feature = "audio")]
    fn play_notification_sound(&self, window: &Window, cx: &mut Context<Self>) {
        let visible = window.is_window_active()
            && if let Some(mw) = window.root::<MultiWorkspace>().flatten() {
                self.is_visible(&mw, cx)
            } else {
                self.workspace
                    .upgrade()
                    .is_some_and(|workspace| self.is_visible_in_agent_panel(&workspace, cx))
            };
        let settings = AgentSettings::get_global(cx);
        if settings.play_sound_when_agent_done.should_play(visible) {
            Audio::play_sound(Sound::AgentDone, cx);
        }
    }

    fn show_notification(
        &mut self,
        caption: impl Into<SharedString>,
        icon: IconName,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.notifications.is_empty() {
            return;
        }

        let settings = AgentSettings::get_global(cx);

        let should_notify = !self.agent_status_visible(window, cx);

        if !should_notify {
            return;
        }

        let Some(root_thread) = self.root_thread_view() else {
            return;
        };
        let root_thread = root_thread.read(cx).thread.read(cx);
        let root_thread_id = self.thread_id;
        let root_work_dirs = root_thread.work_dirs().cloned();
        let root_title = root_thread.title_or_first_user_message(cx);

        let title = root_title
            .clone()
            .unwrap_or_else(|| self.agent.agent_id().0);

        match settings.notify_when_agent_waiting {
            NotifyWhenAgentWaiting::PrimaryScreen => {
                window.request_attention();
                if let Some(primary) = cx.primary_display() {
                    self.pop_up(
                        icon,
                        caption.into(),
                        title,
                        root_thread_id,
                        root_work_dirs,
                        root_title,
                        window,
                        primary,
                        cx,
                    );
                }
            }
            NotifyWhenAgentWaiting::AllScreens => {
                window.request_attention();
                let caption = caption.into();
                for screen in cx.displays() {
                    self.pop_up(
                        icon,
                        caption.clone(),
                        title.clone(),
                        root_thread_id,
                        root_work_dirs.clone(),
                        root_title.clone(),
                        window,
                        screen,
                        cx,
                    );
                }
            }
            NotifyWhenAgentWaiting::Never => {
                // Don't show anything
            }
        }
    }

    fn pop_up(
        &mut self,
        icon: IconName,
        caption: SharedString,
        title: SharedString,
        root_thread_id: ThreadId,
        root_work_dirs: Option<PathList>,
        root_title: Option<SharedString>,
        window: &mut Window,
        screen: Rc<dyn PlatformDisplay>,
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

        if let Some(screen_window) = cx
            .open_window(options, |_window, cx| {
                cx.new(|_cx| {
                    AgentNotification::new(title.clone(), Some(caption.clone()), icon, project_name)
                })
            })
            .log_err()
            && let Some(pop_up) = screen_window.entity(cx).log_err()
        {
            self.notification_subscriptions
                .entry(screen_window)
                .or_insert_with(Vec::new)
                .push(cx.subscribe_in(&pop_up, window, {
                    move |this, _, event, window, cx| match event {
                        AgentNotificationEvent::Accepted => {
                            let Some(handle) = window.window_handle().downcast::<MultiWorkspace>()
                            else {
                                log::error!("root view should be a MultiWorkspace");
                                return;
                            };
                            cx.activate(true);

                            let workspace_handle = this.workspace.clone();
                            let agent = this.connection_key.clone();
                            let root_work_dirs = root_work_dirs.clone();
                            let root_title = root_title.clone();

                            cx.defer(move |cx| {
                                handle
                                    .update(cx, |multi_workspace, window, cx| {
                                        window.activate_window();
                                        if let Some(workspace) = workspace_handle.upgrade() {
                                            multi_workspace.activate(
                                                workspace.clone(),
                                                None,
                                                window,
                                                cx,
                                            );
                                            workspace.update(cx, |workspace, cx| {
                                                workspace.reveal_panel::<AgentPanel>(window, cx);
                                                if let Some(panel) =
                                                    workspace.panel::<AgentPanel>(cx)
                                                {
                                                    panel.update(cx, |panel, cx| {
                                                        panel.load_agent_thread(
                                                            agent.clone(),
                                                            root_thread_id,
                                                            root_work_dirs.clone(),
                                                            root_title.clone(),
                                                            true,
                                                            AgentThreadSource::AgentPanel,
                                                            window,
                                                            cx,
                                                        );
                                                    });
                                                }
                                                workspace.focus_panel::<AgentPanel>(window, cx);
                                            });
                                        }
                                    })
                                    .log_err();
                            });

                            this.dismiss_notifications(cx);
                        }
                        AgentNotificationEvent::Dismissed => {
                            this.dismiss_notifications(cx);
                        }
                    }
                }));

            self.notifications.push(screen_window);

            let dismiss_if_visible = {
                let pop_up_weak = pop_up.downgrade();
                move |this: &ConversationView,
                      window: &mut Window,
                      cx: &mut Context<ConversationView>| {
                    if this.agent_status_visible(window, cx)
                        && let Some(pop_up) = pop_up_weak.upgrade()
                    {
                        pop_up.update(cx, |notification, cx| {
                            notification.dismiss(cx);
                        });
                    }
                }
            };

            let subscriptions = self
                .notification_subscriptions
                .entry(screen_window)
                .or_insert_with(Vec::new);

            subscriptions.push({
                let dismiss_if_visible = dismiss_if_visible.clone();
                cx.observe_window_activation(window, move |this, window, cx| {
                    dismiss_if_visible(this, window, cx);
                })
            });

            if let Some(multi_workspace) = window.root::<MultiWorkspace>().flatten() {
                let dismiss_if_visible = dismiss_if_visible.clone();
                subscriptions.push(cx.observe_in(
                    &multi_workspace,
                    window,
                    move |this, _, window, cx| {
                        dismiss_if_visible(this, window, cx);
                    },
                ));
            }

            if let Some(panel) = self
                .workspace
                .upgrade()
                .and_then(|workspace| workspace.read(cx).panel::<AgentPanel>(cx))
            {
                subscriptions.push(cx.subscribe_in(
                    &panel,
                    window,
                    move |this, _, event: &AgentPanelEvent, window, cx| match event {
                        AgentPanelEvent::ActiveViewChanged | AgentPanelEvent::ActiveViewFocused => {
                            dismiss_if_visible(this, window, cx);
                        }
                        AgentPanelEvent::EntryChanged
                        | AgentPanelEvent::TerminalCloseRequested { .. }
                        | AgentPanelEvent::ThreadInteracted { .. } => {}
                    },
                ));
            }
        }
    }

    pub(crate) fn dismiss_notifications(&mut self, cx: &mut Context<Self>) -> bool {
        let had_notifications = !self.notifications.is_empty();
        for window in self.notifications.drain(..) {
            window
                .update(cx, |_, window, _| {
                    window.remove_window();
                })
                .ok();

            self.notification_subscriptions.remove(&window);
        }
        had_notifications
    }

    fn agent_ui_font_size_changed(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        if let Some(entry_view_state) = self
            .active_thread()
            .map(|active| active.read(cx).entry_view_state.clone())
        {
            entry_view_state.update(cx, |entry_view_state, cx| {
                entry_view_state.agent_ui_font_size_changed(cx);
            });
        }
    }

    fn invalidate_mermaid_caches(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        let current_theme_id = cx.theme().id.clone();
        if self.last_theme_id.as_ref() == Some(&current_theme_id) {
            return;
        }
        self.last_theme_id = Some(current_theme_id);

        if let Some(connected) = self.as_connected() {
            let threads: Vec<_> = connected
                .conversation
                .read(cx)
                .threads
                .values()
                .cloned()
                .collect();
            for thread in threads {
                thread.update(cx, |thread, cx| {
                    thread.invalidate_mermaid_caches(cx);
                });
            }
        }
    }

    pub(crate) fn insert_dragged_files(
        &mut self,
        paths: Vec<project::ProjectPath>,
        added_worktrees: Vec<Entity<project::Worktree>>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(active_thread) = self.active_thread() {
            active_thread.update(cx, |thread, cx| {
                thread.message_editor.update(cx, |editor, cx| {
                    editor.insert_dragged_files(paths, added_worktrees, window, cx);
                    editor.focus_handle(cx).focus(window, cx);
                })
            });
        } else {
            self.pending_dragged_files.push((paths, added_worktrees));
        }
    }

    /// Inserts the selected text into the message editor or the message being
    /// edited, if any.
    pub(crate) fn insert_selection(
        &self,
        selection: AgentContextSelection,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(active_thread) = self.active_thread() {
            active_thread.update(cx, |thread, cx| {
                thread.active_editor(cx).update(cx, |editor, cx| {
                    editor.insert_selections(selection, window, cx);
                })
            });
        }
    }

    fn current_model_name(&self, cx: &App) -> SharedString {
        // For Omega Agent, use the specific model name (e.g., "Claude 3.5 Sonnet")
        // For ACP agents, use the agent name (e.g., "Claude Agent", "Gemini CLI")
        // This provides better clarity about what refused the request
        if self.as_native_connection(cx).is_some() {
            self.root_thread_view()
                .and_then(|active| active.read(cx).model_selector.clone())
                .and_then(|selector| selector.read(cx).active_model(cx))
                .map(|model| model.name.clone())
                .unwrap_or_else(|| SharedString::from("The model"))
        } else {
            // ACP agent - use the agent name (e.g., "Claude Agent", "Gemini CLI")
            self.agent.agent_id().0
        }
    }

    fn create_copy_button(&self, message: impl Into<String>) -> impl IntoElement {
        let message = message.into();

        CopyButton::new("copy-error-message", message).tooltip_label("Copy Error Message")
    }

    pub(crate) fn reauthenticate(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.cancel_request_elicitations(cx);
        if let Some(active) = self.root_thread_view() {
            active.update(cx, |active, cx| active.clear_thread_error(cx));
        }
        let this = cx.weak_entity();
        let Some(connection) = self.as_connected().map(|c| c.connection.clone()) else {
            debug_panic!("This should not be possible");
            return;
        };
        window.defer(cx, |window, cx| {
            Self::handle_auth_required(this, AuthRequired::new(), connection, window, cx);
        })
    }

    pub(crate) fn logout(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.supports_logout() {
            return;
        }

        if let Some(active) = self.root_thread_view() {
            active.update(cx, |active, cx| active.clear_thread_error(cx));
        }
        let Some(connection) = self
            .as_connected()
            .map(|connected| connected.connection.clone())
        else {
            return;
        };
        let logout = connection.logout(cx);
        self.auth_task = Some(cx.spawn_in(window, {
            async move |this, cx| {
                let result = logout.await;
                this.update_in(cx, |this, window, cx| {
                    if let Err(err) = result {
                        if let Some(active) = this.root_thread_view() {
                            active.update(cx, |active, cx| active.handle_thread_error(err, cx));
                        }
                    } else {
                        this.cancel_request_elicitations(cx);
                        if let Some(connected) = this.as_connected_mut() {
                            connected.auth_state = AuthState::Unauthenticated {
                                description: None,
                                pending_auth_method: None,
                            };
                            cx.emit(StateChange);
                            if let Some(view) = connected.active_view()
                                && view
                                    .read(cx)
                                    .message_editor
                                    .focus_handle(cx)
                                    .is_focused(window)
                            {
                                this.focus_handle.focus(window, cx)
                            }
                            cx.notify();
                        }
                    }
                    drop(this.auth_task.take());
                })
                .ok();
            }
        }));
    }
}

fn routed_executor_for_owner(
    owner: &Agent,
    routed_executor: Option<crate::omega_executor_selector::SelectableExecutor>,
) -> Option<crate::omega_executor_selector::SelectableExecutor> {
    matches!(owner, Agent::NativeAgent)
        .then_some(routed_executor)
        .flatten()
}

fn loading_contents_spinner(size: IconSize) -> AnyElement {
    Icon::new(IconName::LoadCircle)
        .size(size)
        .color(Color::Accent)
        .with_rotate_animation(3)
        .into_any_element()
}

fn native_available_skills(
    native_connection: &agent::NativeAgentConnection,
    session_id: &acp::SessionId,
    cx: &App,
) -> Vec<AvailableSkill> {
    native_connection
        .available_skills(session_id, cx)
        .into_iter()
        .map(|skill| AvailableSkill {
            name: skill.name.into(),
            description: skill.description.into(),
            source: skill.source,
            skill_file_path: skill.skill_file_path,
            warning: skill.warning,
        })
        .collect()
}

fn placeholder_text(agent_name: &str) -> String {
    let agent_name = if agent_name == agent::OMEGA_AGENT_ID.as_ref() {
        "Omega"
    } else {
        agent_name
    };
    format!("Message {agent_name}")
}

impl Focusable for ConversationView {
    fn focus_handle(&self, cx: &App) -> FocusHandle {
        // `OMEGA-DELTA-0122`. While connecting, focus belongs to the composer
        // that is on screen. Without this, focusing the panel during `Loading`
        // lands on the container, and the field a person can see and type in is
        // one they have to click first — which is the complaint one step over,
        // not answered.
        if let Some(loading_composer) = self.loading_composer.as_ref()
            && matches!(self.server_state, ServerState::Loading { .. })
        {
            return loading_composer.focus_handle(cx);
        }
        match self.active_thread() {
            Some(thread) => thread.read(cx).focus_handle(cx),
            None => self.focus_handle.clone(),
        }
    }
}

impl ConversationView {
    pub(crate) fn activation_focus_handle(&self, cx: &App) -> FocusHandle {
        // `OMEGA-DELTA-0122`, restated for activation (omega#165): while the
        // pre-session composer is the surface on screen — connecting, or a
        // RouterReady conversation waiting for its first send — that composer
        // owns the caret. The composer front door re-activates a conversation
        // that is already the base view, and activating the container here
        // would steal focus from the loading composer its first render just
        // focused — a composer that looks ready and ignores the keyboard.
        if let Some(loading_composer) = self.loading_composer.as_ref()
            && self.active_thread().is_none()
            && !matches!(self.server_state, ServerState::LoadError { .. })
        {
            return loading_composer.focus_handle(cx);
        }
        self.active_thread()
            .map(|thread| thread.read(cx).activation_focus_handle(cx))
            .unwrap_or_else(|| self.focus_handle.clone())
    }
}

#[cfg(any(test, feature = "test-support"))]
impl ConversationView {
    /// Expands a tool call so its content is visible.
    /// This is primarily useful for visual testing.
    pub fn expand_tool_call(&mut self, tool_call_id: acp::ToolCallId, cx: &mut Context<Self>) {
        if let Some(active) = self.active_thread() {
            active.update(cx, |active, cx| {
                active.entry_view_state.update(cx, |state, _cx| {
                    state.expand_tool_call(tool_call_id);
                });
            });
            cx.notify();
        }
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn set_updated_at(&mut self, updated_at: Instant, cx: &mut Context<Self>) {
        let Some(connected) = self.as_connected_mut() else {
            return;
        };

        connected.conversation.update(cx, |conversation, _cx| {
            conversation.updated_at = Some(updated_at);
        });
    }
}

impl Render for ConversationView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.sync_request_elicitation_states(window, cx);
        let request_elicitation_connection = self.request_elicitation_connection();
        let active_thread_renders_request_elicitations =
            self.active_thread_renders_request_elicitations();
        // omega#112, then `OMEGA-DELTA-0122`. Bottom left, not the middle of an
        // empty window — and in a composer, not on its own.
        //
        // The owner, switching executors: "it shows me a fullscreen loading
        // window with nothing on it except for the center message. that is
        // unacceptable." A centred label in an otherwise blank pane reads as
        // the application having gone somewhere, for something that is usually
        // a second of process startup.
        //
        // Moving it down was accepted and was not enough: "you still dont show
        // the input bar while its fucking loading". So the label is now the
        // status line of a real, typable composer, which is the window still
        // looking like the window you were just in. The bottom-left corner also
        // holds the persistent Vim readout when modal editing is enabled.
        //
        // Built before the match rather than inside it: drawing it needs
        // `&mut self`, and the match holds `&self.server_state` across every
        // arm.
        let router_ready = matches!(
            self.preparation_state(cx),
            ConversationPreparation::RouterReady
        );
        let mut pre_session_composer = (matches!(self.server_state, ServerState::Loading { .. })
            || router_ready)
            .then(|| self.render_loading_composer(router_ready, window, cx));

        let content = match &self.server_state {
            ServerState::Loading { .. } => pre_session_composer
                .take()
                .unwrap_or_else(|| div().into_any_element()),
            ServerState::LoadError { error: e, .. } => v_flex()
                .flex_1()
                .size_full()
                .items_center()
                .justify_end()
                // `OMEGA-DELTA-0170`. Turns sent while connecting outlive the
                // connection that failed: the text stays on screen, marked
                // unsent, with a retry — never silently dropped.
                .children(self.render_pending_connect_messages(true, cx))
                .child(self.render_load_error(e, window, cx))
                .into_any(),
            ServerState::Connected(ConnectedServerState {
                connection,
                auth_state:
                    AuthState::Unauthenticated {
                        description,
                        pending_auth_method,
                    },
                ..
            }) => v_flex()
                .flex_1()
                .size_full()
                .justify_end()
                // `OMEGA-DELTA-0170`. Pending turns stay visible while the
                // executor waits on sign-in; a successful authentication
                // rebuilds the session and dispatches them.
                .children(self.render_pending_connect_messages(false, cx))
                .child(self.render_auth_required_state(
                    connection,
                    description.as_ref(),
                    pending_auth_method.as_ref(),
                    window,
                    cx,
                ))
                .into_any_element(),
            ServerState::Connected(connected) => {
                if let Some(view) = connected.active_view() {
                    if let Some(right_pane_view) = connected
                        .right_pane_session_id
                        .as_ref()
                        .and_then(|session_id| connected.threads.get(session_id))
                    {
                        h_flex()
                            .size_full()
                            .child(div().flex_1().min_w_0().size_full().child(view.clone()))
                            .child(Divider::vertical())
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .size_full()
                                    .child(right_pane_view.clone()),
                            )
                            .into_any_element()
                    } else {
                        view.clone().into_any_element()
                    }
                } else {
                    if router_ready {
                        pre_session_composer
                            .take()
                            .unwrap_or_else(|| div().into_any_element())
                    } else {
                        debug_panic!("connected agent has no active session");
                        div().into_any_element()
                    }
                }
            }
        };

        v_flex()
            .track_focus(&self.focus_handle)
            .size_full()
            .bg(cx.theme().colors().panel_background)
            .child(v_flex().flex_1().min_h_0().child(content))
            .when(!active_thread_renders_request_elicitations, |this| {
                this.children(request_elicitation_connection.as_ref().map_or_else(
                    Vec::new,
                    |connection| {
                        self.render_request_elicitations(connection, cx.entity().downgrade(), cx)
                    },
                ))
            })
    }
}

fn render_agent_markdown(
    markdown: Entity<Markdown>,
    style: MarkdownStyle,
    workspace: &WeakEntity<Workspace>,
    code_span_resolver: &AgentCodeSpanResolver,
    work_dirs: Option<&PathList>,
    cx: &App,
) -> MarkdownElement {
    let workspace = workspace.clone();
    let worktree_roots = code_span_resolver.worktree_roots(cx);
    // `OMEGA-DELTA-0119`. The thread's own working directories come first. They
    // are where the agent actually runs, which is not always where this window
    // has worktrees: an external executor's session carries its own `cwd`, and
    // resolving a relative path against the wrong root is how a link ends up
    // naming a file that is not there.
    let peek_roots = work_dirs
        .into_iter()
        .flat_map(PathList::ordered_paths)
        .cloned()
        .chain(worktree_roots.iter().cloned())
        .fold(Vec::new(), |mut roots: Vec<PathBuf>, root| {
            if !roots.contains(&root) {
                roots.push(root);
            }
            roots
        });
    let resolver = code_span_resolver.clone();
    MarkdownElement::new(markdown, style)
        .code_block_renderer(markdown::CodeBlockRenderer::Default {
            copy_button_visibility: markdown::CopyButtonVisibility::VisibleOnHover,
            wrap_button_visibility: markdown::WrapButtonVisibility::VisibleOnHover,
            border: false,
        })
        .image_resolver(move |dest_url| resolve_agent_image(dest_url, &worktree_roots))
        .on_url_click(move |text, window, cx| {
            // `OMEGA-DELTA-0139`. Plain clicks open the ordinary editable
            // editor. A secondary click preserves the compact reader for
            // people who only want to inspect the source without changing
            // their workspace layout.
            let mode = if window.modifiers().secondary() {
                crate::omega_file_peek::TranscriptFileOpenMode::ReadOnlyPeek
            } else {
                crate::omega_file_peek::TranscriptFileOpenMode::EditablePane
            };
            if crate::omega_file_peek::open_from_transcript_link(
                &text,
                &peek_roots,
                mode,
                &workspace,
                window,
                cx,
            ) {
                return;
            }
            thread_view::open_link(text, &workspace, window, cx);
        })
        .on_code_span_link(move |text, cx| resolver.try_resolve(text, cx))
}

/// Shared, cloneable handle for resolving inline markdown code spans like
/// `` `src/main.rs:42` `` to clickable workspace file links.
#[derive(Clone)]
pub(crate) struct AgentCodeSpanResolver {
    inner: Arc<AgentCodeSpanResolverInner>,
}

/// Maximum number of memoized code-span resolutions kept in the cache.
const CODE_SPAN_CACHE_CAPACITY: NonZeroUsize = match NonZeroUsize::new(2048) {
    Some(n) => n,
    None => unreachable!(),
};

struct AgentCodeSpanResolverInner {
    project: WeakEntity<Project>,
    cache: Mutex<LruCache<Arc<str>, Option<SharedString>>>,
}

impl AgentCodeSpanResolver {
    pub(crate) fn new(project: &WeakEntity<Project>, _cx: &App) -> Self {
        Self {
            inner: Arc::new(AgentCodeSpanResolverInner {
                project: project.clone(),
                cache: Mutex::new(LruCache::new(CODE_SPAN_CACHE_CAPACITY)),
            }),
        }
    }

    pub(crate) fn clear_cache(&self) {
        self.inner.cache.lock().clear();
    }

    /// Absolute paths of every current worktree.
    /// Used by the markdown image resolver, which needs the same set of roots.
    fn worktree_roots(&self, cx: &App) -> Vec<PathBuf> {
        self.inner
            .project
            .upgrade()
            .map(|project| {
                project
                    .read(cx)
                    .visible_worktrees(cx)
                    .map(|worktree| worktree.read(cx).abs_path().to_path_buf())
                    .collect()
            })
            .unwrap_or_default()
    }

    fn try_resolve(&self, text: &str, cx: &App) -> Option<SharedString> {
        let trimmed = sanitize_path_text(text.trim());
        if !Self::is_path_like(trimmed) {
            return None;
        }

        if let Some(cached) = self.inner.cache.lock().get(trimmed).cloned() {
            return cached;
        }

        let resolved = self.resolve_uncached(trimmed, cx);
        self.inner
            .cache
            .lock()
            .push(Arc::from(trimmed), resolved.clone());
        resolved
    }

    fn resolve_uncached(&self, trimmed: &str, cx: &App) -> Option<SharedString> {
        let path_with_position = PathWithPosition::parse_str(trimmed);
        let candidate_path = &path_with_position.path;
        if candidate_path.as_os_str().is_empty() {
            return None;
        }

        let project = self.inner.project.upgrade()?;
        let project = project.read(cx);
        for worktree in project.visible_worktrees(cx) {
            let worktree = worktree.read(cx);
            for relative_path in Self::candidate_relative_paths(
                candidate_path,
                &worktree.abs_path(),
                worktree.path_style(),
            ) {
                let project_path = ProjectPath {
                    worktree_id: worktree.id(),
                    path: relative_path.clone(),
                };
                let Some(entry) = project.entry_for_path(&project_path, cx) else {
                    continue;
                };
                if !entry.is_file() {
                    continue;
                }

                let abs_path = worktree.absolutize(&relative_path);
                let mention = match path_with_position.row.and_then(|row| row.checked_sub(1)) {
                    Some(line) => MentionUri::Selection {
                        abs_path: Some(abs_path),
                        line_range: line..=line,
                        column: path_with_position
                            .column
                            .map(|column| column.saturating_sub(1)),
                    },
                    None => MentionUri::File { abs_path },
                };

                return Some(mention.to_uri().to_string().into());
            }
        }

        None
    }

    fn candidate_relative_paths(
        path: &Path,
        worktree_abs_path: &Path,
        path_style: PathStyle,
    ) -> Vec<Arc<RelPath>> {
        let path_text = path.to_string_lossy();
        let relative_path: Option<Arc<RelPath>> =
            if util::paths::is_absolute(path_text.as_ref(), path_style) {
                path_style
                    .strip_prefix(path, worktree_abs_path)
                    .map(std::borrow::Cow::into_owned)
                    .map(Into::into)
            } else {
                RelPath::new(path, path_style)
                    .ok()
                    .map(std::borrow::Cow::into_owned)
                    .map(Into::into)
            };

        let Some(relative_path) = relative_path else {
            return Vec::new();
        };

        let mut paths = vec![relative_path.clone()];
        if let Some(root_name) = worktree_abs_path.file_name().and_then(|name| name.to_str())
            && let Ok(root_name) = RelPath::new(Path::new(root_name), path_style)
            && let Ok(stripped) = relative_path.strip_prefix(root_name.as_ref())
            && !stripped.is_empty()
        {
            paths.push(Arc::from(stripped));
        }
        paths
    }

    fn is_path_like(text: &str) -> bool {
        if text.len() < 3
            || text.contains("://")
            || text.contains('|')
            || text.chars().any(char::is_control)
            || text.chars().all(|character| character.is_ascii_digit())
        {
            return false;
        }

        let path = PathWithPosition::parse_str(text).path;
        let path_text = path.to_string_lossy();
        if path_text.contains('/') || path_text.contains('\\') {
            return true;
        }

        path.extension()
            .and_then(|extension| extension.to_str())
            .is_some_and(|extension| !extension.is_empty())
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use acp_thread::StubAgentConnection;
    use action_log::ActionLog;
    use agent::{AgentTool, EditFileTool, FetchTool, TerminalTool, ToolPermissionContext};
    use agent_servers::FakeAcpAgentServer;
    use editor::MultiBufferOffset;
    use editor::actions::Paste;
    use feature_flags::{AcpBetaFeatureFlag, FeatureFlag as _, FeatureFlagAppExt as _};
    use fs::FakeFs;
    use gpui::{
        ClipboardItem, EventEmitter, TestAppContext, UpdateGlobal, VisualTestContext, point, size,
    };
    use parking_lot::Mutex;
    use project::Project;
    use serde_json::json;
    use settings::SettingsStore;
    use std::any::Any;
    use std::path::{Path, PathBuf};
    use std::rc::Rc;
    use std::sync::Arc;
    use workspace::{Item, MultiWorkspace};

    use crate::agent_panel;
    use crate::completion_provider::AgentContextSource;
    use crate::mention_set::Mention;
    use crate::test_support::register_test_sidebar;
    use crate::thread_metadata_store::ThreadMetadataStore;

    use super::*;

    #[test]
    fn composer_uses_omega_geometry() {
        assert_eq!(COMPOSER_COMPACT_HEIGHT, 49.);
        assert_eq!(COMPOSER_EXPANDED_MIN_HEIGHT, 124.);
        assert_eq!(COMPOSER_EXPANDED_MAX_HEIGHT, 308.);
        assert_eq!(COMPOSER_RADIUS, 26.);
        assert_eq!(composer_max_width(None).as_f32(), 768.);
        assert_eq!(composer_max_width(Some(px(850.))).as_f32(), 768.);
        assert_eq!(composer_max_width(Some(px(640.))).as_f32(), 640.);
    }

    #[test]
    fn composer_layout_is_consistent_across_thread_lifecycle() {
        assert_eq!(composer_layout(false, "one line"), ComposerLayout::Compact);
        assert_eq!(composer_layout(false, ""), ComposerLayout::Compact);
        assert_eq!(
            composer_layout(false, "first\nsecond"),
            ComposerLayout::Expanded
        );
        assert_eq!(
            composer_layout(true, "one line"),
            ComposerLayout::ManuallyExpanded
        );
        assert_eq!(
            composer_layout(false, &"x".repeat(COMPOSER_SINGLE_LINE_CHARACTER_LIMIT + 1),),
            ComposerLayout::Expanded
        );
    }

    struct RouterTestServer {
        native: StubAgentConnection,
        journal_path: PathBuf,
    }

    impl AgentServer for RouterTestServer {
        fn logo(&self) -> ui::IconName {
            ui::IconName::OmegaAgent
        }

        fn agent_id(&self) -> AgentId {
            agent::OMEGA_AGENT_ID.clone()
        }

        fn connect(
            &self,
            _delegate: AgentServerDelegate,
            _project: Entity<Project>,
            _cx: &mut App,
        ) -> Task<Result<Rc<dyn AgentConnection>>> {
            let native: Rc<dyn AgentConnection> = Rc::new(self.native.clone());
            let router = crate::omega_router::OmegaAgentConnection::new(
                native,
                crate::omega_router::RouteJournal::at(self.journal_path.clone()),
            );
            Task::ready(Ok(Rc::new(router)))
        }

        fn into_any(self: Rc<Self>) -> Rc<dyn Any> {
            self
        }
    }

    #[test]
    fn direct_owner_ignores_a_stale_router_selection() {
        let stale = Some(crate::omega_executor_selector::SelectableExecutor::Grok);
        assert_eq!(
            routed_executor_for_owner(
                &Agent::Custom {
                    id: AgentId::new("grok-build"),
                },
                stale,
            ),
            None
        );
        assert_eq!(routed_executor_for_owner(&Agent::NativeAgent, stale), stale);
    }

    #[test]
    fn omega_requirements_come_from_typed_context_not_prompt_words() {
        use omega_front_door::router::TaskKind;

        let conversational = AgentInitialContent::ContentBlock {
            blocks: vec![acp::ContentBlock::Text(acp::TextContent::new(
                "edit every repository file".to_string(),
            ))],
            auto_submit: false,
        };
        assert_eq!(
            omega_task_requirements(Some(&conversational)).kind,
            TaskKind::GeneralReasoning,
            "repository-sounding words are not repository context"
        );
        assert!(omega_initial_content_has_request(&conversational));

        let empty_draft = AgentInitialContent::ContentBlock {
            blocks: Vec::new(),
            auto_submit: false,
        };
        assert!(
            !omega_initial_content_has_request(&empty_draft),
            "an empty restored draft is not a first request and must not freeze a route"
        );
        let blank_draft = AgentInitialContent::ContentBlock {
            blocks: vec![acp::ContentBlock::Text(acp::TextContent::new(
                "  \n".to_owned(),
            ))],
            auto_submit: false,
        };
        assert!(
            !omega_initial_content_has_request(&blank_draft),
            "a serialized blank text block is still an empty restored draft"
        );

        let repository = AgentInitialContent::ContentBlock {
            blocks: vec![acp::ContentBlock::ResourceLink(acp::ResourceLink::new(
                "src/main.rs",
                "file:///workspace/src/main.rs",
            ))],
            auto_submit: false,
        };
        assert_eq!(
            omega_task_requirements(Some(&repository)).kind,
            TaskKind::RepositoryWork,
            "a typed file reference is repository context regardless of its prose"
        );
        assert_eq!(
            omega_task_requirements(None).kind,
            TaskKind::GeneralReasoning,
            "an open project is not itself a first-request task requirement"
        );
    }

    #[test]
    fn test_data_retention_error_maps_from_provider_error() {
        // The agent wraps the provider error in a fresh `anyhow::Error`, so
        // the mapping must downcast to `LanguageModelCompletionError` rather
        // than matching on the anyhow error directly.
        let provider_error = LanguageModelCompletionError::DataRetentionConsentRequired {
            model_name: "Claude Fable 5".to_string(),
        };
        let error = ThreadError::from(anyhow!(provider_error));
        assert!(
            matches!(error, ThreadError::DataRetentionConsentRequired),
            "expected ThreadError::DataRetentionConsentRequired, got: {error:?}"
        );
    }

    #[test]
    fn work_dir_transaction_reports_partial_success_with_failed_rollback_as_inconsistent() {
        let mut operations = Vec::new();
        let error = apply_work_dir_transaction(&[0, 1], |session, rollback| {
            operations.push((*session, rollback));
            match (*session, rollback) {
                (0, false) => Ok(()),
                (1, false) => Err(anyhow!("second session rejected the target")),
                (0, true) => Err(anyhow!("first session rejected rollback")),
                _ => Ok(()),
            }
        })
        .expect_err("a failed rollback must poison the transaction");

        assert!(
            error.downcast_ref::<InconsistentWorkDirsError>().is_some(),
            "callers need a typed signal that displayed identity is no longer authoritative"
        );
        assert_eq!(operations, vec![(0, false), (1, false), (0, true)]);
        assert!(error.to_string().contains("Reselect a repository target"));
    }

    #[gpui::test]
    async fn test_drop(cx: &mut TestAppContext) {
        init_test(cx);

        let (conversation_view, _cx) =
            setup_conversation_view(StubAgentServer::default_response(), cx).await;
        let weak_view = conversation_view.downgrade();
        drop(conversation_view);
        assert!(!weak_view.is_upgradable());
    }

    /// omega#121. Choosing an executor has to actually rebuild the thread.
    ///
    /// The owner selected Exo, saw the selector say Exo, sent `who are you`,
    /// and Codex answered — in a thread still titled "New Codex Thread", with
    /// "Message Codex" in the composer. Every one of those surfaces was right
    /// except the selector, which omega#120 had just changed to show the
    /// *choice*. The choice was never being connected.
    ///
    /// The log is what says so and what this test replaces: three Shift-Tabs,
    /// four seconds apart, each one logging `OMEGA-DELTA-0115: a person chose
    /// ...` and not one of them followed by an attach. Selecting was reaching
    /// the selection global and stopping there.
    ///
    /// This is the assertion that was missing while three separate causes of
    /// the same symptom were fixed one at a time by hand. It does not name a
    /// mechanism — not the debounce, not the connection store, not the
    /// disclosure — because each fix so far has been a different mechanism
    /// producing this one outcome. It holds the outcome: choose something
    /// other than what is attached, let the presses stop, and the thread you
    /// end up in is a new one.
    #[gpui::test]
    async fn choosing_an_executor_rebuilds_the_thread_once_the_presses_stop(
        cx: &mut TestAppContext,
    ) {
        use crate::omega_executor_selector::{
            SelectableExecutor, clear_selection_for_test, select_for_test,
        };

        init_test(cx);

        let (conversation_view, cx) =
            setup_conversation_view(StubAgentServer::new(StubAgentConnection::new()), cx).await;

        let before = active_thread(&conversation_view, cx)
            .read_with(cx, |view, cx| view.thread.read(cx).session_id().clone());

        // Whatever this stub reads as, choose something else. The rebuild is
        // allowed to skip the case where the choice is already attached, so a
        // test that happened to choose the attached executor would pass while
        // proving nothing.
        let attached = active_thread(&conversation_view, cx).read_with(cx, |view, cx| {
            let disclosure = view.executor_disclosure(cx);
            SelectableExecutor::of(disclosure.class, &disclosure.agent_id)
        });
        let choice = SelectableExecutor::ALL
            .iter()
            .copied()
            .find(|candidate| Some(*candidate) != attached)
            .expect("four names cannot all be the attached one");

        select_for_test(choice);
        conversation_view.update_in(cx, |view, window, cx| {
            view.reset_onto_new_executor(window, cx);
        });

        // Past the settle window, which is the whole of what "once the presses
        // stop" means.
        cx.executor().advance_clock(Duration::from_millis(1_000));
        cx.run_until_parked();

        let after = active_thread(&conversation_view, cx)
            .read_with(cx, |view, cx| view.thread.read(cx).session_id().clone());
        clear_selection_for_test();

        assert_ne!(
            before,
            after,
            "choosing {} left the thread on the executor that was already \
             attached: the selection changed and nothing reconnected, which is \
             the owner sending a message to Exo and getting Codex",
            choice.name()
        );
    }

    /// omega#109. The panel resolves an external subagent it cannot load.
    ///
    /// A subagent spawned with an `executor` runs as Codex or Claude Code, on
    /// that agent's own server. Its session is not this connection's and never
    /// will be: `NativeAgent::sessions` has no entry, `load_session` here
    /// cannot produce it, and all the panel is handed is an id. So the card had
    /// a name and nothing to resolve it to — the whole of omega#109's second
    /// gap.
    ///
    /// The stub connection this view is running is deliberately one that does
    /// not support loading sessions, which is the honest shape of the problem:
    /// there is no route from this connection to that thread, and the only
    /// reason the card can render one is the lookup added here. Point the
    /// lookup back at this connection alone and the assertion below fails —
    /// which is the check the falsifier asks for.
    #[gpui::test]
    async fn an_external_subagent_is_resolved_by_id_and_names_its_own_executor(
        cx: &mut TestAppContext,
    ) {
        init_test(cx);

        let (conversation_view, cx) =
            setup_conversation_view(StubAgentServer::new(StubAgentConnection::new()), cx).await;

        let (root_thread, project) = conversation_view.read_with(cx, |view, cx| {
            (
                view.root_thread_view()
                    .expect("the conversation must have a root thread")
                    .read(cx)
                    .thread
                    .clone(),
                view.project.clone(),
            )
        });

        // A session on somebody else's connection. That is what an external ACP
        // subagent is, and why the panel could not find one.
        let external_connection: Rc<dyn AgentConnection> =
            Rc::new(StubAgentConnection::new().with_agent_id(
                project::agent_server_store::AgentId::new("codex-acp".to_string()),
            ));
        let external_thread = cx
            .update(|_window, cx| {
                external_connection
                    .clone()
                    .new_session(project.clone(), PathList::default(), cx)
            })
            .await
            .expect("the external agent must open a session");
        let external_id = external_thread.read_with(cx, |thread, _cx| thread.session_id().clone());

        assert!(
            conversation_view
                .read_with(cx, |view, _cx| view.thread_view(&external_id))
                .is_none(),
            "the external session must not already be in this connection's map; \
             if it is, this test is not testing what it says it is"
        );

        // What `create_external_acp_subagent` records when it opens one.
        cx.update(|_window, cx| {
            agent::register_external_subagent_session(external_id.clone(), &external_thread, cx);
        });

        // And what the parent announces: an id, and nothing else.
        root_thread.update(cx, |thread, cx| {
            thread.subagent_spawned(external_id.clone(), cx);
        });
        cx.run_until_parked();

        let resolved = conversation_view
            .read_with(cx, |view, _cx| view.thread_view(&external_id))
            .expect(
                "the panel was handed an external subagent's session id and \
                 could not resolve it, so its card has no thread: no \
                 transcript, and no executor to name",
            );

        resolved.read_with(cx, |view, cx| {
            assert_eq!(
                view.thread.read(cx).session_id(),
                &external_id,
                "the card resolved to the wrong thread"
            );

            // And the card can name what ran it. Classified from the thread's
            // own connection, so the card and the thread cannot disagree.
            use crate::omega_executor_disclosure::ThreadExecutorDisclosure as _;
            let disclosure = view.thread.read(cx).omega_executor_disclosure(cx);
            assert_eq!(
                disclosure.class,
                omega_front_door::ExecutorClass::ExternalAcp,
                "an external subagent must not read as Omega's own loop"
            );
            assert_eq!(disclosure.agent_id, "codex-acp");
        });
    }

    #[gpui::test]
    async fn test_drop_preserves_shared_pending_request_elicitations(cx: &mut TestAppContext) {
        init_test(cx);
        cx.update(|cx| {
            cx.update_flags(true, vec![AcpBetaFeatureFlag::NAME.to_string()]);
        });

        let response = Arc::new(Mutex::new(None));
        let server = ReleaseRequestElicitationServer {
            response: response.clone(),
        };
        let (conversation_view, cx) = setup_conversation_view(server, cx).await;
        let _connection = conversation_view
            .read_with(cx, |view, _cx| view.request_elicitation_connection())
            .expect("conversation should have an active connection");
        let store = _connection
            .request_elicitations()
            .expect("connection should expose request elicitations");
        store.read_with(cx, |store, _cx| {
            assert_eq!(
                store.elicitations().len(),
                1,
                "test should start with one pending request elicitation"
            );
        });

        assert_eq!(*response.lock(), None);
        let weak_view = conversation_view.downgrade();
        drop(conversation_view);
        cx.update(|_, _| {});
        cx.run_until_parked();

        assert!(!weak_view.is_upgradable());
        store.read_with(cx, |store, _cx| {
            assert_eq!(
                store.elicitations().len(),
                1,
                "view release should not clear connection-wide request elicitations"
            );
        });
        assert_eq!(*response.lock(), None);

        store.update(cx, |store, cx| store.clear(cx));
        cx.run_until_parked();
        assert!(matches!(
            response.lock().as_ref(),
            Some(acp::ElicitationAction::Cancel)
        ));
    }

    #[gpui::test]
    async fn test_state_transition_preserves_shared_pending_request_elicitations(
        cx: &mut TestAppContext,
    ) {
        init_test(cx);
        cx.update(|cx| {
            cx.update_flags(true, vec![AcpBetaFeatureFlag::NAME.to_string()]);
        });

        let response = Arc::new(Mutex::new(None));
        let server = ReleaseRequestElicitationServer {
            response: response.clone(),
        };
        let (conversation_view, cx) = setup_conversation_view(server, cx).await;
        let connection = conversation_view
            .read_with(cx, |view, _cx| view.request_elicitation_connection())
            .expect("conversation should have an active connection");
        let store = connection
            .request_elicitations()
            .expect("connection should expose request elicitations");
        store.read_with(cx, |store, _cx| {
            assert_eq!(
                store.elicitations().len(),
                1,
                "test should start with one pending request elicitation"
            );
        });

        conversation_view.update(cx, |view, cx| {
            view.set_server_state(
                ServerState::LoadError {
                    error: LoadError::Other("load failed".into()),
                },
                cx,
            );
        });
        cx.run_until_parked();

        store.read_with(cx, |store, _cx| {
            assert_eq!(
                store.elicitations().len(),
                1,
                "leaving a connection should not clear connection-wide request elicitations"
            );
        });
        assert_eq!(*response.lock(), None);

        store.update(cx, |store, cx| store.clear(cx));
        cx.run_until_parked();
        assert!(matches!(
            response.lock().as_ref(),
            Some(acp::ElicitationAction::Cancel)
        ));
    }

    #[gpui::test]
    async fn test_successful_session_creation_clears_resolved_request_elicitations(
        cx: &mut TestAppContext,
    ) {
        init_test(cx);
        cx.update(|cx| {
            cx.update_flags(true, vec![AcpBetaFeatureFlag::NAME.to_string()]);
        });

        let store = cx.update(|cx| cx.new(|_| ElicitationStore::default()));
        let response = Arc::new(Mutex::new(None));
        let server = SessionCreationRequestElicitationServer {
            store: store.clone(),
            response: response.clone(),
        };
        let (conversation_view, cx) = setup_conversation_view(server, cx).await;
        let first_request_id = acp::RequestId::Number(1);
        let second_request_id = acp::RequestId::Number(2);
        let first_elicitation_id = store.read_with(cx, |store, _cx| {
            assert_eq!(
                store.elicitations().len(),
                2,
                "session creation should be waiting on one prompt with another prompt still pending"
            );
            store
                .elicitations()
                .iter()
                .find_map(|elicitation| {
                    let acp::ElicitationScope::Request(scope) = elicitation.request.scope() else {
                        return None;
                    };
                    (&scope.request_id == &first_request_id).then(|| elicitation.id.clone())
                })
                .expect("first request-scoped elicitation should exist")
        });

        store.update(cx, |store, cx| {
            store.respond_to_elicitation(
                &first_elicitation_id,
                acp::CreateElicitationResponse::new(acp::ElicitationAction::Accept(
                    acp::ElicitationAcceptAction::new(),
                )),
                cx,
            );
        });
        cx.run_until_parked();

        assert!(matches!(
            response.lock().as_ref(),
            Some(acp::ElicitationAction::Accept(_))
        ));
        conversation_view.read_with(cx, |view, _cx| {
            let connected = view
                .as_connected()
                .expect("session creation should complete successfully");
            assert!(
                connected.active_id.is_some(),
                "successful session creation should install an active thread"
            );
        });
        store.read_with(cx, |store, _cx| {
            let [remaining] = store.elicitations() else {
                panic!(
                    "expected only the pending request elicitation to remain, got {:?}",
                    store.elicitations()
                );
            };
            let acp::ElicitationScope::Request(scope) = remaining.request.scope() else {
                panic!("expected request-scoped elicitation");
            };
            assert_eq!(scope.request_id, second_request_id);
            assert!(matches!(
                remaining.status,
                ElicitationStatus::Pending { .. }
            ));
        });

        store.update(cx, |store, cx| store.clear(cx));
        cx.run_until_parked();
    }

    #[gpui::test]
    async fn test_external_source_prompt_requires_manual_send(cx: &mut TestAppContext) {
        init_test(cx);

        let Some(prompt) = crate::ExternalSourcePrompt::new("Write me a script") else {
            panic!("expected prompt from external source to sanitize successfully");
        };
        let initial_content = AgentInitialContent::FromExternalSource(prompt);

        let (conversation_view, cx) = setup_conversation_view_with_initial_content(
            StubAgentServer::default_response(),
            initial_content,
            cx,
        )
        .await;

        active_thread(&conversation_view, cx).read_with(cx, |view, cx| {
            assert!(view.show_external_source_prompt_warning);
            assert_eq!(view.thread.read(cx).entries().len(), 0);
            assert_eq!(view.message_editor.read(cx).text(cx), "Write me a script");
        });
    }

    #[gpui::test]
    async fn test_external_source_prompt_warning_clears_after_send(cx: &mut TestAppContext) {
        init_test(cx);

        let Some(prompt) = crate::ExternalSourcePrompt::new("Write me a script") else {
            panic!("expected prompt from external source to sanitize successfully");
        };
        let initial_content = AgentInitialContent::FromExternalSource(prompt);

        let (conversation_view, cx) = setup_conversation_view_with_initial_content(
            StubAgentServer::default_response(),
            initial_content,
            cx,
        )
        .await;

        active_thread(&conversation_view, cx)
            .update_in(cx, |view, window, cx| view.send(window, cx));
        cx.run_until_parked();

        active_thread(&conversation_view, cx).read_with(cx, |view, cx| {
            assert!(!view.show_external_source_prompt_warning);
            assert_eq!(view.message_editor.read(cx).text(cx), "");
            assert_eq!(view.thread.read(cx).entries().len(), 2);
        });
    }

    #[gpui::test]
    async fn test_agent_code_span_resolver_resolves_worktree_paths(cx: &mut TestAppContext) {
        init_test(cx);

        let fs = FakeFs::new(cx.executor());
        fs.insert_tree(
            util::path!("/project"),
            json!({
                "src": {
                    "main.rs": ""
                },
                "README.md": ""
            }),
        )
        .await;

        let project = Project::test(fs, [Path::new(util::path!("/project"))], cx).await;
        let resolver = cx.update(|cx| AgentCodeSpanResolver::new(&project.downgrade(), cx));

        let uri = cx
            .update(|cx| resolver.try_resolve("src/main.rs:10", cx))
            .expect("expected worktree-relative file path to resolve");
        assert_eq!(
            MentionUri::parse(&uri, PathStyle::local()).unwrap(),
            MentionUri::Selection {
                abs_path: Some(PathBuf::from(util::path!("/project/src/main.rs"))),
                line_range: 9..=9,
                column: None,
            }
        );

        let uri = cx
            .update(|cx| resolver.try_resolve("src/main.rs:10:5", cx))
            .expect("expected worktree-relative file path with row and column to resolve");
        assert_eq!(
            MentionUri::parse(&uri, PathStyle::local()).unwrap(),
            MentionUri::Selection {
                abs_path: Some(PathBuf::from(util::path!("/project/src/main.rs"))),
                line_range: 9..=9,
                column: Some(4),
            }
        );

        let uri = cx
            .update(|cx| resolver.try_resolve("src/main.rs:0", cx))
            .expect("`:0` should fall back to a file mention instead of returning None");
        assert_eq!(
            MentionUri::parse(&uri, PathStyle::local()).unwrap(),
            MentionUri::File {
                abs_path: PathBuf::from(util::path!("/project/src/main.rs")),
            }
        );

        assert!(cx.update(|cx| resolver.try_resolve("String", cx)).is_none());
        assert!(
            cx.update(|cx| resolver.try_resolve("does/not/exist.rs", cx))
                .is_none()
        );
        assert!(
            cx.update(|cx| resolver.try_resolve("src/main.rs.", cx))
                .is_some()
        );

        let uri = cx
            .update(|cx| resolver.try_resolve("project/src/main.rs:10", cx))
            .expect("expected root-prefixed worktree path to resolve");
        assert_eq!(
            MentionUri::parse(&uri, PathStyle::local()).unwrap(),
            MentionUri::Selection {
                abs_path: Some(PathBuf::from(util::path!("/project/src/main.rs"))),
                line_range: 9..=9,
                column: None,
            }
        );
    }

    #[gpui::test]
    async fn test_notification_for_stop_event(cx: &mut TestAppContext) {
        init_test(cx);

        let (conversation_view, cx) =
            setup_conversation_view(StubAgentServer::default_response(), cx).await;

        let message_editor = message_editor(&conversation_view, cx);
        message_editor.update_in(cx, |editor, window, cx| {
            editor.set_text("Hello", window, cx);
        });

        cx.deactivate_window();

        active_thread(&conversation_view, cx)
            .update_in(cx, |view, window, cx| view.send(window, cx));

        cx.run_until_parked();

        assert!(
            cx.windows()
                .iter()
                .any(|window| window.downcast::<AgentNotification>().is_some())
        );
    }

    #[gpui::test]
    async fn test_no_notification_when_queued_message_will_be_auto_sent(cx: &mut TestAppContext) {
        init_test(cx);

        let connection = StubAgentConnection::new();
        let (conversation_view, cx) =
            setup_conversation_view(StubAgentServer::new(connection.clone()), cx).await;
        add_to_workspace(conversation_view.clone(), cx);

        let message_editor = message_editor(&conversation_view, cx);
        message_editor.update_in(cx, |editor, window, cx| {
            editor.set_text("first", window, cx);
        });

        active_thread(&conversation_view, cx)
            .update_in(cx, |view, window, cx| view.send(window, cx));

        cx.run_until_parked();

        let session_id = conversation_view.read_with(cx, |view, cx| {
            view.active_thread()
                .unwrap()
                .read(cx)
                .thread
                .read(cx)
                .session_id()
                .clone()
        });

        active_thread(&conversation_view, cx).update_in(cx, |thread, window, cx| {
            thread
                .add_to_queue(
                    vec![acp::ContentBlock::Text(acp::TextContent::new(
                        "queued".to_string(),
                    ))],
                    vec![],
                    window,
                    cx,
                )
                .expect("queue admission");
        });

        cx.deactivate_window();
        cx.run_until_parked();

        cx.update(|_, cx| {
            connection.send_update(
                session_id.clone(),
                acp::SessionUpdate::AgentMessageChunk(acp::ContentChunk::new(
                    "first response".into(),
                )),
                cx,
            );
            connection.end_turn(session_id, acp::StopReason::EndTurn);
        });

        cx.run_until_parked();

        assert_eq!(
            cx.windows()
                .iter()
                .filter(|window| window.downcast::<AgentNotification>().is_some())
                .count(),
            0,
            "No notification should fire when a queued message will be auto-sent on Stopped"
        );
    }

    #[gpui::test]
    async fn test_queued_message_steer_defaults_off_and_toggles(cx: &mut TestAppContext) {
        init_test(cx);

        let (conversation_view, cx) =
            setup_conversation_view(StubAgentServer::default_response(), cx).await;
        add_to_workspace(conversation_view.clone(), cx);

        let id = active_thread(&conversation_view, cx).update_in(cx, |thread, window, cx| {
            thread
                .add_to_queue(
                    vec![acp::ContentBlock::Text(acp::TextContent::new(
                        "queued".to_string(),
                    ))],
                    vec![],
                    window,
                    cx,
                )
                .expect("queue admission");
            thread.message_queue.first_id().unwrap()
        });
        cx.run_until_parked();

        // Default: steering is off, so the message waits for end-of-generation
        // rather than interrupting the agent at the next boundary.
        active_thread(&conversation_view, cx).read_with(cx, |thread, _cx| {
            assert!(
                !thread.message_queue.front_wants_steer(),
                "steering should default off"
            );
        });

        active_thread(&conversation_view, cx).update(cx, |thread, _cx| {
            thread
                .message_queue
                .toggle_steer(id)
                .expect("steer preference persisted");
        });
        active_thread(&conversation_view, cx).read_with(cx, |thread, _cx| {
            assert!(
                thread.message_queue.front_wants_steer(),
                "steering should be on after toggling"
            );
        });
    }

    #[gpui::test]
    async fn queued_image_is_kept_in_memory_and_dispatches_without_an_error(
        cx: &mut TestAppContext,
    ) {
        init_test(cx);

        let (conversation_view, cx) =
            setup_conversation_view(StubAgentServer::default_response(), cx).await;
        add_to_workspace(conversation_view.clone(), cx);

        let image = acp::ContentBlock::Image(acp::ImageContent::new(
            "iVBORw0KGgo=".to_string(),
            "image/png".to_string(),
        ));
        active_thread(&conversation_view, cx).update_in(cx, |thread, window, cx| {
            thread
                .add_to_queue(vec![image.clone()], Vec::new(), window, cx)
                .expect("rich input should enter the in-memory queue");
            let id = thread.message_queue.first_id().expect("queued image");
            let entry = thread.message_queue.first().expect("queued image entry");
            assert!(entry.durable_item_id.is_none());
            assert!(entry.can_dispatch);
            assert_eq!(thread.message_queue.send_now(id, false), Ok(Some(id)));
            let promoted = thread
                .message_queue
                .promote_for_dispatch(id, omega_front_door::Quiescence::Proven)
                .expect("in-memory queue promotion")
                .expect("queued image promoted");
            assert_eq!(promoted.content, vec![image]);
        });
    }

    #[gpui::test]
    async fn failed_queued_edit_cannot_send_stale_text_and_moves_new_text_to_composer(
        cx: &mut TestAppContext,
    ) {
        init_test(cx);
        let directory = tempfile::tempdir().expect("temporary queue directory");
        let journal_path = directory.path().join("queue.json");
        let journal = Rc::new(crate::omega_send_queue::SendQueueJournal::at(
            journal_path.clone(),
        ));
        cx.update(|cx| {
            crate::omega_send_queue::SendQueueJournal::set_global_for_tests(journal.clone(), cx);
        });

        let (conversation_view, cx) =
            setup_conversation_view(StubAgentServer::default_response(), cx).await;
        let thread = active_thread(&conversation_view, cx);
        let id = thread.update_in(cx, |thread, window, cx| {
            thread
                .add_to_queue(
                    vec![acp::ContentBlock::Text(acp::TextContent::new("old body"))],
                    Vec::new(),
                    window,
                    cx,
                )
                .expect("initial queue admission");
            thread.message_queue.first_id().expect("queued item")
        });

        std::fs::remove_file(&journal_path).expect("remove queue journal");
        std::fs::create_dir(&journal_path).expect("make journal rewrite fail");
        thread.update_in(cx, |thread, window, cx| {
            let new_content = vec![acp::ContentBlock::Text(acp::TextContent::new(
                "new visible body",
            ))];
            let editor = thread
                .message_queue
                .entry_by_id(id)
                .expect("queued item")
                .editor
                .clone();
            editor.update(cx, |editor, cx| {
                editor.set_message(new_content.clone(), window, cx);
            });
            assert!(matches!(
                thread.message_queue.update(id, new_content, Vec::new()),
                Err(MessageQueueError::Journal(
                    crate::omega_send_queue::SendQueueRefusal::NotPersisted
                ))
            ));
            assert!(
                matches!(
                    thread.message_queue.send_now(id, false),
                    Err(MessageQueueError::UnsavedEntry)
                ),
                "Send Now must not dispatch the stale durable body under newer visible text"
            );
        });

        std::fs::remove_dir(&journal_path).expect("restore writable journal path");
        let temporary_path = journal_path.with_extension("json.tmp");
        if temporary_path.exists() {
            std::fs::remove_file(temporary_path).expect("remove failed rewrite temporary");
        }
        thread.update_in(cx, |thread, window, cx| {
            assert!(thread.move_queued_message_to_main_editor(id, None, None, window, cx));
            assert!(thread.message_queue.is_empty());
            assert_eq!(thread.message_editor.read(cx).text(cx), "new visible body");
        });
        assert!(
            journal
                .open_items(
                    &thread.read_with(cx, |thread, _cx| { thread.root_thread_id.to_key_string() })
                )
                .is_empty()
        );
    }

    #[gpui::test]
    async fn restored_queues_are_rehydrated_by_logical_thread_without_crossing(
        cx: &mut TestAppContext,
    ) {
        init_test(cx);
        let directory = tempfile::tempdir().expect("temporary queue directory");
        let journal = Rc::new(crate::omega_send_queue::SendQueueJournal::at(
            directory.path().join("queue.json"),
        ));
        let first_thread_id = ThreadId::new();
        let second_thread_id = ThreadId::new();
        for (thread_id, item_id, text) in [
            (first_thread_id, "first-item", "first thread only"),
            (second_thread_id, "second-item", "second thread only"),
        ] {
            let thread_key = thread_id.to_key_string();
            journal
                .admit(
                    &thread_key,
                    item_id,
                    text,
                    omega_front_door::SendCommand::Enqueue,
                    omega_front_door::ExecutorClass::ExternalAcp,
                    omega_front_door::SteerCapability::Unknown,
                )
                .expect("queue item persisted");
            journal
                .set_processing_state(
                    &thread_key,
                    crate::omega_send_queue::SendQueueProcessingState::Paused,
                )
                .expect("paused recovery state persisted");
        }
        cx.update(|cx| {
            crate::omega_send_queue::SendQueueJournal::set_global_for_tests(journal, cx);
        });

        let (first, cx) = setup_conversation_view_for_agent_without_settling(
            StubAgentServer::default_response(),
            Agent::Custom { id: "Test".into() },
            None,
            Some(first_thread_id),
            cx,
        )
        .await;
        cx.run_until_parked();

        let (workspace, project, thread_store) = first.read_with(cx, |view, _cx| {
            (
                view.workspace.clone(),
                view.project.clone(),
                view.thread_store.clone(),
            )
        });
        let connection_store =
            cx.update(|_window, cx| cx.new(|cx| AgentConnectionStore::new(project.clone(), cx)));
        let second = cx.update(|window, cx| {
            cx.new(|cx| {
                ConversationView::new(
                    Rc::new(StubAgentServer::default_response()),
                    connection_store,
                    Agent::Custom { id: "Test".into() },
                    None,
                    Some(second_thread_id),
                    None,
                    None,
                    None,
                    workspace,
                    project,
                    thread_store,
                    AgentThreadSource::AgentPanel,
                    window,
                    cx,
                )
            })
        });
        cx.run_until_parked();

        let queued_text = |view: &Entity<ConversationView>, cx: &VisualTestContext| {
            active_thread(view, cx).read_with(cx, |thread, _cx| {
                let entry = thread.message_queue.first().expect("restored queue item");
                match entry.content.first() {
                    Some(acp::ContentBlock::Text(text)) => text.text.clone(),
                    _ => panic!("restored durable queue item must be plain text"),
                }
            })
        };
        assert_eq!(queued_text(&first, cx), "first thread only");
        assert_eq!(queued_text(&second, cx), "second thread only");
    }

    #[gpui::test]
    async fn test_queue_resumes_after_stop_and_new_message(cx: &mut TestAppContext) {
        init_test(cx);

        let connection = StubAgentConnection::new();
        let (conversation_view, cx) =
            setup_conversation_view(StubAgentServer::new(connection.clone()), cx).await;
        add_to_workspace(conversation_view.clone(), cx);

        let message_editor = message_editor(&conversation_view, cx);
        message_editor.update_in(cx, |editor, window, cx| {
            editor.set_text("first", window, cx);
        });
        active_thread(&conversation_view, cx)
            .update_in(cx, |view, window, cx| view.send(window, cx));
        cx.run_until_parked();

        // Queue a follow-up while the agent is generating.
        active_thread(&conversation_view, cx).update_in(cx, |thread, window, cx| {
            thread
                .add_to_queue(
                    vec![acp::ContentBlock::Text(acp::TextContent::new(
                        "queued".to_string(),
                    ))],
                    vec![],
                    window,
                    cx,
                )
                .expect("queue admission");
        });

        // User stops generation: the queued message must NOT be sent.
        active_thread(&conversation_view, cx)
            .update_in(cx, |thread, _window, cx| thread.cancel_generation(cx));
        cx.run_until_parked();

        let queue_len = active_thread(&conversation_view, cx)
            .read_with(cx, |thread, _cx| thread.message_queue.len());
        assert_eq!(queue_len, 1, "stopping must not send the queued message");

        // User sends a new message, which should resume queue auto-processing.
        message_editor.update_in(cx, |editor, window, cx| {
            editor.set_text("second", window, cx);
        });
        active_thread(&conversation_view, cx)
            .update_in(cx, |view, window, cx| view.send(window, cx));
        cx.run_until_parked();

        let session_id = conversation_view.read_with(cx, |view, cx| {
            view.active_thread()
                .unwrap()
                .read(cx)
                .thread
                .read(cx)
                .session_id()
                .clone()
        });

        // When this generation completes, the queued message should be picked
        // up automatically (regression test for the "frozen queue" bug).
        connection.end_turn(session_id, acp::StopReason::EndTurn);
        cx.run_until_parked();

        let queue_len = active_thread(&conversation_view, cx)
            .read_with(cx, |thread, _cx| thread.message_queue.len());
        assert_eq!(
            queue_len, 0,
            "queued message should be auto-sent after the user re-engages"
        );
    }

    #[gpui::test]
    async fn failed_queue_pause_does_not_cancel_the_running_turn(cx: &mut TestAppContext) {
        init_test(cx);
        let directory = tempfile::tempdir().expect("temporary queue directory");
        let journal_path = directory.path().join("queue.json");
        let journal = Rc::new(crate::omega_send_queue::SendQueueJournal::at(
            journal_path.clone(),
        ));
        cx.update(|cx| {
            crate::omega_send_queue::SendQueueJournal::set_global_for_tests(journal, cx);
        });

        let (conversation_view, cx) =
            setup_conversation_view(StubAgentServer::new(StubAgentConnection::new()), cx).await;
        let message_editor = message_editor(&conversation_view, cx);
        message_editor.update_in(cx, |editor, window, cx| {
            editor.set_text("running turn", window, cx);
        });
        let thread = active_thread(&conversation_view, cx);
        thread.update_in(cx, |thread, window, cx| thread.send(window, cx));
        cx.run_until_parked();
        thread.update_in(cx, |thread, window, cx| {
            thread
                .add_to_queue(
                    vec![acp::ContentBlock::Text(acp::TextContent::new(
                        "must remain queued",
                    ))],
                    Vec::new(),
                    window,
                    cx,
                )
                .expect("queue admission");
        });

        std::fs::remove_file(&journal_path).expect("remove queue journal");
        std::fs::create_dir(&journal_path).expect("make pause persistence fail");
        thread.update(cx, |thread, cx| thread.cancel_generation(cx));
        cx.run_until_parked();

        thread.read_with(cx, |thread, cx| {
            assert_eq!(thread.thread.read(cx).status(), ThreadStatus::Generating);
            assert_eq!(thread.message_queue.len(), 1);
            assert!(thread.thread_error.is_some());
        });

        std::fs::remove_dir(&journal_path).expect("restore writable journal path");
        let temporary_path = journal_path.with_extension("json.tmp");
        if temporary_path.exists() {
            std::fs::remove_file(temporary_path).expect("remove failed rewrite temporary");
        }
    }

    #[gpui::test]
    async fn test_notification_for_error(cx: &mut TestAppContext) {
        init_test(cx);

        let server = FakeAcpAgentServer::new();
        let (conversation_view, cx) = setup_conversation_view(server.clone(), cx).await;

        let message_editor = message_editor(&conversation_view, cx);
        message_editor.update_in(cx, |editor, window, cx| {
            editor.set_text("Hello", window, cx);
        });

        cx.deactivate_window();
        server.fail_next_prompt();

        active_thread(&conversation_view, cx)
            .update_in(cx, |view, window, cx| view.send(window, cx));

        cx.run_until_parked();

        assert!(
            cx.windows()
                .iter()
                .any(|window| window.downcast::<AgentNotification>().is_some())
        );
    }

    #[gpui::test]
    async fn test_acp_server_exit_keeps_transcript_and_appends_thread_error(
        cx: &mut TestAppContext,
    ) {
        init_test(cx);

        let server = FakeAcpAgentServer::new();
        let close_session_count = server.close_session_count();
        let (conversation_view, cx) = setup_conversation_view(server.clone(), cx).await;

        cx.run_until_parked();

        server.simulate_server_exit();
        cx.run_until_parked();

        // omega#167. This used to assert a transition to
        // `ServerState::LoadError`, which is exactly the defect: replacing the
        // connected state dropped `ConnectedServerState::threads`, so the
        // streamed transcript vanished and the sidebar row opened nothing.
        conversation_view.read_with(cx, |view, cx| {
            assert!(
                matches!(view.server_state, ServerState::Connected(_)),
                "a server exit on a live thread must keep the connected state: \
                 the transcript and the sidebar row's target live there"
            );
            let thread_view = view
                .active_thread()
                .expect("the thread view must survive the server exit");
            let error = thread_view
                .read(cx)
                .thread_error
                .as_ref()
                .expect("the failure must land in the affected thread as an error card");
            assert!(
                matches!(
                    error,
                    ThreadError::Other { message, .. } if message.contains("agent server quit")
                ),
                "the error card must name the server exit, got: {error:?}"
            );
        });
        assert_eq!(
            close_session_count.load(std::sync::atomic::Ordering::SeqCst),
            0,
            "the retained transcript's session must not be torn down when the server dies"
        );
    }

    #[gpui::test]
    async fn test_thread_view_seeds_existing_elicitation_form_state(cx: &mut TestAppContext) {
        init_test(cx);
        cx.update(|cx| {
            cx.update_flags(true, vec![AcpBetaFeatureFlag::NAME.to_string()]);
        });

        let connection = PreloadedElicitationConnection::default();
        let elicitation_id = connection.elicitation_id.clone();
        let (conversation_view, cx) =
            setup_conversation_view(StubAgentServer::new(connection), cx).await;

        let elicitation_id = elicitation_id
            .lock()
            .clone()
            .expect("connection should preload an elicitation");
        let active_thread = active_thread(&conversation_view, cx);
        active_thread.read_with(cx, |thread, _cx| {
            assert!(
                thread.has_elicitation_form_state(&elicitation_id),
                "pending form elicitations that predate ThreadView construction should be usable"
            );
        });
    }

    #[gpui::test]
    async fn test_resume_without_history_adds_notice(cx: &mut TestAppContext) {
        init_test(cx);

        let fs = FakeFs::new(cx.executor());
        let project = Project::test(fs, [], cx).await;
        let (multi_workspace, cx) =
            cx.add_window_view(|window, cx| MultiWorkspace::test_new(project.clone(), window, cx));
        let workspace = multi_workspace.read_with(cx, |mw, _| mw.workspace().clone());

        let thread_store = cx.update(|_window, cx| cx.new(|cx| ThreadStore::new(cx)));
        let connection_store =
            cx.update(|_window, cx| cx.new(|cx| AgentConnectionStore::new(project.clone(), cx)));

        let conversation_view = cx.update(|window, cx| {
            cx.new(|cx| {
                ConversationView::new(
                    Rc::new(StubAgentServer::new(ResumeOnlyAgentConnection)),
                    connection_store,
                    Agent::Custom { id: "Test".into() },
                    Some(acp::SessionId::new("resume-session")),
                    None,
                    None,
                    None,
                    None,
                    workspace.downgrade(),
                    project,
                    Some(thread_store),
                    AgentThreadSource::AgentPanel,
                    window,
                    cx,
                )
            })
        });

        cx.run_until_parked();

        conversation_view.read_with(cx, |view, cx| {
            let state = view.active_thread().unwrap();
            assert!(state.read(cx).resumed_without_history);
            assert_eq!(state.read(cx).list_state.item_count(), 0);
        });
    }

    #[derive(Clone)]
    struct RestoredAvailableCommandsConnection;

    impl AgentConnection for RestoredAvailableCommandsConnection {
        fn agent_id(&self) -> AgentId {
            AgentId::new("restored-available-commands")
        }

        fn telemetry_id(&self) -> SharedString {
            "restored-available-commands".into()
        }

        fn new_session(
            self: Rc<Self>,
            project: Entity<Project>,
            _work_dirs: PathList,
            cx: &mut App,
        ) -> Task<gpui::Result<Entity<AcpThread>>> {
            let thread = build_test_thread(
                self,
                project,
                "RestoredAvailableCommandsConnection",
                acp::SessionId::new("new-session"),
                cx,
            );
            Task::ready(Ok(thread))
        }

        fn supports_load_session(&self) -> bool {
            true
        }

        fn load_session(
            self: Rc<Self>,
            session_id: acp::SessionId,
            project: Entity<Project>,
            _work_dirs: PathList,
            _title: Option<SharedString>,
            cx: &mut App,
        ) -> Task<gpui::Result<Entity<AcpThread>>> {
            let thread = build_test_thread(
                self,
                project,
                "RestoredAvailableCommandsConnection",
                session_id,
                cx,
            );

            thread
                .update(cx, |thread, cx| {
                    thread.handle_session_update(
                        acp::SessionUpdate::AvailableCommandsUpdate(
                            acp::AvailableCommandsUpdate::new(vec![acp::AvailableCommand::new(
                                "help", "Get help",
                            )]),
                        ),
                        cx,
                    )
                })
                .expect("available commands update should succeed");

            Task::ready(Ok(thread))
        }

        fn auth_methods(&self) -> &[acp::AuthMethod] {
            &[]
        }

        fn authenticate(
            &self,
            _method_id: acp::AuthMethodId,
            _cx: &mut App,
        ) -> Task<gpui::Result<()>> {
            Task::ready(Ok(()))
        }

        fn prompt(
            &self,
            _params: acp::PromptRequest,
            _cx: &mut App,
        ) -> Task<gpui::Result<acp::PromptResponse>> {
            Task::ready(Ok(acp::PromptResponse::new(acp::StopReason::EndTurn)))
        }

        fn cancel(&self, _session_id: &acp::SessionId, _cx: &mut App) {}

        fn into_any(self: Rc<Self>) -> Rc<dyn Any> {
            self
        }
    }

    #[gpui::test]
    async fn test_restored_threads_keep_available_commands(cx: &mut TestAppContext) {
        init_test(cx);

        let fs = FakeFs::new(cx.executor());
        let project = Project::test(fs, [], cx).await;
        let (multi_workspace, cx) =
            cx.add_window_view(|window, cx| MultiWorkspace::test_new(project.clone(), window, cx));
        let workspace = multi_workspace.read_with(cx, |mw, _| mw.workspace().clone());

        let thread_store = cx.update(|_window, cx| cx.new(|cx| ThreadStore::new(cx)));
        let connection_store =
            cx.update(|_window, cx| cx.new(|cx| AgentConnectionStore::new(project.clone(), cx)));

        let conversation_view = cx.update(|window, cx| {
            cx.new(|cx| {
                ConversationView::new(
                    Rc::new(StubAgentServer::new(RestoredAvailableCommandsConnection)),
                    connection_store,
                    Agent::Custom { id: "Test".into() },
                    Some(acp::SessionId::new("restored-session")),
                    None,
                    None,
                    None,
                    None,
                    workspace.downgrade(),
                    project,
                    Some(thread_store),
                    AgentThreadSource::AgentPanel,
                    window,
                    cx,
                )
            })
        });

        cx.run_until_parked();

        let message_editor = message_editor(&conversation_view, cx);
        let editor =
            message_editor.update(cx, |message_editor, _cx| message_editor.editor().clone());
        let placeholder = editor.update(cx, |editor, cx| editor.placeholder_text(cx));

        active_thread(&conversation_view, cx).read_with(cx, |view, _cx| {
            let available_commands = view
                .session_capabilities
                .read()
                .available_commands()
                .to_vec();
            assert_eq!(available_commands.len(), 1);
            assert_eq!(available_commands[0].name.as_str(), "help");
            assert_eq!(available_commands[0].description.as_str(), "Get help");
        });

        assert_eq!(placeholder, Some("Message Test".to_string()));

        message_editor.update_in(cx, |editor, window, cx| {
            editor.set_text("/help", window, cx);
        });

        let contents_result = message_editor
            .update(cx, |editor, cx| editor.contents(false, cx))
            .await;

        assert!(contents_result.is_ok());
    }

    #[gpui::test]
    async fn test_resume_thread_uses_session_cwd_when_inside_project(cx: &mut TestAppContext) {
        init_test(cx);

        let fs = FakeFs::new(cx.executor());
        fs.insert_tree(
            "/project",
            json!({
                "subdir": {
                    "file.txt": "hello"
                }
            }),
        )
        .await;
        let project = Project::test(fs, [Path::new("/project")], cx).await;
        let (multi_workspace, cx) =
            cx.add_window_view(|window, cx| MultiWorkspace::test_new(project.clone(), window, cx));
        let workspace = multi_workspace.read_with(cx, |mw, _| mw.workspace().clone());

        let connection = CwdCapturingConnection::new();
        let captured_cwd = connection.captured_work_dirs.clone();

        let thread_store = cx.update(|_window, cx| cx.new(|cx| ThreadStore::new(cx)));
        let connection_store =
            cx.update(|_window, cx| cx.new(|cx| AgentConnectionStore::new(project.clone(), cx)));

        let _conversation_view = cx.update(|window, cx| {
            cx.new(|cx| {
                ConversationView::new(
                    Rc::new(StubAgentServer::new(connection)),
                    connection_store,
                    Agent::Custom { id: "Test".into() },
                    Some(acp::SessionId::new("session-1")),
                    None,
                    Some(PathList::new(&[PathBuf::from("/project/subdir")])),
                    None,
                    None,
                    workspace.downgrade(),
                    project,
                    Some(thread_store),
                    AgentThreadSource::AgentPanel,
                    window,
                    cx,
                )
            })
        });

        cx.run_until_parked();

        assert_eq!(
            captured_cwd.lock().as_ref().unwrap(),
            &PathList::new(&[Path::new("/project/subdir")]),
            "Should use session cwd when it's inside the project"
        );
    }

    #[gpui::test]
    async fn test_refusal_handling(cx: &mut TestAppContext) {
        init_test(cx);

        let (conversation_view, cx) =
            setup_conversation_view(StubAgentServer::new(RefusalAgentConnection), cx).await;

        let message_editor = message_editor(&conversation_view, cx);
        message_editor.update_in(cx, |editor, window, cx| {
            editor.set_text("Do something harmful", window, cx);
        });

        active_thread(&conversation_view, cx)
            .update_in(cx, |view, window, cx| view.send(window, cx));

        cx.run_until_parked();

        // Check that the refusal error is set
        conversation_view.read_with(cx, |thread_view, cx| {
            let state = thread_view.active_thread().unwrap();
            assert!(
                matches!(state.read(cx).thread_error, Some(ThreadError::Refusal)),
                "Expected refusal error to be set"
            );
        });
    }

    #[gpui::test]
    async fn test_connect_failure_transitions_to_load_error(cx: &mut TestAppContext) {
        init_test(cx);

        let (conversation_view, cx) = setup_conversation_view(FailingAgentServer, cx).await;

        conversation_view.read_with(cx, |view, cx| {
            let title = view.title(cx);
            assert_eq!(
                title.as_ref(),
                "Error Loading Codex CLI",
                "Tab title should show the agent name with an error prefix"
            );
            match &view.server_state {
                ServerState::LoadError {
                    error: LoadError::Other(msg),
                    ..
                } => {
                    assert!(
                        msg.contains("Invalid gzip header"),
                        "Error callout should contain the underlying extraction error, got: {msg}"
                    );
                }
                other => panic!(
                    "Expected LoadError::Other, got: {}",
                    match other {
                        ServerState::Loading { .. } => "Loading (stuck!)",
                        ServerState::LoadError { .. } => "LoadError (wrong variant)",
                        ServerState::Connected(_) => "Connected",
                    }
                ),
            }
            assert!(matches!(
                view.preparation_state(cx),
                ConversationPreparation::SetupRequired { .. }
            ));
        });
    }

    #[gpui::test]
    async fn test_reset_preserves_session_id_after_load_error(cx: &mut TestAppContext) {
        use crate::thread_metadata_store::{ThreadId, ThreadMetadata};
        use chrono::Utc;
        use project::{AgentId as ProjectAgentId, WorktreePaths};
        use std::sync::atomic::Ordering;

        init_test(cx);

        let fs = FakeFs::new(cx.executor());
        let project = Project::test(fs, [], cx).await;
        let (multi_workspace, cx) =
            cx.add_window_view(|window, cx| MultiWorkspace::test_new(project.clone(), window, cx));
        let workspace = multi_workspace.read_with(cx, |mw, _| mw.workspace().clone());

        let thread_store = cx.update(|_window, cx| cx.new(|cx| ThreadStore::new(cx)));
        let connection_store =
            cx.update(|_window, cx| cx.new(|cx| AgentConnectionStore::new(project.clone(), cx)));

        // Simulate a previous run that persisted metadata for this session.
        let resume_session_id = acp::SessionId::new("persistent-session");
        let persisted_thread_id = ThreadId::new();
        let stored_title: SharedString = "Persistent chat".into();
        cx.update(|_window, cx| {
            ThreadMetadataStore::global(cx).update(cx, |store, cx| {
                store.save(
                    ThreadMetadata {
                        thread_id: persisted_thread_id,
                        session_id: Some(resume_session_id.clone()),
                        agent_id: ProjectAgentId::new("Flaky"),
                        conversation_owner_version:
                            crate::thread_metadata_store::ConversationOwnerVersion::V1,
                        title: Some(stored_title.clone()),
                        title_override: None,
                        updated_at: Utc::now(),
                        created_at: Some(Utc::now()),
                        interacted_at: None,
                        worktree_paths: WorktreePaths::from_folder_paths(&PathList::default()),
                        remote_connection: None,
                        archived: false,
                        lifecycle:
                            crate::omega_agent_supervision::SupervisedThreadLifecycle::Failed,
                    },
                    cx,
                );
            });
        });

        let connection = StubAgentConnection::new().with_supports_load_session(true);
        let (server, fail) = FlakyAgentServer::new(connection);

        let conversation_view = cx.update(|window, cx| {
            cx.new(|cx| {
                ConversationView::new(
                    Rc::new(server),
                    connection_store,
                    Agent::Custom { id: "Flaky".into() },
                    Some(resume_session_id.clone()),
                    Some(persisted_thread_id),
                    None,
                    None,
                    None,
                    workspace.downgrade(),
                    project.clone(),
                    Some(thread_store),
                    AgentThreadSource::AgentPanel,
                    window,
                    cx,
                )
            })
        });
        cx.run_until_parked();

        // The first connect() fails, so we land in LoadError.
        conversation_view.read_with(cx, |view, _cx| {
            assert!(
                matches!(view.server_state, ServerState::LoadError { .. }),
                "expected LoadError after failed initial connect"
            );
            assert_eq!(
                view.root_session_id.as_ref(),
                Some(&resume_session_id),
                "root_session_id should still hold the original id while in LoadError"
            );
        });

        // Now let the agent come online and emit AgentServersUpdated. This is
        // the moment the bug would have stomped on root_session_id.
        fail.store(false, Ordering::SeqCst);
        project.update(cx, |project, cx| {
            project
                .agent_server_store()
                .update(cx, |_store, cx| cx.emit(project::AgentServersUpdated));
        });
        cx.run_until_parked();

        // The retry should have resumed the ORIGINAL session, not created a
        // brand-new one.
        conversation_view.read_with(cx, |view, cx| {
            let connected = view
                .as_connected()
                .expect("should be Connected after flaky server comes online");
            let active_id = connected
                .active_id
                .as_ref()
                .expect("Connected state should have an active_id");
            assert_eq!(
                active_id, &resume_session_id,
                "reset() must resume the original session id, not call new_session()"
            );
            let active_thread = view
                .active_thread()
                .expect("should have an active thread view");
            let thread_session = active_thread.read(cx).thread.read(cx).session_id().clone();
            assert_eq!(
                thread_session, resume_session_id,
                "the live AcpThread should hold the resumed session id"
            );
            assert_eq!(
                active_thread.read(cx).thread.read(cx).terminal_status(),
                acp_thread::ThreadTerminalStatus::Failed,
                "a failed terminal state must survive reopening"
            );
        });

        cx.update(|window, cx| {
            ThreadMetadataStore::global(cx).update(cx, |store, cx| {
                let mut metadata = store
                    .entry(persisted_thread_id)
                    .cloned()
                    .expect("persisted metadata should remain available");
                metadata.lifecycle =
                    crate::omega_agent_supervision::SupervisedThreadLifecycle::Cancelled;
                store.save(metadata, cx);
            });
            conversation_view.update(cx, |view, cx| view.reset(window, cx));
        });
        cx.run_until_parked();
        conversation_view.read_with(cx, |view, cx| {
            let active_thread = view
                .active_thread()
                .expect("cancelled thread should reopen");
            assert_eq!(
                active_thread.read(cx).thread.read(cx).terminal_status(),
                acp_thread::ThreadTerminalStatus::Cancelled,
                "a cancelled terminal state must survive reopening"
            );
        });
    }

    #[gpui::test]
    async fn test_auth_required_on_initial_connect(cx: &mut TestAppContext) {
        init_test(cx);

        let connection = AuthGatedAgentConnection::new();
        let (conversation_view, cx) =
            setup_conversation_view(StubAgentServer::new(connection), cx).await;

        // When new_session returns AuthRequired, the server should transition
        // to Connected + Unauthenticated rather than getting stuck in Loading.
        conversation_view.read_with(cx, |view, cx| {
            let connected = view
                .as_connected()
                .expect("Should be in Connected state even though auth is required");
            assert!(
                !connected.auth_state.is_ok(),
                "Auth state should be Unauthenticated"
            );
            assert!(
                !view.supports_logout(),
                "Logout should be hidden while unauthenticated"
            );
            assert!(
                connected.active_id.is_none(),
                "There should be no active thread since no session was created"
            );
            assert!(
                !view.active_thread_renders_request_elicitations(),
                "request elicitations should render outside ThreadView when no thread exists"
            );
            assert!(
                connected.threads.is_empty(),
                "There should be no threads since no session was created"
            );
            assert!(matches!(
                view.preparation_state(cx),
                ConversationPreparation::SetupRequired { .. }
            ));
        });

        conversation_view.read_with(cx, |view, _cx| {
            assert!(
                view.active_thread().is_none(),
                "active_thread() should be None when unauthenticated without a session"
            );
        });

        // Authenticate using the real authenticate flow on ConnectionView.
        // This calls connection.authenticate(), which flips the internal flag,
        // then on success triggers reset() -> new_session() which now succeeds.
        conversation_view.update_in(cx, |view, window, cx| {
            view.authenticate(
                acp::AuthMethodId::new(AuthGatedAgentConnection::AUTH_METHOD_ID),
                window,
                cx,
            );
        });
        cx.run_until_parked();

        // After auth, the server should have an active thread in the Ok state.
        conversation_view.read_with(cx, |view, cx| {
            let connected = view
                .as_connected()
                .expect("Should still be in Connected state after auth");
            assert!(connected.auth_state.is_ok(), "Auth state should be Ok");
            assert!(
                view.supports_logout(),
                "Logout should be available after authentication"
            );
            assert!(
                connected.active_id.is_some(),
                "There should be an active thread after successful auth"
            );
            assert!(
                view.active_thread_renders_request_elicitations(),
                "request elicitations should render inside ThreadView while authenticated"
            );
            assert_eq!(
                connected.threads.len(),
                1,
                "There should be exactly one thread"
            );

            let active = view
                .active_thread()
                .expect("active_thread() should return the new thread");
            assert!(
                active.read(cx).thread_error.is_none(),
                "The new thread should have no errors"
            );
            assert!(matches!(
                view.preparation_state(cx),
                ConversationPreparation::Ready { .. }
            ));
        });

        conversation_view.update_in(cx, |view, window, cx| view.logout(window, cx));
        cx.run_until_parked();

        conversation_view.read_with(cx, |view, _cx| {
            let connected = view
                .as_connected()
                .expect("Should still be in Connected state after logout");
            assert!(
                !connected.auth_state.is_ok(),
                "Auth state should be Unauthenticated after logout"
            );
            assert!(
                !view.supports_logout(),
                "Logout should be hidden after logout"
            );
            assert!(
                view.active_thread().is_some(),
                "The existing thread should still exist after logout"
            );
            assert!(
                !view.active_thread_renders_request_elicitations(),
                "Unauthenticated auth UI should render request elicitations outside ThreadView"
            );
        });
    }

    #[gpui::test]
    async fn test_notification_for_tool_authorization(cx: &mut TestAppContext) {
        init_test(cx);

        let tool_call_id = acp::ToolCallId::new("1");
        let tool_call = acp::ToolCall::new(tool_call_id.clone(), "Label")
            .kind(acp::ToolKind::Edit)
            .content(vec!["hi".into()]);
        let connection =
            StubAgentConnection::new().with_permission_requests(HashMap::from_iter([(
                tool_call_id,
                PermissionOptions::Flat(vec![acp::PermissionOption::new(
                    "1",
                    "Allow",
                    acp::PermissionOptionKind::AllowOnce,
                )]),
            )]));

        connection.set_next_prompt_updates(vec![acp::SessionUpdate::ToolCall(tool_call)]);

        let (conversation_view, cx) =
            setup_conversation_view(StubAgentServer::new(connection), cx).await;

        let message_editor = message_editor(&conversation_view, cx);
        message_editor.update_in(cx, |editor, window, cx| {
            editor.set_text("Hello", window, cx);
        });

        cx.deactivate_window();

        active_thread(&conversation_view, cx)
            .update_in(cx, |view, window, cx| view.send(window, cx));

        cx.run_until_parked();

        assert!(
            cx.windows()
                .iter()
                .any(|window| window.downcast::<AgentNotification>().is_some())
        );
    }

    #[gpui::test]
    async fn test_notification_when_panel_hidden(cx: &mut TestAppContext) {
        init_test(cx);

        let (conversation_view, cx) =
            setup_conversation_view(StubAgentServer::default_response(), cx).await;

        add_to_workspace(conversation_view.clone(), cx);

        let message_editor = message_editor(&conversation_view, cx);

        message_editor.update_in(cx, |editor, window, cx| {
            editor.set_text("Hello", window, cx);
        });

        // Window is active (don't deactivate), but panel will be hidden
        // Note: In the test environment, the panel is not actually added to the dock,
        // so is_agent_panel_hidden will return true

        active_thread(&conversation_view, cx)
            .update_in(cx, |view, window, cx| view.send(window, cx));

        cx.run_until_parked();

        // Should show notification because window is active but panel is hidden
        assert!(
            cx.windows()
                .iter()
                .any(|window| window.downcast::<AgentNotification>().is_some()),
            "Expected notification when panel is hidden"
        );
    }

    #[gpui::test]
    async fn test_notification_still_works_when_window_inactive(cx: &mut TestAppContext) {
        init_test(cx);

        let (conversation_view, cx) =
            setup_conversation_view(StubAgentServer::default_response(), cx).await;

        let message_editor = message_editor(&conversation_view, cx);
        message_editor.update_in(cx, |editor, window, cx| {
            editor.set_text("Hello", window, cx);
        });

        // Deactivate window - should show notification regardless of setting
        cx.deactivate_window();

        active_thread(&conversation_view, cx)
            .update_in(cx, |view, window, cx| view.send(window, cx));

        cx.run_until_parked();

        // Should still show notification when window is inactive (existing behavior)
        assert!(
            cx.windows()
                .iter()
                .any(|window| window.downcast::<AgentNotification>().is_some()),
            "Expected notification when window is inactive"
        );
    }

    #[gpui::test]
    async fn test_notification_when_different_conversation_is_active_in_visible_panel(
        cx: &mut TestAppContext,
    ) {
        init_test(cx);

        let fs = FakeFs::new(cx.executor());

        cx.update(|cx| {
            cx.update_flags(true, vec!["agent-v2".to_string()]);
            agent::ThreadStore::init_global(cx);
            language_model::LanguageModelRegistry::test(cx);
            <dyn Fs>::set_global(fs.clone(), cx);
        });

        let project = Project::test(fs, [], cx).await;
        let multi_workspace_handle =
            cx.add_window(|window, cx| MultiWorkspace::test_new(project.clone(), window, cx));

        let workspace = multi_workspace_handle
            .read_with(cx, |mw, _cx| mw.workspace().clone())
            .unwrap();

        let cx = &mut VisualTestContext::from_window(multi_workspace_handle.into(), cx);

        let panel = workspace.update_in(cx, |workspace, window, cx| {
            let panel = cx.new(|cx| crate::AgentPanel::new(workspace, window, cx));
            workspace.add_panel(panel.clone(), window, cx);
            workspace.focus_panel::<crate::AgentPanel>(window, cx);
            panel
        });

        cx.run_until_parked();

        panel.update_in(cx, |panel, window, cx| {
            panel.open_external_thread_with_server(
                Rc::new(StubAgentServer::default_response()),
                window,
                cx,
            );
        });

        cx.run_until_parked();

        panel.read_with(cx, |panel, cx| {
            assert!(crate::AgentPanel::is_visible(&workspace, cx));
            assert!(panel.active_conversation_view().is_some());
        });

        let thread_store = cx.update(|_window, cx| cx.new(|cx| ThreadStore::new(cx)));
        let connection_store =
            cx.update(|_window, cx| cx.new(|cx| AgentConnectionStore::new(project.clone(), cx)));

        let conversation_view = cx.update(|window, cx| {
            cx.new(|cx| {
                ConversationView::new(
                    Rc::new(StubAgentServer::default_response()),
                    connection_store,
                    Agent::Custom { id: "Test".into() },
                    None,
                    None,
                    None,
                    None,
                    None,
                    workspace.downgrade(),
                    project.clone(),
                    Some(thread_store),
                    AgentThreadSource::AgentPanel,
                    window,
                    cx,
                )
            })
        });

        cx.run_until_parked();

        panel.read_with(cx, |panel, _cx| {
            assert_ne!(
                panel
                    .active_conversation_view()
                    .map(|view| view.entity_id()),
                Some(conversation_view.entity_id()),
                "The visible panel should still be showing a different conversation"
            );
        });

        let message_editor = message_editor(&conversation_view, cx);
        message_editor.update_in(cx, |editor, window, cx| {
            editor.set_text("Hello", window, cx);
        });

        active_thread(&conversation_view, cx)
            .update_in(cx, |view, window, cx| view.send(window, cx));

        cx.run_until_parked();

        assert!(
            cx.windows()
                .iter()
                .any(|window| window.downcast::<AgentNotification>().is_some()),
            "Expected notification when a different conversation is active in the visible panel"
        );
    }

    #[gpui::test]
    async fn test_no_notification_when_sidebar_open_but_different_thread_focused(
        cx: &mut TestAppContext,
    ) {
        init_test(cx);

        let fs = FakeFs::new(cx.executor());

        cx.update(|cx| {
            cx.update_flags(true, vec!["agent-v2".to_string()]);
            agent::ThreadStore::init_global(cx);
            language_model::LanguageModelRegistry::test(cx);
            <dyn Fs>::set_global(fs.clone(), cx);
        });

        let project = Project::test(fs, [], cx).await;
        let multi_workspace_handle =
            cx.add_window(|window, cx| MultiWorkspace::test_new(project.clone(), window, cx));

        let workspace = multi_workspace_handle
            .read_with(cx, |mw, _cx| mw.workspace().clone())
            .unwrap();

        let cx = &mut VisualTestContext::from_window(multi_workspace_handle.into(), cx);
        register_test_sidebar(true, cx);

        // Open the sidebar so that sidebar_open() returns true.
        multi_workspace_handle
            .update(cx, |mw, _window, cx| {
                mw.open_sidebar(cx);
            })
            .unwrap();

        cx.run_until_parked();

        assert!(
            multi_workspace_handle
                .read_with(cx, |mw, _cx| mw.sidebar_open())
                .unwrap(),
            "Sidebar should be open"
        );

        // Create a conversation view that is NOT the active one in the panel.
        let thread_store = cx.update(|_window, cx| cx.new(|cx| ThreadStore::new(cx)));
        let connection_store =
            cx.update(|_window, cx| cx.new(|cx| AgentConnectionStore::new(project.clone(), cx)));

        let conversation_view = cx.update(|window, cx| {
            cx.new(|cx| {
                ConversationView::new(
                    Rc::new(StubAgentServer::default_response()),
                    connection_store,
                    Agent::Custom { id: "Test".into() },
                    None,
                    None,
                    None,
                    None,
                    None,
                    workspace.downgrade(),
                    project.clone(),
                    Some(thread_store),
                    AgentThreadSource::AgentPanel,
                    window,
                    cx,
                )
            })
        });

        cx.run_until_parked();

        let message_editor = message_editor(&conversation_view, cx);
        message_editor.update_in(cx, |editor, window, cx| {
            editor.set_text("Hello", window, cx);
        });

        active_thread(&conversation_view, cx)
            .update_in(cx, |view, window, cx| view.send(window, cx));

        cx.run_until_parked();

        assert!(
            !cx.windows()
                .iter()
                .any(|window| window.downcast::<AgentNotification>().is_some()),
            "Expected no notification when the sidebar is open, even if focused on another thread"
        );
    }

    #[gpui::test]
    async fn test_notification_when_sidebar_open_but_thread_list_hidden(cx: &mut TestAppContext) {
        init_test(cx);

        let fs = FakeFs::new(cx.executor());

        cx.update(|cx| {
            cx.update_flags(true, vec!["agent-v2".to_string()]);
            agent::ThreadStore::init_global(cx);
            language_model::LanguageModelRegistry::test(cx);
            <dyn Fs>::set_global(fs.clone(), cx);
        });

        let project = Project::test(fs, [], cx).await;
        let multi_workspace_handle =
            cx.add_window(|window, cx| MultiWorkspace::test_new(project.clone(), window, cx));

        let workspace = multi_workspace_handle
            .read_with(cx, |mw, _cx| mw.workspace().clone())
            .unwrap();

        let cx = &mut VisualTestContext::from_window(multi_workspace_handle.into(), cx);
        register_test_sidebar(false, cx);
        multi_workspace_handle
            .update(cx, |mw, _window, cx| {
                mw.open_sidebar(cx);
            })
            .unwrap();
        cx.run_until_parked();

        let thread_store = cx.update(|_window, cx| cx.new(|cx| ThreadStore::new(cx)));
        let connection_store =
            cx.update(|_window, cx| cx.new(|cx| AgentConnectionStore::new(project.clone(), cx)));

        let conversation_view = cx.update(|window, cx| {
            cx.new(|cx| {
                ConversationView::new(
                    Rc::new(StubAgentServer::default_response()),
                    connection_store,
                    Agent::Custom { id: "Test".into() },
                    None,
                    None,
                    None,
                    None,
                    None,
                    workspace.downgrade(),
                    project.clone(),
                    Some(thread_store),
                    AgentThreadSource::AgentPanel,
                    window,
                    cx,
                )
            })
        });
        cx.run_until_parked();

        let message_editor = message_editor(&conversation_view, cx);
        message_editor.update_in(cx, |editor, window, cx| {
            editor.set_text("Hello", window, cx);
        });

        active_thread(&conversation_view, cx)
            .update_in(cx, |view, window, cx| view.send(window, cx));
        cx.run_until_parked();

        assert!(
            cx.windows()
                .iter()
                .any(|window| window.downcast::<AgentNotification>().is_some()),
            "Expected notification when the sidebar is open but the thread list is hidden"
        );
    }

    #[gpui::test]
    async fn test_notification_dismissed_when_sidebar_opens(cx: &mut TestAppContext) {
        init_test(cx);

        let fs = FakeFs::new(cx.executor());

        cx.update(|cx| {
            cx.update_flags(true, vec!["agent-v2".to_string()]);
            agent::ThreadStore::init_global(cx);
            language_model::LanguageModelRegistry::test(cx);
            <dyn Fs>::set_global(fs.clone(), cx);
        });

        let project = Project::test(fs, [], cx).await;
        let multi_workspace_handle =
            cx.add_window(|window, cx| MultiWorkspace::test_new(project.clone(), window, cx));

        let workspace = multi_workspace_handle
            .read_with(cx, |mw, _cx| mw.workspace().clone())
            .unwrap();

        let cx = &mut VisualTestContext::from_window(multi_workspace_handle.into(), cx);
        register_test_sidebar(true, cx);

        let thread_store = cx.update(|_window, cx| cx.new(|cx| ThreadStore::new(cx)));
        let connection_store =
            cx.update(|_window, cx| cx.new(|cx| AgentConnectionStore::new(project.clone(), cx)));

        let conversation_view = cx.update(|window, cx| {
            cx.new(|cx| {
                ConversationView::new(
                    Rc::new(StubAgentServer::default_response()),
                    connection_store,
                    Agent::Custom { id: "Test".into() },
                    None,
                    None,
                    None,
                    None,
                    None,
                    workspace.downgrade(),
                    project.clone(),
                    Some(thread_store),
                    AgentThreadSource::AgentPanel,
                    window,
                    cx,
                )
            })
        });

        cx.run_until_parked();

        let message_editor = message_editor(&conversation_view, cx);
        message_editor.update_in(cx, |editor, window, cx| {
            editor.set_text("Hello", window, cx);
        });

        active_thread(&conversation_view, cx)
            .update_in(cx, |view, window, cx| view.send(window, cx));

        cx.run_until_parked();

        assert_eq!(
            cx.windows()
                .iter()
                .filter(|window| window.downcast::<AgentNotification>().is_some())
                .count(),
            1,
            "Expected a notification while the thread is not visible"
        );

        multi_workspace_handle
            .update(cx, |mw, _window, cx| {
                mw.open_sidebar(cx);
            })
            .unwrap();

        cx.run_until_parked();

        assert_eq!(
            cx.windows()
                .iter()
                .filter(|window| window.downcast::<AgentNotification>().is_some())
                .count(),
            0,
            "Notification should auto-dismiss when the sidebar opens and makes the thread visible"
        );
    }

    #[gpui::test]
    async fn test_notification_when_workspace_is_background_in_multi_workspace(
        cx: &mut TestAppContext,
    ) {
        init_test(cx);

        // Enable multi-workspace feature flag and init globals needed by AgentPanel
        let fs = FakeFs::new(cx.executor());

        cx.update(|cx| {
            agent::ThreadStore::init_global(cx);
            language_model::LanguageModelRegistry::test(cx);
            <dyn Fs>::set_global(fs.clone(), cx);
        });

        let project1 = Project::test(fs.clone(), [], cx).await;

        // Create a MultiWorkspace window with one workspace
        let multi_workspace_handle =
            cx.add_window(|window, cx| MultiWorkspace::test_new(project1.clone(), window, cx));

        // Get workspace 1 (the initial workspace)
        let workspace1 = multi_workspace_handle
            .read_with(cx, |mw, _cx| mw.workspace().clone())
            .unwrap();

        let cx = &mut VisualTestContext::from_window(multi_workspace_handle.into(), cx);

        let panel = workspace1.update_in(cx, |workspace, window, cx| {
            let panel = cx.new(|cx| crate::AgentPanel::new(workspace, window, cx));
            workspace.add_panel(panel.clone(), window, cx);

            // Open the dock and activate the agent panel so it's visible
            workspace.focus_panel::<crate::AgentPanel>(window, cx);
            panel
        });

        cx.run_until_parked();

        panel.update_in(cx, |panel, window, cx| {
            panel.open_external_thread_with_server(
                Rc::new(StubAgentServer::new(RestoredAvailableCommandsConnection)),
                window,
                cx,
            );
        });

        cx.run_until_parked();

        cx.read(|cx| {
            assert!(
                crate::AgentPanel::is_visible(&workspace1, cx),
                "AgentPanel should be visible in workspace1's dock"
            );
        });

        // Set up thread view in workspace 1
        let thread_store = cx.update(|_window, cx| cx.new(|cx| ThreadStore::new(cx)));
        let connection_store =
            cx.update(|_window, cx| cx.new(|cx| AgentConnectionStore::new(project1.clone(), cx)));

        let conversation_view = cx.update(|window, cx| {
            cx.new(|cx| {
                ConversationView::new(
                    Rc::new(StubAgentServer::new(RestoredAvailableCommandsConnection)),
                    connection_store,
                    Agent::Custom { id: "Test".into() },
                    None,
                    None,
                    None,
                    None,
                    None,
                    workspace1.downgrade(),
                    project1.clone(),
                    Some(thread_store),
                    AgentThreadSource::AgentPanel,
                    window,
                    cx,
                )
            })
        });
        cx.run_until_parked();

        let root_session_id = conversation_view
            .read_with(cx, |view, cx| {
                view.root_thread_view()
                    .map(|thread| thread.read(cx).thread.read(cx).session_id().clone())
            })
            .expect("Conversation view should have a root thread");

        let message_editor = message_editor(&conversation_view, cx);
        message_editor.update_in(cx, |editor, window, cx| {
            editor.set_text("Hello", window, cx);
        });

        // Create a second workspace and switch to it.
        // This makes workspace1 the "background" workspace.
        let project2 = Project::test(fs, [], cx).await;
        multi_workspace_handle
            .update(cx, |mw, window, cx| {
                mw.test_add_workspace(project2, window, cx);
            })
            .unwrap();

        cx.run_until_parked();

        // Verify workspace1 is no longer the active workspace
        multi_workspace_handle
            .read_with(cx, |mw, _cx| {
                assert_ne!(mw.workspace(), &workspace1);
            })
            .unwrap();

        // Window is active, agent panel is visible in workspace1, but workspace1
        // is in the background. The notification should show because the user
        // can't actually see the agent panel.
        active_thread(&conversation_view, cx)
            .update_in(cx, |view, window, cx| view.send(window, cx));

        cx.run_until_parked();

        assert!(
            cx.windows()
                .iter()
                .any(|window| window.downcast::<AgentNotification>().is_some()),
            "Expected notification when workspace is in background within MultiWorkspace"
        );

        // Also verify: clicking "View Panel" should switch to workspace1.
        cx.windows()
            .iter()
            .find_map(|window| window.downcast::<AgentNotification>())
            .unwrap()
            .update(cx, |window, _, cx| window.accept(cx))
            .unwrap();

        cx.run_until_parked();

        multi_workspace_handle
            .read_with(cx, |mw, _cx| {
                assert_eq!(
                    mw.workspace(),
                    &workspace1,
                    "Expected workspace1 to become the active workspace after accepting notification"
                );
            })
            .unwrap();

        panel.read_with(cx, |panel, cx| {
            let active_session_id = panel
                .active_agent_thread(cx)
                .map(|thread| thread.read(cx).session_id().clone());
            assert_eq!(
                active_session_id,
                Some(root_session_id),
                "Expected accepting the notification to load the notified thread in AgentPanel"
            );
        });
    }

    #[gpui::test]
    async fn test_notification_respects_never_setting(cx: &mut TestAppContext) {
        init_test(cx);

        // Set notify_when_agent_waiting to Never
        cx.update(|cx| {
            AgentSettings::override_global(
                AgentSettings {
                    notify_when_agent_waiting: NotifyWhenAgentWaiting::Never,
                    ..AgentSettings::get_global(cx).clone()
                },
                cx,
            );
        });

        let (conversation_view, cx) =
            setup_conversation_view(StubAgentServer::default_response(), cx).await;

        let message_editor = message_editor(&conversation_view, cx);
        message_editor.update_in(cx, |editor, window, cx| {
            editor.set_text("Hello", window, cx);
        });

        // Window is active

        active_thread(&conversation_view, cx)
            .update_in(cx, |view, window, cx| view.send(window, cx));

        cx.run_until_parked();

        // Should NOT show notification because notify_when_agent_waiting is Never
        assert!(
            !cx.windows()
                .iter()
                .any(|window| window.downcast::<AgentNotification>().is_some()),
            "Expected no notification when notify_when_agent_waiting is Never"
        );
    }

    #[gpui::test]
    async fn test_notification_closed_when_thread_view_dropped(cx: &mut TestAppContext) {
        init_test(cx);

        let (conversation_view, cx) =
            setup_conversation_view(StubAgentServer::default_response(), cx).await;

        let weak_view = conversation_view.downgrade();

        let message_editor = message_editor(&conversation_view, cx);
        message_editor.update_in(cx, |editor, window, cx| {
            editor.set_text("Hello", window, cx);
        });

        cx.deactivate_window();

        active_thread(&conversation_view, cx)
            .update_in(cx, |view, window, cx| view.send(window, cx));

        cx.run_until_parked();

        // Verify notification is shown
        assert!(
            cx.windows()
                .iter()
                .any(|window| window.downcast::<AgentNotification>().is_some()),
            "Expected notification to be shown"
        );

        // Drop the thread view (simulating navigation to a new thread)
        drop(conversation_view);
        drop(message_editor);
        // Trigger an update to flush effects, which will call release_dropped_entities
        cx.update(|_window, _cx| {});
        cx.run_until_parked();

        // Verify the entity was actually released
        assert!(
            !weak_view.is_upgradable(),
            "Thread view entity should be released after dropping"
        );

        // The notification should be automatically closed via on_release
        assert!(
            !cx.windows()
                .iter()
                .any(|window| window.downcast::<AgentNotification>().is_some()),
            "Notification should be closed when thread view is dropped"
        );
    }

    async fn setup_conversation_view(
        agent: impl AgentServer + 'static,
        cx: &mut TestAppContext,
    ) -> (Entity<ConversationView>, &mut VisualTestContext) {
        setup_conversation_view_with_initial_content_opt(agent, None, cx).await
    }

    #[gpui::test]
    async fn test_completed_plan_snapshot_keeps_list_state_in_sync(cx: &mut TestAppContext) {
        init_test(cx);

        let connection = StubAgentConnection::new();
        let (conversation_view, cx) =
            setup_conversation_view(StubAgentServer::new(connection.clone()), cx).await;

        message_editor(&conversation_view, cx).update_in(cx, |editor, window, cx| {
            editor.set_text("Hello", window, cx);
        });
        active_thread(&conversation_view, cx).update_in(cx, |view, window, cx| {
            view.send(window, cx);
        });
        cx.run_until_parked();

        let session_id = active_thread(&conversation_view, cx).read_with(cx, |view, cx| {
            assert_thread_list_item_count_matches_entries(view, cx);
            view.thread.read(cx).session_id().clone()
        });

        cx.update(|_, cx| {
            connection.send_update(
                session_id.clone(),
                acp::SessionUpdate::Plan(acp::Plan::new(vec![acp::PlanEntry::new(
                    "Do the thing",
                    acp::PlanEntryPriority::Medium,
                    acp::PlanEntryStatus::InProgress,
                )])),
                cx,
            );
        });
        cx.run_until_parked();
        active_thread(&conversation_view, cx).read_with(cx, |view, cx| {
            assert_thread_list_item_count_matches_entries(view, cx);
        });

        cx.update(|_, cx| {
            connection.send_update(
                session_id.clone(),
                acp::SessionUpdate::Plan(acp::Plan::new(vec![acp::PlanEntry::new(
                    "Do the thing",
                    acp::PlanEntryPriority::Medium,
                    acp::PlanEntryStatus::Completed,
                )])),
                cx,
            );
        });
        cx.run_until_parked();
        active_thread(&conversation_view, cx).read_with(cx, |view, cx| {
            assert_thread_list_item_count_matches_entries(view, cx);
        });

        connection.end_turn(session_id, acp::StopReason::EndTurn);
        cx.run_until_parked();
        active_thread(&conversation_view, cx).read_with(cx, |view, cx| {
            assert_thread_list_item_count_matches_entries(view, cx);
        });
    }

    async fn setup_conversation_view_with_initial_content(
        agent: impl AgentServer + 'static,
        initial_content: AgentInitialContent,
        cx: &mut TestAppContext,
    ) -> (Entity<ConversationView>, &mut VisualTestContext) {
        setup_conversation_view_with_initial_content_opt(agent, Some(initial_content), cx).await
    }

    async fn setup_conversation_view_with_initial_content_opt(
        agent: impl AgentServer + 'static,
        initial_content: Option<AgentInitialContent>,
        cx: &mut TestAppContext,
    ) -> (Entity<ConversationView>, &mut VisualTestContext) {
        let (conversation_view, cx) =
            setup_conversation_view_without_settling(agent, initial_content, cx).await;
        cx.run_until_parked();
        (conversation_view, cx)
    }

    /// Like [`setup_conversation_view`], but stops before the executor's
    /// connect task has been polled, so the view is still in
    /// `ServerState::Loading` — the state a person is in when they type into
    /// a brand-new thread and press Enter before the executor is warm.
    async fn setup_conversation_view_still_connecting(
        agent: impl AgentServer + 'static,
        cx: &mut TestAppContext,
    ) -> (Entity<ConversationView>, &mut VisualTestContext) {
        setup_conversation_view_without_settling(agent, None, cx).await
    }

    async fn setup_conversation_view_without_settling(
        agent: impl AgentServer + 'static,
        initial_content: Option<AgentInitialContent>,
        cx: &mut TestAppContext,
    ) -> (Entity<ConversationView>, &mut VisualTestContext) {
        setup_conversation_view_for_agent_without_settling(
            agent,
            Agent::Custom { id: "Test".into() },
            initial_content,
            None,
            cx,
        )
        .await
    }

    async fn setup_conversation_view_for_agent_without_settling(
        agent: impl AgentServer + 'static,
        agent_key: Agent,
        initial_content: Option<AgentInitialContent>,
        thread_id: Option<ThreadId>,
        cx: &mut TestAppContext,
    ) -> (Entity<ConversationView>, &mut VisualTestContext) {
        let fs = FakeFs::new(cx.executor());
        // A project with no worktrees names the home directory as its working
        // directory (`Project::default_path_list`), and a session now opens
        // only where the agent will really find one (OMEGA-DELTA-0158). Without
        // this the fake filesystem contradicts the project standing on it.
        fs.insert_tree(util::paths::home_dir().as_path(), serde_json::json!({}))
            .await;
        let project = Project::test(fs, [], cx).await;
        setup_conversation_view_with_project_without_settling(
            agent,
            agent_key,
            initial_content,
            thread_id,
            project,
            cx,
        )
        .await
    }

    async fn setup_conversation_view_with_project_without_settling(
        agent: impl AgentServer + 'static,
        agent_key: Agent,
        initial_content: Option<AgentInitialContent>,
        thread_id: Option<ThreadId>,
        project: Entity<Project>,
        cx: &mut TestAppContext,
    ) -> (Entity<ConversationView>, &mut VisualTestContext) {
        let (multi_workspace, cx) =
            cx.add_window_view(|window, cx| MultiWorkspace::test_new(project.clone(), window, cx));
        let workspace = multi_workspace.read_with(cx, |mw, _| mw.workspace().clone());

        let thread_store = cx.update(|_window, cx| cx.new(|cx| ThreadStore::new(cx)));
        let connection_store =
            cx.update(|_window, cx| cx.new(|cx| AgentConnectionStore::new(project.clone(), cx)));

        let conversation_view = cx.update(|window, cx| {
            cx.new(|cx| {
                ConversationView::new(
                    Rc::new(agent),
                    connection_store.clone(),
                    agent_key.clone(),
                    None,
                    thread_id,
                    None,
                    None,
                    initial_content,
                    workspace.downgrade(),
                    project,
                    Some(thread_store),
                    AgentThreadSource::AgentPanel,
                    window,
                    cx,
                )
            })
        });

        (conversation_view, cx)
    }

    fn user_message_markdown(thread_view: &Entity<ThreadView>, cx: &TestAppContext) -> Vec<String> {
        thread_view.read_with(cx, |view, cx| {
            view.thread
                .read(cx)
                .entries()
                .iter()
                .filter(|entry| matches!(entry, AgentThreadEntry::UserMessage(_)))
                .map(|entry| entry.to_markdown(cx))
                .collect()
        })
    }

    #[gpui::test]
    async fn initial_auto_submit_waits_for_the_durable_queue_binding(cx: &mut TestAppContext) {
        init_test(cx);
        let connection = StubAgentConnection::new();
        let initial_content = AgentInitialContent::ContentBlock {
            blocks: vec![acp::ContentBlock::Text(acp::TextContent::new(
                "Run the entropy scan now",
            ))],
            auto_submit: true,
        };
        let (conversation_view, cx) = setup_conversation_view_with_initial_content(
            StubAgentServer::new(connection),
            initial_content,
            cx,
        )
        .await;

        let thread_view = active_thread(&conversation_view, cx);
        thread_view.read_with(cx, |view, _cx| {
            assert!(
                view.thread_error.is_none(),
                "auto-submit must not race durable queue configuration: {:?}",
                view.thread_error
            );
            assert!(
                !view.pending_initial_auto_submit,
                "the initial send must be consumed after queue binding"
            );
            assert!(view.message_queue.is_empty());
        });
        assert_eq!(
            user_message_markdown(&thread_view, cx),
            vec!["## User\n\nRun the entropy scan now\n\n".to_string()]
        );
    }

    /// `OMEGA-DELTA-0170`, the core promise. Enter while the executor is
    /// still connecting accepts the message; when the connection lands, every
    /// pending message dispatches automatically, in order, exactly once, with
    /// exactly the text that was typed.
    #[gpui::test]
    async fn a_message_sent_while_connecting_dispatches_once_on_connect(cx: &mut TestAppContext) {
        init_test(cx);

        let connection = StubAgentConnection::new();
        let (conversation_view, cx) =
            setup_conversation_view_still_connecting(StubAgentServer::new(connection.clone()), cx)
                .await;

        conversation_view.update_in(cx, |this, window, cx| {
            assert!(
                matches!(this.server_state, ServerState::Loading { .. }),
                "the executor must still be connecting for this test to mean anything"
            );
            assert_eq!(
                this.preparation_state(cx),
                ConversationPreparation::Loading
            );
            let editor = this.loading_composer(window, cx);
            editor.update(cx, |editor, cx| editor.set_text("hi", window, cx));
            this.submit_while_connecting(window, cx);

            let editor = this.loading_composer(window, cx);
            assert_eq!(
                editor.read(cx).text(cx),
                "",
                "Enter must accept the message: it leaves the composer instead of \
                 staying behind a refusal"
            );
            assert_eq!(
                this.pending_connect_messages
                    .iter()
                    .map(|message| message.text.as_str())
                    .collect::<Vec<_>>(),
                vec!["hi"],
                "the accepted message must be held as a pending turn"
            );

            editor.update(cx, |editor, cx| {
                editor.set_text("and a second one", window, cx)
            });
            this.submit_while_connecting(window, cx);
            assert_eq!(
                this.pending_connect_messages
                    .iter()
                    .map(|message| message.text.as_str())
                    .collect::<Vec<_>>(),
                vec!["hi", "and a second one"],
                "several Enters while connecting must queue in order"
            );
            assert!(
                this.has_unsubmitted_or_pending_content(cx),
                "accepted loading messages must prevent a new-conversation gesture from reusing this view"
            );
        });

        cx.run_until_parked();

        let thread_view = active_thread(&conversation_view, cx);
        assert_eq!(
            user_message_markdown(&thread_view, cx),
            vec!["## User\n\nhi\n\n".to_string()],
            "connecting must dispatch the first pending message automatically, \
             exactly once, with the exact text"
        );
        thread_view.read_with(cx, |view, _| {
            assert_eq!(
                view.message_queue.len(),
                1,
                "the second pending message must wait its turn in the ordinary queue"
            );
        });
        conversation_view.read_with(cx, |this, cx| {
            assert!(
                this.pending_connect_messages.is_empty(),
                "dispatch must take the pending list so nothing can dispatch twice"
            );
            assert!(
                this.has_unsubmitted_or_pending_content(cx),
                "a turn handed to the connected queue must remain protected from reuse"
            );
        });

        let session_id =
            thread_view.read_with(cx, |view, cx| view.thread.read(cx).session_id().clone());
        connection.end_turn(session_id, acp::StopReason::EndTurn);
        cx.run_until_parked();

        assert_eq!(
            user_message_markdown(&thread_view, cx),
            vec![
                "## User\n\nhi\n\n".to_string(),
                "## User\n\nand a second one\n\n".to_string(),
            ],
            "when the first turn stops, the queue must auto-dispatch the second \
             pending message, in order, exactly once"
        );
    }

    /// omega#153. Omega's router is a usable logical destination before an
    /// executor session exists. The first accepted turn is what crosses that
    /// boundary; connecting the router alone must not create a hidden session.
    #[gpui::test]
    async fn omega_defers_physical_session_creation_until_the_first_turn(cx: &mut TestAppContext) {
        init_test(cx);

        let directory = tempfile::tempdir().expect("temporary route journal directory");
        let (conversation_view, cx) = setup_conversation_view_for_agent_without_settling(
            RouterTestServer {
                native: StubAgentConnection::new(),
                journal_path: directory.path().join("routes.json"),
            },
            Agent::NativeAgent,
            None,
            None,
            cx,
        )
        .await;
        cx.run_until_parked();

        conversation_view.read_with(cx, |this, cx| {
            assert_eq!(
                this.preparation_state(cx),
                ConversationPreparation::RouterReady,
                "connecting Omega should prepare its logical router"
            );
            assert!(
                this.root_session_id.is_none() && this.active_thread().is_none(),
                "router readiness must not manufacture a physical executor session"
            );
        });

        conversation_view.update_in(cx, |this, window, cx| {
            let editor = this.loading_composer(window, cx);
            editor.update(cx, |editor, cx| {
                editor.set_text("explain this design", window, cx)
            });
            this.submit_before_session(window, cx);

            assert_eq!(
                this.pending_connect_messages
                    .iter()
                    .map(|message| message.text.as_str())
                    .collect::<Vec<_>>(),
                vec!["explain this design"],
                "the first turn must be accepted before session creation completes"
            );
            assert!(
                this.root_session_id.is_none(),
                "session creation is asynchronous, so the accepted text must not depend on it"
            );
            assert!(
                this.omega_route_summary.is_some() && !this.omega_route_not_yet_recorded(),
                "the recorded first route must freeze the conversation's draft state"
            );
        });
        cx.run_until_parked();

        let thread_view = active_thread(&conversation_view, cx);
        assert_eq!(
            user_message_markdown(&thread_view, cx),
            vec!["## User\n\nexplain this design\n\n".to_string()],
            "the accepted first turn must dispatch once into the selected executor session"
        );
        conversation_view.read_with(cx, |this, cx| {
            assert!(matches!(
                this.preparation_state(cx),
                ConversationPreparation::Ready { .. }
            ));
            assert!(this.pending_connect_messages.is_empty());
        });
    }

    /// omega#217. A launched window with Omega's router ready and no executor
    /// session yet draws the pre-session composer, not the thread composer.
    /// That is the composer macOS assistive technology could not find: the
    /// thread composer carried the text-input node, this one carried nothing,
    /// so the installed tree published the model, voice, and send controls
    /// with no field between them. The assertion is on the same
    /// `accesskit::TreeUpdate` that GPUI hands the platform adapter.
    #[gpui::test]
    async fn pre_session_composer_publishes_its_text_input_node(cx: &mut TestAppContext) {
        init_test(cx);

        let directory = tempfile::tempdir().expect("temporary route journal directory");
        let (conversation_view, cx) = setup_conversation_view_for_agent_without_settling(
            RouterTestServer {
                native: StubAgentConnection::new(),
                journal_path: directory.path().join("routes.json"),
            },
            Agent::NativeAgent,
            None,
            None,
            cx,
        )
        .await;
        cx.run_until_parked();

        conversation_view.read_with(cx, |this, cx| {
            assert_eq!(
                this.preparation_state(cx),
                ConversationPreparation::RouterReady,
                "this test only means something while the pre-session composer is on screen"
            );
            assert!(
                this.active_thread().is_none(),
                "a thread composer would defeat the point of the test"
            );
        });

        add_to_workspace(conversation_view.clone(), cx);
        cx.set_debug_accessibility_active(true);

        let snapshot = cx.debug_render_snapshot();
        let tree = snapshot
            .accessibility_tree_json()
            .expect("forced accessibility should capture the pre-session composer");
        assert!(
            tree.contains("MultilineTextInput") && tree.contains("Message composer"),
            "the pre-session composer must publish its text-input role and name: {tree}"
        );
        assert!(
            tree.contains("SetValue") && tree.contains("Focus"),
            "assistive technology must be able to focus the pre-session composer and \
             set its text: {tree}"
        );
    }

    /// omega#217. The independent VoiceOver pass on `v0.2.0-rc31` found that a
    /// full accessibility dump taken after a completed turn contained **zero**
    /// occurrences of either message: a screen-reader user could send a prompt
    /// and had no way to read the answer. This asserts against the same
    /// `accesskit::TreeUpdate` GPUI hands the macOS adapter.
    #[gpui::test]
    async fn a_completed_turn_publishes_its_transcript(cx: &mut TestAppContext) {
        init_test(cx);

        let connection = StubAgentConnection::new();
        connection.set_next_prompt_updates(vec![acp::SessionUpdate::AgentMessageChunk(
            acp::ContentChunk::new("Hello! I am Omega, glad to meet you.".into()),
        )]);
        let (conversation_view, cx) =
            setup_conversation_view(StubAgentServer::new(connection.clone()), cx).await;
        add_to_workspace(conversation_view.clone(), cx);

        message_editor(&conversation_view, cx).update_in(cx, |editor, window, cx| {
            editor.set_text("Say hello in one short sentence.", window, cx);
        });
        active_thread(&conversation_view, cx).update_in(cx, |view, window, cx| {
            view.send(window, cx);
        });
        cx.run_until_parked();

        cx.set_debug_accessibility_active(true);
        let snapshot = cx.debug_render_snapshot();
        let tree = snapshot
            .accessibility_tree_json()
            .expect("forced accessibility should capture the transcript");
        let nodes = accessibility_nodes(tree);

        assert!(
            nodes
                .iter()
                .any(|node| node.role == "Log" && node.label == "Conversation transcript"),
            "the transcript needs one named container so assistive technology can \
             navigate to the conversation: {tree}"
        );
        assert!(
            nodes.iter().any(|node| node.role == "Label"
                && node.label == "Agent message: Hello! I am Omega, glad to meet you."),
            "the agent's reply must be readable from the accessibility tree — this is \
             the whole product for a screen-reader user: {tree}"
        );

        // The transcript is virtualized: the earlier turn is only in the tree
        // while it is rendered. Scroll it back into view and read it there, the
        // way assistive technology reaches back through a conversation.
        active_thread(&conversation_view, cx).update(cx, |view, cx| {
            view.list_state.scroll_to(gpui::ListOffset {
                item_ix: 0,
                offset_in_item: px(0.0),
            });
            cx.notify();
        });
        cx.run_until_parked();

        let scrolled = cx.debug_render_snapshot();
        let scrolled_tree = scrolled
            .accessibility_tree_json()
            .expect("forced accessibility should survive a scroll");
        let scrolled_nodes = accessibility_nodes(scrolled_tree);
        assert!(
            scrolled_nodes.iter().any(|node| node.role == "Label"
                && node.label == "Your message: Say hello in one short sentence."),
            "the user's own turn must be readable from the accessibility tree: \
             {scrolled_tree}"
        );
    }

    /// omega#217. VoiceOver's Item Chooser reported five controls whose entire
    /// announced name was their role (`toggle button`, `button`, `button`,
    /// `button`, `toggle button`). The close rule names generic placeholder
    /// labels as disqualifying on their own, so this asserts the property
    /// rather than the five names: no control Omega publishes may be nameless.
    #[gpui::test]
    async fn no_transcript_or_composer_control_is_published_without_a_name(
        cx: &mut TestAppContext,
    ) {
        init_test(cx);

        let connection = StubAgentConnection::new();
        connection.set_next_prompt_updates(vec![acp::SessionUpdate::AgentMessageChunk(
            acp::ContentChunk::new("Done.".into()),
        )]);
        let (conversation_view, cx) =
            setup_conversation_view(StubAgentServer::new(connection.clone()), cx).await;
        add_to_workspace(conversation_view.clone(), cx);

        message_editor(&conversation_view, cx).update_in(cx, |editor, window, cx| {
            editor.set_text("Do the thing.", window, cx);
        });
        active_thread(&conversation_view, cx).update_in(cx, |view, window, cx| {
            view.send(window, cx);
        });
        cx.run_until_parked();

        cx.set_debug_accessibility_active(true);
        let snapshot = cx.debug_render_snapshot();
        let tree = snapshot
            .accessibility_tree_json()
            .expect("forced accessibility should capture the completed turn");
        let nodes = accessibility_nodes(tree);

        // The retained workspace pane chrome the test harness mounts has its own
        // unlabelled controls and is a separate crate's defect; the primary
        // interface does not render it. Everything else in the frame is Omega's
        // and must be named. Listing the exemptions explicitly keeps this
        // assertion from quietly becoming vacuous.
        const RETAINED_WORKSPACE_CHROME: [&str; 6] = [
            "navigate_backward",
            "navigate_forward",
            "close tab",
            "plus",
            "split",
            "toggle_zoom",
        ];
        let nameless: Vec<_> = nodes
            .iter()
            .filter(|node| {
                node.element_id
                    .as_deref()
                    .is_none_or(|element_id| !RETAINED_WORKSPACE_CHROME.contains(&element_id))
            })
            .filter(|node| {
                matches!(
                    node.role.as_str(),
                    "Button" | "CheckBox" | "RadioButton" | "MenuItem" | "Link" | "Tab"
                )
            })
            .filter(|node| node.label.trim().is_empty())
            .map(|node| {
                format!(
                    "{} (element_id {:?})",
                    node.role,
                    node.element_id.as_deref().unwrap_or("<none>")
                )
            })
            .collect();
        assert!(
            nameless.is_empty(),
            "every published control needs a real accessible name; these announce \
             only their role: {nameless:?}\n{tree}"
        );

        for expected in [
            "Copy this agent response",
            "Message info",
            "Scroll to user message",
            "Scroll to top",
            "Add context",
        ] {
            assert!(
                nodes.iter().any(|node| node.label == expected),
                "the message-action and composer controls must keep their names; \
                 {expected:?} is missing: {tree}"
            );
        }
    }

    /// omega#217. The independent VoiceOver pass sent one message and sampled
    /// captions at t+2s, t+6s, t+12s and t+20s across the whole turn:
    /// "VoiceOver announced nothing at any point. No send confirmation, no
    /// streaming or progress, no completion."
    ///
    /// The mechanism is exact. `accesskit_macos`'s event generator posts an
    /// `NSAccessibilityAnnouncementRequested` only for a node whose `live` is
    /// not `Off` **and** whose `value` changed, and the announced text is that
    /// value. Omega had no live node anywhere and no status node with a value,
    /// so the two assertions below — the region is live, and its value tracks
    /// the turn — are the two halves of "a screen reader hears the turn".
    #[gpui::test]
    async fn a_turn_announces_that_it_started_and_that_it_finished(cx: &mut TestAppContext) {
        init_test(cx);

        // A permission-gated tool call holds the turn open, so "running" is a
        // state the test can observe rather than a frame it has to race.
        let tool_call_id = acp::ToolCallId::new("turn-status-1");
        let tool_call =
            acp::ToolCall::new(tool_call_id.clone(), "Run `cargo test`").kind(acp::ToolKind::Edit);
        let permission_options =
            ToolPermissionContext::new(TerminalTool::NAME, vec!["cargo test".to_string()])
                .build_permission_options();
        let connection =
            StubAgentConnection::new().with_permission_requests(HashMap::from_iter([(
                tool_call_id.clone(),
                permission_options,
            )]));
        connection.set_next_prompt_updates(vec![acp::SessionUpdate::ToolCall(tool_call)]);

        let (conversation_view, cx) =
            setup_conversation_view(StubAgentServer::new(connection), cx).await;
        add_to_workspace(conversation_view.clone(), cx);
        cx.update(|_window, cx| {
            AgentSettings::override_global(
                AgentSettings {
                    notify_when_agent_waiting: NotifyWhenAgentWaiting::Never,
                    ..AgentSettings::get_global(cx).clone()
                },
                cx,
            );
        });
        cx.set_debug_accessibility_active(true);

        let idle = turn_status_region(cx);
        assert_eq!(
            idle.live.as_deref(),
            Some("Polite"),
            "the turn-status region must be a live region, or the platform adapter \
             never announces it"
        );
        assert_eq!(
            idle.value, None,
            "a thread that has not run must not announce anything on arrival"
        );

        message_editor(&conversation_view, cx).update_in(cx, |editor, window, cx| {
            editor.set_text("Run the tests.", window, cx);
        });
        active_thread(&conversation_view, cx)
            .update_in(cx, |view, window, cx| view.send(window, cx));
        cx.run_until_parked();

        conversation_view.read_with(cx, |view, cx| {
            assert!(
                view.pending_tool_call(cx).is_some(),
                "this test only means something while the turn is still running"
            );
        });
        assert_eq!(
            turn_status_region(cx).value.as_deref(),
            Some("Omega is responding."),
            "a started turn must be announced; a screen-reader user otherwise has no \
             signal that the message was sent"
        );

        conversation_view.update_in(cx, |_, window, cx| {
            window.dispatch_action(
                crate::AuthorizeToolCall {
                    tool_call_id: "turn-status-1".to_string(),
                    option_id: "allow".to_string(),
                    option_kind: "AllowOnce".to_string(),
                }
                .boxed_clone(),
                cx,
            );
        });
        cx.run_until_parked();

        conversation_view.read_with(cx, |view, cx| {
            assert_eq!(
                view.active_thread()
                    .expect("the thread should still exist")
                    .read(cx)
                    .thread
                    .read(cx)
                    .status(),
                acp_thread::ThreadStatus::Idle,
                "the turn must have finished for the completion assertion to mean anything"
            );
        });
        assert_eq!(
            turn_status_region(cx).value.as_deref(),
            Some("Omega finished responding."),
            "a finished turn must be announced; without it the user cannot tell a slow \
             answer from a finished one"
        );
    }

    /// omega#217. The same silence covers failure: a turn that dies leaves the
    /// callout on screen and says nothing. The announced phrase is deliberately
    /// a fixed sentence per error kind rather than the provider's own message,
    /// which is where a raw payload would otherwise be read aloud.
    #[gpui::test]
    async fn a_failed_turn_announces_that_it_failed(cx: &mut TestAppContext) {
        init_test(cx);

        let (conversation_view, cx) =
            setup_conversation_view(StubAgentServer::new(StubAgentConnection::new()), cx).await;
        add_to_workspace(conversation_view.clone(), cx);
        cx.set_debug_accessibility_active(true);

        active_thread(&conversation_view, cx).update(cx, |view, cx| {
            view.handle_thread_error(ThreadError::PaymentRequired, cx);
        });
        cx.run_until_parked();

        let failed = turn_status_region(cx);
        assert_eq!(
            failed.live.as_deref(),
            Some("Polite"),
            "the failure has to land on a live region to be spoken at all"
        );
        assert_eq!(
            failed.value.as_deref(),
            Some("Omega stopped: payment is required."),
            "a failed turn must say so"
        );
    }

    /// The one turn-status live region in the last `accesskit::TreeUpdate`.
    fn turn_status_region(cx: &mut VisualTestContext) -> AccessibilityNode {
        let snapshot = cx.debug_render_snapshot();
        let tree = snapshot
            .accessibility_tree_json()
            .expect("forced accessibility should capture the conversation")
            .to_owned();
        let mut regions: Vec<_> = accessibility_nodes(&tree)
            .into_iter()
            .filter(|node| node.element_id.as_deref() == Some("omega-turn-status"))
            .collect();
        assert_eq!(
            regions.len(),
            1,
            "the conversation must publish exactly one turn-status region: {tree}"
        );
        regions.remove(0)
    }

    pub(crate) struct AccessibilityNode {
        pub(crate) element_id: Option<String>,
        pub(crate) role: String,
        pub(crate) label: String,
        /// What assistive technology speaks for a live region. macOS announces
        /// a live node's value, never its label, so a status region that only
        /// has a label is silent.
        pub(crate) value: Option<String>,
        pub(crate) live: Option<String>,
    }

    /// Flatten the debug serialization of the last `accesskit::TreeUpdate` into
    /// the three fields these tests reason about. Asserting on parsed nodes
    /// rather than substrings keeps a test from passing because the text
    /// happened to appear in an unrelated field.
    pub(crate) fn accessibility_nodes(tree: &str) -> Vec<AccessibilityNode> {
        let value: serde_json::Value =
            serde_json::from_str(tree).expect("the accessibility tree should be valid JSON");
        value
            .get("nodes")
            .and_then(serde_json::Value::as_object)
            .expect("the accessibility tree should have a nodes object")
            .values()
            .map(|node| {
                let aria = node.get("aria");
                AccessibilityNode {
                    element_id: node
                        .get("element_id")
                        .and_then(serde_json::Value::as_str)
                        .map(str::to_owned),
                    role: aria
                        .and_then(|aria| aria.get("role"))
                        .and_then(serde_json::Value::as_str)
                        .unwrap_or_default()
                        .to_owned(),
                    label: aria
                        .and_then(|aria| aria.get("label"))
                        .and_then(serde_json::Value::as_str)
                        .unwrap_or_default()
                        .to_owned(),
                    value: aria
                        .and_then(|aria| aria.get("value"))
                        .and_then(serde_json::Value::as_str)
                        .map(str::to_owned),
                    live: aria
                        .and_then(|aria| aria.get("live"))
                        .and_then(serde_json::Value::as_str)
                        .map(str::to_owned),
                }
            })
            .collect()
    }

    /// omega#153. Direct Agent is deliberately not a logical router: its
    /// readiness continues to mean that the selected executor made a real
    /// session, preserving the mode contract while Omega changes.
    #[gpui::test]
    async fn direct_agent_still_creates_its_session_during_preparation(cx: &mut TestAppContext) {
        init_test(cx);

        let (conversation_view, cx) = setup_conversation_view_for_agent_without_settling(
            StubAgentServer::new(StubAgentConnection::new()),
            Agent::Custom {
                id: "direct-agent".into(),
            },
            None,
            None,
            cx,
        )
        .await;
        cx.run_until_parked();

        conversation_view.read_with(cx, |this, cx| {
            assert!(matches!(
                this.preparation_state(cx),
                ConversationPreparation::Ready { .. }
            ));
            assert!(
                this.root_session_id.is_some() && this.active_thread().is_some(),
                "Direct Agent readiness must retain its physical-session proof"
            );
        });
    }

    #[gpui::test]
    async fn connected_composer_accepts_pointer_input_after_loading(cx: &mut TestAppContext) {
        init_test(cx);

        let (conversation_view, cx) = setup_conversation_view_still_connecting(
            StubAgentServer::new(StubAgentConnection::new()),
            cx,
        )
        .await;
        add_to_workspace(conversation_view.clone(), cx);
        cx.run_until_parked();

        cx.simulate_click_selector("omega.workbench.composer-input")
            .expect("the connected composer must be clickable");
        cx.simulate_keystrokes("pointerinput");

        assert_eq!(
            message_editor(&conversation_view, cx).read_with(cx, |editor, cx| editor.text(cx)),
            "pointerinput",
            "the loaded composer must receive mouse focus and typed text"
        );
    }

    #[gpui::test]
    async fn image_dropped_while_connecting_is_attached_after_loading(cx: &mut TestAppContext) {
        init_test(cx);

        use base64::Engine as _;
        let image_bytes = base64::prelude::BASE64_STANDARD
            .decode("iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mNkYPhfDwAChwGA60e6kgAAAABJRU5ErkJggg==")
            .expect("decode test image");
        let fs = FakeFs::new(cx.executor());
        fs.insert_file("/project/dropped.png", image_bytes).await;
        let project = Project::test(fs, [Path::new("/project")], cx).await;
        let image_path = project.read_with(cx, |project, cx| {
            project
                .project_path_for_absolute_path(Path::new("/project/dropped.png"), cx)
                .expect("test image should be in the project")
        });
        let (conversation_view, cx) = setup_conversation_view_with_project_without_settling(
            StubAgentServer::new(StubAgentConnection::new()),
            Agent::Custom { id: "Test".into() },
            None,
            None,
            project,
            cx,
        )
        .await;

        conversation_view.update_in(cx, |view, window, cx| {
            assert!(view.active_thread().is_none());
            view.insert_dragged_files(vec![image_path], Vec::new(), window, cx);
            assert_eq!(view.pending_dragged_files.len(), 1);
            assert!(view.has_unsubmitted_or_pending_content(cx));
        });
        cx.run_until_parked();

        let editor = message_editor(&conversation_view, cx);
        let expected_uri = MentionUri::File {
            abs_path: PathBuf::from("/project/dropped.png"),
        }
        .to_uri()
        .to_string();
        assert_eq!(
            editor.read_with(cx, |editor, cx| editor.text(cx)),
            format!("[@dropped.png]({expected_uri}) ")
        );
        conversation_view.read_with(cx, |view, _| {
            assert!(view.pending_dragged_files.is_empty());
        });

        let contents = editor
            .update(cx, |editor, cx| {
                editor
                    .mention_set()
                    .update(cx, |mention_set, cx| mention_set.contents(false, cx))
            })
            .await
            .expect("resolve dropped image mention");
        assert!(
            contents
                .values()
                .any(|(_, mention)| matches!(mention, Mention::Image(_)))
        );
    }

    /// `OMEGA-DELTA-0170`, the failure half. A message sent while connecting
    /// survives a terminal connection failure — the text is preserved and
    /// surfaced, never dropped — and a retry that succeeds dispatches it.
    #[gpui::test]
    async fn a_message_sent_while_connecting_survives_a_terminal_failure(cx: &mut TestAppContext) {
        init_test(cx);

        let (agent, fail) = FlakyAgentServer::new(StubAgentConnection::new());
        let (conversation_view, cx) = setup_conversation_view_still_connecting(agent, cx).await;

        conversation_view.update_in(cx, |this, window, cx| {
            let editor = this.loading_composer(window, cx);
            editor.update(cx, |editor, cx| {
                editor.set_text("the task statement", window, cx)
            });
            this.submit_while_connecting(window, cx);
        });

        cx.run_until_parked();

        conversation_view.read_with(cx, |this, _| {
            assert!(
                matches!(this.server_state, ServerState::LoadError { .. }),
                "the connection must have terminally failed for this test to mean anything"
            );
            assert_eq!(
                this.pending_connect_messages
                    .iter()
                    .map(|message| message.text.as_str())
                    .collect::<Vec<_>>(),
                vec!["the task statement"],
                "a terminal connection failure must preserve the submitted text, \
                 not drop it with the connection"
            );
        });

        // The Retry Connection button calls `reset`; the executor is
        // reachable this time.
        fail.store(false, std::sync::atomic::Ordering::SeqCst);
        conversation_view.update_in(cx, |this, window, cx| this.reset(window, cx));
        cx.run_until_parked();

        let thread_view = active_thread(&conversation_view, cx);
        assert_eq!(
            user_message_markdown(&thread_view, cx),
            vec!["## User\n\nthe task statement\n\n".to_string()],
            "a retry that connects must dispatch the preserved message exactly once"
        );
    }

    fn add_to_workspace(conversation_view: Entity<ConversationView>, cx: &mut VisualTestContext) {
        let workspace =
            conversation_view.read_with(cx, |thread_view, _cx| thread_view.workspace.clone());

        workspace
            .update_in(cx, |workspace, window, cx| {
                workspace.add_item_to_active_pane(
                    Box::new(cx.new(|_| ThreadViewItem(conversation_view.clone()))),
                    None,
                    true,
                    window,
                    cx,
                );
            })
            .unwrap();
    }

    fn add_with_vim_indicator(
        conversation_view: Entity<ConversationView>,
        cx: &mut VisualTestContext,
    ) {
        let workspace = conversation_view.read_with(cx, |view, _cx| view.workspace.clone());
        workspace
            .update_in(cx, |workspace, window, cx| {
                workspace.add_item_to_active_pane(
                    Box::new(cx.new(|_| VimIndicatorTestItem { conversation_view })),
                    None,
                    true,
                    window,
                    cx,
                );
            })
            .expect("test workspace should still exist");
    }

    struct ThreadViewItem(Entity<ConversationView>);

    impl Item for ThreadViewItem {
        type Event = ();

        fn include_in_nav_history() -> bool {
            false
        }

        fn tab_content_text(&self, _detail: usize, _cx: &App) -> SharedString {
            "Test".into()
        }
    }

    impl EventEmitter<()> for ThreadViewItem {}

    impl Focusable for ThreadViewItem {
        fn focus_handle(&self, cx: &App) -> FocusHandle {
            self.0.read(cx).focus_handle(cx)
        }
    }

    impl Render for ThreadViewItem {
        fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
            // Render the title editor in the element tree too. In the real app
            // it is part of the agent panel
            let title_editor = self
                .0
                .read(cx)
                .active_thread()
                .map(|t| t.read(cx).title_editor.clone());

            v_flex().children(title_editor).child(self.0.clone())
        }
    }

    // omega#161. The shipped composer bar renders the shared indicator
    // itself (`omega_zero_base::is_active()` is constant `true`), so this
    // item hosts only the conversation view; a second copy of the indicator
    // here would double the rendered selector the test counts.
    struct VimIndicatorTestItem {
        conversation_view: Entity<ConversationView>,
    }

    impl Item for VimIndicatorTestItem {
        type Event = ();

        fn include_in_nav_history() -> bool {
            false
        }

        fn tab_content_text(&self, _detail: usize, _cx: &App) -> SharedString {
            "Vim indicator test".into()
        }
    }

    impl EventEmitter<()> for VimIndicatorTestItem {}

    impl Focusable for VimIndicatorTestItem {
        fn focus_handle(&self, cx: &App) -> FocusHandle {
            self.conversation_view.read(cx).focus_handle(cx)
        }
    }

    impl Render for VimIndicatorTestItem {
        fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
            v_flex().child(self.conversation_view.clone())
        }
    }

    fn init_vim_test(cx: &mut TestAppContext) {
        init_test(cx);
        cx.update(|cx| {
            vim::init(cx);
            SettingsStore::update_global(cx, |store, cx| {
                store.update_user_settings(cx, |settings| settings.vim_mode = Some(true));
            });
            let mut bindings =
                settings::KeymapFile::load_asset_allow_partial_failure("keymaps/vim.json", cx)
                    .expect("the retained Vim keymap should load");
            for binding in &mut bindings {
                binding.set_meta(settings::KeybindSource::Vim.meta());
            }
            cx.bind_keys(bindings);
        });
    }

    #[gpui::test]
    async fn vim_indicator_is_shared_across_composer_transition_and_tracks_mode(
        cx: &mut TestAppContext,
    ) {
        init_vim_test(cx);

        let rejected_actions = Arc::new(Mutex::new(Vec::new()));
        cx.update({
            let rejected_actions = rejected_actions.clone();
            move |cx| {
                cx.set_action_gate(move |action, _cx| {
                    let admitted = omega_zero_base::admits_action(action.name());
                    if !admitted {
                        rejected_actions.lock().push(action.name().to_owned());
                    }
                    admitted
                });
            }
        });

        let connection = StubAgentConnection::new();
        let (conversation_view, cx) =
            setup_conversation_view_still_connecting(StubAgentServer::new(connection), cx).await;

        let loading_indicator_id = conversation_view.update_in(cx, |view, window, cx| {
            assert!(matches!(view.server_state, ServerState::Loading { .. }));
            let indicator_id = view.vim_mode_indicator.entity_id();
            let loading_editor = view.loading_composer(window, cx);
            loading_editor.read(cx).focus_handle(cx).focus(window, cx);
            indicator_id
        });

        cx.run_until_parked();
        let connected_indicator_id = active_thread(&conversation_view, cx)
            .read_with(cx, |view, _| view.vim_mode_indicator.entity_id());
        assert_eq!(
            connected_indicator_id, loading_indicator_id,
            "loading and connected composer bars must retain one ModeIndicator entity"
        );

        add_with_vim_indicator(conversation_view.clone(), cx);
        cx.set_debug_accessibility_active(true);
        message_editor(&conversation_view, cx).update_in(cx, |editor, window, cx| {
            editor.focus_handle(cx).focus(window, cx);
        });
        cx.simulate_keystrokes("escape");

        let snapshot = cx.debug_render_snapshot();
        assert_eq!(
            snapshot.selector_count("vim.mode-indicator"),
            1,
            "the shared indicator should expose one stable rendered selector"
        );
        let normal_tree = snapshot
            .accessibility_tree_json()
            .expect("forced accessibility should capture the indicator");
        assert!(
            normal_tree.contains("Message composer") && normal_tree.contains("MultilineTextInput"),
            "the installed workbench composer must publish its text-input role: {normal_tree}"
        );
        assert!(
            normal_tree.contains("Vim mode: NORMAL"),
            "the indicator should announce the focused composer's normal mode: {normal_tree}"
        );

        cx.simulate_keystrokes("i");
        let insert_snapshot = cx.debug_render_snapshot();
        let insert_tree = insert_snapshot
            .accessibility_tree_json()
            .expect("forced accessibility should retain the indicator");
        assert!(
            insert_tree.contains("Vim mode: INSERT"),
            "the same indicator should follow the focused composer's insert mode: {insert_tree}"
        );
        cx.simulate_keystrokes("escape v l");
        let visual_snapshot = cx.debug_render_snapshot();
        let visual_tree = visual_snapshot
            .accessibility_tree_json()
            .expect("forced accessibility should retain the indicator in visual mode");
        assert!(
            visual_tree.contains("Vim mode: VISUAL"),
            "the gated composer journey must expose visual mode: {visual_tree}"
        );
        cx.simulate_keystrokes("escape");
        assert_eq!(
            *rejected_actions.lock(),
            Vec::<String>::new(),
            "the production zero-base predicate must admit every action in the Vim composer journey"
        );
    }

    pub(crate) struct StubAgentServer<C> {
        connection: C,
    }

    impl<C> StubAgentServer<C> {
        pub(crate) fn new(connection: C) -> Self {
            Self { connection }
        }
    }

    impl StubAgentServer<StubAgentConnection> {
        pub(crate) fn default_response() -> Self {
            let conn = StubAgentConnection::new();
            conn.set_next_prompt_updates(vec![acp::SessionUpdate::AgentMessageChunk(
                acp::ContentChunk::new("Default response".into()),
            )]);
            Self::new(conn)
        }
    }

    impl<C> AgentServer for StubAgentServer<C>
    where
        C: 'static + AgentConnection + Send + Clone,
    {
        fn logo(&self) -> ui::IconName {
            ui::IconName::OmegaAgent
        }

        fn agent_id(&self) -> AgentId {
            "Test".into()
        }

        fn connect(
            &self,
            _delegate: AgentServerDelegate,
            _project: Entity<Project>,
            _cx: &mut App,
        ) -> Task<gpui::Result<Rc<dyn AgentConnection>>> {
            Task::ready(Ok(Rc::new(self.connection.clone())))
        }

        fn into_any(self: Rc<Self>) -> Rc<dyn Any> {
            self
        }
    }

    pub(crate) struct FailingAgentServer;

    impl AgentServer for FailingAgentServer {
        fn logo(&self) -> ui::IconName {
            ui::IconName::AiOpenAi
        }

        fn agent_id(&self) -> AgentId {
            AgentId::new("Codex CLI")
        }

        fn connect(
            &self,
            _delegate: AgentServerDelegate,
            _project: Entity<Project>,
            _cx: &mut App,
        ) -> Task<gpui::Result<Rc<dyn AgentConnection>>> {
            Task::ready(Err(anyhow!(
                "extracting downloaded asset for \
                 https://github.com/zed-industries/codex-acp/releases/download/v0.9.4/\
                 codex-acp-0.9.4-aarch64-pc-windows-msvc.zip: \
                 failed to iterate over archive: Invalid gzip header"
            )))
        }

        fn into_any(self: Rc<Self>) -> Rc<dyn Any> {
            self
        }
    }

    /// Agent server whose `connect()` fails while `fail` is `true` and
    /// returns the wrapped connection otherwise. Used to simulate the
    /// race where an external agent isn't yet registered at startup.
    pub(crate) struct FlakyAgentServer {
        connection: StubAgentConnection,
        fail: Arc<std::sync::atomic::AtomicBool>,
    }

    impl FlakyAgentServer {
        pub(crate) fn new(
            connection: StubAgentConnection,
        ) -> (Self, Arc<std::sync::atomic::AtomicBool>) {
            let fail = Arc::new(std::sync::atomic::AtomicBool::new(true));
            (
                Self {
                    connection,
                    fail: fail.clone(),
                },
                fail,
            )
        }
    }

    impl AgentServer for FlakyAgentServer {
        fn logo(&self) -> ui::IconName {
            ui::IconName::OmegaAgent
        }

        fn agent_id(&self) -> AgentId {
            "Flaky".into()
        }

        fn connect(
            &self,
            _delegate: AgentServerDelegate,
            _project: Entity<Project>,
            _cx: &mut App,
        ) -> Task<gpui::Result<Rc<dyn AgentConnection>>> {
            if self.fail.load(std::sync::atomic::Ordering::SeqCst) {
                Task::ready(Err(anyhow!(
                    "Custom agent server `Flaky` is not registered"
                )))
            } else {
                Task::ready(Ok(Rc::new(self.connection.clone())))
            }
        }

        fn into_any(self: Rc<Self>) -> Rc<dyn Any> {
            self
        }
    }

    fn build_test_thread(
        connection: Rc<dyn AgentConnection>,
        project: Entity<Project>,
        name: &'static str,
        session_id: acp::SessionId,
        cx: &mut App,
    ) -> Entity<AcpThread> {
        let action_log = cx.new(|_| ActionLog::new(project.clone()));
        cx.new(|cx| {
            AcpThread::new(
                None,
                Some(name.into()),
                None,
                connection,
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

    #[derive(Clone, Default)]
    struct PreloadedElicitationConnection {
        elicitation_id: Arc<Mutex<Option<ElicitationEntryId>>>,
    }

    impl AgentConnection for PreloadedElicitationConnection {
        fn agent_id(&self) -> AgentId {
            AgentId::new("preloaded-elicitation")
        }

        fn telemetry_id(&self) -> SharedString {
            "preloaded-elicitation".into()
        }

        fn new_session(
            self: Rc<Self>,
            project: Entity<Project>,
            _work_dirs: PathList,
            cx: &mut App,
        ) -> Task<gpui::Result<Entity<AcpThread>>> {
            let session_id = acp::SessionId::new("new-session");
            let thread = build_test_thread(
                self.clone(),
                project,
                "PreloadedElicitationConnection",
                session_id.clone(),
                cx,
            );
            thread.update(cx, |thread, cx| {
                thread
                    .request_elicitation(
                        acp::CreateElicitationRequest::new(
                            acp::ElicitationFormMode::new(
                                acp::ElicitationSessionScope::new(session_id),
                                acp::ElicitationSchema::new().string("name", true),
                            ),
                            "Provide a name",
                        ),
                        cx,
                    )
                    .expect("preloaded elicitation should be accepted")
                    .detach();
            });
            let elicitation_id = thread.read_with(cx, |thread, _cx| {
                thread.entries().iter().find_map(|entry| {
                    if let AgentThreadEntry::Elicitation(elicitation_id) = entry {
                        Some(elicitation_id.clone())
                    } else {
                        None
                    }
                })
            });
            *self.elicitation_id.lock() = elicitation_id;
            Task::ready(Ok(thread))
        }

        fn auth_methods(&self) -> &[acp::AuthMethod] {
            &[]
        }

        fn authenticate(
            &self,
            _method_id: acp::AuthMethodId,
            _cx: &mut App,
        ) -> Task<gpui::Result<()>> {
            Task::ready(Ok(()))
        }

        fn prompt(
            &self,
            _params: acp::PromptRequest,
            _cx: &mut App,
        ) -> Task<gpui::Result<acp::PromptResponse>> {
            Task::ready(Ok(acp::PromptResponse::new(acp::StopReason::EndTurn)))
        }

        fn cancel(&self, _session_id: &acp::SessionId, _cx: &mut App) {}

        fn into_any(self: Rc<Self>) -> Rc<dyn Any> {
            self
        }
    }

    struct SessionCreationRequestElicitationServer {
        store: Entity<ElicitationStore>,
        response: Arc<Mutex<Option<acp::ElicitationAction>>>,
    }

    impl AgentServer for SessionCreationRequestElicitationServer {
        fn logo(&self) -> ui::IconName {
            ui::IconName::OmegaAgent
        }

        fn agent_id(&self) -> AgentId {
            "SessionCreationRequestElicitation".into()
        }

        fn connect(
            &self,
            _delegate: AgentServerDelegate,
            _project: Entity<Project>,
            _cx: &mut App,
        ) -> Task<gpui::Result<Rc<dyn AgentConnection>>> {
            let connection = SessionCreationRequestElicitationConnection {
                store: self.store.clone(),
                response: self.response.clone(),
            };
            Task::ready(Ok(Rc::new(connection)))
        }

        fn into_any(self: Rc<Self>) -> Rc<dyn Any> {
            self
        }
    }

    struct SessionCreationRequestElicitationConnection {
        store: Entity<ElicitationStore>,
        response: Arc<Mutex<Option<acp::ElicitationAction>>>,
    }

    impl AgentConnection for SessionCreationRequestElicitationConnection {
        fn agent_id(&self) -> AgentId {
            AgentId::new("session-creation-request-elicitation")
        }

        fn telemetry_id(&self) -> SharedString {
            "session-creation-request-elicitation".into()
        }

        fn new_session(
            self: Rc<Self>,
            project: Entity<Project>,
            _work_dirs: PathList,
            cx: &mut App,
        ) -> Task<gpui::Result<Entity<AcpThread>>> {
            let thread = build_test_thread(
                self.clone(),
                project,
                "SessionCreationRequestElicitationConnection",
                acp::SessionId::new("session-creation-request-elicitation-session"),
                cx,
            );
            let first_response_task = self.store.update(cx, |store, cx| {
                store
                    .request_elicitation(
                        acp::CreateElicitationRequest::new(
                            acp::ElicitationFormMode::new(
                                acp::ElicitationRequestScope::new(acp::RequestId::Number(1)),
                                acp::ElicitationSchema::new().string("name", true),
                            ),
                            "Provide a name",
                        ),
                        cx,
                    )
                    .expect("first request-scoped elicitation should be accepted")
            });
            self.store
                .update(cx, |store, cx| {
                    store
                        .request_elicitation(
                            acp::CreateElicitationRequest::new(
                                acp::ElicitationFormMode::new(
                                    acp::ElicitationRequestScope::new(acp::RequestId::Number(2)),
                                    acp::ElicitationSchema::new().string("account", true),
                                ),
                                "Provide an account",
                            ),
                            cx,
                        )
                        .expect("second request-scoped elicitation should be accepted")
                })
                .detach();

            let response = self.response.clone();
            cx.spawn(async move |_cx| {
                let elicitation_response = first_response_task.await;
                *response.lock() = Some(elicitation_response.action);
                Ok(thread)
            })
        }

        fn request_elicitations(&self) -> Option<Entity<ElicitationStore>> {
            Some(self.store.clone())
        }

        fn auth_methods(&self) -> &[acp::AuthMethod] {
            &[]
        }

        fn authenticate(
            &self,
            _method_id: acp::AuthMethodId,
            _cx: &mut App,
        ) -> Task<gpui::Result<()>> {
            Task::ready(Ok(()))
        }

        fn prompt(
            &self,
            _params: acp::PromptRequest,
            _cx: &mut App,
        ) -> Task<gpui::Result<acp::PromptResponse>> {
            Task::ready(Ok(acp::PromptResponse::new(acp::StopReason::EndTurn)))
        }

        fn cancel(&self, _session_id: &acp::SessionId, _cx: &mut App) {}

        fn into_any(self: Rc<Self>) -> Rc<dyn Any> {
            self
        }
    }

    struct ReleaseRequestElicitationServer {
        response: Arc<Mutex<Option<acp::ElicitationAction>>>,
    }

    impl AgentServer for ReleaseRequestElicitationServer {
        fn logo(&self) -> ui::IconName {
            ui::IconName::OmegaAgent
        }

        fn agent_id(&self) -> AgentId {
            "ReleaseRequestElicitation".into()
        }

        fn connect(
            &self,
            _delegate: AgentServerDelegate,
            _project: Entity<Project>,
            cx: &mut App,
        ) -> Task<gpui::Result<Rc<dyn AgentConnection>>> {
            let connection = ReleaseRequestElicitationConnection {
                store: cx.new(|_| ElicitationStore::default()),
                response: self.response.clone(),
            };
            Task::ready(Ok(Rc::new(connection)))
        }

        fn into_any(self: Rc<Self>) -> Rc<dyn Any> {
            self
        }
    }

    struct ReleaseRequestElicitationConnection {
        store: Entity<ElicitationStore>,
        response: Arc<Mutex<Option<acp::ElicitationAction>>>,
    }

    impl AgentConnection for ReleaseRequestElicitationConnection {
        fn agent_id(&self) -> AgentId {
            AgentId::new("release-request-elicitation")
        }

        fn telemetry_id(&self) -> SharedString {
            "release-request-elicitation".into()
        }

        fn new_session(
            self: Rc<Self>,
            project: Entity<Project>,
            _work_dirs: PathList,
            cx: &mut App,
        ) -> Task<gpui::Result<Entity<AcpThread>>> {
            let thread = build_test_thread(
                self.clone(),
                project,
                "ReleaseRequestElicitationConnection",
                acp::SessionId::new("release-request-elicitation-session"),
                cx,
            );
            let response_task = self.store.update(cx, |store, cx| {
                store
                    .request_elicitation(
                        acp::CreateElicitationRequest::new(
                            acp::ElicitationFormMode::new(
                                acp::ElicitationRequestScope::new(acp::RequestId::Number(1)),
                                acp::ElicitationSchema::new().string("name", true),
                            ),
                            "Provide a name",
                        ),
                        cx,
                    )
                    .expect("request-scoped elicitation should be accepted")
            });
            let response = self.response.clone();
            cx.spawn(async move |_cx| {
                let elicitation_response = response_task.await;
                *response.lock() = Some(elicitation_response.action);
            })
            .detach();
            Task::ready(Ok(thread))
        }

        fn request_elicitations(&self) -> Option<Entity<ElicitationStore>> {
            Some(self.store.clone())
        }

        fn auth_methods(&self) -> &[acp::AuthMethod] {
            &[]
        }

        fn authenticate(
            &self,
            _method_id: acp::AuthMethodId,
            _cx: &mut App,
        ) -> Task<gpui::Result<()>> {
            Task::ready(Ok(()))
        }

        fn prompt(
            &self,
            _params: acp::PromptRequest,
            _cx: &mut App,
        ) -> Task<gpui::Result<acp::PromptResponse>> {
            Task::ready(Ok(acp::PromptResponse::new(acp::StopReason::EndTurn)))
        }

        fn cancel(&self, _session_id: &acp::SessionId, _cx: &mut App) {}

        fn into_any(self: Rc<Self>) -> Rc<dyn Any> {
            self
        }
    }

    #[derive(Clone)]
    struct ResumeOnlyAgentConnection;

    impl AgentConnection for ResumeOnlyAgentConnection {
        fn agent_id(&self) -> AgentId {
            AgentId::new("resume-only")
        }

        fn telemetry_id(&self) -> SharedString {
            "resume-only".into()
        }

        fn new_session(
            self: Rc<Self>,
            project: Entity<Project>,
            _work_dirs: PathList,
            cx: &mut gpui::App,
        ) -> Task<gpui::Result<Entity<AcpThread>>> {
            let thread = build_test_thread(
                self,
                project,
                "ResumeOnlyAgentConnection",
                acp::SessionId::new("new-session"),
                cx,
            );
            Task::ready(Ok(thread))
        }

        fn supports_resume_session(&self) -> bool {
            true
        }

        fn resume_session(
            self: Rc<Self>,
            session_id: acp::SessionId,
            project: Entity<Project>,
            _work_dirs: PathList,
            _title: Option<SharedString>,
            cx: &mut App,
        ) -> Task<gpui::Result<Entity<AcpThread>>> {
            let thread =
                build_test_thread(self, project, "ResumeOnlyAgentConnection", session_id, cx);
            Task::ready(Ok(thread))
        }

        fn auth_methods(&self) -> &[acp::AuthMethod] {
            &[]
        }

        fn authenticate(
            &self,
            _method_id: acp::AuthMethodId,
            _cx: &mut App,
        ) -> Task<gpui::Result<()>> {
            Task::ready(Ok(()))
        }

        fn prompt(
            &self,
            _params: acp::PromptRequest,
            _cx: &mut App,
        ) -> Task<gpui::Result<acp::PromptResponse>> {
            Task::ready(Ok(acp::PromptResponse::new(acp::StopReason::EndTurn)))
        }

        fn cancel(&self, _session_id: &acp::SessionId, _cx: &mut App) {}

        fn into_any(self: Rc<Self>) -> Rc<dyn Any> {
            self
        }
    }

    /// Simulates an agent that requires authentication before a session can be
    /// created. `new_session` returns `AuthRequired` until `authenticate` is
    /// called with the correct method, after which sessions are created normally.
    #[derive(Clone)]
    struct AuthGatedAgentConnection {
        authenticated: Arc<Mutex<bool>>,
        auth_method: acp::AuthMethod,
    }

    impl AuthGatedAgentConnection {
        const AUTH_METHOD_ID: &str = "test-login";

        fn new() -> Self {
            Self {
                authenticated: Arc::new(Mutex::new(false)),
                auth_method: acp::AuthMethod::Agent(acp::AuthMethodAgent::new(
                    Self::AUTH_METHOD_ID,
                    "Test Login",
                )),
            }
        }
    }

    impl AgentConnection for AuthGatedAgentConnection {
        fn agent_id(&self) -> AgentId {
            AgentId::new("auth-gated")
        }

        fn telemetry_id(&self) -> SharedString {
            "auth-gated".into()
        }

        fn new_session(
            self: Rc<Self>,
            project: Entity<Project>,
            work_dirs: PathList,
            cx: &mut gpui::App,
        ) -> Task<gpui::Result<Entity<AcpThread>>> {
            if !*self.authenticated.lock() {
                return Task::ready(Err(acp_thread::AuthRequired::new()
                    .with_description("Sign in to continue".to_string())
                    .into()));
            }

            let session_id = acp::SessionId::new("auth-gated-session");
            let action_log = cx.new(|_| ActionLog::new(project.clone()));
            Task::ready(Ok(cx.new(|cx| {
                AcpThread::new(
                    None,
                    None,
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
            })))
        }

        fn auth_methods(&self) -> &[acp::AuthMethod] {
            std::slice::from_ref(&self.auth_method)
        }

        fn authenticate(
            &self,
            method_id: acp::AuthMethodId,
            _cx: &mut App,
        ) -> Task<gpui::Result<()>> {
            if &method_id == self.auth_method.id() {
                *self.authenticated.lock() = true;
                Task::ready(Ok(()))
            } else {
                Task::ready(Err(anyhow::anyhow!("Unknown auth method")))
            }
        }

        fn supports_logout(&self) -> bool {
            true
        }

        fn logout(&self, _cx: &mut App) -> Task<gpui::Result<()>> {
            *self.authenticated.lock() = false;
            Task::ready(Ok(()))
        }

        fn prompt(
            &self,
            _params: acp::PromptRequest,
            _cx: &mut App,
        ) -> Task<gpui::Result<acp::PromptResponse>> {
            unimplemented!()
        }

        fn cancel(&self, _session_id: &acp::SessionId, _cx: &mut App) {
            unimplemented!()
        }

        fn into_any(self: Rc<Self>) -> Rc<dyn Any> {
            self
        }
    }

    /// Simulates a model which always returns a refusal response
    #[derive(Clone)]
    struct RefusalAgentConnection;

    impl AgentConnection for RefusalAgentConnection {
        fn agent_id(&self) -> AgentId {
            AgentId::new("refusal")
        }

        fn telemetry_id(&self) -> SharedString {
            "refusal".into()
        }

        fn new_session(
            self: Rc<Self>,
            project: Entity<Project>,
            work_dirs: PathList,
            cx: &mut gpui::App,
        ) -> Task<gpui::Result<Entity<AcpThread>>> {
            Task::ready(Ok(cx.new(|cx| {
                let action_log = cx.new(|_| ActionLog::new(project.clone()));
                AcpThread::new(
                    None,
                    None,
                    Some(work_dirs),
                    self,
                    project,
                    action_log,
                    acp::SessionId::new("test"),
                    watch::Receiver::constant(
                        acp::PromptCapabilities::new()
                            .image(true)
                            .audio(true)
                            .embedded_context(true),
                    ),
                    cx,
                )
            })))
        }

        fn auth_methods(&self) -> &[acp::AuthMethod] {
            &[]
        }

        fn authenticate(
            &self,
            _method_id: acp::AuthMethodId,
            _cx: &mut App,
        ) -> Task<gpui::Result<()>> {
            unimplemented!()
        }

        fn prompt(
            &self,
            _params: acp::PromptRequest,
            _cx: &mut App,
        ) -> Task<gpui::Result<acp::PromptResponse>> {
            Task::ready(Ok(acp::PromptResponse::new(acp::StopReason::Refusal)))
        }

        fn cancel(&self, _session_id: &acp::SessionId, _cx: &mut App) {
            unimplemented!()
        }

        fn into_any(self: Rc<Self>) -> Rc<dyn Any> {
            self
        }
    }

    #[derive(Clone)]
    struct CwdCapturingConnection {
        captured_work_dirs: Arc<Mutex<Option<PathList>>>,
    }

    impl CwdCapturingConnection {
        fn new() -> Self {
            Self {
                captured_work_dirs: Arc::new(Mutex::new(None)),
            }
        }
    }

    impl AgentConnection for CwdCapturingConnection {
        fn agent_id(&self) -> AgentId {
            AgentId::new("cwd-capturing")
        }

        fn telemetry_id(&self) -> SharedString {
            "cwd-capturing".into()
        }

        fn new_session(
            self: Rc<Self>,
            project: Entity<Project>,
            work_dirs: PathList,
            cx: &mut gpui::App,
        ) -> Task<gpui::Result<Entity<AcpThread>>> {
            *self.captured_work_dirs.lock() = Some(work_dirs.clone());
            let action_log = cx.new(|_| ActionLog::new(project.clone()));
            let thread = cx.new(|cx| {
                AcpThread::new(
                    None,
                    None,
                    Some(work_dirs),
                    self.clone(),
                    project,
                    action_log,
                    acp::SessionId::new("new-session"),
                    watch::Receiver::constant(
                        acp::PromptCapabilities::new()
                            .image(true)
                            .audio(true)
                            .embedded_context(true),
                    ),
                    cx,
                )
            });
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
            _title: Option<SharedString>,
            cx: &mut App,
        ) -> Task<gpui::Result<Entity<AcpThread>>> {
            *self.captured_work_dirs.lock() = Some(work_dirs.clone());
            let action_log = cx.new(|_| ActionLog::new(project.clone()));
            let thread = cx.new(|cx| {
                AcpThread::new(
                    None,
                    None,
                    Some(work_dirs),
                    self.clone(),
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
            });
            Task::ready(Ok(thread))
        }

        fn auth_methods(&self) -> &[acp::AuthMethod] {
            &[]
        }

        fn authenticate(
            &self,
            _method_id: acp::AuthMethodId,
            _cx: &mut App,
        ) -> Task<gpui::Result<()>> {
            Task::ready(Ok(()))
        }

        fn prompt(
            &self,
            _params: acp::PromptRequest,
            _cx: &mut App,
        ) -> Task<gpui::Result<acp::PromptResponse>> {
            Task::ready(Ok(acp::PromptResponse::new(acp::StopReason::EndTurn)))
        }

        fn cancel(&self, _session_id: &acp::SessionId, _cx: &mut App) {}

        fn into_any(self: Rc<Self>) -> Rc<dyn Any> {
            self
        }
    }

    /// `OMEGA-DELTA-0204`. Picking a tier before the session exists moves the
    /// model, not only the label on the control.
    ///
    /// The connected composer's tier control reaches a running session through
    /// its model selector, which also writes `agent.default_model`. The
    /// pre-session composer has no session and no selector, so it writes that
    /// default itself — the one thing a session that does not exist yet
    /// actually reads when it is created. Without this the control would be
    /// decoration: the face would say Pro and Luna would answer, which is
    /// `OMEGA-DELTA-0202`'s defect reintroduced one composer earlier.
    #[gpui::test]
    async fn a_tier_chosen_before_the_session_exists_moves_the_model(cx: &mut TestAppContext) {
        init_test(cx);
        crate::omega_model_tier::clear_selection_for_test();

        let fs = FakeFs::new(cx.executor());
        fs.create_dir(
            paths::settings_file()
                .parent()
                .expect("settings have a parent"),
        )
        .await
        .expect("the settings directory is creatable");
        fs.insert_file(
            paths::settings_file(),
            json!({ "agent": { "default_model": { "provider": "openagents", "model": "gpt-5.6-luna" } } })
                .to_string()
                .into_bytes(),
        )
        .await;

        cx.update(|cx| {
            crate::omega_model_tier::select_before_session(
                crate::omega_model_tier::ModelTier::Pro,
                fs.clone(),
                cx,
            );
        });
        cx.run_until_parked();

        let settings = fs
            .load(paths::settings_file())
            .await
            .expect("the settings file is readable");
        let settings: serde_json::Value =
            serde_json::from_str(&settings).expect("the settings file stays valid JSON");

        assert_eq!(
            settings["agent"]["default_model"]["provider"],
            json!("openagents"),
            "the tier a person picked before connecting must reach the default \
             every new session reads"
        );
        assert_eq!(
            settings["agent"]["default_model"]["model"],
            json!("kimi-k3"),
            "Pro is Kimi K3; a control that only repainted its own label would \
             leave Luna here and answer as Luna"
        );
        assert_eq!(
            crate::omega_model_tier::selected(),
            crate::omega_model_tier::ModelTier::Pro,
            "the standing choice is what the pre-session face reads, so it has \
             to agree with the default that was just written"
        );

        crate::omega_model_tier::clear_selection_for_test();
    }

    #[gpui::test]
    async fn a_direct_model_chosen_before_the_session_exists_moves_the_model(
        cx: &mut TestAppContext,
    ) {
        init_test(cx);

        let fs = FakeFs::new(cx.executor());
        fs.create_dir(
            paths::settings_file()
                .parent()
                .expect("settings have a parent"),
        )
        .await
        .expect("the settings directory is creatable");
        fs.insert_file(
            paths::settings_file(),
            json!({ "agent": { "default_model": { "provider": "openagents", "model": "gpt-5.6-luna" } } })
                .to_string()
                .into_bytes(),
        )
        .await;

        cx.update(|cx| {
            crate::omega_model_tier::select_model_before_session(
                "deepseek/deepseek-v4-flash",
                fs.clone(),
                cx,
            );
        });
        cx.run_until_parked();

        let settings = fs
            .load(paths::settings_file())
            .await
            .expect("the settings file is readable");
        let settings: serde_json::Value =
            serde_json::from_str(&settings).expect("the settings file stays valid JSON");

        assert_eq!(
            settings["agent"]["default_model"]["provider"],
            json!("deepseek")
        );
        assert_eq!(
            settings["agent"]["default_model"]["model"],
            json!("deepseek-v4-flash")
        );
    }

    pub(crate) fn init_test(cx: &mut TestAppContext) {
        cx.update(|cx| {
            let settings_store = SettingsStore::test(cx);
            cx.set_global(settings_store);
            // Use an isolated DB so parallel tests can't overwrite each
            // other's global keys (e.g. the last-created entry kind).
            cx.set_global(db::AppDatabase::test_new());
            ThreadMetadataStore::init_global(cx);
            theme_settings::init(theme::LoadThemes::JustBase, cx);
            editor::init(cx);
            agent_panel::init(cx);
            release_channel::init(semver::Version::new(0, 0, 0), cx);
            prompt_store::init(cx)
        });
    }

    fn active_thread(
        conversation_view: &Entity<ConversationView>,
        cx: &TestAppContext,
    ) -> Entity<ThreadView> {
        cx.read(|cx| {
            conversation_view
                .read(cx)
                .active_thread()
                .expect("No active thread")
                .clone()
        })
    }

    fn assert_thread_list_item_count_matches_entries(view: &ThreadView, cx: &App) {
        assert_eq!(
            view.list_state.item_count(),
            view.thread.read(cx).entries().len() + usize::from(view.generating_indicator_in_list)
        );
    }

    fn message_editor(
        conversation_view: &Entity<ConversationView>,
        cx: &TestAppContext,
    ) -> Entity<MessageEditor> {
        let thread = active_thread(conversation_view, cx);
        cx.read(|cx| thread.read(cx).message_editor.clone())
    }

    #[gpui::test]
    async fn test_rewind_views(cx: &mut TestAppContext) {
        init_test(cx);

        let fs = FakeFs::new(cx.executor());
        fs.insert_tree(
            "/project",
            json!({
                "test1.txt": "old content 1",
                "test2.txt": "old content 2"
            }),
        )
        .await;
        let project = Project::test(fs, [Path::new("/project")], cx).await;
        let (multi_workspace, cx) =
            cx.add_window_view(|window, cx| MultiWorkspace::test_new(project.clone(), window, cx));
        let workspace = multi_workspace.read_with(cx, |mw, _| mw.workspace().clone());

        let thread_store = cx.update(|_window, cx| cx.new(|cx| ThreadStore::new(cx)));
        let connection_store =
            cx.update(|_window, cx| cx.new(|cx| AgentConnectionStore::new(project.clone(), cx)));

        let connection = Rc::new(StubAgentConnection::new());
        let conversation_view = cx.update(|window, cx| {
            cx.new(|cx| {
                ConversationView::new(
                    Rc::new(StubAgentServer::new(connection.as_ref().clone())),
                    connection_store,
                    Agent::Custom { id: "Test".into() },
                    None,
                    None,
                    None,
                    None,
                    None,
                    workspace.downgrade(),
                    project.clone(),
                    Some(thread_store.clone()),
                    AgentThreadSource::AgentPanel,
                    window,
                    cx,
                )
            })
        });

        cx.run_until_parked();

        let thread = conversation_view
            .read_with(cx, |view, cx| {
                view.active_thread().map(|r| r.read(cx).thread.clone())
            })
            .unwrap();

        // First user message
        connection.set_next_prompt_updates(vec![acp::SessionUpdate::ToolCall(
            acp::ToolCall::new("tool1", "Edit file 1")
                .kind(acp::ToolKind::Edit)
                .status(acp::ToolCallStatus::Completed)
                .content(vec![acp::ToolCallContent::Diff(
                    acp::Diff::new("/project/test1.txt", "new content 1").old_text("old content 1"),
                )]),
        )]);

        thread
            .update(cx, |thread, cx| thread.send_raw("Give me a diff", cx))
            .await
            .unwrap();
        cx.run_until_parked();

        thread.read_with(cx, |thread, _cx| {
            assert_eq!(thread.entries().len(), 2);
        });

        conversation_view.read_with(cx, |view, cx| {
            let entry_view_state = view
                .active_thread()
                .map(|active| active.read(cx).entry_view_state.clone())
                .unwrap();
            entry_view_state.read_with(cx, |entry_view_state, _| {
                assert!(
                    entry_view_state
                        .entry(0)
                        .unwrap()
                        .message_editor()
                        .is_some()
                );
                assert!(entry_view_state.entry(1).unwrap().has_content());
            });
        });

        // Second user message
        connection.set_next_prompt_updates(vec![acp::SessionUpdate::ToolCall(
            acp::ToolCall::new("tool2", "Edit file 2")
                .kind(acp::ToolKind::Edit)
                .status(acp::ToolCallStatus::Completed)
                .content(vec![acp::ToolCallContent::Diff(
                    acp::Diff::new("/project/test2.txt", "new content 2").old_text("old content 2"),
                )]),
        )]);

        thread
            .update(cx, |thread, cx| thread.send_raw("Another one", cx))
            .await
            .unwrap();
        cx.run_until_parked();

        let second_user_message_id = thread.read_with(cx, |thread, _| {
            assert_eq!(thread.entries().len(), 4);
            let AgentThreadEntry::UserMessage(user_message) = &thread.entries()[2] else {
                panic!();
            };
            user_message.client_id.clone().unwrap()
        });

        conversation_view.read_with(cx, |view, cx| {
            let entry_view_state = view
                .active_thread()
                .unwrap()
                .read(cx)
                .entry_view_state
                .clone();
            entry_view_state.read_with(cx, |entry_view_state, _| {
                assert!(
                    entry_view_state
                        .entry(0)
                        .unwrap()
                        .message_editor()
                        .is_some()
                );
                assert!(entry_view_state.entry(1).unwrap().has_content());
                assert!(
                    entry_view_state
                        .entry(2)
                        .unwrap()
                        .message_editor()
                        .is_some()
                );
                assert!(entry_view_state.entry(3).unwrap().has_content());
            });
        });

        // Rewind to first message
        thread
            .update(cx, |thread, cx| thread.rewind(second_user_message_id, cx))
            .await
            .unwrap();

        cx.run_until_parked();

        thread.read_with(cx, |thread, _| {
            assert_eq!(thread.entries().len(), 2);
        });

        conversation_view.read_with(cx, |view, cx| {
            let active = view.active_thread().unwrap();
            active
                .read(cx)
                .entry_view_state
                .read_with(cx, |entry_view_state, _| {
                    assert!(
                        entry_view_state
                            .entry(0)
                            .unwrap()
                            .message_editor()
                            .is_some()
                    );
                    assert!(entry_view_state.entry(1).unwrap().has_content());

                    // Old views should be dropped
                    assert!(entry_view_state.entry(2).is_none());
                    assert!(entry_view_state.entry(3).is_none());
                });
        });
    }

    #[gpui::test]
    async fn test_regenerate_keeps_pending_subagent_edits(cx: &mut TestAppContext) {
        init_test(cx);

        let fs = FakeFs::new(cx.executor());
        fs.insert_tree(
            "/project",
            json!({
                "file.txt": "original content"
            }),
        )
        .await;
        let project = Project::test(fs, [Path::new("/project")], cx).await;
        let (multi_workspace, cx) =
            cx.add_window_view(|window, cx| MultiWorkspace::test_new(project.clone(), window, cx));
        let workspace = multi_workspace.read_with(cx, |mw, _| mw.workspace().clone());

        let thread_store = cx.update(|_window, cx| cx.new(|cx| ThreadStore::new(cx)));
        let connection_store =
            cx.update(|_window, cx| cx.new(|cx| AgentConnectionStore::new(project.clone(), cx)));

        let connection = Rc::new(StubAgentConnection::new());
        let conversation_view = cx.update(|window, cx| {
            cx.new(|cx| {
                ConversationView::new(
                    Rc::new(StubAgentServer::new(connection.as_ref().clone())),
                    connection_store,
                    Agent::Custom { id: "Test".into() },
                    None,
                    None,
                    None,
                    None,
                    None,
                    workspace.downgrade(),
                    project.clone(),
                    Some(thread_store.clone()),
                    AgentThreadSource::AgentPanel,
                    window,
                    cx,
                )
            })
        });

        cx.run_until_parked();

        let thread = conversation_view
            .read_with(cx, |view, cx| {
                view.active_thread().map(|r| r.read(cx).thread.clone())
            })
            .unwrap();

        // First turn: a subagent tool call. Subagent edits never appear as
        // diffs in the parent thread's entries; they are only forwarded to the
        // parent's action log through the linked-log mechanism.
        connection.set_next_prompt_updates(vec![acp::SessionUpdate::ToolCall(
            acp::ToolCall::new("spawn1", "Subagent task")
                .kind(acp::ToolKind::Other)
                .status(acp::ToolCallStatus::Completed)
                .meta(acp_thread::meta_with_tool_name("spawn_agent")),
        )]);

        thread
            .update(cx, |thread, cx| thread.send_raw("Use a subagent", cx))
            .await
            .unwrap();
        cx.run_until_parked();

        // Simulate the subagent editing a file: edits performed through a
        // child action log are forwarded to the parent thread's action log,
        // just like `Thread::new_subagent` wires it up.
        let parent_action_log = thread.read_with(cx, |thread, _| thread.action_log().clone());
        let subagent_action_log = cx.update(|_, cx| {
            cx.new(|_| {
                ActionLog::new(project.clone()).with_linked_action_log(parent_action_log.clone())
            })
        });

        let buffer = project
            .update(cx, |project, cx| {
                let path = project.find_project_path("file.txt", cx).unwrap();
                project.open_buffer(path, cx)
            })
            .await
            .unwrap();
        cx.update(|_, cx| {
            subagent_action_log.update(cx, |log, cx| log.buffer_read(buffer.clone(), cx));
            buffer.update(cx, |buffer, cx| {
                buffer.set_text("edited by subagent", cx);
            });
            subagent_action_log.update(cx, |log, cx| log.buffer_edited(buffer.clone(), cx));
        });
        cx.run_until_parked();

        parent_action_log.read_with(cx, |log, cx| {
            assert_eq!(
                log.changed_buffers(cx).count(),
                1,
                "the subagent edit should be pending review in the parent's action log"
            );
        });

        // Second turn: a plain follow-up.
        connection.set_next_prompt_updates(vec![acp::SessionUpdate::AgentMessageChunk(
            acp::ContentChunk::new("Response".into()),
        )]);
        thread
            .update(cx, |thread, cx| thread.send_raw("Follow-up", cx))
            .await
            .unwrap();
        cx.run_until_parked();

        let follow_up_ix = thread.read_with(cx, |thread, cx| {
            thread
                .entries()
                .iter()
                .position(|entry| entry.to_markdown(cx) == "## User\n\nFollow-up\n\n")
                .unwrap()
        });

        // Edit and regenerate the follow-up message.
        let user_message_editor = conversation_view.read_with(cx, |view, cx| {
            view.active_thread()
                .unwrap()
                .read(cx)
                .entry_view_state
                .read(cx)
                .entry(follow_up_ix)
                .unwrap()
                .message_editor()
                .unwrap()
                .clone()
        });
        user_message_editor.update_in(cx, |editor, window, cx| {
            editor.set_text("Edited follow-up", window, cx);
        });

        connection.set_next_prompt_updates(vec![acp::SessionUpdate::AgentMessageChunk(
            acp::ContentChunk::new("New response".into()),
        )]);
        active_thread(&conversation_view, cx).update_in(cx, |view, window, cx| {
            view.regenerate(follow_up_ix, user_message_editor.clone(), window, cx);
        });
        cx.run_until_parked();

        // The thread should have been rewound and the edited message resent.
        thread.read_with(cx, |thread, cx| {
            let entries = thread.entries();
            assert_eq!(entries.len(), 4);
            assert_eq!(
                entries[2].to_markdown(cx),
                "## User\n\nEdited follow-up\n\n"
            );
        });

        // The subagent's edits predate the regenerated prompt, so they must be
        // auto-kept rather than rejected by the rewind.
        buffer.read_with(cx, |buffer, _| {
            assert_eq!(
                buffer.text(),
                "edited by subagent",
                "pending subagent edits should be kept when regenerating a later prompt"
            );
        });
        parent_action_log.read_with(cx, |log, cx| {
            assert_eq!(
                log.changed_buffers(cx).count(),
                0,
                "the subagent edit should have been auto-kept"
            );
        });
    }

    #[gpui::test]
    async fn test_scroll_to_most_recent_user_prompt(cx: &mut TestAppContext) {
        init_test(cx);

        let connection = StubAgentConnection::new();

        // Each user prompt will result in a user message entry plus an agent message entry.
        connection.set_next_prompt_updates(vec![acp::SessionUpdate::AgentMessageChunk(
            acp::ContentChunk::new("Response 1".into()),
        )]);

        let (conversation_view, cx) =
            setup_conversation_view(StubAgentServer::new(connection.clone()), cx).await;

        let thread = conversation_view
            .read_with(cx, |view, cx| {
                view.active_thread().map(|r| r.read(cx).thread.clone())
            })
            .unwrap();

        thread
            .update(cx, |thread, cx| thread.send_raw("Prompt 1", cx))
            .await
            .unwrap();
        cx.run_until_parked();

        connection.set_next_prompt_updates(vec![acp::SessionUpdate::AgentMessageChunk(
            acp::ContentChunk::new("Response 2".into()),
        )]);

        thread
            .update(cx, |thread, cx| thread.send_raw("Prompt 2", cx))
            .await
            .unwrap();
        cx.run_until_parked();

        // Move somewhere else first so we're not trivially already on the last user prompt.
        active_thread(&conversation_view, cx).update(cx, |view, cx| {
            view.scroll_to_top(cx);
        });
        cx.run_until_parked();

        active_thread(&conversation_view, cx).update(cx, |view, cx| {
            view.scroll_to_user_message_index(None, cx);
            let scroll_top = view.list_state.logical_scroll_top();
            // Entries layout is: [User1, Assistant1, User2, Assistant2]
            assert_eq!(scroll_top.item_ix, 2);

            view.scroll_to_top(cx);
            view.scroll_to_user_message_index(Some(0), cx);
            let scroll_top = view.list_state.logical_scroll_top();
            assert_eq!(scroll_top.item_ix, 0);

            view.scroll_to_top(cx);
            view.scroll_to_user_message_index(Some(2), cx);
            let scroll_top = view.list_state.logical_scroll_top();
            assert_eq!(scroll_top.item_ix, 2);
        });
    }

    #[gpui::test]
    async fn test_scroll_to_most_recent_user_prompt_falls_back_to_bottom_without_user_messages(
        cx: &mut TestAppContext,
    ) {
        init_test(cx);

        let (conversation_view, cx) =
            setup_conversation_view(StubAgentServer::default_response(), cx).await;

        // With no entries, scrolling should be a no-op and must not panic.
        active_thread(&conversation_view, cx).update(cx, |view, cx| {
            view.scroll_to_user_message_index(None, cx);
            let scroll_top = view.list_state.logical_scroll_top();
            assert_eq!(scroll_top.item_ix, 0);
        });
    }

    #[gpui::test]
    async fn test_thread_search_finds_matches_across_entries(cx: &mut TestAppContext) {
        init_test(cx);

        let connection = StubAgentConnection::new();
        connection.set_next_prompt_updates(vec![acp::SessionUpdate::AgentMessageChunk(
            acp::ContentChunk::new(
                "Yes, you can substitute banana for plantain in this recipe.".into(),
            ),
        )]);

        let (conversation_view, cx) =
            setup_conversation_view(StubAgentServer::new(connection.clone()), cx).await;

        let thread =
            active_thread(&conversation_view, cx).read_with(cx, |view, _| view.thread.clone());

        thread
            .update(cx, |thread, cx| {
                thread.send_raw("Can I use banana here?", cx)
            })
            .await
            .unwrap();
        cx.run_until_parked();

        connection.set_next_prompt_updates(vec![acp::SessionUpdate::AgentMessageChunk(
            acp::ContentChunk::new(
                "Banana yogurt also works as a topping; whisk the banana smooth first.".into(),
            ),
        )]);

        thread
            .update(cx, |thread, cx| {
                thread.send_raw("What about as a topping?", cx)
            })
            .await
            .unwrap();
        cx.run_until_parked();

        let thread_view = active_thread(&conversation_view, cx);
        thread_view.update_in(cx, |view, window, cx| {
            view.toggle_search(&crate::ToggleSearch, window, cx);
        });
        cx.run_until_parked();

        let bar = thread_view
            .read_with(cx, |view, _| view.thread_search_bar.clone())
            .expect("thread_search_bar should be set after toggle_search");
        bar.update_in(cx, |bar, window, cx| {
            bar.query_editor.update(cx, |editor, cx| {
                editor.set_text("banana", window, cx);
            });
            bar.update_matches(window, cx);
        });
        cx.run_until_parked();

        let (match_count, active_text) =
            bar.read_with(cx, |bar, cx| (bar.match_count(), bar.active_match_text(cx)));
        assert_eq!(
            match_count, 4,
            "expected 4 matches for case-insensitive 'banana'"
        );
        assert_eq!(active_text.as_deref(), Some("1/4"));

        thread_view.read_with(cx, |view, _| {
            assert_eq!(view.list_state.logical_scroll_top().item_ix, 0);
        });

        bar.update_in(cx, |bar, window, cx| {
            bar.select_next_match(&super::thread_search_bar::SelectNextThreadMatch, window, cx);
        });
        cx.run_until_parked();
        let active_text_2 = bar.read_with(cx, |bar, cx| bar.active_match_text(cx));
        assert_eq!(active_text_2.as_deref(), Some("2/4"));

        bar.update_in(cx, |bar, window, cx| {
            bar.select_prev_match(
                &super::thread_search_bar::SelectPreviousThreadMatch,
                window,
                cx,
            );
        });
        let active_text_3 = bar.read_with(cx, |bar, cx| bar.active_match_text(cx));
        assert_eq!(active_text_3.as_deref(), Some("1/4"));

        bar.update_in(cx, |bar, window, cx| {
            bar.query_editor.update(cx, |editor, cx| {
                editor.set_text("apple", window, cx);
            });
            bar.update_matches(window, cx);
        });
        cx.run_until_parked();
        let (match_count_apple, active_text_apple) =
            bar.read_with(cx, |bar, cx| (bar.match_count(), bar.active_match_text(cx)));
        assert_eq!(match_count_apple, 0);
        assert_eq!(active_text_apple.as_deref(), Some("0/0"));
    }

    #[gpui::test]
    async fn test_thread_search_includes_expanded_thinking_blocks(cx: &mut TestAppContext) {
        init_test(cx);

        // omega#112. Ask for a collapsed block rather than inheriting one.
        //
        // This test is about what search does with a *collapsed* thinking
        // block, and it used to get one for free because "auto" was the
        // shipped default. Omega now ships "always_expanded", so the premise
        // has to be stated: without this the test builds an expanded block and
        // then asserts the behaviour of a collapsed one.
        cx.update(|cx| {
            let mut settings = AgentSettings::get_global(cx).clone();
            settings.thinking_display = settings::ThinkingBlockDisplay::Auto;
            AgentSettings::override_global(settings, cx);
        });

        let connection = StubAgentConnection::new();
        connection.set_next_prompt_updates(vec![
            acp::SessionUpdate::AgentThoughtChunk(acp::ContentChunk::new(
                "Hidden papaya reasoning.".into(),
            )),
            acp::SessionUpdate::AgentMessageChunk(acp::ContentChunk::new(
                "Final answer without that fruit.".into(),
            )),
        ]);

        let (conversation_view, cx) =
            setup_conversation_view(StubAgentServer::new(connection.clone()), cx).await;

        let thread =
            active_thread(&conversation_view, cx).read_with(cx, |view, _| view.thread.clone());
        thread
            .update(cx, |thread, cx| thread.send_raw("Think this through", cx))
            .await
            .unwrap();
        cx.run_until_parked();

        let (assistant_entry_ix, thought_chunk_ix) = thread.read_with(cx, |thread, _| {
            thread
                .entries()
                .iter()
                .enumerate()
                .find_map(|(entry_ix, entry)| match entry {
                    AgentThreadEntry::AssistantMessage(message) => message
                        .chunks
                        .iter()
                        .position(|chunk| matches!(chunk, AssistantMessageChunk::Thought { .. }))
                        .map(|chunk_ix| (entry_ix, chunk_ix)),
                    _ => None,
                })
                .expect("assistant thought chunk should exist")
        });

        let thread_view = active_thread(&conversation_view, cx);
        thread_view.update_in(cx, |view, window, cx| {
            view.toggle_search(&crate::ToggleSearch, window, cx);
        });
        cx.run_until_parked();

        let bar = thread_view
            .read_with(cx, |view, _| view.thread_search_bar.clone())
            .expect("thread_search_bar should be set after toggle_search");
        bar.update_in(cx, |bar, window, cx| {
            bar.query_editor.update(cx, |editor, cx| {
                editor.set_text("papaya", window, cx);
            });
            bar.update_matches(window, cx);
        });
        cx.run_until_parked();
        assert_eq!(
            bar.read_with(cx, |bar, _| bar.match_count()),
            0,
            "collapsed thinking content should not be searched",
        );

        thread_view.update(cx, |view, cx| {
            view.entry_view_state.update(cx, |state, cx| {
                state.toggle_thinking_block_expansion((assistant_entry_ix, thought_chunk_ix), cx);
            });
        });
        bar.update_in(cx, |bar, window, cx| {
            bar.update_matches(window, cx);
        });
        cx.run_until_parked();

        assert_eq!(
            bar.read_with(cx, |bar, _| bar.match_count()),
            1,
            "expanded thinking content should be searchable",
        );
    }

    #[gpui::test]
    async fn test_thread_search_includes_expanded_tool_call_content(cx: &mut TestAppContext) {
        init_test(cx);

        let tool_call_id = acp::ToolCallId::new("search-tool-content");
        let connection = StubAgentConnection::new();
        connection.set_next_prompt_updates(vec![acp::SessionUpdate::ToolCall(
            acp::ToolCall::new(tool_call_id.clone(), "Inspect output")
                .kind(acp::ToolKind::Other)
                .status(acp::ToolCallStatus::Completed)
                .content(vec!["Tool output mentions papaya once.".into()]),
        )]);

        let (conversation_view, cx) =
            setup_conversation_view(StubAgentServer::new(connection.clone()), cx).await;

        let thread =
            active_thread(&conversation_view, cx).read_with(cx, |view, _| view.thread.clone());
        thread
            .update(cx, |thread, cx| thread.send_raw("Run the tool", cx))
            .await
            .unwrap();
        cx.run_until_parked();

        let thread_view = active_thread(&conversation_view, cx);
        thread_view.update_in(cx, |view, window, cx| {
            view.toggle_search(&crate::ToggleSearch, window, cx);
        });
        cx.run_until_parked();

        let bar = thread_view
            .read_with(cx, |view, _| view.thread_search_bar.clone())
            .expect("thread_search_bar should be set after toggle_search");
        bar.update_in(cx, |bar, window, cx| {
            bar.query_editor.update(cx, |editor, cx| {
                editor.set_text("papaya", window, cx);
            });
            bar.update_matches(window, cx);
        });
        cx.run_until_parked();
        assert_eq!(
            bar.read_with(cx, |bar, _| bar.match_count()),
            0,
            "collapsed tool-call content should not be searched",
        );

        thread_view.update(cx, |view, cx| {
            view.entry_view_state.update(cx, |state, _cx| {
                state.expand_tool_call(tool_call_id);
            });
        });
        bar.update_in(cx, |bar, window, cx| {
            bar.update_matches(window, cx);
        });
        cx.run_until_parked();

        assert_eq!(
            bar.read_with(cx, |bar, _| bar.match_count()),
            1,
            "expanded tool-call content should be searchable",
        );
    }

    #[gpui::test]
    async fn test_thread_search_scrolls_to_later_user_message_match(cx: &mut TestAppContext) {
        init_test(cx);

        let connection = StubAgentConnection::new();
        connection.set_next_prompt_updates(vec![acp::SessionUpdate::AgentMessageChunk(
            acp::ContentChunk::new("First reply, no fruit here.".into()),
        )]);

        let (conversation_view, cx) =
            setup_conversation_view(StubAgentServer::new(connection.clone()), cx).await;
        add_to_workspace(conversation_view.clone(), cx);

        let thread =
            active_thread(&conversation_view, cx).read_with(cx, |view, _| view.thread.clone());
        thread
            .update(cx, |thread, cx| thread.send_raw("First question", cx))
            .await
            .unwrap();
        cx.run_until_parked();
        connection.set_next_prompt_updates(vec![acp::SessionUpdate::AgentMessageChunk(
            acp::ContentChunk::new("Second reply, still no fruit.".into()),
        )]);
        thread
            .update(cx, |thread, cx| thread.send_raw("Where is the papaya?", cx))
            .await
            .unwrap();
        cx.run_until_parked();

        let thread_view = active_thread(&conversation_view, cx);
        let papaya_entry_ix = thread.read_with(cx, |thread, _| {
            thread
                .entries()
                .iter()
                .rposition(|entry| matches!(entry, AgentThreadEntry::UserMessage(_)))
                .expect("a user message entry should exist")
        });

        thread_view.update_in(cx, |view, window, cx| {
            view.toggle_search(&crate::ToggleSearch, window, cx);
        });
        cx.run_until_parked();
        let bar = thread_view
            .read_with(cx, |view, _| view.thread_search_bar.clone())
            .expect("thread_search_bar should be set after toggle_search");
        bar.update_in(cx, |bar, window, cx| {
            bar.query_editor.update(cx, |editor, cx| {
                editor.set_text("papaya", window, cx);
            });
            bar.update_matches(window, cx);
        });
        cx.run_until_parked();

        bar.read_with(cx, |bar, _| {
            assert_eq!(
                bar.match_count(),
                1,
                "only the second user message matches 'papaya'"
            );
        });

        thread_view.read_with(cx, |view, _| {
            assert_eq!(
                view.list_state.logical_scroll_top().item_ix,
                papaya_entry_ix,
                "list should scroll to the user-message entry that owns the match",
            );
        });
    }

    /// Passive rescans (streaming updates, unrelated expansion toggles, query
    /// refinement) must not yank the list back to the active match.
    #[gpui::test]
    async fn test_thread_search_passive_rescan_preserves_scroll(cx: &mut TestAppContext) {
        init_test(cx);

        let connection = StubAgentConnection::new();
        connection.set_next_prompt_updates(vec![acp::SessionUpdate::AgentMessageChunk(
            acp::ContentChunk::new("First reply, no fruit here.".into()),
        )]);

        let (conversation_view, cx) =
            setup_conversation_view(StubAgentServer::new(connection.clone()), cx).await;
        add_to_workspace(conversation_view.clone(), cx);

        let thread =
            active_thread(&conversation_view, cx).read_with(cx, |view, _| view.thread.clone());
        thread
            .update(cx, |thread, cx| thread.send_raw("First question", cx))
            .await
            .unwrap();
        cx.run_until_parked();
        connection.set_next_prompt_updates(vec![acp::SessionUpdate::AgentMessageChunk(
            acp::ContentChunk::new("Second reply, still no fruit.".into()),
        )]);
        thread
            .update(cx, |thread, cx| thread.send_raw("Where is the papaya?", cx))
            .await
            .unwrap();
        cx.run_until_parked();

        let thread_view = active_thread(&conversation_view, cx);
        let papaya_entry_ix = thread.read_with(cx, |thread, _| {
            thread
                .entries()
                .iter()
                .rposition(|entry| matches!(entry, AgentThreadEntry::UserMessage(_)))
                .expect("a user message entry should exist")
        });

        thread_view.update_in(cx, |view, window, cx| {
            view.toggle_search(&crate::ToggleSearch, window, cx);
        });
        cx.run_until_parked();
        let bar = thread_view
            .read_with(cx, |view, _| view.thread_search_bar.clone())
            .expect("thread_search_bar should be set after toggle_search");
        bar.update_in(cx, |bar, window, cx| {
            bar.query_editor.update(cx, |editor, cx| {
                editor.set_text("papaya", window, cx);
            });
            bar.update_matches(window, cx);
        });
        cx.run_until_parked();

        // The initial query activates (and scrolls to) the only match.
        thread_view.read_with(cx, |view, _| {
            assert_eq!(
                view.list_state.logical_scroll_top().item_ix,
                papaya_entry_ix,
            );
        });

        // Simulate the user scrolling elsewhere, then a passive rescan re-running
        // the matcher while the same hit stays active.
        thread_view.update(cx, |view, _| {
            view.list_state.scroll_to(gpui::ListOffset {
                item_ix: 0,
                offset_in_item: gpui::px(0.),
            });
        });
        bar.update_in(cx, |bar, window, cx| {
            bar.update_matches(window, cx);
        });
        cx.run_until_parked();

        thread_view.read_with(cx, |view, _| {
            assert_eq!(
                view.list_state.logical_scroll_top().item_ix,
                0,
                "a passive rescan must not scroll back to the active match",
            );
        });
    }

    #[gpui::test]
    async fn test_thread_search_dismiss_clears_highlights(cx: &mut TestAppContext) {
        init_test(cx);

        let connection = StubAgentConnection::new();
        connection.set_next_prompt_updates(vec![acp::SessionUpdate::AgentMessageChunk(
            acp::ContentChunk::new("Mango is a tropical fruit.".into()),
        )]);

        let (conversation_view, cx) =
            setup_conversation_view(StubAgentServer::new(connection.clone()), cx).await;

        let thread =
            active_thread(&conversation_view, cx).read_with(cx, |view, _| view.thread.clone());

        thread
            .update(cx, |thread, cx| thread.send_raw("Tell me about mango", cx))
            .await
            .unwrap();
        cx.run_until_parked();

        let thread_view = active_thread(&conversation_view, cx);
        thread_view.update_in(cx, |view, window, cx| {
            view.toggle_search(&crate::ToggleSearch, window, cx);
        });
        cx.run_until_parked();

        let bar = thread_view
            .read_with(cx, |view, _| view.thread_search_bar.clone())
            .unwrap();
        bar.update_in(cx, |bar, window, cx| {
            bar.query_editor.update(cx, |editor, cx| {
                editor.set_text("mango", window, cx);
            });
            bar.update_matches(window, cx);
        });
        cx.run_until_parked();

        let entries = thread.read_with(cx, |thread, _| thread.entries().len());
        assert!(entries > 0);

        bar.update(cx, |bar, cx| bar.clear_highlights(cx));
        cx.run_until_parked();

        bar.read_with(cx, |bar, _| {
            assert_eq!(bar.match_count(), 0);
            assert!(bar.active_match_index().is_none());
        });
    }

    #[gpui::test]
    async fn test_thread_search_release_clears_markdown_highlights(cx: &mut TestAppContext) {
        init_test(cx);

        let connection = StubAgentConnection::new();
        connection.set_next_prompt_updates(vec![acp::SessionUpdate::AgentMessageChunk(
            acp::ContentChunk::new("Mango is a tropical fruit.".into()),
        )]);

        let (conversation_view, cx) =
            setup_conversation_view(StubAgentServer::new(connection.clone()), cx).await;

        let thread =
            active_thread(&conversation_view, cx).read_with(cx, |view, _| view.thread.clone());
        thread
            .update(cx, |thread, cx| thread.send_raw("Tell me about mango", cx))
            .await
            .unwrap();
        cx.run_until_parked();

        let assistant_markdown = thread.read_with(cx, |thread, _| {
            thread
                .entries()
                .iter()
                .find_map(|entry| match entry {
                    AgentThreadEntry::AssistantMessage(message) => {
                        message.chunks.iter().find_map(|chunk| match chunk {
                            AssistantMessageChunk::Message { block, .. } => {
                                block.markdown().cloned()
                            }
                            AssistantMessageChunk::Thought { .. } => None,
                        })
                    }
                    _ => None,
                })
                .expect("assistant message should have markdown")
        });

        let entry_view_state = active_thread(&conversation_view, cx)
            .read_with(cx, |view, _| view.entry_view_state.clone());
        let on_activate_match: Arc<dyn Fn(usize, &mut Window, &mut App)> = Arc::new(|_, _, _| {});
        let bar = cx.update(|window, cx| {
            cx.new(|cx| {
                super::thread_search_bar::ThreadSearchBar::new(
                    thread.clone(),
                    entry_view_state,
                    on_activate_match,
                    window,
                    cx,
                )
            })
        });

        bar.update_in(cx, |bar, window, cx| {
            bar.query_editor.update(cx, |editor, cx| {
                editor.set_text("mango", window, cx);
            });
            bar.update_matches(window, cx);
        });
        cx.run_until_parked();

        assert!(
            assistant_markdown
                .read_with(cx, |markdown, _| !markdown.search_highlights().is_empty()),
            "search should have highlighted the assistant markdown before release",
        );

        drop(bar);
        cx.update(|_, _| {});
        cx.run_until_parked();

        assert!(
            assistant_markdown.read_with(cx, |markdown, _| markdown.search_highlights().is_empty()),
            "releasing the search bar should clear retained markdown highlights",
        );
    }

    #[gpui::test]
    async fn test_thread_search_refreshes_on_new_thread_entry(cx: &mut TestAppContext) {
        init_test(cx);

        let connection = StubAgentConnection::new();
        connection.set_next_prompt_updates(vec![acp::SessionUpdate::AgentMessageChunk(
            acp::ContentChunk::new("First reply mentions banana once.".into()),
        )]);

        let (conversation_view, cx) =
            setup_conversation_view(StubAgentServer::new(connection.clone()), cx).await;

        let thread =
            active_thread(&conversation_view, cx).read_with(cx, |view, _| view.thread.clone());
        thread
            .update(cx, |thread, cx| thread.send_raw("Tell me about banana", cx))
            .await
            .unwrap();
        cx.run_until_parked();

        let thread_view = active_thread(&conversation_view, cx);
        thread_view.update_in(cx, |view, window, cx| {
            view.toggle_search(&crate::ToggleSearch, window, cx);
        });
        cx.run_until_parked();

        let bar = thread_view
            .read_with(cx, |view, _| view.thread_search_bar.clone())
            .expect("thread_search_bar should be set after toggle_search");
        bar.update_in(cx, |bar, window, cx| {
            bar.query_editor.update(cx, |editor, cx| {
                editor.set_text("banana", window, cx);
            });
            bar.update_matches(window, cx);
        });
        cx.run_until_parked();

        let count_before = bar.read_with(cx, |bar, _| bar.match_count());
        assert!(
            count_before >= 2,
            "expected at least two initial matches, got {count_before}",
        );

        bar.update_in(cx, |bar, window, cx| {
            bar.select_next_match(&super::thread_search_bar::SelectNextThreadMatch, window, cx);
        });
        cx.run_until_parked();
        assert_eq!(
            bar.read_with(cx, |bar, _| bar.active_match_index()),
            Some(1),
            "setup precondition: second match should be active before the refresh",
        );

        // Advance past the debounced thread-update rescan.
        connection.set_next_prompt_updates(vec![acp::SessionUpdate::AgentMessageChunk(
            acp::ContentChunk::new("Banana banana: two more banana hits here.".into()),
        )]);
        thread
            .update(cx, |thread, cx| thread.send_raw("More banana please", cx))
            .await
            .unwrap();
        cx.run_until_parked();
        cx.executor()
            .advance_clock(super::thread_search_bar::SEARCH_UPDATE_DEBOUNCE * 2);
        cx.run_until_parked();

        let (count_after, active_after) =
            bar.read_with(cx, |bar, _| (bar.match_count(), bar.active_match_index()));
        assert!(
            count_after > count_before,
            "thread subscription should refresh matches after new content \
             streamed in: before={count_before}, after={count_after}",
        );
        assert_eq!(
            active_after,
            Some(1),
            "refreshing matches should preserve the active result when it still exists",
        );
    }

    /// Regression test for re-entering `ThreadView` during search navigation.
    #[gpui::test]
    async fn test_thread_search_select_next_from_thread_view_update_does_not_panic(
        cx: &mut TestAppContext,
    ) {
        init_test(cx);

        let connection = StubAgentConnection::new();
        connection.set_next_prompt_updates(vec![acp::SessionUpdate::AgentMessageChunk(
            acp::ContentChunk::new(
                "Banana banana banana, the banana fits the banana bread.".into(),
            ),
        )]);

        let (conversation_view, cx) =
            setup_conversation_view(StubAgentServer::new(connection.clone()), cx).await;

        let thread =
            active_thread(&conversation_view, cx).read_with(cx, |view, _| view.thread.clone());
        thread
            .update(cx, |thread, cx| thread.send_raw("Need banana help", cx))
            .await
            .unwrap();
        cx.run_until_parked();

        let thread_view = active_thread(&conversation_view, cx);
        thread_view.update_in(cx, |view, window, cx| {
            view.toggle_search(&crate::ToggleSearch, window, cx);
        });
        cx.run_until_parked();

        let bar = thread_view
            .read_with(cx, |view, _| view.thread_search_bar.clone())
            .expect("thread_search_bar should be set after toggle_search");
        bar.update_in(cx, |bar, window, cx| {
            bar.query_editor.update(cx, |editor, cx| {
                editor.set_text("banana", window, cx);
            });
            bar.update_matches(window, cx);
        });
        cx.run_until_parked();

        let initial_match_count = bar.read_with(cx, |bar, _| bar.match_count());
        assert!(
            initial_match_count >= 2,
            "setup precondition: expected at least 2 matches, got {}",
            initial_match_count,
        );

        thread_view.update_in(cx, |view, window, cx| {
            let bar = view
                .thread_search_bar
                .clone()
                .expect("bar should still be set");
            bar.update(cx, |bar, cx| {
                bar.select_next_match(&super::thread_search_bar::SelectNextThreadMatch, window, cx);
            });
        });
        cx.run_until_parked();

        let active_after = bar.read_with(cx, |bar, _| bar.active_match_index());
        assert_eq!(
            active_after,
            Some(1),
            "select_next_match should have advanced from match 0 to match 1",
        );
    }

    /// Past user-message hits must be painted on the inner `Editor`.
    #[gpui::test]
    async fn test_thread_search_highlights_user_message_editor(cx: &mut TestAppContext) {
        init_test(cx);

        let connection = StubAgentConnection::new();
        connection.set_next_prompt_updates(vec![acp::SessionUpdate::AgentMessageChunk(
            acp::ContentChunk::new("Sure, I can help with that.".into()),
        )]);

        let (conversation_view, cx) =
            setup_conversation_view(StubAgentServer::new(connection.clone()), cx).await;
        add_to_workspace(conversation_view.clone(), cx);

        let thread =
            active_thread(&conversation_view, cx).read_with(cx, |view, _| view.thread.clone());
        thread
            .update(cx, |thread, cx| {
                thread.send_raw("Where do I find a kumquat?", cx)
            })
            .await
            .unwrap();
        cx.run_until_parked();

        let thread_view = active_thread(&conversation_view, cx);
        thread_view.update_in(cx, |view, window, cx| {
            view.toggle_search(&crate::ToggleSearch, window, cx);
        });
        cx.run_until_parked();

        let bar = thread_view
            .read_with(cx, |view, _| view.thread_search_bar.clone())
            .expect("thread_search_bar should be set after toggle_search");
        bar.update_in(cx, |bar, window, cx| {
            bar.query_editor.update(cx, |editor, cx| {
                editor.set_text("kumquat", window, cx);
            });
            bar.update_matches(window, cx);
        });
        cx.run_until_parked();

        let match_count = bar.read_with(cx, |bar, _| bar.match_count());
        assert_eq!(
            match_count, 1,
            "expected exactly one match for 'kumquat' (in the user message)",
        );

        let user_message_editor = thread_view.read_with(cx, |view, cx| {
            view.entry_view_state
                .read(cx)
                .entry(0)
                .and_then(|entry| entry.message_editor())
                .map(|message_editor| message_editor.read(cx).editor().clone())
                .expect("entry 0 should be a user message with a message editor")
        });
        let has_highlight = user_message_editor.read_with(cx, |editor, _cx| {
            editor.has_background_highlights(editor::HighlightKey::BufferSearchHighlights)
        });
        assert!(
            has_highlight,
            "user message editor should carry BufferSearchHighlights after the bar's matcher ran",
        );

        bar.update(cx, |bar, cx| bar.clear_highlights(cx));
        cx.run_until_parked();
        let has_highlight_after_clear = user_message_editor.read_with(cx, |editor, _cx| {
            editor.has_background_highlights(editor::HighlightKey::BufferSearchHighlights)
        });
        assert!(
            !has_highlight_after_clear,
            "clear_highlights should remove the editor-backed highlights",
        );
    }

    /// `editor::Cancel` should dismiss thread search before reaching workspace handlers.
    #[gpui::test]
    async fn test_thread_search_editor_cancel_dismisses_bar(cx: &mut TestAppContext) {
        init_test(cx);

        let (conversation_view, cx) =
            setup_conversation_view(StubAgentServer::new(StubAgentConnection::new()), cx).await;
        add_to_workspace(conversation_view.clone(), cx);

        let thread_view = active_thread(&conversation_view, cx);
        thread_view.update_in(cx, |view, window, cx| {
            view.toggle_search(&crate::ToggleSearch, window, cx);
        });
        cx.run_until_parked();

        let visible_before = thread_view.read_with(cx, |view, _| view.thread_search_visible);
        assert!(
            visible_before,
            "search bar should be visible after toggle_search"
        );

        let bar = thread_view
            .read_with(cx, |view, _| view.thread_search_bar.clone())
            .expect("bar should be set");
        let query_focus = bar.read_with(cx, |bar, cx| bar.query_editor.focus_handle(cx));
        cx.update(|window, cx| {
            window.focus(&query_focus, cx);
        });
        cx.run_until_parked();

        conversation_view.update_in(cx, |_, window, cx| {
            window.dispatch_action(editor::actions::Cancel.boxed_clone(), cx);
        });
        cx.run_until_parked();

        let visible_after = thread_view.read_with(cx, |view, _| view.thread_search_visible);
        assert!(
            !visible_after,
            "editor::Cancel should have dismissed the bar before reaching the workspace",
        );
    }

    /// JetBrains keymaps route Shift+Enter through `editor::NewlineBelow`.
    #[gpui::test]
    async fn test_thread_search_shift_enter_navigates_with_jetbrains_keymap(
        cx: &mut TestAppContext,
    ) {
        init_test(cx);
        cx.update(|cx| {
            // Load both the default binding and the conflicting base keymap.
            search::init(cx);

            let mut default_bindings = settings::KeymapFile::load_asset_allow_partial_failure(
                "keymaps/default-linux.json",
                cx,
            )
            .unwrap();
            for binding in &mut default_bindings {
                binding.set_meta(settings::KeybindSource::Default.meta());
            }
            cx.bind_keys(default_bindings);

            let mut jetbrains_bindings = settings::KeymapFile::load_asset_allow_partial_failure(
                "keymaps/linux/jetbrains.json",
                cx,
            )
            .unwrap();
            for binding in &mut jetbrains_bindings {
                binding.set_meta(settings::KeybindSource::Base.meta());
            }
            cx.bind_keys(jetbrains_bindings);
        });

        let connection = StubAgentConnection::new();
        connection.set_next_prompt_updates(vec![acp::SessionUpdate::AgentMessageChunk(
            acp::ContentChunk::new(
                "Banana banana banana, multiple banana mentions in this reply.".into(),
            ),
        )]);

        let (conversation_view, cx) =
            setup_conversation_view(StubAgentServer::new(connection.clone()), cx).await;
        add_to_workspace(conversation_view.clone(), cx);

        let thread =
            active_thread(&conversation_view, cx).read_with(cx, |view, _| view.thread.clone());
        thread
            .update(cx, |thread, cx| thread.send_raw("Need banana help", cx))
            .await
            .unwrap();
        cx.run_until_parked();

        let thread_view = active_thread(&conversation_view, cx);
        thread_view.update_in(cx, |view, window, cx| {
            view.toggle_search(&crate::ToggleSearch, window, cx);
        });
        cx.run_until_parked();

        let bar = thread_view
            .read_with(cx, |view, _| view.thread_search_bar.clone())
            .expect("thread_search_bar should be set after toggle_search");

        bar.update_in(cx, |bar, window, cx| {
            bar.query_editor.update(cx, |editor, cx| {
                editor.set_text("banana", window, cx);
            });
            bar.update_matches(window, cx);
        });
        cx.run_until_parked();

        let initial_count = bar.read_with(cx, |bar, _| bar.match_count());
        assert!(
            initial_count >= 2,
            "test precondition: need ≥2 matches across the thread, got {}",
            initial_count,
        );
        assert_eq!(
            bar.read_with(cx, |bar, _| bar.active_match_index()),
            Some(0),
            "first match should be active after the bar populates its match list",
        );

        let query_focus = bar.read_with(cx, |bar, cx| bar.query_editor.focus_handle(cx));
        cx.update(|window, cx| {
            window.focus(&query_focus, cx);
        });
        cx.run_until_parked();
        cx.update(|window, cx| {
            assert!(
                query_focus.contains_focused(window, cx),
                "query editor must be focused before simulating shift-enter",
            );
        });

        cx.simulate_keystrokes("shift-enter");
        cx.run_until_parked();

        let query_text_after = bar.read_with(cx, |bar, cx| bar.query_editor.read(cx).text(cx));
        assert!(
            !query_text_after.contains('\n'),
            "shift-enter must not insert a newline into the query buffer; got {:?}",
            query_text_after,
        );

        let active_after = bar.read_with(cx, |bar, _| bar.active_match_index());
        assert_eq!(
            active_after,
            Some(initial_count - 1),
            "shift-enter should have wrapped active match from 0 to {} (got {:?})",
            initial_count - 1,
            active_after,
        );
    }

    /// `f3`/`shift-f3` are bound in the broad `AcpThread` context (like buffer
    /// search's pane-level `cmd-g`), so they must navigate matches even when
    /// focus is outside the search bar.
    #[gpui::test]
    async fn test_thread_search_navigates_from_outside_search_bar(cx: &mut TestAppContext) {
        init_test(cx);
        cx.update(|cx| {
            search::init(cx);
            let mut default_bindings = settings::KeymapFile::load_asset_allow_partial_failure(
                "keymaps/default-linux.json",
                cx,
            )
            .unwrap();
            for binding in &mut default_bindings {
                binding.set_meta(settings::KeybindSource::Default.meta());
            }
            cx.bind_keys(default_bindings);
        });

        let connection = StubAgentConnection::new();
        connection.set_next_prompt_updates(vec![acp::SessionUpdate::AgentMessageChunk(
            acp::ContentChunk::new(
                "Banana banana banana, multiple banana mentions in this reply.".into(),
            ),
        )]);

        let (conversation_view, cx) =
            setup_conversation_view(StubAgentServer::new(connection.clone()), cx).await;
        add_to_workspace(conversation_view.clone(), cx);

        let thread =
            active_thread(&conversation_view, cx).read_with(cx, |view, _| view.thread.clone());
        thread
            .update(cx, |thread, cx| thread.send_raw("Need banana help", cx))
            .await
            .unwrap();
        cx.run_until_parked();

        let thread_view = active_thread(&conversation_view, cx);
        thread_view.update_in(cx, |view, window, cx| {
            view.toggle_search(&crate::ToggleSearch, window, cx);
        });
        cx.run_until_parked();

        let bar = thread_view
            .read_with(cx, |view, _| view.thread_search_bar.clone())
            .expect("thread_search_bar should be set after toggle_search");
        bar.update_in(cx, |bar, window, cx| {
            bar.query_editor.update(cx, |editor, cx| {
                editor.set_text("banana", window, cx);
            });
            bar.update_matches(window, cx);
        });
        cx.run_until_parked();

        let initial_count = bar.read_with(cx, |bar, _| bar.match_count());
        assert!(
            initial_count >= 2,
            "test precondition: need ≥ 2 matches, got {}",
            initial_count,
        );
        assert_eq!(
            bar.read_with(cx, |bar, _| bar.active_match_index()),
            Some(0)
        );

        // Move focus out of the search bar, back into the thread view itself.
        let thread_focus = thread_view.read_with(cx, |view, cx| view.focus_handle(cx));
        cx.update(|window, cx| window.focus(&thread_focus, cx));
        cx.run_until_parked();
        cx.update(|window, cx| {
            let bar_focused = bar.read_with(cx, |bar, cx| {
                bar.query_editor
                    .focus_handle(cx)
                    .contains_focused(window, cx)
            });
            assert!(!bar_focused, "search bar must not be focused for this test");
        });

        cx.simulate_keystrokes("f3");
        cx.run_until_parked();
        assert_eq!(
            bar.read_with(cx, |bar, _| bar.active_match_index()),
            Some(1),
            "f3 from outside the bar should advance to the next match",
        );

        cx.simulate_keystrokes("shift-f3");
        cx.run_until_parked();
        assert_eq!(
            bar.read_with(cx, |bar, _| bar.active_match_index()),
            Some(0),
            "shift-f3 from outside the bar should return to the previous match",
        );
    }

    #[gpui::test]
    async fn test_message_editing_cancel(cx: &mut TestAppContext) {
        init_test(cx);

        let connection = StubAgentConnection::new();

        connection.set_next_prompt_updates(vec![acp::SessionUpdate::AgentMessageChunk(
            acp::ContentChunk::new("Response".into()),
        )]);

        let (conversation_view, cx) =
            setup_conversation_view(StubAgentServer::new(connection), cx).await;
        add_to_workspace(conversation_view.clone(), cx);

        let message_editor = message_editor(&conversation_view, cx);
        message_editor.update_in(cx, |editor, window, cx| {
            editor.set_text("Original message to edit", window, cx);
        });
        active_thread(&conversation_view, cx)
            .update_in(cx, |view, window, cx| view.send(window, cx));

        cx.run_until_parked();

        let user_message_editor = conversation_view.read_with(cx, |view, cx| {
            assert_eq!(
                view.active_thread()
                    .and_then(|active| active.read(cx).editing_message),
                None
            );

            view.active_thread()
                .map(|active| &active.read(cx).entry_view_state)
                .as_ref()
                .unwrap()
                .read(cx)
                .entry(0)
                .unwrap()
                .message_editor()
                .unwrap()
                .clone()
        });

        // Focus
        cx.focus(&user_message_editor);
        conversation_view.read_with(cx, |view, cx| {
            assert_eq!(
                view.active_thread()
                    .and_then(|active| active.read(cx).editing_message),
                Some(0)
            );
        });

        // Edit
        user_message_editor.update_in(cx, |editor, window, cx| {
            editor.set_text("Edited message content", window, cx);
        });

        // Cancel
        user_message_editor.update_in(cx, |_editor, window, cx| {
            window.dispatch_action(Box::new(editor::actions::Cancel), cx);
        });

        conversation_view.read_with(cx, |view, cx| {
            assert_eq!(
                view.active_thread()
                    .and_then(|active| active.read(cx).editing_message),
                None
            );
        });

        user_message_editor.read_with(cx, |editor, cx| {
            assert_eq!(editor.text(cx), "Original message to edit");
        });
    }

    #[gpui::test]
    async fn test_message_doesnt_send_if_empty(cx: &mut TestAppContext) {
        init_test(cx);

        let connection = StubAgentConnection::new();

        let (conversation_view, cx) =
            setup_conversation_view(StubAgentServer::new(connection), cx).await;
        add_to_workspace(conversation_view.clone(), cx);

        let message_editor = message_editor(&conversation_view, cx);
        message_editor.update_in(cx, |editor, window, cx| {
            editor.set_text("", window, cx);
        });

        let thread = cx.read(|cx| {
            conversation_view
                .read(cx)
                .active_thread()
                .unwrap()
                .read(cx)
                .thread
                .clone()
        });
        let entries_before = cx.read(|cx| thread.read(cx).entries().len());

        active_thread(&conversation_view, cx).update_in(cx, |view, window, cx| {
            view.send(window, cx);
        });
        cx.run_until_parked();

        let entries_after = cx.read(|cx| thread.read(cx).entries().len());
        assert_eq!(
            entries_before, entries_after,
            "No message should be sent when editor is empty"
        );
    }

    #[gpui::test]
    async fn test_message_editing_regenerate(cx: &mut TestAppContext) {
        init_test(cx);

        let connection = StubAgentConnection::new();

        connection.set_next_prompt_updates(vec![acp::SessionUpdate::AgentMessageChunk(
            acp::ContentChunk::new("Response".into()),
        )]);

        let (conversation_view, cx) =
            setup_conversation_view(StubAgentServer::new(connection.clone()), cx).await;
        add_to_workspace(conversation_view.clone(), cx);

        let message_editor = message_editor(&conversation_view, cx);
        message_editor.update_in(cx, |editor, window, cx| {
            editor.set_text("Original message to edit", window, cx);
        });
        active_thread(&conversation_view, cx)
            .update_in(cx, |view, window, cx| view.send(window, cx));

        cx.run_until_parked();

        let user_message_editor = conversation_view.read_with(cx, |view, cx| {
            assert_eq!(
                view.active_thread()
                    .and_then(|active| active.read(cx).editing_message),
                None
            );
            assert_eq!(
                view.active_thread()
                    .unwrap()
                    .read(cx)
                    .thread
                    .read(cx)
                    .entries()
                    .len(),
                2
            );

            view.active_thread()
                .map(|active| &active.read(cx).entry_view_state)
                .as_ref()
                .unwrap()
                .read(cx)
                .entry(0)
                .unwrap()
                .message_editor()
                .unwrap()
                .clone()
        });

        // Focus
        cx.focus(&user_message_editor);

        // Edit
        user_message_editor.update_in(cx, |editor, window, cx| {
            editor.set_text("Edited message content", window, cx);
        });

        // Send
        connection.set_next_prompt_updates(vec![acp::SessionUpdate::AgentMessageChunk(
            acp::ContentChunk::new("New Response".into()),
        )]);

        user_message_editor.update_in(cx, |_editor, window, cx| {
            window.dispatch_action(Box::new(Chat), cx);
        });

        cx.run_until_parked();

        conversation_view.read_with(cx, |view, cx| {
            assert_eq!(
                view.active_thread()
                    .and_then(|active| active.read(cx).editing_message),
                None
            );

            let entries = view
                .active_thread()
                .unwrap()
                .read(cx)
                .thread
                .read(cx)
                .entries();
            assert_eq!(entries.len(), 2);
            assert_eq!(
                entries[0].to_markdown(cx),
                "## User\n\nEdited message content\n\n"
            );
            assert_eq!(
                entries[1].to_markdown(cx),
                "## Assistant\n\nNew Response\n\n"
            );

            let entry_view_state = view
                .active_thread()
                .map(|active| &active.read(cx).entry_view_state)
                .unwrap();
            let new_editor = entry_view_state.read_with(cx, |state, _cx| {
                assert!(!state.entry(1).unwrap().has_content());
                state.entry(0).unwrap().message_editor().unwrap().clone()
            });

            assert_eq!(new_editor.read(cx).text(cx), "Edited message content");
        })
    }

    #[gpui::test]
    async fn test_message_editing_while_generating(cx: &mut TestAppContext) {
        init_test(cx);

        let connection = StubAgentConnection::new();

        let (conversation_view, cx) =
            setup_conversation_view(StubAgentServer::new(connection.clone()), cx).await;
        add_to_workspace(conversation_view.clone(), cx);

        let message_editor = message_editor(&conversation_view, cx);
        message_editor.update_in(cx, |editor, window, cx| {
            editor.set_text("Original message to edit", window, cx);
        });
        active_thread(&conversation_view, cx)
            .update_in(cx, |view, window, cx| view.send(window, cx));

        cx.run_until_parked();

        let (user_message_editor, session_id) = conversation_view.read_with(cx, |view, cx| {
            let thread = view.active_thread().unwrap().read(cx).thread.read(cx);
            assert_eq!(thread.entries().len(), 1);

            let editor = view
                .active_thread()
                .map(|active| &active.read(cx).entry_view_state)
                .as_ref()
                .unwrap()
                .read(cx)
                .entry(0)
                .unwrap()
                .message_editor()
                .unwrap()
                .clone();

            (editor, thread.session_id().clone())
        });

        // Focus
        cx.focus(&user_message_editor);

        conversation_view.read_with(cx, |view, cx| {
            assert_eq!(
                view.active_thread()
                    .and_then(|active| active.read(cx).editing_message),
                Some(0)
            );
        });

        // Edit
        user_message_editor.update_in(cx, |editor, window, cx| {
            editor.set_text("Edited message content", window, cx);
        });

        conversation_view.read_with(cx, |view, cx| {
            assert_eq!(
                view.active_thread()
                    .and_then(|active| active.read(cx).editing_message),
                Some(0)
            );
        });

        // Finish streaming response
        cx.update(|_, cx| {
            connection.send_update(
                session_id.clone(),
                acp::SessionUpdate::AgentMessageChunk(acp::ContentChunk::new("Response".into())),
                cx,
            );
            connection.end_turn(session_id, acp::StopReason::EndTurn);
        });

        conversation_view.read_with(cx, |view, cx| {
            assert_eq!(
                view.active_thread()
                    .and_then(|active| active.read(cx).editing_message),
                Some(0)
            );
        });

        cx.run_until_parked();

        // Should still be editing
        cx.update(|window, cx| {
            assert!(user_message_editor.focus_handle(cx).is_focused(window));
            assert_eq!(
                conversation_view
                    .read(cx)
                    .active_thread()
                    .and_then(|active| active.read(cx).editing_message),
                Some(0)
            );
            assert_eq!(
                user_message_editor.read(cx).text(cx),
                "Edited message content"
            );
        });
    }

    #[gpui::test]
    async fn test_stale_stop_does_not_disable_follow_tail_during_regenerate(
        cx: &mut TestAppContext,
    ) {
        init_test(cx);

        let connection = StubAgentConnection::new();

        let (conversation_view, cx) =
            setup_conversation_view(StubAgentServer::new(connection.clone()), cx).await;
        add_to_workspace(conversation_view.clone(), cx);

        let message_editor = message_editor(&conversation_view, cx);
        message_editor.update_in(cx, |editor, window, cx| {
            editor.set_text("Original message to edit", window, cx);
        });
        active_thread(&conversation_view, cx)
            .update_in(cx, |view, window, cx| view.send(window, cx));

        cx.run_until_parked();

        let user_message_editor = conversation_view.read_with(cx, |view, cx| {
            view.active_thread()
                .map(|active| &active.read(cx).entry_view_state)
                .as_ref()
                .unwrap()
                .read(cx)
                .entry(0)
                .unwrap()
                .message_editor()
                .unwrap()
                .clone()
        });

        cx.focus(&user_message_editor);
        user_message_editor.update_in(cx, |editor, window, cx| {
            editor.set_text("Edited message content", window, cx);
        });

        user_message_editor.update_in(cx, |_editor, window, cx| {
            window.dispatch_action(Box::new(Chat), cx);
        });

        cx.run_until_parked();

        conversation_view.read_with(cx, |view, cx| {
            let active = view.active_thread().unwrap();
            let active = active.read(cx);

            assert_eq!(active.thread.read(cx).status(), ThreadStatus::Generating);
            assert!(
                active.list_state.is_following_tail(),
                "stale stop events from the cancelled turn must not disable follow-tail for the new turn"
            );
        });
    }

    struct GeneratingThreadSetup {
        conversation_view: Entity<ConversationView>,
        thread: Entity<AcpThread>,
        message_editor: Entity<MessageEditor>,
    }

    async fn setup_generating_thread(
        cx: &mut TestAppContext,
    ) -> (GeneratingThreadSetup, &mut VisualTestContext) {
        let connection = StubAgentConnection::new();

        let (conversation_view, cx) =
            setup_conversation_view(StubAgentServer::new(connection.clone()), cx).await;
        add_to_workspace(conversation_view.clone(), cx);

        let message_editor = message_editor(&conversation_view, cx);
        message_editor.update_in(cx, |editor, window, cx| {
            editor.set_text("Hello", window, cx);
        });
        active_thread(&conversation_view, cx)
            .update_in(cx, |view, window, cx| view.send(window, cx));

        let (thread, session_id) = conversation_view.read_with(cx, |view, cx| {
            let thread = view
                .active_thread()
                .as_ref()
                .unwrap()
                .read(cx)
                .thread
                .clone();
            (thread.clone(), thread.read(cx).session_id().clone())
        });

        cx.run_until_parked();

        cx.update(|_, cx| {
            connection.send_update(
                session_id.clone(),
                acp::SessionUpdate::AgentMessageChunk(acp::ContentChunk::new(
                    "Response chunk".into(),
                )),
                cx,
            );
        });

        cx.run_until_parked();

        thread.read_with(cx, |thread, _cx| {
            assert_eq!(thread.status(), ThreadStatus::Generating);
        });

        (
            GeneratingThreadSetup {
                conversation_view,
                thread,
                message_editor,
            },
            cx,
        )
    }

    #[gpui::test]
    async fn test_escape_cancels_generation_from_conversation_focus(cx: &mut TestAppContext) {
        init_test(cx);

        let (setup, cx) = setup_generating_thread(cx).await;

        let focus_handle = setup
            .conversation_view
            .read_with(cx, |view, cx| view.focus_handle(cx));
        cx.update(|window, cx| {
            window.focus(&focus_handle, cx);
        });

        setup.conversation_view.update_in(cx, |_, window, cx| {
            window.dispatch_action(menu::Cancel.boxed_clone(), cx);
        });

        cx.run_until_parked();

        setup.thread.read_with(cx, |thread, _cx| {
            assert_eq!(thread.status(), ThreadStatus::Idle);
        });
    }

    #[gpui::test]
    async fn test_escape_cancels_generation_from_editor_focus(cx: &mut TestAppContext) {
        init_test(cx);

        let (setup, cx) = setup_generating_thread(cx).await;

        let editor_focus_handle = setup
            .message_editor
            .read_with(cx, |editor, cx| editor.focus_handle(cx));
        cx.update(|window, cx| {
            window.focus(&editor_focus_handle, cx);
        });

        setup.message_editor.update_in(cx, |_, window, cx| {
            window.dispatch_action(editor::actions::Cancel.boxed_clone(), cx);
        });

        cx.run_until_parked();

        setup.thread.read_with(cx, |thread, _cx| {
            assert_eq!(thread.status(), ThreadStatus::Idle);
        });
    }

    #[gpui::test]
    async fn test_escape_when_idle_is_noop(cx: &mut TestAppContext) {
        init_test(cx);

        let (conversation_view, cx) =
            setup_conversation_view(StubAgentServer::new(StubAgentConnection::new()), cx).await;
        add_to_workspace(conversation_view.clone(), cx);

        let thread = conversation_view.read_with(cx, |view, cx| {
            view.active_thread().unwrap().read(cx).thread.clone()
        });

        thread.read_with(cx, |thread, _cx| {
            assert_eq!(thread.status(), ThreadStatus::Idle);
        });

        let focus_handle = conversation_view.read_with(cx, |view, _cx| view.focus_handle.clone());
        cx.update(|window, cx| {
            window.focus(&focus_handle, cx);
        });

        conversation_view.update_in(cx, |_, window, cx| {
            window.dispatch_action(menu::Cancel.boxed_clone(), cx);
        });

        cx.run_until_parked();

        thread.read_with(cx, |thread, _cx| {
            assert_eq!(thread.status(), ThreadStatus::Idle);
        });
    }

    #[gpui::test]
    async fn test_interrupt(cx: &mut TestAppContext) {
        init_test(cx);

        let connection = StubAgentConnection::new();

        let (conversation_view, cx) =
            setup_conversation_view(StubAgentServer::new(connection.clone()), cx).await;
        add_to_workspace(conversation_view.clone(), cx);

        let message_editor = message_editor(&conversation_view, cx);
        message_editor.update_in(cx, |editor, window, cx| {
            editor.set_text("Message 1", window, cx);
        });
        active_thread(&conversation_view, cx)
            .update_in(cx, |view, window, cx| view.send(window, cx));

        let (thread, session_id) = conversation_view.read_with(cx, |view, cx| {
            let thread = view.active_thread().unwrap().read(cx).thread.clone();

            (thread.clone(), thread.read(cx).session_id().clone())
        });

        cx.run_until_parked();

        cx.update(|_, cx| {
            connection.send_update(
                session_id.clone(),
                acp::SessionUpdate::AgentMessageChunk(acp::ContentChunk::new(
                    "Message 1 resp".into(),
                )),
                cx,
            );
        });

        cx.run_until_parked();

        thread.read_with(cx, |thread, cx| {
            assert_eq!(
                thread.to_markdown(cx),
                indoc::indoc! {"
                        ## User

                        Message 1

                        ## Assistant

                        Message 1 resp

                    "}
            )
        });

        message_editor.update_in(cx, |editor, window, cx| {
            editor.set_text("Message 2", window, cx);
        });
        active_thread(&conversation_view, cx)
            .update_in(cx, |view, window, cx| view.interrupt_and_send(window, cx));

        cx.update(|_, cx| {
            // Simulate a response sent after beginning to cancel
            connection.send_update(
                session_id.clone(),
                acp::SessionUpdate::AgentMessageChunk(acp::ContentChunk::new("onse".into())),
                cx,
            );
        });

        cx.run_until_parked();

        // Last Message 1 response should appear before Message 2
        thread.read_with(cx, |thread, cx| {
            assert_eq!(
                thread.to_markdown(cx),
                indoc::indoc! {"
                        ## User

                        Message 1

                        ## Assistant

                        Message 1 response

                        ## User

                        Message 2

                    "}
            )
        });

        cx.update(|_, cx| {
            connection.send_update(
                session_id.clone(),
                acp::SessionUpdate::AgentMessageChunk(acp::ContentChunk::new(
                    "Message 2 response".into(),
                )),
                cx,
            );
            connection.end_turn(session_id.clone(), acp::StopReason::EndTurn);
        });

        cx.run_until_parked();

        thread.read_with(cx, |thread, cx| {
            assert_eq!(
                thread.to_markdown(cx),
                indoc::indoc! {"
                        ## User

                        Message 1

                        ## Assistant

                        Message 1 response

                        ## User

                        Message 2

                        ## Assistant

                        Message 2 response

                    "}
            )
        });
    }

    #[gpui::test]
    async fn test_message_editing_insert_selections(cx: &mut TestAppContext) {
        init_test(cx);

        let connection = StubAgentConnection::new();
        connection.set_next_prompt_updates(vec![acp::SessionUpdate::AgentMessageChunk(
            acp::ContentChunk::new("Response".into()),
        )]);

        let (conversation_view, cx) =
            setup_conversation_view(StubAgentServer::new(connection), cx).await;
        add_to_workspace(conversation_view.clone(), cx);

        let message_editor = message_editor(&conversation_view, cx);
        message_editor.update_in(cx, |editor, window, cx| {
            editor.set_text("Original message to edit", window, cx)
        });
        active_thread(&conversation_view, cx)
            .update_in(cx, |view, window, cx| view.send(window, cx));
        cx.run_until_parked();

        let user_message_editor = conversation_view.read_with(cx, |conversation_view, cx| {
            conversation_view
                .active_thread()
                .map(|active| &active.read(cx).entry_view_state)
                .as_ref()
                .unwrap()
                .read(cx)
                .entry(0)
                .expect("Should have at least one entry")
                .message_editor()
                .expect("Should have message editor")
                .clone()
        });

        cx.focus(&user_message_editor);
        conversation_view.read_with(cx, |view, cx| {
            assert_eq!(
                view.active_thread()
                    .and_then(|active| active.read(cx).editing_message),
                Some(0)
            );
        });

        // Ensure to edit the focused message before proceeding otherwise, since
        // its content is not different from what was sent, focus will be lost.
        user_message_editor.update_in(cx, |editor, window, cx| {
            editor.set_text("Original message to edit with ", window, cx)
        });

        // Create a simple buffer with some text so we can create a selection
        // that will then be added to the message being edited.
        let (workspace, project) = conversation_view.read_with(cx, |conversation_view, _cx| {
            (
                conversation_view.workspace.clone(),
                conversation_view.project.clone(),
            )
        });
        let buffer = project.update(cx, |project, cx| {
            project.create_local_buffer("let a = 10 + 10;", None, false, cx)
        });

        workspace
            .update_in(cx, |workspace, window, cx| {
                let editor = cx.new(|cx| {
                    let mut editor =
                        Editor::for_buffer(buffer.clone(), Some(project.clone()), window, cx);

                    editor.change_selections(Default::default(), window, cx, |selections| {
                        selections.select_ranges([MultiBufferOffset(8)..MultiBufferOffset(15)]);
                    });

                    editor
                });
                workspace.add_item_to_active_pane(Box::new(editor), None, false, window, cx);
            })
            .unwrap();

        conversation_view.update_in(cx, |view, window, cx| {
            assert_eq!(
                view.active_thread()
                    .and_then(|active| active.read(cx).editing_message),
                Some(0)
            );
            let workspace = workspace.upgrade().unwrap();
            let selection = workspace
                .update(cx, |workspace, cx| {
                    AgentContextSource::from_active(workspace, cx)?
                        .read_selection(workspace, false, cx)
                })
                .unwrap();
            view.insert_selection(selection, window, cx);
        });

        user_message_editor.read_with(cx, |editor, cx| {
            let text = editor.editor().read(cx).text(cx);
            let expected_text = String::from("Original message to edit with selection ");

            assert_eq!(text, expected_text);
        });
    }

    #[gpui::test]
    async fn test_insert_selections(cx: &mut TestAppContext) {
        init_test(cx);

        let connection = StubAgentConnection::new();
        connection.set_next_prompt_updates(vec![acp::SessionUpdate::AgentMessageChunk(
            acp::ContentChunk::new("Response".into()),
        )]);

        let (conversation_view, cx) =
            setup_conversation_view(StubAgentServer::new(connection), cx).await;
        add_to_workspace(conversation_view.clone(), cx);

        let message_editor = message_editor(&conversation_view, cx);
        message_editor.update_in(cx, |editor, window, cx| {
            editor.set_text("Can you review this snippet ", window, cx)
        });

        // Create a simple buffer with some text so we can create a selection
        // that will then be added to the message being edited.
        let (workspace, project) = conversation_view.read_with(cx, |conversation_view, _cx| {
            (
                conversation_view.workspace.clone(),
                conversation_view.project.clone(),
            )
        });
        let buffer = project.update(cx, |project, cx| {
            project.create_local_buffer("let a = 10 + 10;", None, false, cx)
        });

        workspace
            .update_in(cx, |workspace, window, cx| {
                let editor = cx.new(|cx| {
                    let mut editor =
                        Editor::for_buffer(buffer.clone(), Some(project.clone()), window, cx);

                    editor.change_selections(Default::default(), window, cx, |selections| {
                        selections.select_ranges([MultiBufferOffset(8)..MultiBufferOffset(15)]);
                    });

                    editor
                });
                workspace.add_item_to_active_pane(Box::new(editor), None, false, window, cx);
            })
            .unwrap();

        conversation_view.update_in(cx, |view, window, cx| {
            assert_eq!(
                view.active_thread()
                    .and_then(|active| active.read(cx).editing_message),
                None
            );
            let workspace = view.workspace.upgrade().unwrap();
            let selection = workspace
                .update(cx, |workspace, cx| {
                    AgentContextSource::from_active(workspace, cx)?
                        .read_selection(workspace, false, cx)
                })
                .unwrap();
            view.insert_selection(selection, window, cx);
        });

        message_editor.read_with(cx, |editor, cx| {
            let text = editor.text(cx);
            let expected_txt = String::from("Can you review this snippet selection ");

            assert_eq!(text, expected_txt);
        })
    }

    #[gpui::test]
    async fn test_tool_permission_buttons_terminal_with_pattern(cx: &mut TestAppContext) {
        init_test(cx);

        let tool_call_id = acp::ToolCallId::new("terminal-1");
        let tool_call = acp::ToolCall::new(tool_call_id.clone(), "Run `cargo build --release`")
            .kind(acp::ToolKind::Edit);

        let permission_options = ToolPermissionContext::new(
            TerminalTool::NAME,
            vec!["cargo build --release".to_string()],
        )
        .build_permission_options();

        let connection =
            StubAgentConnection::new().with_permission_requests(HashMap::from_iter([(
                tool_call_id.clone(),
                permission_options,
            )]));

        connection.set_next_prompt_updates(vec![acp::SessionUpdate::ToolCall(tool_call)]);

        let (conversation_view, cx) =
            setup_conversation_view(StubAgentServer::new(connection), cx).await;

        // Disable notifications to avoid popup windows
        cx.update(|_window, cx| {
            AgentSettings::override_global(
                AgentSettings {
                    notify_when_agent_waiting: NotifyWhenAgentWaiting::Never,
                    ..AgentSettings::get_global(cx).clone()
                },
                cx,
            );
        });

        let message_editor = message_editor(&conversation_view, cx);
        message_editor.update_in(cx, |editor, window, cx| {
            editor.set_text("Run cargo build", window, cx);
        });

        active_thread(&conversation_view, cx)
            .update_in(cx, |view, window, cx| view.send(window, cx));

        cx.run_until_parked();

        // Verify the tool call is in WaitingForConfirmation state with the expected options
        conversation_view.read_with(cx, |conversation_view, cx| {
            let thread = conversation_view
                .active_thread()
                .expect("Thread should exist")
                .read(cx)
                .thread
                .clone();
            let thread = thread.read(cx);

            let tool_call = thread.entries().iter().find_map(|entry| {
                if let acp_thread::AgentThreadEntry::ToolCall(call) = entry {
                    Some(call)
                } else {
                    None
                }
            });

            assert!(tool_call.is_some(), "Expected a tool call entry");
            let tool_call = tool_call.unwrap();

            // Verify it's waiting for confirmation
            assert!(
                matches!(
                    tool_call.status,
                    acp_thread::ToolCallStatus::WaitingForConfirmation { .. }
                ),
                "Expected WaitingForConfirmation status, got {:?}",
                tool_call.status
            );

            // Verify the options count (granularity options only, no separate Deny option)
            if let acp_thread::ToolCallStatus::WaitingForConfirmation { options, .. } =
                &tool_call.status
            {
                let PermissionOptions::Dropdown(choices) = options else {
                    panic!("Expected dropdown permission options");
                };

                assert_eq!(
                    choices.len(),
                    3,
                    "Expected 3 permission options (granularity only)"
                );

                // Verify specific button labels (now using neutral names)
                let labels: Vec<&str> = choices
                    .iter()
                    .map(|choice| choice.allow.name.as_ref())
                    .collect();
                assert!(
                    labels.contains(&"Always for terminal"),
                    "Missing 'Always for terminal' option"
                );
                assert!(
                    labels.contains(&"Always for `cargo build` commands"),
                    "Missing pattern option"
                );
                assert!(
                    labels.contains(&"Only this time"),
                    "Missing 'Only this time' option"
                );
            }
        });
    }

    #[gpui::test]
    async fn test_tool_permission_buttons_edit_file_with_path_pattern(cx: &mut TestAppContext) {
        init_test(cx);

        let tool_call_id = acp::ToolCallId::new("edit-file-1");
        let tool_call = acp::ToolCall::new(tool_call_id.clone(), "Edit `src/main.rs`")
            .kind(acp::ToolKind::Edit);

        let permission_options =
            ToolPermissionContext::new(EditFileTool::NAME, vec!["src/main.rs".to_string()])
                .build_permission_options();

        let connection =
            StubAgentConnection::new().with_permission_requests(HashMap::from_iter([(
                tool_call_id.clone(),
                permission_options,
            )]));

        connection.set_next_prompt_updates(vec![acp::SessionUpdate::ToolCall(tool_call)]);

        let (conversation_view, cx) =
            setup_conversation_view(StubAgentServer::new(connection), cx).await;

        // Disable notifications
        cx.update(|_window, cx| {
            AgentSettings::override_global(
                AgentSettings {
                    notify_when_agent_waiting: NotifyWhenAgentWaiting::Never,
                    ..AgentSettings::get_global(cx).clone()
                },
                cx,
            );
        });

        let message_editor = message_editor(&conversation_view, cx);
        message_editor.update_in(cx, |editor, window, cx| {
            editor.set_text("Edit the main file", window, cx);
        });

        active_thread(&conversation_view, cx)
            .update_in(cx, |view, window, cx| view.send(window, cx));

        cx.run_until_parked();

        // Verify the options
        conversation_view.read_with(cx, |conversation_view, cx| {
            let thread = conversation_view
                .active_thread()
                .expect("Thread should exist")
                .read(cx)
                .thread
                .clone();
            let thread = thread.read(cx);

            let tool_call = thread.entries().iter().find_map(|entry| {
                if let acp_thread::AgentThreadEntry::ToolCall(call) = entry {
                    Some(call)
                } else {
                    None
                }
            });

            assert!(tool_call.is_some(), "Expected a tool call entry");
            let tool_call = tool_call.unwrap();

            if let acp_thread::ToolCallStatus::WaitingForConfirmation { options, .. } =
                &tool_call.status
            {
                let PermissionOptions::Dropdown(choices) = options else {
                    panic!("Expected dropdown permission options");
                };

                let labels: Vec<&str> = choices
                    .iter()
                    .map(|choice| choice.allow.name.as_ref())
                    .collect();
                assert!(
                    labels.contains(&"Always for edit file"),
                    "Missing 'Always for edit file' option"
                );
                assert!(
                    labels.contains(&"Always for `src/`"),
                    "Missing path pattern option"
                );
            } else {
                panic!("Expected WaitingForConfirmation status");
            }
        });
    }

    #[gpui::test]
    async fn test_tool_permission_buttons_fetch_with_domain_pattern(cx: &mut TestAppContext) {
        init_test(cx);

        let tool_call_id = acp::ToolCallId::new("fetch-1");
        let tool_call = acp::ToolCall::new(tool_call_id.clone(), "Fetch `https://docs.rs/gpui`")
            .kind(acp::ToolKind::Fetch);

        let permission_options =
            ToolPermissionContext::new(FetchTool::NAME, vec!["https://docs.rs/gpui".to_string()])
                .build_permission_options();

        let connection =
            StubAgentConnection::new().with_permission_requests(HashMap::from_iter([(
                tool_call_id.clone(),
                permission_options,
            )]));

        connection.set_next_prompt_updates(vec![acp::SessionUpdate::ToolCall(tool_call)]);

        let (conversation_view, cx) =
            setup_conversation_view(StubAgentServer::new(connection), cx).await;

        // Disable notifications
        cx.update(|_window, cx| {
            AgentSettings::override_global(
                AgentSettings {
                    notify_when_agent_waiting: NotifyWhenAgentWaiting::Never,
                    ..AgentSettings::get_global(cx).clone()
                },
                cx,
            );
        });

        let message_editor = message_editor(&conversation_view, cx);
        message_editor.update_in(cx, |editor, window, cx| {
            editor.set_text("Fetch the docs", window, cx);
        });

        active_thread(&conversation_view, cx)
            .update_in(cx, |view, window, cx| view.send(window, cx));

        cx.run_until_parked();

        // Verify the options
        conversation_view.read_with(cx, |conversation_view, cx| {
            let thread = conversation_view
                .active_thread()
                .expect("Thread should exist")
                .read(cx)
                .thread
                .clone();
            let thread = thread.read(cx);

            let tool_call = thread.entries().iter().find_map(|entry| {
                if let acp_thread::AgentThreadEntry::ToolCall(call) = entry {
                    Some(call)
                } else {
                    None
                }
            });

            assert!(tool_call.is_some(), "Expected a tool call entry");
            let tool_call = tool_call.unwrap();

            if let acp_thread::ToolCallStatus::WaitingForConfirmation { options, .. } =
                &tool_call.status
            {
                let PermissionOptions::Dropdown(choices) = options else {
                    panic!("Expected dropdown permission options");
                };

                let labels: Vec<&str> = choices
                    .iter()
                    .map(|choice| choice.allow.name.as_ref())
                    .collect();
                assert!(
                    labels.contains(&"Always for fetch"),
                    "Missing 'Always for fetch' option"
                );
                assert!(
                    labels.contains(&"Always for `docs.rs`"),
                    "Missing domain pattern option"
                );
            } else {
                panic!("Expected WaitingForConfirmation status");
            }
        });
    }

    #[gpui::test]
    async fn test_tool_permission_buttons_without_pattern(cx: &mut TestAppContext) {
        init_test(cx);

        let tool_call_id = acp::ToolCallId::new("terminal-no-pattern-1");
        let tool_call = acp::ToolCall::new(tool_call_id.clone(), "Run `./deploy.sh --production`")
            .kind(acp::ToolKind::Edit);

        // No pattern button since ./deploy.sh doesn't match the alphanumeric pattern
        let permission_options = ToolPermissionContext::new(
            TerminalTool::NAME,
            vec!["./deploy.sh --production".to_string()],
        )
        .build_permission_options();

        let connection =
            StubAgentConnection::new().with_permission_requests(HashMap::from_iter([(
                tool_call_id.clone(),
                permission_options,
            )]));

        connection.set_next_prompt_updates(vec![acp::SessionUpdate::ToolCall(tool_call)]);

        let (conversation_view, cx) =
            setup_conversation_view(StubAgentServer::new(connection), cx).await;

        // Disable notifications
        cx.update(|_window, cx| {
            AgentSettings::override_global(
                AgentSettings {
                    notify_when_agent_waiting: NotifyWhenAgentWaiting::Never,
                    ..AgentSettings::get_global(cx).clone()
                },
                cx,
            );
        });

        let message_editor = message_editor(&conversation_view, cx);
        message_editor.update_in(cx, |editor, window, cx| {
            editor.set_text("Run the deploy script", window, cx);
        });

        active_thread(&conversation_view, cx)
            .update_in(cx, |view, window, cx| view.send(window, cx));

        cx.run_until_parked();

        // Verify only 2 options (no pattern button when command doesn't match pattern)
        conversation_view.read_with(cx, |conversation_view, cx| {
            let thread = conversation_view
                .active_thread()
                .expect("Thread should exist")
                .read(cx)
                .thread
                .clone();
            let thread = thread.read(cx);

            let tool_call = thread.entries().iter().find_map(|entry| {
                if let acp_thread::AgentThreadEntry::ToolCall(call) = entry {
                    Some(call)
                } else {
                    None
                }
            });

            assert!(tool_call.is_some(), "Expected a tool call entry");
            let tool_call = tool_call.unwrap();

            if let acp_thread::ToolCallStatus::WaitingForConfirmation { options, .. } =
                &tool_call.status
            {
                let PermissionOptions::Dropdown(choices) = options else {
                    panic!("Expected dropdown permission options");
                };

                assert_eq!(
                    choices.len(),
                    2,
                    "Expected 2 permission options (no pattern option)"
                );

                let labels: Vec<&str> = choices
                    .iter()
                    .map(|choice| choice.allow.name.as_ref())
                    .collect();
                assert!(
                    labels.contains(&"Always for terminal"),
                    "Missing 'Always for terminal' option"
                );
                assert!(
                    labels.contains(&"Only this time"),
                    "Missing 'Only this time' option"
                );
                // Should NOT contain a pattern option
                assert!(
                    !labels.iter().any(|l| l.contains("commands")),
                    "Should not have pattern option"
                );
            } else {
                panic!("Expected WaitingForConfirmation status");
            }
        });
    }

    #[gpui::test]
    async fn test_authorize_tool_call_action_triggers_authorization(cx: &mut TestAppContext) {
        init_test(cx);

        let tool_call_id = acp::ToolCallId::new("action-test-1");
        let tool_call =
            acp::ToolCall::new(tool_call_id.clone(), "Run `cargo test`").kind(acp::ToolKind::Edit);

        let permission_options =
            ToolPermissionContext::new(TerminalTool::NAME, vec!["cargo test".to_string()])
                .build_permission_options();

        let connection =
            StubAgentConnection::new().with_permission_requests(HashMap::from_iter([(
                tool_call_id.clone(),
                permission_options,
            )]));

        connection.set_next_prompt_updates(vec![acp::SessionUpdate::ToolCall(tool_call)]);

        let (conversation_view, cx) =
            setup_conversation_view(StubAgentServer::new(connection), cx).await;
        add_to_workspace(conversation_view.clone(), cx);

        cx.update(|_window, cx| {
            AgentSettings::override_global(
                AgentSettings {
                    notify_when_agent_waiting: NotifyWhenAgentWaiting::Never,
                    ..AgentSettings::get_global(cx).clone()
                },
                cx,
            );
        });

        let message_editor = message_editor(&conversation_view, cx);
        message_editor.update_in(cx, |editor, window, cx| {
            editor.set_text("Run tests", window, cx);
        });

        active_thread(&conversation_view, cx)
            .update_in(cx, |view, window, cx| view.send(window, cx));

        cx.run_until_parked();

        // Verify tool call is waiting for confirmation
        conversation_view.read_with(cx, |conversation_view, cx| {
            let tool_call = conversation_view.pending_tool_call(cx);
            assert!(
                tool_call.is_some(),
                "Expected a tool call waiting for confirmation"
            );
        });

        // Dispatch the AuthorizeToolCall action (simulating dropdown menu selection)
        conversation_view.update_in(cx, |_, window, cx| {
            window.dispatch_action(
                crate::AuthorizeToolCall {
                    tool_call_id: "action-test-1".to_string(),
                    option_id: "allow".to_string(),
                    option_kind: "AllowOnce".to_string(),
                }
                .boxed_clone(),
                cx,
            );
        });

        cx.run_until_parked();

        // Verify tool call is no longer waiting for confirmation (was authorized)
        conversation_view.read_with(cx, |conversation_view, cx| {
            let tool_call = conversation_view.pending_tool_call(cx);
            assert!(
                tool_call.is_none(),
                "Tool call should no longer be waiting for confirmation after AuthorizeToolCall action"
            );
        });
    }

    #[gpui::test]
    async fn test_authorize_tool_call_action_with_pattern_option(cx: &mut TestAppContext) {
        init_test(cx);

        let tool_call_id = acp::ToolCallId::new("pattern-action-test-1");
        let tool_call =
            acp::ToolCall::new(tool_call_id.clone(), "Run `npm install`").kind(acp::ToolKind::Edit);

        let permission_options =
            ToolPermissionContext::new(TerminalTool::NAME, vec!["npm install".to_string()])
                .build_permission_options();

        let connection =
            StubAgentConnection::new().with_permission_requests(HashMap::from_iter([(
                tool_call_id.clone(),
                permission_options.clone(),
            )]));

        connection.set_next_prompt_updates(vec![acp::SessionUpdate::ToolCall(tool_call)]);

        let (conversation_view, cx) =
            setup_conversation_view(StubAgentServer::new(connection), cx).await;
        add_to_workspace(conversation_view.clone(), cx);

        cx.update(|_window, cx| {
            AgentSettings::override_global(
                AgentSettings {
                    notify_when_agent_waiting: NotifyWhenAgentWaiting::Never,
                    ..AgentSettings::get_global(cx).clone()
                },
                cx,
            );
        });

        let message_editor = message_editor(&conversation_view, cx);
        message_editor.update_in(cx, |editor, window, cx| {
            editor.set_text("Install dependencies", window, cx);
        });

        active_thread(&conversation_view, cx)
            .update_in(cx, |view, window, cx| view.send(window, cx));

        cx.run_until_parked();

        // Find the pattern option ID (the choice with non-empty sub_patterns)
        let pattern_option = match &permission_options {
            PermissionOptions::Dropdown(choices) => choices
                .iter()
                .find(|choice| !choice.sub_patterns.is_empty())
                .map(|choice| &choice.allow)
                .expect("Should have a pattern option for npm command"),
            _ => panic!("Expected dropdown permission options"),
        };

        // Dispatch action with the pattern option (simulating "Always allow `npm` commands")
        conversation_view.update_in(cx, |_, window, cx| {
            window.dispatch_action(
                crate::AuthorizeToolCall {
                    tool_call_id: "pattern-action-test-1".to_string(),
                    option_id: pattern_option.option_id.0.to_string(),
                    option_kind: "AllowAlways".to_string(),
                }
                .boxed_clone(),
                cx,
            );
        });

        cx.run_until_parked();

        // Verify tool call was authorized
        conversation_view.read_with(cx, |conversation_view, cx| {
            let tool_call = conversation_view.pending_tool_call(cx);
            assert!(
                tool_call.is_none(),
                "Tool call should be authorized after selecting pattern option"
            );
        });
    }

    #[gpui::test]
    async fn test_granularity_selection_updates_state(cx: &mut TestAppContext) {
        init_test(cx);

        let tool_call_id = acp::ToolCallId::new("granularity-test-1");
        let tool_call =
            acp::ToolCall::new(tool_call_id.clone(), "Run `cargo build`").kind(acp::ToolKind::Edit);

        let permission_options =
            ToolPermissionContext::new(TerminalTool::NAME, vec!["cargo build".to_string()])
                .build_permission_options();

        let connection =
            StubAgentConnection::new().with_permission_requests(HashMap::from_iter([(
                tool_call_id.clone(),
                permission_options.clone(),
            )]));

        connection.set_next_prompt_updates(vec![acp::SessionUpdate::ToolCall(tool_call)]);

        let (thread_view, cx) = setup_conversation_view(StubAgentServer::new(connection), cx).await;
        add_to_workspace(thread_view.clone(), cx);

        cx.update(|_window, cx| {
            AgentSettings::override_global(
                AgentSettings {
                    notify_when_agent_waiting: NotifyWhenAgentWaiting::Never,
                    ..AgentSettings::get_global(cx).clone()
                },
                cx,
            );
        });

        let message_editor = message_editor(&thread_view, cx);
        message_editor.update_in(cx, |editor, window, cx| {
            editor.set_text("Build the project", window, cx);
        });

        active_thread(&thread_view, cx).update_in(cx, |view, window, cx| view.send(window, cx));

        cx.run_until_parked();

        // Verify default granularity is the last option (index 2 = "Only this time")
        thread_view.read_with(cx, |thread_view, cx| {
            let state = thread_view.active_thread().unwrap();
            let selected = state.read(cx).permission_selections.get(&tool_call_id);
            assert!(
                selected.is_none(),
                "Should have no selection initially (defaults to last)"
            );
        });

        // Select the first option (index 0 = "Always for terminal")
        thread_view.update_in(cx, |_, window, cx| {
            window.dispatch_action(
                crate::SelectPermissionGranularity {
                    tool_call_id: "granularity-test-1".to_string(),
                    index: 0,
                }
                .boxed_clone(),
                cx,
            );
        });

        cx.run_until_parked();

        // Verify the selection was updated
        thread_view.read_with(cx, |thread_view, cx| {
            let state = thread_view.active_thread().unwrap();
            let selected = state.read(cx).permission_selections.get(&tool_call_id);
            assert_eq!(
                selected.and_then(|s| s.choice_index()),
                Some(0),
                "Should have selected index 0"
            );
        });
    }

    #[gpui::test]
    async fn test_allow_button_uses_selected_granularity(cx: &mut TestAppContext) {
        init_test(cx);

        let tool_call_id = acp::ToolCallId::new("allow-granularity-test-1");
        let tool_call =
            acp::ToolCall::new(tool_call_id.clone(), "Run `npm install`").kind(acp::ToolKind::Edit);

        let permission_options =
            ToolPermissionContext::new(TerminalTool::NAME, vec!["npm install".to_string()])
                .build_permission_options();

        // Verify we have the expected options
        let PermissionOptions::Dropdown(choices) = &permission_options else {
            panic!("Expected dropdown permission options");
        };

        assert_eq!(choices.len(), 3);
        assert!(
            choices[0]
                .allow
                .option_id
                .0
                .contains("always_allow:terminal")
        );
        assert!(
            choices[1]
                .allow
                .option_id
                .0
                .contains("always_allow:terminal")
        );
        assert!(!choices[1].sub_patterns.is_empty());
        assert_eq!(choices[2].allow.option_id.0.as_ref(), "allow");

        let connection =
            StubAgentConnection::new().with_permission_requests(HashMap::from_iter([(
                tool_call_id.clone(),
                permission_options.clone(),
            )]));

        connection.set_next_prompt_updates(vec![acp::SessionUpdate::ToolCall(tool_call)]);

        let (thread_view, cx) = setup_conversation_view(StubAgentServer::new(connection), cx).await;
        add_to_workspace(thread_view.clone(), cx);

        cx.update(|_window, cx| {
            AgentSettings::override_global(
                AgentSettings {
                    notify_when_agent_waiting: NotifyWhenAgentWaiting::Never,
                    ..AgentSettings::get_global(cx).clone()
                },
                cx,
            );
        });

        let message_editor = message_editor(&thread_view, cx);
        message_editor.update_in(cx, |editor, window, cx| {
            editor.set_text("Install dependencies", window, cx);
        });

        active_thread(&thread_view, cx).update_in(cx, |view, window, cx| view.send(window, cx));

        cx.run_until_parked();

        // Select the pattern option (index 1 = "Always for `npm` commands")
        thread_view.update_in(cx, |_, window, cx| {
            window.dispatch_action(
                crate::SelectPermissionGranularity {
                    tool_call_id: "allow-granularity-test-1".to_string(),
                    index: 1,
                }
                .boxed_clone(),
                cx,
            );
        });

        cx.run_until_parked();

        // Simulate clicking the Allow button by dispatching AllowOnce action
        // which should use the selected granularity
        active_thread(&thread_view, cx).update_in(cx, |view, window, cx| {
            view.allow_once(&AllowOnce, window, cx)
        });

        cx.run_until_parked();

        // Verify tool call was authorized
        thread_view.read_with(cx, |thread_view, cx| {
            let tool_call = thread_view.pending_tool_call(cx);
            assert!(
                tool_call.is_none(),
                "Tool call should be authorized after Allow with pattern granularity"
            );
        });
    }

    #[gpui::test]
    async fn test_deny_button_uses_selected_granularity(cx: &mut TestAppContext) {
        init_test(cx);

        let tool_call_id = acp::ToolCallId::new("deny-granularity-test-1");
        let tool_call =
            acp::ToolCall::new(tool_call_id.clone(), "Run `git push`").kind(acp::ToolKind::Edit);

        let permission_options =
            ToolPermissionContext::new(TerminalTool::NAME, vec!["git push".to_string()])
                .build_permission_options();

        let connection =
            StubAgentConnection::new().with_permission_requests(HashMap::from_iter([(
                tool_call_id.clone(),
                permission_options.clone(),
            )]));

        connection.set_next_prompt_updates(vec![acp::SessionUpdate::ToolCall(tool_call)]);

        let (conversation_view, cx) =
            setup_conversation_view(StubAgentServer::new(connection), cx).await;
        add_to_workspace(conversation_view.clone(), cx);

        cx.update(|_window, cx| {
            AgentSettings::override_global(
                AgentSettings {
                    notify_when_agent_waiting: NotifyWhenAgentWaiting::Never,
                    ..AgentSettings::get_global(cx).clone()
                },
                cx,
            );
        });

        let message_editor = message_editor(&conversation_view, cx);
        message_editor.update_in(cx, |editor, window, cx| {
            editor.set_text("Push changes", window, cx);
        });

        active_thread(&conversation_view, cx)
            .update_in(cx, |view, window, cx| view.send(window, cx));

        cx.run_until_parked();

        // Use default granularity (last option = "Only this time")
        // Simulate clicking the Deny button
        active_thread(&conversation_view, cx).update_in(cx, |view, window, cx| {
            view.reject_once(&RejectOnce, window, cx)
        });

        cx.run_until_parked();

        // Verify tool call was rejected (no longer waiting for confirmation)
        conversation_view.read_with(cx, |conversation_view, cx| {
            let tool_call = conversation_view.pending_tool_call(cx);
            assert!(
                tool_call.is_none(),
                "Tool call should be rejected after Deny"
            );
        });
    }

    #[gpui::test]
    async fn test_option_id_transformation_for_allow() {
        let permission_options = ToolPermissionContext::new(
            TerminalTool::NAME,
            vec!["cargo build --release".to_string()],
        )
        .build_permission_options();

        let PermissionOptions::Dropdown(choices) = permission_options else {
            panic!("Expected dropdown permission options");
        };

        let allow_ids: Vec<String> = choices
            .iter()
            .map(|choice| choice.allow.option_id.0.to_string())
            .collect();

        assert!(allow_ids.contains(&"allow".to_string()));
        assert_eq!(
            allow_ids
                .iter()
                .filter(|id| *id == "always_allow:terminal")
                .count(),
            2,
            "Expected two always_allow:terminal IDs (one whole-tool, one pattern with sub_patterns)"
        );
    }

    #[gpui::test]
    async fn test_option_id_transformation_for_deny() {
        let permission_options = ToolPermissionContext::new(
            TerminalTool::NAME,
            vec!["cargo build --release".to_string()],
        )
        .build_permission_options();

        let PermissionOptions::Dropdown(choices) = permission_options else {
            panic!("Expected dropdown permission options");
        };

        let deny_ids: Vec<String> = choices
            .iter()
            .map(|choice| choice.deny.option_id.0.to_string())
            .collect();

        assert!(deny_ids.contains(&"deny".to_string()));
        assert_eq!(
            deny_ids
                .iter()
                .filter(|id| *id == "always_deny:terminal")
                .count(),
            2,
            "Expected two always_deny:terminal IDs (one whole-tool, one pattern with sub_patterns)"
        );
    }

    fn flat_allow_deny_options() -> PermissionOptions {
        PermissionOptions::Flat(vec![
            acp::PermissionOption::new(
                acp::PermissionOptionId::new("allow"),
                "Yes",
                acp::PermissionOptionKind::AllowOnce,
            ),
            acp::PermissionOption::new(
                acp::PermissionOptionId::new("deny"),
                "No",
                acp::PermissionOptionKind::RejectOnce,
            ),
        ])
    }

    fn sandbox_permission_options() -> PermissionOptions {
        PermissionOptions::Flat(vec![
            acp::PermissionOption::new(
                acp::PermissionOptionId::new("allow"),
                "Allow once",
                acp::PermissionOptionKind::AllowOnce,
            ),
            acp::PermissionOption::new(
                acp::PermissionOptionId::new("allow_thread"),
                "Allow for this thread",
                acp::PermissionOptionKind::AllowAlways,
            ),
            acp::PermissionOption::new(
                acp::PermissionOptionId::new("allow_always"),
                "Allow always",
                acp::PermissionOptionKind::AllowAlways,
            ),
            acp::PermissionOption::new(
                acp::PermissionOptionId::new("deny"),
                "Deny",
                acp::PermissionOptionKind::RejectOnce,
            ),
        ])
    }

    #[test]
    fn permission_option_for_action_prefers_explicit_sandbox_allow_always() {
        let options = sandbox_permission_options();

        let option =
            super::permission_option_for_action(&options, acp::PermissionOptionKind::AllowAlways)
                .unwrap();

        assert_eq!(option.option_id.0.as_ref(), "allow_always");
    }

    #[test]
    fn resolve_outcome_from_selection_flat_allow_picks_allow_once() {
        let options = flat_allow_deny_options();

        let outcome = super::resolve_outcome_from_selection(&options, None, true).unwrap();

        assert_eq!(outcome.option_id.0.as_ref(), "allow");
        assert_eq!(outcome.option_kind, acp::PermissionOptionKind::AllowOnce);
    }

    #[test]
    fn resolve_outcome_from_selection_flat_deny_picks_reject_once() {
        let options = flat_allow_deny_options();

        let outcome = super::resolve_outcome_from_selection(&options, None, false).unwrap();

        assert_eq!(outcome.option_id.0.as_ref(), "deny");
        assert_eq!(outcome.option_kind, acp::PermissionOptionKind::RejectOnce);
    }

    #[test]
    fn resolve_outcome_from_selection_flat_ignores_selection() {
        let options = flat_allow_deny_options();
        // Flat options never consult the granularity choice, even if one is set.
        let selection = thread_view::PermissionSelection::Choice(42);

        let outcome =
            super::resolve_outcome_from_selection(&options, Some(&selection), true).unwrap();

        assert_eq!(outcome.option_id.0.as_ref(), "allow");
    }

    #[test]
    fn resolve_outcome_from_selection_dropdown_defaults_to_last_choice_when_no_selection() {
        let options =
            ToolPermissionContext::new(TerminalTool::NAME, vec!["cargo build".to_string()])
                .build_permission_options();

        let outcome = super::resolve_outcome_from_selection(&options, None, true).unwrap();

        // Last choice is "Only this time" → option_id "allow".
        assert_eq!(outcome.option_id.0.as_ref(), "allow");
        assert_eq!(outcome.option_kind, acp::PermissionOptionKind::AllowOnce);
    }

    #[test]
    fn resolve_outcome_from_selection_dropdown_uses_selected_choice() {
        let options =
            ToolPermissionContext::new(TerminalTool::NAME, vec!["cargo build".to_string()])
                .build_permission_options();
        let selection = thread_view::PermissionSelection::Choice(0);

        let outcome =
            super::resolve_outcome_from_selection(&options, Some(&selection), true).unwrap();

        // Choice 0 = "Always for terminal".
        assert!(outcome.option_id.0.contains("always_allow:terminal"));
        assert_eq!(outcome.option_kind, acp::PermissionOptionKind::AllowAlways);
    }

    #[test]
    fn resolve_outcome_from_selection_dropdown_out_of_range_falls_back_to_last() {
        let options =
            ToolPermissionContext::new(TerminalTool::NAME, vec!["cargo build".to_string()])
                .build_permission_options();
        let selection = thread_view::PermissionSelection::Choice(999);

        let outcome =
            super::resolve_outcome_from_selection(&options, Some(&selection), true).unwrap();

        // choices.get(999) is None, falls back to choices.last() → "Only this time".
        assert_eq!(outcome.option_id.0.as_ref(), "allow");
    }

    #[test]
    fn resolve_outcome_from_selection_pattern_mode_with_empty_checked_falls_back_to_last_choice() {
        // Pipeline commands produce `DropdownWithPatterns`, which is required for
        // `SelectedPatterns` to be meaningful.
        let options = ToolPermissionContext::new(
            TerminalTool::NAME,
            vec!["cargo test 2>&1 | tail".to_string()],
        )
        .build_permission_options();
        assert!(matches!(
            options,
            PermissionOptions::DropdownWithPatterns { .. }
        ));
        // Pattern mode with zero checked patterns: `build_outcome_for_checked_patterns`
        // returns None, so we fall through to `choice_index()` (which is None for
        // `SelectedPatterns`) and default to `choices.last()`.
        let selection = thread_view::PermissionSelection::SelectedPatterns(vec![]);

        let outcome =
            super::resolve_outcome_from_selection(&options, Some(&selection), true).unwrap();

        assert_eq!(outcome.option_id.0.as_ref(), "allow");
        assert_eq!(outcome.option_kind, acp::PermissionOptionKind::AllowOnce);
    }

    #[test]
    fn resolve_outcome_from_selection_pattern_mode_with_checked_uses_always_with_params() {
        let options = ToolPermissionContext::new(
            TerminalTool::NAME,
            vec!["cargo test 2>&1 | tail".to_string()],
        )
        .build_permission_options();
        assert!(matches!(
            options,
            PermissionOptions::DropdownWithPatterns { .. }
        ));
        let selection = thread_view::PermissionSelection::SelectedPatterns(vec![0]);

        let outcome =
            super::resolve_outcome_from_selection(&options, Some(&selection), true).unwrap();

        assert_eq!(outcome.option_kind, acp::PermissionOptionKind::AllowAlways);
        assert!(
            outcome.params.is_some(),
            "checked patterns should attach terminal params"
        );
    }

    #[gpui::test]
    async fn test_manually_editing_title_updates_acp_thread_title(cx: &mut TestAppContext) {
        init_test(cx);

        let (conversation_view, cx) =
            setup_conversation_view(StubAgentServer::default_response(), cx).await;
        add_to_workspace(conversation_view.clone(), cx);

        let active = active_thread(&conversation_view, cx);
        let title_editor = cx.read(|cx| active.read(cx).title_editor.clone());
        let thread = cx.read(|cx| active.read(cx).thread.clone());

        title_editor.read_with(cx, |editor, cx| {
            assert!(!editor.read_only(cx));
        });

        cx.focus(&conversation_view);
        cx.focus(&title_editor);

        cx.dispatch_action(editor::actions::DeleteLine);
        cx.simulate_input("My Custom Title");

        cx.run_until_parked();

        title_editor.read_with(cx, |editor, cx| {
            assert_eq!(editor.text(cx), "My Custom Title");
        });
        thread.read_with(cx, |thread, _cx| {
            assert_eq!(thread.title(), Some("My Custom Title".into()));
        });
    }

    #[gpui::test]
    async fn test_max_tokens_error_is_rendered(cx: &mut TestAppContext) {
        init_test(cx);

        let connection = StubAgentConnection::new();

        let (conversation_view, cx) =
            setup_conversation_view(StubAgentServer::new(connection.clone()), cx).await;

        let message_editor = message_editor(&conversation_view, cx);
        message_editor.update_in(cx, |editor, window, cx| {
            editor.set_text("Some prompt", window, cx);
        });
        active_thread(&conversation_view, cx)
            .update_in(cx, |view, window, cx| view.send(window, cx));

        let session_id = conversation_view.read_with(cx, |view, cx| {
            view.active_thread()
                .unwrap()
                .read(cx)
                .thread
                .read(cx)
                .session_id()
                .clone()
        });

        cx.run_until_parked();

        cx.update(|_, _cx| {
            connection.end_turn(session_id, acp::StopReason::MaxTokens);
        });

        cx.run_until_parked();

        conversation_view.read_with(cx, |conversation_view, cx| {
            let state = conversation_view.active_thread().unwrap();
            let error = &state.read(cx).thread_error;
            assert!(
                matches!(error, Some(ThreadError::MaxOutputTokens)),
                "Expected ThreadError::MaxOutputTokens, got: {:?}",
                error.is_some()
            );
        });
    }

    fn create_test_acp_thread(
        parent_session_id: Option<acp::SessionId>,
        session_id: &str,
        connection: Rc<dyn AgentConnection>,
        project: Entity<Project>,
        cx: &mut App,
    ) -> Entity<AcpThread> {
        let action_log = cx.new(|_| ActionLog::new(project.clone()));
        cx.new(|cx| {
            AcpThread::new(
                parent_session_id,
                None,
                None,
                connection,
                project,
                action_log,
                acp::SessionId::new(session_id),
                watch::Receiver::constant(acp::PromptCapabilities::new()),
                cx,
            )
        })
    }

    fn request_test_tool_authorization(
        thread: &Entity<AcpThread>,
        tool_call_id: &str,
        option_id: &str,
        cx: &mut TestAppContext,
    ) -> Task<acp_thread::RequestPermissionOutcome> {
        let tool_call_id = acp::ToolCallId::new(tool_call_id);
        let label = format!("Tool {tool_call_id}");
        let option_id = acp::PermissionOptionId::new(option_id);
        cx.update(|cx| {
            thread.update(cx, |thread, cx| {
                thread
                    .request_tool_call_authorization(
                        acp::ToolCall::new(tool_call_id, label)
                            .kind(acp::ToolKind::Edit)
                            .into(),
                        PermissionOptions::Flat(vec![acp::PermissionOption::new(
                            option_id,
                            "Allow",
                            acp::PermissionOptionKind::AllowOnce,
                        )]),
                        acp_thread::AuthorizationKind::PermissionGrant,
                        cx,
                    )
                    .unwrap()
            })
        })
    }

    #[gpui::test]
    async fn test_conversation_multiple_tool_calls_fifo_ordering(cx: &mut TestAppContext) {
        init_test(cx);

        let fs = FakeFs::new(cx.executor());
        let project = Project::test(fs, [], cx).await;
        let connection: Rc<dyn AgentConnection> = Rc::new(StubAgentConnection::new());

        let session_id = acp::SessionId::new("session-1");
        let (thread, conversation) = cx.update(|cx| {
            let thread =
                create_test_acp_thread(None, "session-1", connection.clone(), project.clone(), cx);
            let conversation = cx.new(|cx| {
                let mut conversation = Conversation::default();
                conversation.register_thread(thread.clone(), cx);
                conversation
            });
            (thread, conversation)
        });

        let _task1 = request_test_tool_authorization(&thread, "tc-1", "allow-1", cx);
        let _task2 = request_test_tool_authorization(&thread, "tc-2", "allow-2", cx);

        cx.read(|cx| {
            let (_, tool_call_id, _) = conversation
                .read(cx)
                .pending_tool_call(&session_id, cx)
                .expect("Expected a pending tool call");
            assert_eq!(tool_call_id, acp::ToolCallId::new("tc-1"));
        });

        cx.update(|cx| {
            conversation.update(cx, |conversation, cx| {
                conversation.authorize_tool_call(
                    session_id.clone(),
                    acp::ToolCallId::new("tc-1"),
                    SelectedPermissionOutcome::new(
                        acp::PermissionOptionId::new("allow-1"),
                        acp::PermissionOptionKind::AllowOnce,
                    ),
                    cx,
                );
            });
        });

        cx.run_until_parked();

        cx.read(|cx| {
            let (_, tool_call_id, _) = conversation
                .read(cx)
                .pending_tool_call(&session_id, cx)
                .expect("Expected tc-2 to be pending after tc-1 was authorized");
            assert_eq!(tool_call_id, acp::ToolCallId::new("tc-2"));
        });

        cx.update(|cx| {
            conversation.update(cx, |conversation, cx| {
                conversation.authorize_tool_call(
                    session_id.clone(),
                    acp::ToolCallId::new("tc-2"),
                    SelectedPermissionOutcome::new(
                        acp::PermissionOptionId::new("allow-2"),
                        acp::PermissionOptionKind::AllowOnce,
                    ),
                    cx,
                );
            });
        });

        cx.run_until_parked();

        cx.read(|cx| {
            assert!(
                conversation
                    .read(cx)
                    .pending_tool_call(&session_id, cx)
                    .is_none(),
                "Expected no pending tool calls after both were authorized"
            );
        });
    }

    #[gpui::test]
    async fn test_conversation_subagent_scoped_pending_tool_call(cx: &mut TestAppContext) {
        init_test(cx);

        let fs = FakeFs::new(cx.executor());
        let project = Project::test(fs, [], cx).await;
        let connection: Rc<dyn AgentConnection> = Rc::new(StubAgentConnection::new());

        let parent_session_id = acp::SessionId::new("parent");
        let subagent_session_id = acp::SessionId::new("subagent");
        let (parent_thread, subagent_thread, conversation) = cx.update(|cx| {
            let parent_thread =
                create_test_acp_thread(None, "parent", connection.clone(), project.clone(), cx);
            let subagent_thread = create_test_acp_thread(
                Some(acp::SessionId::new("parent")),
                "subagent",
                connection.clone(),
                project.clone(),
                cx,
            );
            let conversation = cx.new(|cx| {
                let mut conversation = Conversation::default();
                conversation.register_thread(parent_thread.clone(), cx);
                conversation.register_thread(subagent_thread.clone(), cx);
                conversation
            });
            (parent_thread, subagent_thread, conversation)
        });

        let _parent_task =
            request_test_tool_authorization(&parent_thread, "parent-tc", "allow-parent", cx);
        let _subagent_task =
            request_test_tool_authorization(&subagent_thread, "subagent-tc", "allow-subagent", cx);

        // Querying with the subagent's session ID returns only the
        // subagent's own tool call (subagent path is scoped to its session)
        cx.read(|cx| {
            let (returned_session_id, tool_call_id, _) = conversation
                .read(cx)
                .pending_tool_call(&subagent_session_id, cx)
                .expect("Expected subagent's pending tool call");
            assert_eq!(returned_session_id, subagent_session_id);
            assert_eq!(tool_call_id, acp::ToolCallId::new("subagent-tc"));
        });

        // Querying with the parent's session ID returns the first pending
        // request in FIFO order across all sessions
        cx.read(|cx| {
            let (returned_session_id, tool_call_id, _) = conversation
                .read(cx)
                .pending_tool_call(&parent_session_id, cx)
                .expect("Expected a pending tool call from parent query");
            assert_eq!(returned_session_id, parent_session_id);
            assert_eq!(tool_call_id, acp::ToolCallId::new("parent-tc"));
        });
    }

    #[gpui::test]
    async fn test_conversation_parent_pending_tool_call_returns_first_across_threads(
        cx: &mut TestAppContext,
    ) {
        init_test(cx);

        let fs = FakeFs::new(cx.executor());
        let project = Project::test(fs, [], cx).await;
        let connection: Rc<dyn AgentConnection> = Rc::new(StubAgentConnection::new());

        let session_id_a = acp::SessionId::new("thread-a");
        let session_id_b = acp::SessionId::new("thread-b");
        let (thread_a, thread_b, conversation) = cx.update(|cx| {
            let thread_a =
                create_test_acp_thread(None, "thread-a", connection.clone(), project.clone(), cx);
            let thread_b =
                create_test_acp_thread(None, "thread-b", connection.clone(), project.clone(), cx);
            let conversation = cx.new(|cx| {
                let mut conversation = Conversation::default();
                conversation.register_thread(thread_a.clone(), cx);
                conversation.register_thread(thread_b.clone(), cx);
                conversation
            });
            (thread_a, thread_b, conversation)
        });

        let _task_a = request_test_tool_authorization(&thread_a, "tc-a", "allow-a", cx);
        let _task_b = request_test_tool_authorization(&thread_b, "tc-b", "allow-b", cx);

        // Both threads are non-subagent, so pending_tool_call always returns
        // the first entry from permission_requests (FIFO across all sessions)
        cx.read(|cx| {
            let (returned_session_id, tool_call_id, _) = conversation
                .read(cx)
                .pending_tool_call(&session_id_a, cx)
                .expect("Expected a pending tool call");
            assert_eq!(returned_session_id, session_id_a);
            assert_eq!(tool_call_id, acp::ToolCallId::new("tc-a"));
        });

        // Querying with thread-b also returns thread-a's tool call,
        // because non-subagent queries always use permission_requests.first()
        cx.read(|cx| {
            let (returned_session_id, tool_call_id, _) = conversation
                .read(cx)
                .pending_tool_call(&session_id_b, cx)
                .expect("Expected a pending tool call from thread-b query");
            assert_eq!(
                returned_session_id, session_id_a,
                "Non-subagent queries always return the first pending request in FIFO order"
            );
            assert_eq!(tool_call_id, acp::ToolCallId::new("tc-a"));
        });

        // After authorizing thread-a's tool call, thread-b's becomes first
        cx.update(|cx| {
            conversation.update(cx, |conversation, cx| {
                conversation.authorize_tool_call(
                    session_id_a.clone(),
                    acp::ToolCallId::new("tc-a"),
                    SelectedPermissionOutcome::new(
                        acp::PermissionOptionId::new("allow-a"),
                        acp::PermissionOptionKind::AllowOnce,
                    ),
                    cx,
                );
            });
        });

        cx.run_until_parked();

        cx.read(|cx| {
            let (returned_session_id, tool_call_id, _) = conversation
                .read(cx)
                .pending_tool_call(&session_id_b, cx)
                .expect("Expected thread-b's tool call after thread-a's was authorized");
            assert_eq!(returned_session_id, session_id_b);
            assert_eq!(tool_call_id, acp::ToolCallId::new("tc-b"));
        });
    }

    /// Set up a `ConversationView` whose active thread has a single tool call
    /// awaiting permission. Returns the conversation view, its active
    /// `ThreadView`, and the entry index of the tool call within the thread.
    async fn setup_pending_permission_thread<'a>(
        tool_call_id: &str,
        cx: &'a mut TestAppContext,
    ) -> (
        Entity<ConversationView>,
        Entity<ThreadView>,
        usize,
        &'a mut VisualTestContext,
    ) {
        let tool_call_id_value = acp::ToolCallId::new(tool_call_id);
        let tool_call = acp::ToolCall::new(tool_call_id_value.clone(), "Run something")
            .kind(acp::ToolKind::Edit);

        let connection =
            StubAgentConnection::new().with_permission_requests(HashMap::from_iter([(
                tool_call_id_value.clone(),
                PermissionOptions::Flat(vec![acp::PermissionOption::new(
                    "allow",
                    "Allow",
                    acp::PermissionOptionKind::AllowOnce,
                )]),
            )]));
        connection.set_next_prompt_updates(vec![acp::SessionUpdate::ToolCall(tool_call)]);

        let (conversation_view, cx) =
            setup_conversation_view(StubAgentServer::new(connection), cx).await;
        add_to_workspace(conversation_view.clone(), cx);

        cx.update(|_window, cx| {
            AgentSettings::override_global(
                AgentSettings {
                    notify_when_agent_waiting: NotifyWhenAgentWaiting::Never,
                    ..AgentSettings::get_global(cx).clone()
                },
                cx,
            );
        });

        let message_editor = message_editor(&conversation_view, cx);
        message_editor.update_in(cx, |editor, window, cx| {
            editor.set_text("Hello", window, cx);
        });

        active_thread(&conversation_view, cx)
            .update_in(cx, |view, window, cx| view.send(window, cx));

        cx.run_until_parked();

        let thread_view = active_thread(&conversation_view, cx);
        let entry_ix = thread_view.read_with(cx, |view, cx| {
            view.thread
                .read(cx)
                .entries()
                .iter()
                .position(|entry| {
                    matches!(
                        entry,
                        acp_thread::AgentThreadEntry::ToolCall(call)
                            if call.id == tool_call_id_value
                    )
                })
                .expect("tool call entry should exist after run_until_parked")
        });

        (conversation_view, thread_view, entry_ix, cx)
    }

    struct TestListView {
        list_state: ListState,
    }

    impl Render for TestListView {
        fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
            list(self.list_state.clone(), |_, _, _| {
                div().h(px(20.0)).w_full().into_any_element()
            })
            .size_full()
        }
    }

    fn draw_thread_list_at(
        thread_view: &Entity<ThreadView>,
        scroll_top: ListOffset,
        cx: &mut VisualTestContext,
    ) {
        let list_state = thread_view.read_with(cx, |view, _cx| view.list_state.clone());
        list_state.scroll_to(scroll_top);
        cx.draw(
            point(px(0.0), px(0.0)),
            size(px(100.0), px(20.0)),
            |_, cx| {
                cx.new(|_| TestListView {
                    list_state: list_state.clone(),
                })
                .into_any_element()
            },
        );
    }

    #[gpui::test]
    async fn test_permission_row_hidden_when_inline_bounds_unavailable(cx: &mut TestAppContext) {
        init_test(cx);

        let (_view, thread_view, entry_ix, cx) =
            setup_pending_permission_thread("perm-no-bounds", cx).await;

        // Pin the scroll top to the entry so it isn't treated as above the
        // viewport, forcing the unmeasured-bounds path we want to exercise.
        thread_view.read_with(cx, |view, _cx| {
            view.list_state.scroll_to(ListOffset {
                item_ix: entry_ix,
                offset_in_item: px(0.0),
            });
        });
        thread_view.update_in(cx, |view, window, cx| {
            assert!(
                view.render_main_agent_awaiting_permission(window, cx)
                    .is_none(),
                "Floating row should stay hidden until the inline prompt has known list bounds"
            );
        });
    }

    #[gpui::test]
    async fn test_pending_tool_call_for_session_scopes_to_that_session(cx: &mut TestAppContext) {
        init_test(cx);

        let fs = FakeFs::new(cx.executor());
        let project = Project::test(fs, [], cx).await;
        let connection: Rc<dyn AgentConnection> = Rc::new(StubAgentConnection::new());

        let session_id_a = acp::SessionId::new("thread-a");
        let session_id_b = acp::SessionId::new("thread-b");
        let (thread_a, thread_b, conversation) = cx.update(|cx| {
            let thread_a =
                create_test_acp_thread(None, "thread-a", connection.clone(), project.clone(), cx);
            let thread_b =
                create_test_acp_thread(None, "thread-b", connection.clone(), project.clone(), cx);
            let conversation = cx.new(|cx| {
                let mut conversation = Conversation::default();
                conversation.register_thread(thread_a.clone(), cx);
                conversation.register_thread(thread_b.clone(), cx);
                conversation
            });
            (thread_a, thread_b, conversation)
        });

        // Pending tool calls in both threads. Unlike `pending_tool_call`,
        // `pending_tool_call_for_session` must not fall back across threads.
        let _task_a = request_test_tool_authorization(&thread_a, "tc-a", "allow-a", cx);
        let _task_b = request_test_tool_authorization(&thread_b, "tc-b", "allow-b", cx);

        cx.read(|cx| {
            let tool_call_id_a = conversation
                .read(cx)
                .pending_tool_call_for_session(&session_id_a, cx)
                .expect("Expected a pending tool call in thread A");
            assert_eq!(tool_call_id_a, acp::ToolCallId::new("tc-a"));

            let tool_call_id_b = conversation
                .read(cx)
                .pending_tool_call_for_session(&session_id_b, cx)
                .expect("Expected a pending tool call in thread B");
            assert_eq!(tool_call_id_b, acp::ToolCallId::new("tc-b"));
        });
    }

    #[gpui::test]
    async fn test_permission_row_scroll_to_dismisses_row(cx: &mut TestAppContext) {
        init_test(cx);

        let (_view, thread_view, entry_ix, cx) =
            setup_pending_permission_thread("perm-scroll", cx).await;

        // Start off-screen below the viewport. The row is visible because the
        // item has bounds that do not intersect the viewport.
        draw_thread_list_at(
            &thread_view,
            ListOffset {
                item_ix: 0,
                offset_in_item: px(0.0),
            },
            cx,
        );
        thread_view.read_with(cx, |view, _cx| {
            assert!(
                view.list_state.bounds_for_item(entry_ix).is_some(),
                "The tool call entry must be measured for this test to exercise the\
                 \"entry below viewport\" branch. If list overdraw stops measuring\
                 offscreen items, this test needs to drive measurement another way."
            );
        });
        thread_view.update_in(cx, |view, window, cx| {
            assert!(
                view.render_main_agent_awaiting_permission(window, cx)
                    .is_some()
            );
        });

        // Simulate clicking "Scroll to": the list scrolls to the entry and the
        // measured item bounds intersect the viewport.
        draw_thread_list_at(
            &thread_view,
            ListOffset {
                item_ix: entry_ix,
                offset_in_item: px(0.0),
            },
            cx,
        );

        thread_view.update_in(cx, |view, window, cx| {
            assert!(
                view.render_main_agent_awaiting_permission(window, cx)
                    .is_none(),
                "Floating row should disappear after scrolling brings the inline prompt into view"
            );
        });
    }

    #[gpui::test]
    async fn test_permission_row_does_not_flicker_when_activity_bar_squeezes_list(
        cx: &mut TestAppContext,
    ) {
        init_test(cx);

        let (_view, thread_view, _entry_ix, cx) =
            setup_pending_permission_thread("perm-flicker", cx).await;

        // Give the pending tool call tall content (like a full plan awaiting
        // approval), so the floating row embedding it dwarfs the panel.
        let thread = thread_view.read_with(cx, |view, _cx| view.thread.clone());
        thread.update(cx, |thread, cx| {
            thread
                .handle_session_update(
                    acp::SessionUpdate::ToolCallUpdate(acp::ToolCallUpdate::new(
                        acp::ToolCallId::new("perm-flicker"),
                        acp::ToolCallUpdateFields::new().content(vec![
                            acp::ToolCallContent::Content(acp::Content::new(
                                acp::ContentBlock::Text(acp::TextContent::new(
                                    "Plan step\n\n".repeat(100),
                                )),
                            )),
                        ]),
                    )),
                    cx,
                )
                .expect("tool call content update should be accepted");
        });
        cx.run_until_parked();

        // Park the inline prompt below the viewport so the floating row renders.
        thread_view.read_with(cx, |view, _cx| {
            view.list_state.scroll_to(ListOffset {
                item_ix: 0,
                offset_in_item: px(0.0),
            });
        });

        // Drive several real window draws. Each draw lays out the activity bar
        // (containing the floating row) and the conversation list together, so
        // the row's height feeds back into the list viewport height that the
        // next frame's visibility decision is based on. Since showing the row
        // squeezes the list to zero height, a decision that treats a
        // zero-height viewport as "unknown" makes the row's visibility
        // oscillate from frame to frame, flickering between the conversation
        // and the permission prompt.
        let mut row_visibility = Vec::new();
        for _ in 0..4 {
            thread_view.update(cx, |_, cx| cx.notify());
            cx.run_until_parked();
            thread_view.update_in(cx, |view, window, cx| {
                row_visibility.push(
                    view.render_main_agent_awaiting_permission(window, cx)
                        .is_some(),
                );
            });
        }
        assert_eq!(
            row_visibility,
            vec![true; 4],
            "Floating row visibility must be stable across frames (false entries mean flicker)"
        );
    }

    #[gpui::test]
    async fn test_permission_row_shown_when_inline_prompt_is_above_viewport(
        cx: &mut TestAppContext,
    ) {
        init_test(cx);

        let (_view, thread_view, entry_ix, cx) =
            setup_pending_permission_thread("perm-above", cx).await;

        let thread = thread_view.read_with(cx, |view, _cx| view.thread.clone());
        thread.update(cx, |thread, cx| {
            let result = thread.handle_session_update(
                acp::SessionUpdate::AgentMessageChunk(acp::ContentChunk::new(
                    "More content".into(),
                )),
                cx,
            );
            assert!(
                result.is_ok(),
                "following assistant message should be accepted"
            );
        });

        draw_thread_list_at(
            &thread_view,
            ListOffset {
                item_ix: entry_ix + 1,
                offset_in_item: px(0.0),
            },
            cx,
        );
        thread_view.read_with(cx, |view, _cx| {
            assert!(
                entry_ix < view.list_state.logical_scroll_top().item_ix,
                "The tool call entry should be above the logical scroll top"
            );
        });
        thread_view.update_in(cx, |view, window, cx| {
            assert!(
                view.render_main_agent_awaiting_permission(window, cx)
                    .is_some(),
                "Floating row should be visible when the inline prompt is above the viewport"
            );
        });

        // Scrolling up to the entry brings it back into view.
        draw_thread_list_at(
            &thread_view,
            ListOffset {
                item_ix: entry_ix,
                offset_in_item: px(0.0),
            },
            cx,
        );
        thread_view.update_in(cx, |view, window, cx| {
            assert!(
                view.render_main_agent_awaiting_permission(window, cx)
                    .is_none(),
                "Floating row should disappear after scrolling brings the inline prompt into view"
            );
        });
    }

    #[gpui::test]
    async fn test_permission_row_disappears_when_authorized(cx: &mut TestAppContext) {
        init_test(cx);

        let (conversation_view, thread_view, _entry_ix, cx) =
            setup_pending_permission_thread("perm-allow", cx).await;

        // Park the inline prompt below the viewport so the floating row would render.
        draw_thread_list_at(
            &thread_view,
            ListOffset {
                item_ix: 0,
                offset_in_item: px(0.0),
            },
            cx,
        );
        thread_view.update_in(cx, |view, window, cx| {
            assert!(
                view.render_main_agent_awaiting_permission(window, cx)
                    .is_some(),
                "Floating row should be visible before authorizing"
            );
        });

        // Dispatch the same AuthorizeToolCall action the row's Allow button
        // wires up.
        conversation_view.update_in(cx, |_, window, cx| {
            window.dispatch_action(
                crate::AuthorizeToolCall {
                    tool_call_id: "perm-allow".to_string(),
                    option_id: "allow".to_string(),
                    option_kind: "AllowOnce".to_string(),
                }
                .boxed_clone(),
                cx,
            );
        });
        cx.run_until_parked();

        conversation_view.read_with(cx, |view, cx| {
            assert!(
                view.pending_tool_call(cx).is_none(),
                "Tool call should no longer be pending after Allow is clicked"
            );
        });
        thread_view.update_in(cx, |view, window, cx| {
            assert!(
                view.render_main_agent_awaiting_permission(window, cx)
                    .is_none(),
                "Floating row should disappear once the permission is granted"
            );
        });
    }

    /// A queued message is the person's typing, and only the thread it was
    /// queued on may spend it.
    ///
    /// The subagent here records no parent, which is not a contrived state: an
    /// external ACP subagent is opened by `new_session` on the *other* agent's
    /// connection, which has never heard of the thread that asked for it, so
    /// `parent_session_id` is `None` on every Codex or Claude delegation. The
    /// root's queue was driven by asking the thread that stopped whether it
    /// named a parent, so a delegation finishing read as the root finishing:
    /// the queued message was dispatched mid-turn and left the queue, and the
    /// person who had queued a follow-up and gone to watch the delegation work
    /// saw it disappear with nothing said.
    #[gpui::test]
    async fn a_subagent_turn_ending_cannot_spend_the_root_threads_queue(cx: &mut TestAppContext) {
        init_test(cx);

        let (conversation_view, cx) =
            setup_conversation_view(StubAgentServer::default_response(), cx).await;
        add_to_workspace(conversation_view.clone(), cx);

        let fs = FakeFs::new(cx.executor());
        let project = Project::test(fs, [], cx).await;
        let stub: Rc<dyn AgentConnection> = Rc::new(StubAgentConnection::new());
        let subagent_thread = cx.update(|_window, cx| {
            create_test_acp_thread(None, "external-subagent", stub, project, cx)
        });
        conversation_view.update_in(cx, |view, window, cx| {
            view.register_subagent_thread_view(subagent_thread.clone(), window, cx);
        });

        active_thread(&conversation_view, cx).update_in(cx, |thread, window, cx| {
            thread
                .add_to_queue(
                    vec![acp::ContentBlock::Text(acp::TextContent::new(
                        "queued message".to_string(),
                    ))],
                    vec![],
                    window,
                    cx,
                )
                .expect("queue admission");
        });
        cx.run_until_parked();

        subagent_thread.update(cx, |_thread, cx| {
            cx.emit(AcpThreadEvent::Stopped(acp::StopReason::EndTurn));
        });
        cx.run_until_parked();

        let root_thread = cx.read(|cx| conversation_view.read(cx).root_thread_view().unwrap());
        let (queued, text) = root_thread.read_with(cx, |thread, _cx| {
            (
                thread.message_queue.len(),
                thread
                    .message_queue
                    .first()
                    .and_then(|entry| match entry.content.first() {
                        Some(acp::ContentBlock::Text(text)) => Some(text.text.clone()),
                        _ => None,
                    }),
            )
        });
        assert_eq!(
            queued, 1,
            "a subagent's turn ending spent the root thread's queued message"
        );
        assert_eq!(text.as_deref(), Some("queued message"));
    }

    #[gpui::test]
    async fn opening_subagent_in_right_pane_preserves_active_thread(cx: &mut TestAppContext) {
        init_test(cx);

        let (conversation_view, cx) =
            setup_conversation_view(StubAgentServer::default_response(), cx).await;
        add_to_workspace(conversation_view.clone(), cx);

        let root_session_id = conversation_view.read_with(cx, |view, cx| {
            view.active_thread()
                .expect("conversation should have an active thread")
                .read(cx)
                .session_id
                .clone()
        });
        let subagent_session_id = acp::SessionId::new("right-pane-subagent");
        let fs = FakeFs::new(cx.executor());
        let project = Project::test(fs, [], cx).await;
        let connection: Rc<dyn AgentConnection> = Rc::new(StubAgentConnection::new());
        let subagent_thread = cx.update(|_window, cx| {
            create_test_acp_thread(
                Some(root_session_id.clone()),
                subagent_session_id.0.as_ref(),
                connection,
                project,
                cx,
            )
        });
        conversation_view.update_in(cx, |view, window, cx| {
            view.register_subagent_thread_view(subagent_thread, window, cx);
            view.open_subagent_in_right_pane(subagent_session_id.clone(), window, cx);
        });

        conversation_view.read_with(cx, |view, _cx| {
            let connected = view
                .as_connected()
                .expect("conversation should be connected");
            assert_eq!(connected.active_id.as_ref(), Some(&root_session_id));
            assert_eq!(
                connected.right_pane_session_id.as_ref(),
                Some(&subagent_session_id)
            );
        });

        conversation_view.update(cx, |view, cx| view.close_right_pane(cx));
        conversation_view.read_with(cx, |view, _cx| {
            let connected = view
                .as_connected()
                .expect("conversation should be connected");
            assert_eq!(connected.active_id.as_ref(), Some(&root_session_id));
            assert!(connected.right_pane_session_id.is_none());
        });
    }

    #[gpui::test]
    async fn test_permission_row_ignores_subagent_requests(cx: &mut TestAppContext) {
        init_test(cx);

        // Build a baseline ConversationView with no permission requests, so we
        // have a real `ThreadView` to call `render_main_agent_awaiting_permission` on.
        let (conversation_view, cx) =
            setup_conversation_view(StubAgentServer::default_response(), cx).await;
        add_to_workspace(conversation_view.clone(), cx);

        let message_editor = message_editor(&conversation_view, cx);
        message_editor.update_in(cx, |editor, window, cx| {
            editor.set_text("Hello", window, cx);
        });
        active_thread(&conversation_view, cx)
            .update_in(cx, |view, window, cx| view.send(window, cx));
        cx.run_until_parked();

        let thread_view = active_thread(&conversation_view, cx);
        let parent_session_id =
            thread_view.read_with(cx, |view, cx| view.thread.read(cx).session_id().clone());
        let conversation = thread_view.read_with(cx, |view, _cx| view.conversation.clone());

        // Attach a subagent thread with a pending tool-call permission request.
        let fs = FakeFs::new(cx.executor());
        let project = Project::test(fs, [], cx).await;
        let stub: Rc<dyn AgentConnection> = Rc::new(StubAgentConnection::new());
        let subagent_thread = cx.update(|_window, cx| {
            create_test_acp_thread(
                Some(parent_session_id.clone()),
                "subagent",
                stub,
                project,
                cx,
            )
        });
        conversation.update(cx, |conversation, cx| {
            conversation.register_thread(subagent_thread.clone(), cx);
        });
        let _subagent_task =
            request_test_tool_authorization(&subagent_thread, "sub-tc", "allow-sub", cx);
        cx.run_until_parked();

        cx.read(|cx| {
            assert!(
                conversation
                    .read(cx)
                    .pending_tool_call_for_session(&parent_session_id, cx)
                    .is_none(),
                "Subagent requests must not surface as pending in the parent session"
            );
            assert!(
                !conversation
                    .read(cx)
                    .subagents_awaiting_permission(cx)
                    .is_empty(),
                "Subagent permission row should still see the pending request"
            );
        });

        thread_view.update_in(cx, |view, window, cx| {
            assert!(
                view.render_main_agent_awaiting_permission(window, cx)
                    .is_none(),
                "Subagent permission requests should not trigger the main-agent floating row"
            );
        });
    }

    #[gpui::test]
    async fn test_move_queued_message_to_empty_main_editor(cx: &mut TestAppContext) {
        init_test(cx);

        let (conversation_view, cx) =
            setup_conversation_view(StubAgentServer::default_response(), cx).await;

        // Add a plain-text message to the queue directly.
        active_thread(&conversation_view, cx).update_in(cx, |thread, window, cx| {
            thread
                .add_to_queue(
                    vec![acp::ContentBlock::Text(acp::TextContent::new(
                        "queued message".to_string(),
                    ))],
                    vec![],
                    window,
                    cx,
                )
                .expect("queue admission");
            // Main editor must be empty for this path — it is by default, but
            // assert to make the precondition explicit.
            assert!(thread.message_editor.read(cx).is_empty(cx));
            let id = thread.message_queue.first_id().unwrap();
            thread.move_queued_message_to_main_editor(id, None, None, window, cx);
        });

        cx.run_until_parked();

        // Queue should now be empty.
        let queue_len = active_thread(&conversation_view, cx)
            .read_with(cx, |thread, _cx| thread.message_queue.len());
        assert_eq!(queue_len, 0, "Queue should be empty after move");

        // Main editor should contain the queued message text.
        let text = message_editor(&conversation_view, cx).update(cx, |editor, cx| editor.text(cx));
        assert_eq!(
            text, "queued message",
            "Main editor should contain the moved queued message"
        );
    }

    #[gpui::test]
    async fn test_move_queued_message_to_non_empty_main_editor(cx: &mut TestAppContext) {
        init_test(cx);

        let (conversation_view, cx) =
            setup_conversation_view(StubAgentServer::default_response(), cx).await;

        // Seed the main editor with existing content.
        message_editor(&conversation_view, cx).update_in(cx, |editor, window, cx| {
            editor.set_message(
                vec![acp::ContentBlock::Text(acp::TextContent::new(
                    "existing content".to_string(),
                ))],
                window,
                cx,
            );
        });

        // Add a plain-text message to the queue.
        active_thread(&conversation_view, cx).update_in(cx, |thread, window, cx| {
            thread
                .add_to_queue(
                    vec![acp::ContentBlock::Text(acp::TextContent::new(
                        "queued message".to_string(),
                    ))],
                    vec![],
                    window,
                    cx,
                )
                .expect("queue admission");
            let id = thread.message_queue.first_id().unwrap();
            thread.move_queued_message_to_main_editor(id, None, None, window, cx);
        });

        cx.run_until_parked();

        // Queue should now be empty.
        let queue_len = active_thread(&conversation_view, cx)
            .read_with(cx, |thread, _cx| thread.message_queue.len());
        assert_eq!(queue_len, 0, "Queue should be empty after move");

        // Main editor should contain existing content + separator + queued content.
        let text = message_editor(&conversation_view, cx).update(cx, |editor, cx| editor.text(cx));
        assert_eq!(
            text, "existing content\n\nqueued message",
            "Main editor should have existing content and queued message separated by two newlines"
        );
    }

    #[gpui::test]
    async fn test_move_up_in_empty_editor_restores_last_queued_message(cx: &mut TestAppContext) {
        init_test(cx);

        let (conversation_view, cx) =
            setup_conversation_view(StubAgentServer::default_response(), cx).await;
        add_to_workspace(conversation_view.clone(), cx);

        active_thread(&conversation_view, cx).update_in(cx, |thread, window, cx| {
            thread
                .add_to_queue(
                    vec![acp::ContentBlock::Text(acp::TextContent::new(
                        "first queued".to_string(),
                    ))],
                    vec![],
                    window,
                    cx,
                )
                .expect("first queue admission");
            thread
                .add_to_queue(
                    vec![acp::ContentBlock::Text(acp::TextContent::new(
                        "second queued".to_string(),
                    ))],
                    vec![],
                    window,
                    cx,
                )
                .expect("second queue admission");
        });
        cx.run_until_parked();

        let editor = message_editor(&conversation_view, cx);
        cx.focus(&editor);

        editor.update_in(cx, |_editor, window, cx| {
            window.dispatch_action(Box::new(omega_actions::editor::MoveUp), cx);
        });
        cx.run_until_parked();

        let queue_len = active_thread(&conversation_view, cx)
            .read_with(cx, |thread, _cx| thread.message_queue.len());
        assert_eq!(
            queue_len, 1,
            "Up arrow should pull the last queued message out of the queue"
        );
        let text = editor.update(cx, |editor, cx| editor.text(cx));
        assert_eq!(
            text, "second queued",
            "Main editor should contain the last queued message"
        );

        // With a non-empty editor, another MoveUp must not consume the queue.
        editor.update_in(cx, |_editor, window, cx| {
            window.dispatch_action(Box::new(omega_actions::editor::MoveUp), cx);
        });
        cx.run_until_parked();

        let queue_len = active_thread(&conversation_view, cx)
            .read_with(cx, |thread, _cx| thread.message_queue.len());
        assert_eq!(queue_len, 1, "Queue should be untouched");
        let text = editor.update(cx, |editor, cx| editor.text(cx));
        assert_eq!(text, "second queued");
    }

    #[gpui::test]
    async fn test_paste_text_into_queued_message_promotes_to_main_editor(cx: &mut TestAppContext) {
        init_test(cx);

        let (conversation_view, cx) =
            paste_into_queued_message(cx, ClipboardItem::new_string("PASTED".to_string())).await;

        let queue_len = active_thread(&conversation_view, cx)
            .read_with(cx, |thread, _cx| thread.message_queue.len());
        assert_eq!(queue_len, 0);

        let text = message_editor(&conversation_view, cx).update(cx, |editor, cx| editor.text(cx));
        assert_eq!(text, "queued PASTEDmessage");
    }

    #[gpui::test]
    async fn test_paste_image_into_queued_message_promotes_to_main_editor(cx: &mut TestAppContext) {
        init_test(cx);

        use base64::Engine as _;
        use std::io::Write as _;
        let png_bytes = base64::prelude::BASE64_STANDARD
            .decode("iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mNkYPhfDwAChwGA60e6kgAAAABJRU5ErkJggg==")
            .unwrap();
        let mut image_file = tempfile::Builder::new().suffix(".png").tempfile().unwrap();
        image_file.write_all(&png_bytes).unwrap();

        let (conversation_view, cx) = paste_into_queued_message(
            cx,
            ClipboardItem {
                entries: vec![gpui::ClipboardEntry::ExternalPaths(gpui::ExternalPaths(
                    vec![image_file.path().to_path_buf()].into(),
                ))],
            },
        )
        .await;

        let queue_len = active_thread(&conversation_view, cx)
            .read_with(cx, |thread, _cx| thread.message_queue.len());
        assert_eq!(queue_len, 0);

        let text = message_editor(&conversation_view, cx).update(cx, |editor, cx| editor.text(cx));
        let image_name = image_file.path().file_name().unwrap().to_string_lossy();
        let expected_uri = acp_thread::MentionUri::PastedImage {
            name: image_name.to_string(),
        }
        .to_uri()
        .to_string();
        assert_eq!(
            text,
            format!("queued [@{image_name}]({expected_uri}) message"),
        );
    }

    async fn paste_into_queued_message(
        cx: &mut TestAppContext,
        clipboard: ClipboardItem,
    ) -> (Entity<ConversationView>, &mut VisualTestContext) {
        let (conversation_view, cx) =
            setup_conversation_view(StubAgentServer::default_response(), cx).await;
        add_to_workspace(conversation_view.clone(), cx);

        active_thread(&conversation_view, cx).update_in(cx, |thread, window, cx| {
            thread
                .session_capabilities
                .write()
                .set_prompt_capabilities(acp::PromptCapabilities::new().image(true));
            thread
                .add_to_queue(
                    vec![acp::ContentBlock::Text(acp::TextContent::new(
                        "queued message".to_string(),
                    ))],
                    vec![],
                    window,
                    cx,
                )
                .expect("queue admission");
        });
        conversation_view.update(cx, |_, cx| cx.notify());
        cx.run_until_parked();

        let queued_editor = active_thread(&conversation_view, cx).read_with(cx, |thread, _cx| {
            thread
                .message_queue
                .first()
                .map(|entry| entry.editor.clone())
                .expect("queued message editor not created")
        });

        cx.write_to_clipboard(clipboard);

        queued_editor.update_in(cx, |message_editor, window, cx| {
            message_editor.editor().update(cx, |editor, cx| {
                editor.change_selections(SelectionEffects::no_scroll(), window, cx, |selections| {
                    selections.select_ranges([MultiBufferOffset(7)..MultiBufferOffset(7)]);
                });
            });
            message_editor.paste(&Paste, window, cx);
        });
        cx.run_until_parked();

        (conversation_view, cx)
    }

    #[gpui::test]
    async fn test_close_all_sessions_skips_when_unsupported(cx: &mut TestAppContext) {
        init_test(cx);

        let fs = FakeFs::new(cx.executor());
        let project = Project::test(fs, [], cx).await;
        let (multi_workspace, cx) =
            cx.add_window_view(|window, cx| MultiWorkspace::test_new(project.clone(), window, cx));
        let workspace = multi_workspace.read_with(cx, |mw, _| mw.workspace().clone());

        let thread_store = cx.update(|_window, cx| cx.new(|cx| ThreadStore::new(cx)));
        let connection_store =
            cx.update(|_window, cx| cx.new(|cx| AgentConnectionStore::new(project.clone(), cx)));

        // StubAgentConnection defaults to supports_close_session() -> false
        let conversation_view = cx.update(|window, cx| {
            cx.new(|cx| {
                ConversationView::new(
                    Rc::new(StubAgentServer::default_response()),
                    connection_store,
                    Agent::Custom { id: "Test".into() },
                    None,
                    None,
                    None,
                    None,
                    None,
                    workspace.downgrade(),
                    project,
                    Some(thread_store),
                    AgentThreadSource::AgentPanel,
                    window,
                    cx,
                )
            })
        });

        cx.run_until_parked();

        conversation_view.read_with(cx, |view, _cx| {
            let connected = view.as_connected().expect("Should be connected");
            assert!(
                !connected.threads.is_empty(),
                "There should be at least one thread"
            );
            assert!(
                !connected.connection.supports_close_session(),
                "StubAgentConnection should not support close"
            );
        });

        conversation_view
            .update(cx, |view, cx| {
                view.as_connected()
                    .expect("Should be connected")
                    .close_all_sessions(cx)
            })
            .await;
    }

    #[gpui::test]
    async fn test_close_all_sessions_calls_close_when_supported(cx: &mut TestAppContext) {
        init_test(cx);

        let (conversation_view, cx) =
            setup_conversation_view(StubAgentServer::new(CloseCapableConnection::new()), cx).await;

        cx.run_until_parked();

        let close_capable = conversation_view.read_with(cx, |view, _cx| {
            let connected = view.as_connected().expect("Should be connected");
            assert!(
                !connected.threads.is_empty(),
                "There should be at least one thread"
            );
            assert!(
                connected.connection.supports_close_session(),
                "CloseCapableConnection should support close"
            );
            connected
                .connection
                .clone()
                .into_any()
                .downcast::<CloseCapableConnection>()
                .expect("Should be CloseCapableConnection")
        });

        conversation_view
            .update(cx, |view, cx| {
                view.as_connected()
                    .expect("Should be connected")
                    .close_all_sessions(cx)
            })
            .await;

        let closed_count = close_capable.closed_sessions.lock().len();
        assert!(
            closed_count > 0,
            "close_session should have been called for each thread"
        );
    }

    #[gpui::test]
    async fn test_close_session_returns_error_when_unsupported(cx: &mut TestAppContext) {
        init_test(cx);

        let (conversation_view, cx) =
            setup_conversation_view(StubAgentServer::default_response(), cx).await;

        cx.run_until_parked();

        let result = conversation_view
            .update(cx, |view, cx| {
                let connected = view.as_connected().expect("Should be connected");
                assert!(
                    !connected.connection.supports_close_session(),
                    "StubAgentConnection should not support close"
                );
                let thread_view = connected
                    .threads
                    .values()
                    .next()
                    .expect("Should have at least one thread");
                let session_id = thread_view.read(cx).thread.read(cx).session_id().clone();
                connected.connection.clone().close_session(&session_id, cx)
            })
            .await;

        assert!(
            result.is_err(),
            "close_session should return an error when close is not supported"
        );
        assert!(
            result.unwrap_err().to_string().contains("not supported"),
            "Error message should indicate that closing is not supported"
        );
    }

    #[derive(Clone)]
    struct CloseCapableConnection {
        closed_sessions: Arc<Mutex<Vec<acp::SessionId>>>,
    }

    impl CloseCapableConnection {
        fn new() -> Self {
            Self {
                closed_sessions: Arc::new(Mutex::new(Vec::new())),
            }
        }
    }

    impl AgentConnection for CloseCapableConnection {
        fn agent_id(&self) -> AgentId {
            AgentId::new("close-capable")
        }

        fn telemetry_id(&self) -> SharedString {
            "close-capable".into()
        }

        fn new_session(
            self: Rc<Self>,
            project: Entity<Project>,
            work_dirs: PathList,
            cx: &mut gpui::App,
        ) -> Task<gpui::Result<Entity<AcpThread>>> {
            let action_log = cx.new(|_| ActionLog::new(project.clone()));
            let thread = cx.new(|cx| {
                AcpThread::new(
                    None,
                    Some("CloseCapableConnection".into()),
                    Some(work_dirs),
                    self,
                    project,
                    action_log,
                    acp::SessionId::new("close-capable-session"),
                    watch::Receiver::constant(
                        acp::PromptCapabilities::new()
                            .image(true)
                            .audio(true)
                            .embedded_context(true),
                    ),
                    cx,
                )
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
            self.closed_sessions.lock().push(session_id.clone());
            Task::ready(Ok(()))
        }

        fn auth_methods(&self) -> &[acp::AuthMethod] {
            &[]
        }

        fn authenticate(
            &self,
            _method_id: acp::AuthMethodId,
            _cx: &mut App,
        ) -> Task<gpui::Result<()>> {
            Task::ready(Ok(()))
        }

        fn prompt(
            &self,
            _params: acp::PromptRequest,
            _cx: &mut App,
        ) -> Task<gpui::Result<acp::PromptResponse>> {
            Task::ready(Ok(acp::PromptResponse::new(acp::StopReason::EndTurn)))
        }

        fn cancel(&self, _session_id: &acp::SessionId, _cx: &mut App) {}

        fn into_any(self: Rc<Self>) -> Rc<dyn Any> {
            self
        }
    }
}
