use crate::{AgentMessage, AgentMessageContent, Message, UserMessage, UserMessageContent};
use acp_thread::ClientUserMessageId;
use agent_client_protocol::schema::v1 as acp;
use agent_settings::AgentProfileId;
use anyhow::{Context as _, Result};
use chrono::{DateTime, Utc};
use collections::{HashMap, IndexMap};
use futures::{FutureExt, future::Shared};
use gpui::{BackgroundExecutor, Global, Task};
use indoc::indoc;
use language_model::Speed;
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use sqlez::{
    bindable::{Bind, Column},
    connection::Connection,
    statement::Statement,
};
use std::{io::ErrorKind, path::PathBuf, sync::Arc};
use ui::{App, SharedString};
use util::path_list::PathList;
use zed_env_vars::ZED_STATELESS;

pub type DbMessage = crate::Message;
pub type DbSummary = crate::legacy_thread::DetailedSummaryState;
pub type DbLanguageModel = crate::legacy_thread::SerializedLanguageModel;

/// A stable position in a thread's append-only event log.
pub type ThreadEventSequence = u64;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ThreadForkOrigin {
    pub session_id: acp::SessionId,
    pub event_sequence: ThreadEventSequence,
}

/// The model-facing prefix that must remain byte-for-byte stable after reload.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PromptCacheLayout {
    pub system_prompt: SharedString,
    pub tool_order: Vec<SharedString>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolResultReplacementReference {
    pub tool_use_id: SharedString,
    pub content_index: usize,
    pub marker: SharedString,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum ThreadLogEventKind {
    MessageAppended {
        message: Arc<Message>,
    },
    MessageInserted {
        index: usize,
        message: Arc<Message>,
    },
    MessagesTruncated {
        len: usize,
    },
    PromptCacheLayout {
        layout: PromptCacheLayout,
    },
    ToolResultReplacement {
        reference: ToolResultReplacementReference,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ThreadLogEvent {
    pub sequence: ThreadEventSequence,
    pub parent_sequence: Option<ThreadEventSequence>,
    pub kind: ThreadLogEventKind,
}

/// Structured thread authority. Events are never removed or rewritten.
///
/// `active_sequence` selects one leaf in the event graph. Loading an earlier
/// sequence or forking from it changes the active leaf without deleting the
/// abandoned branch.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ThreadEventLog {
    #[serde(default)]
    pub events: Vec<ThreadLogEvent>,
    #[serde(default)]
    pub active_sequence: Option<ThreadEventSequence>,
}

impl ThreadEventLog {
    pub fn from_messages(messages: &[Arc<Message>]) -> Self {
        let mut log = Self::default();
        for message in messages {
            log.append_message(message.clone());
        }
        log
    }

    pub fn append_message(&mut self, message: Arc<Message>) -> ThreadEventSequence {
        let sequence = self.append(ThreadLogEventKind::MessageAppended {
            message: message.clone(),
        });
        self.append_replacement_references(&message);
        sequence
    }

    pub fn insert_message(&mut self, index: usize, message: Arc<Message>) -> ThreadEventSequence {
        let sequence = self.append(ThreadLogEventKind::MessageInserted {
            index,
            message: message.clone(),
        });
        self.append_replacement_references(&message);
        sequence
    }

    pub fn truncate_messages(&mut self, len: usize) -> ThreadEventSequence {
        self.append(ThreadLogEventKind::MessagesTruncated { len })
    }

    pub fn append_prompt_cache_layout(&mut self, layout: PromptCacheLayout) -> ThreadEventSequence {
        self.append(ThreadLogEventKind::PromptCacheLayout { layout })
    }

    pub fn messages(&self) -> Result<Vec<Arc<Message>>> {
        self.messages_at(self.active_sequence)
    }

    pub fn messages_at(&self, sequence: Option<ThreadEventSequence>) -> Result<Vec<Arc<Message>>> {
        let mut messages = Vec::new();
        let mut last_message_position = None;
        for event in self.events_on_path(sequence)? {
            match &event.kind {
                ThreadLogEventKind::MessageAppended { message } => {
                    messages.push(message.clone());
                    last_message_position = messages.len().checked_sub(1);
                }
                ThreadLogEventKind::MessageInserted { index, message } => {
                    let index = (*index).min(messages.len());
                    messages.insert(index, message.clone());
                    last_message_position = Some(index);
                }
                ThreadLogEventKind::MessagesTruncated { len } => {
                    messages.truncate(*len);
                    last_message_position = None;
                }
                ThreadLogEventKind::PromptCacheLayout { .. } => {}
                ThreadLogEventKind::ToolResultReplacement { reference } => {
                    let position = last_message_position
                        .context("tool-result replacement does not follow a message event")?;
                    Self::restore_replacement_reference(&mut messages[position], reference)?;
                }
            }
        }
        Ok(messages)
    }

    pub fn prompt_cache_layout(
        &self,
        sequence: Option<ThreadEventSequence>,
    ) -> Result<Option<PromptCacheLayout>> {
        Ok(self
            .events_on_path(sequence)?
            .into_iter()
            .rev()
            .find_map(|event| match &event.kind {
                ThreadLogEventKind::PromptCacheLayout { layout } => Some(layout.clone()),
                _ => None,
            }))
    }

    pub fn replacement_references(
        &self,
        sequence: Option<ThreadEventSequence>,
    ) -> Result<Vec<ToolResultReplacementReference>> {
        Ok(self
            .events_on_path(sequence)?
            .into_iter()
            .filter_map(|event| match &event.kind {
                ThreadLogEventKind::ToolResultReplacement { reference } => Some(reference.clone()),
                _ => None,
            })
            .collect())
    }

    pub fn select(&mut self, sequence: ThreadEventSequence) -> Result<()> {
        self.event(sequence)?;
        self.active_sequence = Some(sequence);
        Ok(())
    }

    pub fn fork_at(&self, sequence: ThreadEventSequence) -> Result<Self> {
        let events = self.events_on_path(Some(sequence))?;
        Ok(Self {
            events: events.into_iter().cloned().collect(),
            active_sequence: Some(sequence),
        })
    }

    fn append(&mut self, kind: ThreadLogEventKind) -> ThreadEventSequence {
        let sequence = self
            .events
            .last()
            .map_or(0, |event| event.sequence.saturating_add(1));
        self.events.push(ThreadLogEvent {
            sequence,
            parent_sequence: self.active_sequence,
            kind,
        });
        self.active_sequence = Some(sequence);
        sequence
    }

    fn append_replacement_references(&mut self, message: &Message) {
        let Message::Agent(message) = message else {
            return;
        };
        for result in message.tool_results.values() {
            for (content_index, content) in result.content.iter().enumerate() {
                let language_model::LanguageModelToolResultContent::Text(text) = content else {
                    continue;
                };
                let (_, marker) = acp_thread::split_truncation_marker(text);
                if marker.is_empty() {
                    continue;
                }
                self.append(ThreadLogEventKind::ToolResultReplacement {
                    reference: ToolResultReplacementReference {
                        tool_use_id: result.tool_use_id.to_string().into(),
                        content_index,
                        marker: marker.into(),
                    },
                });
            }
        }
    }

    fn restore_replacement_reference(
        message: &mut Arc<Message>,
        reference: &ToolResultReplacementReference,
    ) -> Result<()> {
        let Message::Agent(agent_message) = message.as_ref() else {
            anyhow::bail!("tool-result replacement follows a non-agent message");
        };
        let mut restored_message = agent_message.clone();
        let result = restored_message
            .tool_results
            .values_mut()
            .find(|result| result.tool_use_id.to_string() == reference.tool_use_id)
            .with_context(|| {
                format!(
                    "tool-result replacement refers to missing tool use {}",
                    reference.tool_use_id
                )
            })?;
        let content = result
            .content
            .get_mut(reference.content_index)
            .with_context(|| {
                format!(
                    "tool-result replacement content index {} does not exist for tool use {}",
                    reference.content_index, reference.tool_use_id
                )
            })?;
        let language_model::LanguageModelToolResultContent::Text(text) = content else {
            anyhow::bail!(
                "tool-result replacement content {} for tool use {} is not text",
                reference.content_index,
                reference.tool_use_id
            );
        };
        let (body, current_marker) = acp_thread::split_truncation_marker(text);
        if current_marker != reference.marker.as_ref() {
            *text = format!("{body}{}", reference.marker).into();
        }
        *message = Arc::new(Message::Agent(restored_message));
        Ok(())
    }

    fn event(&self, sequence: ThreadEventSequence) -> Result<&ThreadLogEvent> {
        self.events
            .iter()
            .find(|event| event.sequence == sequence)
            .ok_or_else(|| anyhow::anyhow!("thread event sequence {sequence} does not exist"))
    }

    fn events_on_path(
        &self,
        sequence: Option<ThreadEventSequence>,
    ) -> Result<Vec<&ThreadLogEvent>> {
        let Some(mut sequence) = sequence else {
            return Ok(Vec::new());
        };
        let mut events = Vec::new();
        let mut remaining = self.events.len();
        loop {
            let event = self.event(sequence)?;
            events.push(event);
            let Some(parent) = event.parent_sequence else {
                break;
            };
            sequence = parent;
            remaining = remaining.saturating_sub(1);
            anyhow::ensure!(remaining > 0, "thread event log contains a parent cycle");
        }
        events.reverse();
        Ok(events)
    }
}

#[derive(Debug, Clone)]
pub struct DbThreadMetadata {
    pub id: acp::SessionId,
    pub parent_session_id: Option<acp::SessionId>,
    pub title: SharedString,
    pub updated_at: DateTime<Utc>,
    pub created_at: Option<DateTime<Utc>>,
    /// The workspace folder paths this thread was created against, sorted
    /// lexicographically. Used for grouping threads by project in the sidebar.
    pub folder_paths: PathList,
}

impl From<&DbThreadMetadata> for acp_thread::AgentSessionInfo {
    fn from(meta: &DbThreadMetadata) -> Self {
        Self {
            session_id: meta.id.clone(),
            work_dirs: Some(meta.folder_paths.clone()),
            title: Some(meta.title.clone()),
            updated_at: Some(meta.updated_at),
            created_at: meta.created_at,
            meta: None,
        }
    }
}

#[derive(Debug, Serialize, Deserialize)]
pub struct DbThread {
    pub title: SharedString,
    pub messages: Vec<Arc<DbMessage>>,
    pub updated_at: DateTime<Utc>,
    #[serde(default)]
    pub detailed_summary: Option<SharedString>,
    #[serde(default)]
    pub initial_project_snapshot: Option<Arc<crate::ProjectSnapshot>>,
    #[serde(default)]
    pub cumulative_token_usage: language_model::TokenUsage,
    #[serde(default)]
    pub request_token_usage: HashMap<acp_thread::ClientUserMessageId, language_model::TokenUsage>,
    #[serde(default)]
    pub model: Option<DbLanguageModel>,
    #[serde(default)]
    pub profile: Option<AgentProfileId>,
    #[serde(default)]
    pub subagent_context: Option<crate::SubagentContext>,
    #[serde(default)]
    pub speed: Option<Speed>,
    #[serde(default)]
    pub thinking_enabled: bool,
    #[serde(default)]
    pub thinking_effort: Option<String>,
    #[serde(default)]
    pub draft_prompt: Option<Vec<acp::ContentBlock>>,
    #[serde(default)]
    pub ui_scroll_position: Option<SerializedScrollPosition>,
    #[serde(default)]
    pub sandboxed_terminal_temp_dir: Option<PathBuf>,
    /// Sandbox escalations the user approved "for the rest of this thread".
    /// Persisted so reopening a thread keeps its grants. See
    /// [`crate::sandboxing::ThreadSandboxGrants`].
    #[serde(default)]
    pub sandbox_grants: DbSandboxGrants,
    #[serde(default)]
    pub thread_log: ThreadEventLog,
    #[serde(default)]
    pub fork_origin: Option<ThreadForkOrigin>,
}

/// Serialized form of the sandbox permissions the user granted "for the rest of
/// this thread" (the "Allow for this thread" prompt option). Stored inside the
/// thread blob; round-trips with [`crate::sandboxing::ThreadSandboxGrants`].
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct DbSandboxGrants {
    /// Paths granted write access, each paired with the canonical
    /// (symlink-resolved) target established when the grant was approved; each
    /// covers its whole subtree. Legacy rows stored a bare path string per
    /// entry, which still deserializes (as a grant with no resolved canonical)
    /// via [`settings::GrantedWritePath`]'s string-or-object format.
    #[serde(default)]
    pub write_paths: Vec<settings::GrantedWritePath>,
    /// Host patterns granted network access, in canonical string form (e.g.
    /// `github.com`, `*.npmjs.org`). Parsed back into patterns on load.
    #[serde(default)]
    pub network_hosts: Vec<String>,
    /// Whether arbitrary-host network access was granted.
    #[serde(default)]
    pub network_any_host: bool,
    /// Whether unrestricted filesystem writes (the broad escape hatch) were
    /// granted.
    #[serde(default)]
    pub allow_fs_write_all: bool,

    /// Whether the model-requested fully-unsandboxed escape was granted.
    #[serde(default)]
    pub unsandboxed: bool,
    /// Whether running commands unsandboxed was allowed because the OS sandbox
    /// could not be created (the fallback prompt's "for this thread" option).
    #[serde(default)]
    pub sandbox_fallback: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct SerializedScrollPosition {
    pub item_ix: usize,
    pub offset_in_item: f32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SharedThread {
    pub title: SharedString,
    pub messages: Vec<Arc<DbMessage>>,
    pub updated_at: DateTime<Utc>,
    #[serde(default)]
    pub model: Option<DbLanguageModel>,
    pub version: String,
}

impl SharedThread {
    pub const VERSION: &'static str = "1.0.0";

    pub fn from_db_thread(thread: &DbThread) -> Self {
        Self {
            title: thread.title.clone(),
            messages: thread.messages.clone(),
            updated_at: thread.updated_at,
            model: thread.model.clone(),
            version: Self::VERSION.to_string(),
        }
    }

    pub fn to_db_thread(self) -> DbThread {
        let thread_log = ThreadEventLog::from_messages(&self.messages);
        DbThread {
            title: format!("🔗 {}", self.title).into(),
            messages: self.messages,
            updated_at: self.updated_at,
            detailed_summary: None,
            initial_project_snapshot: None,
            cumulative_token_usage: Default::default(),
            request_token_usage: Default::default(),
            model: self.model,
            profile: None,
            subagent_context: None,
            speed: None,
            thinking_enabled: false,
            thinking_effort: None,
            draft_prompt: None,
            ui_scroll_position: None,
            sandboxed_terminal_temp_dir: None,
            sandbox_grants: DbSandboxGrants::default(),
            thread_log,
            fork_origin: None,
        }
    }

    pub fn to_bytes(&self) -> Result<Vec<u8>> {
        const COMPRESSION_LEVEL: i32 = 3;
        let json = serde_json::to_vec(self)?;
        let compressed = zstd::encode_all(json.as_slice(), COMPRESSION_LEVEL)?;
        Ok(compressed)
    }

    pub fn from_bytes(data: &[u8]) -> Result<Self> {
        let decompressed = zstd::decode_all(data)?;
        Ok(serde_json::from_slice(&decompressed)?)
    }
}

impl DbThread {
    pub const VERSION: &'static str = "0.4.0";

    pub fn prepare_for_resume(
        &mut self,
        event_sequence: Option<ThreadEventSequence>,
    ) -> Result<()> {
        if self.thread_log.events.is_empty() && !self.messages.is_empty() {
            self.thread_log = ThreadEventLog::from_messages(&self.messages);
        }
        if let Some(sequence) = event_sequence {
            self.thread_log.select(sequence)?;
        }
        self.messages = self.thread_log.messages()?;
        Ok(())
    }

    pub fn prepare_for_resume_at_message_index(&mut self, message_index: usize) -> Result<()> {
        self.prepare_for_resume(None)?;
        anyhow::ensure!(
            message_index < self.messages.len(),
            "thread message index {message_index} does not exist; thread has {} messages",
            self.messages.len()
        );
        self.thread_log
            .truncate_messages(message_index.saturating_add(1));
        self.messages = self.thread_log.messages()?;
        Ok(())
    }

    pub fn fork_at(
        &self,
        source_session_id: acp::SessionId,
        event_sequence: ThreadEventSequence,
    ) -> Result<Self> {
        let mut fork = self.clone_for_fork();
        fork.thread_log = self.thread_log.fork_at(event_sequence)?;
        fork.messages = fork.thread_log.messages()?;
        fork.fork_origin = Some(ThreadForkOrigin {
            session_id: source_session_id,
            event_sequence,
        });
        fork.updated_at = Utc::now();
        Ok(fork)
    }

    pub fn fork_at_message_index(
        &self,
        source_session_id: acp::SessionId,
        message_index: usize,
    ) -> Result<Self> {
        let source_messages = self.thread_log.messages()?;
        anyhow::ensure!(
            message_index < source_messages.len(),
            "thread message index {message_index} does not exist; thread has {} messages",
            source_messages.len()
        );
        let source_sequence = self
            .thread_log
            .active_sequence
            .context("cannot fork a thread with no active event")?;
        let prompt_cache_layout = self.thread_log.prompt_cache_layout(Some(source_sequence))?;
        let mut fork = self.clone_for_fork();
        fork.messages = source_messages[..=message_index].to_vec();
        fork.thread_log = ThreadEventLog::from_messages(&fork.messages);
        if let Some(layout) = prompt_cache_layout {
            fork.thread_log.append_prompt_cache_layout(layout);
        }
        fork.fork_origin = Some(ThreadForkOrigin {
            session_id: source_session_id,
            event_sequence: source_sequence,
        });
        fork.updated_at = Utc::now();
        Ok(fork)
    }

    fn clone_for_fork(&self) -> Self {
        Self {
            title: self.title.clone(),
            messages: self.messages.clone(),
            updated_at: self.updated_at,
            detailed_summary: self.detailed_summary.clone(),
            initial_project_snapshot: self.initial_project_snapshot.clone(),
            cumulative_token_usage: self.cumulative_token_usage,
            request_token_usage: self.request_token_usage.clone(),
            model: self.model.clone(),
            profile: self.profile.clone(),
            subagent_context: None,
            speed: self.speed,
            thinking_enabled: self.thinking_enabled,
            thinking_effort: self.thinking_effort.clone(),
            draft_prompt: None,
            ui_scroll_position: None,
            sandboxed_terminal_temp_dir: None,
            sandbox_grants: self.sandbox_grants.clone(),
            thread_log: self.thread_log.clone(),
            fork_origin: self.fork_origin.clone(),
        }
    }

    pub fn to_markdown(&self) -> String {
        crate::messages_to_markdown(&self.messages)
    }

    pub fn from_json(json: &[u8]) -> Result<Self> {
        let saved_thread_json = serde_json::from_slice::<serde_json::Value>(json)?;
        match saved_thread_json.get("version") {
            Some(serde_json::Value::String(version)) => match version.as_str() {
                Self::VERSION | "0.3.0" => Ok(serde_json::from_value(saved_thread_json)?),
                _ => Self::upgrade_from_agent_1(crate::legacy_thread::SerializedThread::from_json(
                    json,
                )?),
            },
            _ => {
                Self::upgrade_from_agent_1(crate::legacy_thread::SerializedThread::from_json(json)?)
            }
        }
    }

    fn upgrade_from_agent_1(thread: crate::legacy_thread::SerializedThread) -> Result<Self> {
        let mut messages = Vec::new();
        let mut request_token_usage = HashMap::default();

        let mut last_user_message_id = None;
        for (ix, msg) in thread.messages.into_iter().enumerate() {
            let message = match msg.role {
                language_model::Role::User => {
                    let mut content = Vec::new();

                    // Convert segments to content
                    for segment in msg.segments {
                        match segment {
                            crate::legacy_thread::SerializedMessageSegment::Text { text } => {
                                content.push(UserMessageContent::Text(text));
                            }
                            crate::legacy_thread::SerializedMessageSegment::Thinking {
                                text,
                                ..
                            } => {
                                // User messages don't have thinking segments, but handle gracefully
                                content.push(UserMessageContent::Text(text));
                            }
                            crate::legacy_thread::SerializedMessageSegment::RedactedThinking {
                                ..
                            } => {
                                // User messages don't have redacted thinking, skip.
                            }
                        }
                    }

                    // If no content was added, add context as text if available
                    if content.is_empty() && !msg.context.is_empty() {
                        content.push(UserMessageContent::Text(msg.context));
                    }

                    let id = ClientUserMessageId::new();
                    last_user_message_id = Some(id.clone());

                    crate::Message::User(UserMessage {
                        // MessageId from old format can't be meaningfully converted, so generate a new one
                        id,
                        content: Arc::from(content),
                    })
                }
                language_model::Role::Assistant => {
                    let mut content = Vec::new();

                    // Convert segments to content
                    for segment in msg.segments {
                        match segment {
                            crate::legacy_thread::SerializedMessageSegment::Text { text } => {
                                content.push(AgentMessageContent::Text(text));
                            }
                            crate::legacy_thread::SerializedMessageSegment::Thinking {
                                text,
                                signature,
                            } => {
                                content.push(AgentMessageContent::Thinking { text, signature });
                            }
                            crate::legacy_thread::SerializedMessageSegment::RedactedThinking {
                                data,
                            } => {
                                content.push(AgentMessageContent::RedactedThinking(data));
                            }
                        }
                    }

                    // Convert tool uses
                    let mut tool_names_by_id = HashMap::default();
                    for tool_use in msg.tool_uses {
                        tool_names_by_id.insert(tool_use.id.clone(), tool_use.name.clone());
                        content.push(AgentMessageContent::ToolUse(
                            language_model::LanguageModelToolUse {
                                id: tool_use.id,
                                name: tool_use.name.into(),
                                raw_input: serde_json::to_string(&tool_use.input)
                                    .unwrap_or_default(),
                                input: language_model::LanguageModelToolUseInput::Json(
                                    tool_use.input,
                                ),
                                is_input_complete: true,
                                thought_signature: None,
                            },
                        ));
                    }

                    // Convert tool results
                    let mut tool_results = IndexMap::default();
                    for tool_result in msg.tool_results {
                        let name = tool_names_by_id
                            .remove(&tool_result.tool_use_id)
                            .unwrap_or_else(|| SharedString::from("unknown"));
                        tool_results.insert(
                            tool_result.tool_use_id.clone(),
                            language_model::LanguageModelToolResult {
                                tool_use_id: tool_result.tool_use_id,
                                tool_name: name.into(),
                                is_error: tool_result.is_error,
                                content: vec![tool_result.content],
                                output: tool_result.output,
                            },
                        );
                    }

                    if let Some(last_user_message_id) = &last_user_message_id
                        && let Some(token_usage) = thread.request_token_usage.get(ix).copied()
                    {
                        request_token_usage.insert(last_user_message_id.clone(), token_usage);
                    }

                    crate::Message::Agent(AgentMessage {
                        content,
                        tool_results,
                        reasoning_details: None,
                    })
                }
                language_model::Role::System => {
                    // Skip system messages as they're not supported in the new format
                    continue;
                }
            };

            messages.push(Arc::new(message));
        }

        let thread_log = ThreadEventLog::from_messages(&messages);
        Ok(Self {
            title: thread.summary,
            messages,
            updated_at: thread.updated_at,
            detailed_summary: match thread.detailed_summary_state {
                crate::legacy_thread::DetailedSummaryState::NotGenerated
                | crate::legacy_thread::DetailedSummaryState::Generating => None,
                crate::legacy_thread::DetailedSummaryState::Generated { text, .. } => Some(text),
            },
            initial_project_snapshot: thread.initial_project_snapshot,
            cumulative_token_usage: thread.cumulative_token_usage,
            request_token_usage,
            model: thread.model,
            profile: thread.profile,
            subagent_context: None,
            speed: None,
            thinking_enabled: false,
            thinking_effort: None,
            draft_prompt: None,
            ui_scroll_position: None,
            sandboxed_terminal_temp_dir: None,
            sandbox_grants: DbSandboxGrants::default(),
            thread_log,
            fork_origin: None,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum DataType {
    #[serde(rename = "json")]
    Json,
    #[serde(rename = "zstd")]
    Zstd,
}

impl Bind for DataType {
    fn bind(&self, statement: &Statement, start_index: i32) -> Result<i32> {
        let value = match self {
            DataType::Json => "json",
            DataType::Zstd => "zstd",
        };
        value.bind(statement, start_index)
    }
}

impl Column for DataType {
    fn column(statement: &mut Statement, start_index: i32) -> Result<(Self, i32)> {
        let (value, next_index) = String::column(statement, start_index)?;
        let data_type = match value.as_str() {
            "json" => DataType::Json,
            "zstd" => DataType::Zstd,
            _ => anyhow::bail!("Unknown data type: {}", value),
        };
        Ok((data_type, next_index))
    }
}

pub(crate) struct ThreadsDatabase {
    executor: BackgroundExecutor,
    connection: Arc<Mutex<Connection>>,
}

struct GlobalThreadsDatabase(Shared<Task<Result<Arc<ThreadsDatabase>, Arc<anyhow::Error>>>>);

impl Global for GlobalThreadsDatabase {}

impl ThreadsDatabase {
    pub fn connect(cx: &mut App) -> Shared<Task<Result<Arc<ThreadsDatabase>, Arc<anyhow::Error>>>> {
        if cx.has_global::<GlobalThreadsDatabase>() {
            return cx.global::<GlobalThreadsDatabase>().0.clone();
        }
        let executor = cx.background_executor().clone();
        let task = executor
            .spawn({
                let executor = executor.clone();
                async move {
                    match ThreadsDatabase::new(executor) {
                        Ok(db) => Ok(Arc::new(db)),
                        Err(err) => Err(Arc::new(err)),
                    }
                }
            })
            .shared();

        cx.set_global(GlobalThreadsDatabase(task.clone()));
        task
    }

    pub fn new(executor: BackgroundExecutor) -> Result<Self> {
        let connection = if *ZED_STATELESS {
            Connection::open_memory(Some("THREAD_FALLBACK_DB"))
        } else if cfg!(any(feature = "test-support", test)) {
            // rust stores the name of the test on the current thread.
            // We use this to automatically create a database that will
            // be shared within the test (for the test_retrieve_old_thread)
            // but not with concurrent tests.
            let thread = std::thread::current();
            let test_name = thread.name();
            Connection::open_memory(Some(&format!(
                "THREAD_FALLBACK_{}",
                test_name.unwrap_or_default()
            )))
        } else {
            let threads_dir = paths::data_dir().join("threads");
            std::fs::create_dir_all(&threads_dir)?;
            let sqlite_path = threads_dir.join("threads.db");
            Connection::open_file(&sqlite_path.to_string_lossy())
        };

        connection.exec(indoc! {"
            CREATE TABLE IF NOT EXISTS threads (
                id TEXT PRIMARY KEY,
                summary TEXT NOT NULL,
                updated_at TEXT NOT NULL,
                data_type TEXT NOT NULL,
                data BLOB NOT NULL
            )
        "})?()
        .map_err(|e| e.context("Failed to create threads table"))?;

        connection.exec(indoc! {"
            CREATE TABLE IF NOT EXISTS thread_events (
                thread_id TEXT NOT NULL,
                sequence INTEGER NOT NULL,
                parent_sequence INTEGER,
                event_json TEXT NOT NULL,
                PRIMARY KEY (thread_id, sequence)
            )
        "})?()
        .map_err(|e| e.context("Failed to create append-only thread event table"))?;

        if let Ok(mut s) = connection.exec(indoc! {"
            ALTER TABLE threads ADD COLUMN parent_id TEXT
        "})
        {
            s().ok();
        }

        if let Ok(mut s) = connection.exec(indoc! {"
            ALTER TABLE threads ADD COLUMN folder_paths TEXT;
            ALTER TABLE threads ADD COLUMN folder_paths_order TEXT;
        "})
        {
            s().ok();
        }

        if let Ok(mut s) = connection.exec(indoc! {"
            ALTER TABLE threads ADD COLUMN created_at TEXT;
        "})
        {
            if s().is_ok() {
                connection.exec(indoc! {"
                    UPDATE threads SET created_at = updated_at WHERE created_at IS NULL
                "})?()?;
            }
        }

        let db = Self {
            executor,
            connection: Arc::new(Mutex::new(connection)),
        };

        Ok(db)
    }

    fn save_thread_sync(
        connection: &Arc<Mutex<Connection>>,
        id: acp::SessionId,
        thread: DbThread,
        folder_paths: &PathList,
    ) -> Result<()> {
        const COMPRESSION_LEVEL: i32 = 3;

        #[derive(Serialize)]
        struct SerializedThread {
            #[serde(flatten)]
            thread: DbThread,
            version: &'static str,
        }

        let title = thread.title.to_string();
        let event_rows = thread.thread_log.events.clone();
        let updated_at = thread.updated_at.to_rfc3339();
        let parent_id = thread
            .subagent_context
            .as_ref()
            .map(|ctx| ctx.parent_thread_id.0.clone());
        let serialized_folder_paths = folder_paths.serialize();
        let (folder_paths_str, folder_paths_order_str): (Option<String>, Option<String>) =
            if folder_paths.is_empty() {
                (None, None)
            } else {
                (
                    Some(serialized_folder_paths.paths),
                    Some(serialized_folder_paths.order),
                )
            };
        let json_data = serde_json::to_string(&SerializedThread {
            thread,
            version: DbThread::VERSION,
        })?;

        let connection = connection.lock();

        let compressed = zstd::encode_all(json_data.as_bytes(), COMPRESSION_LEVEL)?;
        let data_type = DataType::Zstd;
        let data = compressed;

        // Use the thread's updated_at as created_at for new threads.
        // This ensures the creation time reflects when the thread was conceptually
        // created, not when it was saved to the database.
        let created_at = updated_at.clone();

        let mut insert = connection.exec_bound::<(Arc<str>, Option<Arc<str>>, Option<String>, Option<String>, String, String, DataType, Vec<u8>, String)>(indoc! {"
            INSERT INTO threads (id, parent_id, folder_paths, folder_paths_order, summary, updated_at, data_type, data, created_at)
            VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
            ON CONFLICT(id) DO UPDATE SET
                parent_id = excluded.parent_id,
                folder_paths = excluded.folder_paths,
                folder_paths_order = excluded.folder_paths_order,
                summary = excluded.summary,
                updated_at = excluded.updated_at,
                data_type = excluded.data_type,
                data = excluded.data
        "})?;

        insert((
            id.0.clone(),
            parent_id,
            folder_paths_str,
            folder_paths_order_str,
            title,
            updated_at,
            data_type,
            data,
            created_at,
        ))?;

        let mut append_event =
            connection.exec_bound::<(Arc<str>, i64, Option<i64>, String)>(indoc! {"
                INSERT OR IGNORE INTO thread_events
                    (thread_id, sequence, parent_sequence, event_json)
                VALUES (?1, ?2, ?3, ?4)
            "})?;
        for event in event_rows {
            append_event((
                id.0.clone(),
                i64::try_from(event.sequence).context("thread event sequence exceeds SQLite")?,
                event
                    .parent_sequence
                    .map(i64::try_from)
                    .transpose()
                    .context("thread event parent sequence exceeds SQLite")?,
                serde_json::to_string(&event.kind)?,
            ))?;
        }

        Ok(())
    }

    pub fn list_threads(&self) -> Task<Result<Vec<DbThreadMetadata>>> {
        let connection = self.connection.clone();

        self.executor.spawn(async move {
            let connection = connection.lock();

            let mut select = connection
                .select_bound::<(), (Arc<str>, Option<Arc<str>>, Option<String>, Option<String>, String, String, Option<String>)>(indoc! {"
                SELECT id, parent_id, folder_paths, folder_paths_order, summary, updated_at, created_at FROM threads ORDER BY updated_at DESC, created_at DESC
            "})?;

            let rows = select(())?;
            let mut threads = Vec::new();

            for (id, parent_id, folder_paths, folder_paths_order, summary, updated_at, created_at) in rows {
                let folder_paths = folder_paths
                    .map(|paths| {
                        PathList::deserialize(&util::path_list::SerializedPathList {
                            paths,
                            order: folder_paths_order.unwrap_or_default(),
                        })
                    })
                    .unwrap_or_default();
                let created_at = created_at
                    .as_deref()
                    .map(DateTime::parse_from_rfc3339)
                    .transpose()?
                    .map(|dt| dt.with_timezone(&Utc));

                threads.push(DbThreadMetadata {
                    id: acp::SessionId::new(id),
                    parent_session_id: parent_id.map(acp::SessionId::new),
                    title: summary.into(),
                    updated_at: DateTime::parse_from_rfc3339(&updated_at)?.with_timezone(&Utc),
                    created_at,
                    folder_paths,
                });
            }

            Ok(threads)
        })
    }

    pub fn load_thread(&self, id: acp::SessionId) -> Task<Result<Option<DbThread>>> {
        let connection = self.connection.clone();

        self.executor.spawn(async move {
            let connection = connection.lock();
            let mut select = connection.select_bound::<Arc<str>, (DataType, Vec<u8>)>(indoc! {"
                SELECT data_type, data FROM threads WHERE id = ? LIMIT 1
            "})?;

            let rows = select(id.0.clone())?;
            if let Some((data_type, data)) = rows.into_iter().next() {
                let mut thread = Self::deserialize_thread(data_type, data)?;
                let mut select_events = connection
                    .select_bound::<Arc<str>, (i64, Option<i64>, String)>(indoc! {"
                        SELECT sequence, parent_sequence, event_json
                          FROM thread_events
                         WHERE thread_id = ?
                         ORDER BY sequence
                    "})?;
                let event_rows = select_events(id.0)?;
                if !event_rows.is_empty() {
                    thread.thread_log.events = event_rows
                        .into_iter()
                        .map(|(sequence, parent_sequence, event_json)| {
                            Ok(ThreadLogEvent {
                                sequence: u64::try_from(sequence)
                                    .context("negative thread event sequence")?,
                                parent_sequence: parent_sequence
                                    .map(u64::try_from)
                                    .transpose()
                                    .context("negative thread event parent sequence")?,
                                kind: serde_json::from_str(&event_json)?,
                            })
                        })
                        .collect::<Result<Vec<_>>>()?;
                }
                thread.prepare_for_resume(None)?;
                Ok(Some(thread))
            } else {
                Ok(None)
            }
        })
    }

    pub fn save_thread(
        &self,
        id: acp::SessionId,
        thread: DbThread,
        folder_paths: PathList,
    ) -> Task<Result<()>> {
        let connection = self.connection.clone();

        self.executor
            .spawn(async move { Self::save_thread_sync(&connection, id, thread, &folder_paths) })
    }

    fn deserialize_thread(data_type: DataType, data: Vec<u8>) -> Result<DbThread> {
        let json_data = match data_type {
            DataType::Zstd => {
                let decompressed = zstd::decode_all(&data[..])?;
                String::from_utf8(decompressed)?
            }
            DataType::Json => String::from_utf8(data)?,
        };
        DbThread::from_json(json_data.as_bytes())
    }

    fn sandboxed_terminal_temp_dir(data_type: DataType, data: Vec<u8>) -> Option<PathBuf> {
        match Self::deserialize_thread(data_type, data) {
            Ok(thread) => thread.sandboxed_terminal_temp_dir,
            Err(error) => {
                log::warn!("failed to deserialize thread before deleting it: {error:#}");
                None
            }
        }
    }

    fn remove_sandboxed_terminal_temp_dir(temp_dir: PathBuf) {
        match std::fs::remove_dir_all(&temp_dir) {
            Ok(()) => {}
            Err(error) if error.kind() == ErrorKind::NotFound => {}
            Err(error) => {
                log::warn!(
                    "failed to remove sandboxed terminal temp directory {}: {error}",
                    temp_dir.display()
                );
            }
        }
    }

    pub fn delete_thread(&self, id: acp::SessionId) -> Task<Result<()>> {
        let connection = self.connection.clone();

        self.executor.spawn(async move {
            let sandboxed_terminal_temp_dirs = {
                let connection = connection.lock();

                let mut select_children =
                    connection.select_bound::<Arc<str>, Arc<str>>(indoc! {"
                    SELECT id FROM threads WHERE parent_id = ?
                "})?;

                // Collect target thread together with all of its transitive
                // subagent threads
                let mut ids_to_delete = vec![id.0.clone()];
                let mut frontier = vec![id.0.clone()];
                while let Some(parent) = frontier.pop() {
                    for child in select_children(parent)? {
                        ids_to_delete.push(child.clone());
                        frontier.push(child);
                    }
                }

                let mut select =
                    connection.select_bound::<Arc<str>, (DataType, Vec<u8>)>(indoc! {"
                    SELECT data_type, data FROM threads WHERE id = ? LIMIT 1
                "})?;

                let mut delete = connection.exec_bound::<Arc<str>>(indoc! {"
                    DELETE FROM threads WHERE id = ?
                "})?;
                let mut delete_events = connection.exec_bound::<Arc<str>>(indoc! {"
                    DELETE FROM thread_events WHERE thread_id = ?
                "})?;

                let mut sandboxed_terminal_temp_dirs = Vec::new();
                for thread_id in ids_to_delete {
                    if let Some(temp_dir) = select(thread_id.clone())?.into_iter().next().and_then(
                        |(data_type, data)| Self::sandboxed_terminal_temp_dir(data_type, data),
                    ) {
                        sandboxed_terminal_temp_dirs.push(temp_dir);
                    }
                    delete_events(thread_id.clone())?;
                    delete(thread_id)?;
                }

                sandboxed_terminal_temp_dirs
            };

            for temp_dir in sandboxed_terminal_temp_dirs {
                Self::remove_sandboxed_terminal_temp_dir(temp_dir);
            }

            Ok(())
        })
    }

    pub fn delete_threads(&self) -> Task<Result<()>> {
        let connection = self.connection.clone();

        self.executor.spawn(async move {
            let sandboxed_terminal_temp_dirs = {
                let connection = connection.lock();

                let mut select = connection.select_bound::<(), (DataType, Vec<u8>)>(indoc! {"
                    SELECT data_type, data FROM threads
                "})?;

                let sandboxed_terminal_temp_dirs = select(())?
                    .into_iter()
                    .filter_map(|(data_type, data)| {
                        Self::sandboxed_terminal_temp_dir(data_type, data)
                    })
                    .collect::<Vec<_>>();

                let mut delete = connection.exec_bound::<()>(indoc! {"
                    DELETE FROM threads
                "})?;
                let mut delete_events = connection.exec_bound::<()>(indoc! {"
                    DELETE FROM thread_events
                "})?;

                delete_events(())?;
                delete(())?;

                sandboxed_terminal_temp_dirs
            };

            for temp_dir in sandboxed_terminal_temp_dirs {
                Self::remove_sandboxed_terminal_temp_dir(temp_dir);
            }

            Ok(())
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{DateTime, TimeZone, Utc};
    use collections::HashMap;
    use gpui::TestAppContext;
    use std::sync::Arc;

    #[test]
    fn test_shared_thread_roundtrip() {
        let original = SharedThread {
            title: "Test Thread".into(),
            messages: vec![],
            updated_at: Utc.with_ymd_and_hms(2024, 1, 1, 0, 0, 0).unwrap(),
            model: None,
            version: SharedThread::VERSION.to_string(),
        };

        let bytes = original.to_bytes().expect("Failed to serialize");
        let restored = SharedThread::from_bytes(&bytes).expect("Failed to deserialize");

        assert_eq!(restored.title, original.title);
        assert_eq!(restored.version, original.version);
        assert_eq!(restored.updated_at, original.updated_at);
    }

    fn session_id(value: &str) -> acp::SessionId {
        acp::SessionId::new(Arc::<str>::from(value))
    }

    fn make_thread(title: &str, updated_at: DateTime<Utc>) -> DbThread {
        DbThread {
            title: title.to_string().into(),
            messages: Vec::new(),
            updated_at,
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
            sandbox_grants: DbSandboxGrants::default(),
            thread_log: ThreadEventLog::default(),
            fork_origin: None,
        }
    }

    fn user_message(text: &str) -> Arc<Message> {
        Arc::new(Message::User(UserMessage {
            id: ClientUserMessageId::new(),
            content: Arc::from([UserMessageContent::Text(text.to_string())]),
        }))
    }

    fn message_text(message: &Message) -> &str {
        let Message::User(UserMessage { content, .. }) = message else {
            panic!("expected a user message");
        };
        let UserMessageContent::Text(text) = &content[0] else {
            panic!("expected text");
        };
        text
    }

    #[test]
    fn event_log_keeps_abandoned_events_and_reconstructs_selected_cursor() {
        let mut log = ThreadEventLog::default();
        let first = log.append_message(user_message("first"));
        let second = log.append_message(user_message("second"));
        log.truncate_messages(1);
        let branch = log.append_message(user_message("branch"));

        let active = log.messages().unwrap();
        assert_eq!(
            active
                .iter()
                .map(|message| message_text(message))
                .collect::<Vec<_>>(),
            ["first", "branch"]
        );
        assert_eq!(
            log.messages_at(Some(second))
                .unwrap()
                .iter()
                .map(|message| message_text(message))
                .collect::<Vec<_>>(),
            ["first", "second"]
        );
        assert_eq!(log.events.len(), 4);
        assert_eq!(log.active_sequence, Some(branch));
        assert_eq!(first, 0);
    }

    #[test]
    fn fork_preserves_prompt_prefix_and_restamps_origin() {
        let source_id = session_id("source");
        let mut thread = make_thread("Source", Utc.with_ymd_and_hms(2024, 1, 1, 0, 0, 0).unwrap());
        thread.thread_log.append_message(user_message("first"));
        let layout = PromptCacheLayout {
            system_prompt: "stable system prompt".into(),
            tool_order: vec!["read_file".into(), "terminal".into()],
        };
        let cursor = thread.thread_log.append_prompt_cache_layout(layout.clone());
        thread
            .thread_log
            .append_message(user_message("not inherited"));

        let fork = thread.fork_at(source_id.clone(), cursor).unwrap();

        assert_eq!(
            fork.fork_origin,
            Some(ThreadForkOrigin {
                session_id: source_id,
                event_sequence: cursor,
            })
        );
        assert_eq!(
            fork.thread_log
                .prompt_cache_layout(fork.thread_log.active_sequence)
                .unwrap(),
            Some(layout)
        );
        assert_eq!(fork.messages.len(), 1);
        assert_eq!(message_text(&fork.messages[0]), "first");
    }

    #[test]
    fn fork_at_message_index_uses_the_visible_prefix_after_insertions() {
        let source_id = session_id("source-with-insertion");
        let mut thread = make_thread("Source", Utc.with_ymd_and_hms(2024, 1, 1, 0, 0, 0).unwrap());
        thread.thread_log.append_message(user_message("first"));
        thread.thread_log.append_message(user_message("second"));
        thread.thread_log.insert_message(
            0,
            Arc::new(Message::Compaction(crate::CompactionInfo::Summary(
                "summary".into(),
            ))),
        );
        let layout = PromptCacheLayout {
            system_prompt: "stable prompt".into(),
            tool_order: vec!["terminal".into()],
        };
        let source_sequence = thread.thread_log.append_prompt_cache_layout(layout.clone());

        let fork = thread.fork_at_message_index(source_id.clone(), 1).unwrap();
        assert_eq!(fork.messages.len(), 2);
        assert!(matches!(&*fork.messages[0], Message::Compaction(_)));
        assert_eq!(message_text(&fork.messages[1]), "first");
        assert_eq!(
            fork.thread_log
                .prompt_cache_layout(fork.thread_log.active_sequence)
                .unwrap()
                .as_ref(),
            Some(&layout)
        );
        assert_eq!(
            fork.fork_origin,
            Some(ThreadForkOrigin {
                session_id: source_id,
                event_sequence: source_sequence,
            })
        );
    }

    #[test]
    fn tool_result_replacement_marker_is_recorded_deterministically() {
        let tool_use_id: language_model::LanguageModelToolUseId = "call-1".into();
        let mut tool_results = IndexMap::default();
        tool_results.insert(
            tool_use_id.clone(),
            language_model::LanguageModelToolResult {
                tool_use_id: tool_use_id.clone(),
                tool_name: "terminal".into(),
                is_error: false,
                content: vec![language_model::LanguageModelToolResultContent::Text(
                    "preview\n… [tool result truncated: showed 10 of 100 bytes. \
                     Full result: artifact terminal:call-1@v1.]"
                        .into(),
                )],
                output: None,
            },
        );
        let message = Arc::new(Message::Agent(AgentMessage {
            content: Vec::new(),
            tool_results,
            reasoning_details: None,
        }));
        let mut log = ThreadEventLog::default();
        log.append_message(message);

        let ThreadLogEventKind::MessageAppended { message } = &mut log.events[0].kind else {
            panic!("expected message event");
        };
        let Message::Agent(agent_message) = message.as_ref() else {
            panic!("expected agent message");
        };
        let mut changed_message = agent_message.clone();
        let result = changed_message.tool_results.get_mut(&tool_use_id).unwrap();
        result.content[0] =
            language_model::LanguageModelToolResultContent::Text("preview changed".into());
        *message = Arc::new(Message::Agent(changed_message));

        let references = log.replacement_references(log.active_sequence).unwrap();
        assert_eq!(
            references,
            vec![ToolResultReplacementReference {
                tool_use_id: "call-1".into(),
                content_index: 0,
                marker: "\n… [tool result truncated: showed 10 of 100 bytes. \
                         Full result: artifact terminal:call-1@v1.]"
                    .into(),
            }]
        );
        let restored = log.messages().unwrap();
        let Message::Agent(restored) = &*restored[0] else {
            panic!("expected restored agent message");
        };
        let language_model::LanguageModelToolResultContent::Text(restored_text) =
            &restored.tool_results[&tool_use_id].content[0]
        else {
            panic!("expected restored text");
        };
        assert_eq!(
            restored_text.as_ref(),
            "preview changed\n… [tool result truncated: showed 10 of 100 bytes. \
             Full result: artifact terminal:call-1@v1.]"
        );
    }

    #[gpui::test]
    async fn database_load_resumes_the_saved_active_cursor(cx: &mut TestAppContext) {
        let database = ThreadsDatabase::new(cx.executor()).unwrap();
        let thread_id = session_id("resume-thread");
        let mut thread = make_thread("Resume", Utc.with_ymd_and_hms(2024, 1, 1, 0, 0, 0).unwrap());
        let cursor = thread.thread_log.append_message(user_message("first"));
        thread.thread_log.append_message(user_message("second"));
        thread.thread_log.select(cursor).unwrap();
        thread.messages = thread.thread_log.messages().unwrap();

        database
            .save_thread(thread_id.clone(), thread, PathList::default())
            .await
            .unwrap();
        let loaded = database
            .load_thread(thread_id)
            .await
            .unwrap()
            .expect("saved thread");

        assert_eq!(loaded.thread_log.active_sequence, Some(cursor));
        assert_eq!(loaded.messages.len(), 1);
        assert_eq!(message_text(&loaded.messages[0]), "first");
        assert_eq!(loaded.thread_log.events.len(), 2);
    }

    #[gpui::test]
    async fn test_list_threads_orders_by_created_at(cx: &mut TestAppContext) {
        let database = ThreadsDatabase::new(cx.executor()).unwrap();

        let older_id = session_id("thread-a");
        let newer_id = session_id("thread-b");

        let older_thread = make_thread(
            "Thread A",
            Utc.with_ymd_and_hms(2024, 1, 1, 0, 0, 0).unwrap(),
        );
        let newer_thread = make_thread(
            "Thread B",
            Utc.with_ymd_and_hms(2024, 1, 2, 0, 0, 0).unwrap(),
        );

        database
            .save_thread(older_id.clone(), older_thread, PathList::default())
            .await
            .unwrap();
        database
            .save_thread(newer_id.clone(), newer_thread, PathList::default())
            .await
            .unwrap();

        let entries = database.list_threads().await.unwrap();
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].id, newer_id);
        assert_eq!(entries[1].id, older_id);
    }

    #[gpui::test]
    async fn test_save_thread_replaces_metadata(cx: &mut TestAppContext) {
        let database = ThreadsDatabase::new(cx.executor()).unwrap();

        let thread_id = session_id("thread-a");
        let original_thread = make_thread(
            "Thread A",
            Utc.with_ymd_and_hms(2024, 1, 1, 0, 0, 0).unwrap(),
        );
        let updated_thread = make_thread(
            "Thread B",
            Utc.with_ymd_and_hms(2024, 1, 2, 0, 0, 0).unwrap(),
        );

        database
            .save_thread(thread_id.clone(), original_thread, PathList::default())
            .await
            .unwrap();
        database
            .save_thread(thread_id.clone(), updated_thread, PathList::default())
            .await
            .unwrap();

        let entries = database.list_threads().await.unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].id, thread_id);
        assert_eq!(entries[0].title.as_ref(), "Thread B");
        assert_eq!(
            entries[0].updated_at,
            Utc.with_ymd_and_hms(2024, 1, 2, 0, 0, 0).unwrap()
        );
        assert!(
            entries[0].created_at.is_some(),
            "created_at should be populated"
        );
    }

    #[test]
    fn test_subagent_context_defaults_to_none() {
        let json = r#"{
            "title": "Old Thread",
            "messages": [],
            "updated_at": "2024-01-01T00:00:00Z"
        }"#;

        let db_thread: DbThread = serde_json::from_str(json).expect("Failed to deserialize");

        assert!(
            db_thread.subagent_context.is_none(),
            "Legacy threads without subagent_context should default to None"
        );
    }

    #[test]
    fn test_draft_prompt_defaults_to_none() {
        let json = r#"{
            "title": "Old Thread",
            "messages": [],
            "updated_at": "2024-01-01T00:00:00Z"
        }"#;

        let db_thread: DbThread = serde_json::from_str(json).expect("Failed to deserialize");

        assert!(
            db_thread.draft_prompt.is_none(),
            "Legacy threads without draft_prompt field should default to None"
        );
    }

    #[test]
    fn test_sandboxed_terminal_temp_dir_defaults_to_none() {
        let json = r#"{
            "title": "Old Thread",
            "messages": [],
            "updated_at": "2024-01-01T00:00:00Z"
        }"#;

        let db_thread: DbThread = serde_json::from_str(json).expect("Failed to deserialize");

        assert!(
            db_thread.sandboxed_terminal_temp_dir.is_none(),
            "Legacy threads without sandboxed_terminal_temp_dir should default to None"
        );
    }

    #[test]
    fn test_sandbox_grants_default_when_absent() {
        let json = r#"{
            "title": "Old Thread",
            "messages": [],
            "updated_at": "2024-01-01T00:00:00Z"
        }"#;

        let db_thread: DbThread = serde_json::from_str(json).expect("Failed to deserialize");

        assert_eq!(
            db_thread.sandbox_grants,
            DbSandboxGrants::default(),
            "Legacy threads without sandbox_grants should default to empty grants"
        );
    }

    #[gpui::test]
    async fn test_sandbox_grants_roundtrip_through_save_load(cx: &mut TestAppContext) {
        let database = ThreadsDatabase::new(cx.executor()).unwrap();
        let thread_id = session_id("sandbox-grants-thread");
        let mut thread = make_thread(
            "Sandbox Grants Thread",
            Utc.with_ymd_and_hms(2024, 1, 1, 0, 0, 0).unwrap(),
        );
        let grants = DbSandboxGrants {
            write_paths: vec![
                // A legacy bare-string grant (no resolved canonical) and a grant
                // carrying its resolved canonical, to exercise both forms of the
                // string-or-object round-trip.
                settings::GrantedWritePath::from_requested(PathBuf::from("/tmp/build")),
                settings::GrantedWritePath::resolved(
                    PathBuf::from("/tmp/link"),
                    PathBuf::from("/tmp/real"),
                ),
            ],
            network_hosts: vec!["github.com".to_string(), "*.npmjs.org".to_string()],
            network_any_host: false,
            allow_fs_write_all: false,
            unsandboxed: true,
            sandbox_fallback: true,
        };
        thread.sandbox_grants = grants.clone();

        database
            .save_thread(thread_id.clone(), thread, PathList::default())
            .await
            .unwrap();

        let loaded = database
            .load_thread(thread_id)
            .await
            .unwrap()
            .expect("thread should exist");
        assert_eq!(loaded.sandbox_grants, grants);
    }

    #[gpui::test]
    async fn test_sandboxed_terminal_temp_dir_roundtrips_through_save_load(
        cx: &mut TestAppContext,
    ) {
        let database = ThreadsDatabase::new(cx.executor()).unwrap();
        let thread_id = session_id("sandbox-temp-dir-thread");
        let temp_dir = tempfile::Builder::new()
            .prefix("omega-agent-terminal-test-")
            .tempdir()
            .unwrap()
            .keep();
        let mut thread = make_thread(
            "Sandbox Temp Dir Thread",
            Utc.with_ymd_and_hms(2024, 1, 1, 0, 0, 0).unwrap(),
        );
        thread.sandboxed_terminal_temp_dir = Some(temp_dir.clone());

        database
            .save_thread(thread_id.clone(), thread, PathList::default())
            .await
            .unwrap();

        let loaded = database
            .load_thread(thread_id)
            .await
            .unwrap()
            .expect("thread should exist");
        assert_eq!(loaded.sandboxed_terminal_temp_dir, Some(temp_dir.clone()));
        std::fs::remove_dir_all(temp_dir).unwrap();
    }

    #[gpui::test]
    async fn test_delete_thread_removes_sandboxed_terminal_temp_dir(cx: &mut TestAppContext) {
        let database = ThreadsDatabase::new(cx.executor()).unwrap();
        let thread_id = session_id("sandbox-temp-dir-delete-thread");
        let temp_dir = tempfile::Builder::new()
            .prefix("omega-agent-terminal-test-")
            .tempdir()
            .unwrap()
            .keep();
        std::fs::write(temp_dir.join("sentinel"), b"content").unwrap();
        let mut thread = make_thread(
            "Sandbox Temp Dir Delete Thread",
            Utc.with_ymd_and_hms(2024, 1, 1, 0, 0, 0).unwrap(),
        );
        thread.sandboxed_terminal_temp_dir = Some(temp_dir.clone());

        database
            .save_thread(thread_id.clone(), thread, PathList::default())
            .await
            .unwrap();
        database.delete_thread(thread_id).await.unwrap();

        assert!(!temp_dir.exists());
    }

    #[gpui::test]
    async fn test_delete_thread_deletes_subagent_threads(cx: &mut TestAppContext) {
        let database = ThreadsDatabase::new(cx.executor()).unwrap();

        let parent_id = session_id("parent-thread");
        let child_id = session_id("child-thread");
        let grandchild_id = session_id("grandchild-thread");
        let unrelated_id = session_id("unrelated-thread");

        let parent_thread = make_thread(
            "Parent Thread",
            Utc.with_ymd_and_hms(2024, 1, 1, 0, 0, 0).unwrap(),
        );

        let mut child_thread = make_thread(
            "Child Subagent Thread",
            Utc.with_ymd_and_hms(2024, 1, 1, 0, 0, 0).unwrap(),
        );
        child_thread.subagent_context = Some(crate::SubagentContext {
            parent_thread_id: parent_id.clone(),
            depth: 1,
        });

        let mut grandchild_thread = make_thread(
            "Grandchild Subagent Thread",
            Utc.with_ymd_and_hms(2024, 1, 1, 0, 0, 0).unwrap(),
        );
        grandchild_thread.subagent_context = Some(crate::SubagentContext {
            parent_thread_id: child_id.clone(),
            depth: 2,
        });

        let unrelated_thread = make_thread(
            "Unrelated Thread",
            Utc.with_ymd_and_hms(2024, 1, 1, 0, 0, 0).unwrap(),
        );

        for (id, thread) in [
            (parent_id.clone(), parent_thread),
            (child_id.clone(), child_thread),
            (grandchild_id.clone(), grandchild_thread),
            (unrelated_id.clone(), unrelated_thread),
        ] {
            database
                .save_thread(id, thread, PathList::default())
                .await
                .unwrap();
        }

        database.delete_thread(parent_id.clone()).await.unwrap();

        let remaining = database.list_threads().await.unwrap();
        let remaining_ids: Vec<_> = remaining.iter().map(|thread| thread.id.clone()).collect();
        assert_eq!(remaining_ids, vec![unrelated_id]);
    }

    #[gpui::test]
    async fn test_subagent_context_roundtrips_through_save_load(cx: &mut TestAppContext) {
        let database = ThreadsDatabase::new(cx.executor()).unwrap();

        let parent_id = session_id("parent-thread");
        let child_id = session_id("child-thread");

        let mut child_thread = make_thread(
            "Subagent Thread",
            Utc.with_ymd_and_hms(2024, 1, 1, 0, 0, 0).unwrap(),
        );
        child_thread.subagent_context = Some(crate::SubagentContext {
            parent_thread_id: parent_id.clone(),
            depth: 2,
        });

        database
            .save_thread(child_id.clone(), child_thread, PathList::default())
            .await
            .unwrap();

        let loaded = database
            .load_thread(child_id)
            .await
            .unwrap()
            .expect("thread should exist");

        let context = loaded
            .subagent_context
            .expect("subagent_context should be restored");
        assert_eq!(context.parent_thread_id, parent_id);
        assert_eq!(context.depth, 2);
    }

    #[gpui::test]
    async fn test_non_subagent_thread_has_no_subagent_context(cx: &mut TestAppContext) {
        let database = ThreadsDatabase::new(cx.executor()).unwrap();

        let thread_id = session_id("regular-thread");
        let thread = make_thread(
            "Regular Thread",
            Utc.with_ymd_and_hms(2024, 1, 1, 0, 0, 0).unwrap(),
        );

        database
            .save_thread(thread_id.clone(), thread, PathList::default())
            .await
            .unwrap();

        let loaded = database
            .load_thread(thread_id)
            .await
            .unwrap()
            .expect("thread should exist");

        assert!(
            loaded.subagent_context.is_none(),
            "Regular threads should have no subagent_context"
        );
    }

    #[gpui::test]
    async fn test_folder_paths_roundtrip(cx: &mut TestAppContext) {
        let database = ThreadsDatabase::new(cx.executor()).unwrap();

        let thread_id = session_id("folder-thread");
        let thread = make_thread(
            "Folder Thread",
            Utc.with_ymd_and_hms(2024, 6, 15, 12, 0, 0).unwrap(),
        );

        let folder_paths = PathList::new(&[
            std::path::PathBuf::from("/home/user/project-a"),
            std::path::PathBuf::from("/home/user/project-b"),
        ]);

        database
            .save_thread(thread_id.clone(), thread, folder_paths.clone())
            .await
            .unwrap();

        let threads = database.list_threads().await.unwrap();
        assert_eq!(threads.len(), 1);
    }

    #[gpui::test]
    async fn test_folder_paths_empty_when_not_set(cx: &mut TestAppContext) {
        let database = ThreadsDatabase::new(cx.executor()).unwrap();

        let thread_id = session_id("no-folder-thread");
        let thread = make_thread(
            "No Folder Thread",
            Utc.with_ymd_and_hms(2024, 6, 15, 12, 0, 0).unwrap(),
        );

        database
            .save_thread(thread_id.clone(), thread, PathList::default())
            .await
            .unwrap();

        let threads = database.list_threads().await.unwrap();
        assert_eq!(threads.len(), 1);
    }

    #[test]
    fn test_scroll_position_defaults_to_none() {
        let json = r#"{
            "title": "Old Thread",
            "messages": [],
            "updated_at": "2024-01-01T00:00:00Z"
        }"#;

        let db_thread: DbThread = serde_json::from_str(json).expect("Failed to deserialize");

        assert!(
            db_thread.ui_scroll_position.is_none(),
            "Legacy threads without scroll_position field should default to None"
        );
    }

    #[gpui::test]
    async fn test_scroll_position_roundtrips_through_save_load(cx: &mut TestAppContext) {
        let database = ThreadsDatabase::new(cx.executor()).unwrap();

        let thread_id = session_id("thread-with-scroll");

        let mut thread = make_thread(
            "Thread With Scroll",
            Utc.with_ymd_and_hms(2024, 1, 1, 0, 0, 0).unwrap(),
        );
        thread.ui_scroll_position = Some(SerializedScrollPosition {
            item_ix: 42,
            offset_in_item: 13.5,
        });

        database
            .save_thread(thread_id.clone(), thread, PathList::default())
            .await
            .unwrap();

        let loaded = database
            .load_thread(thread_id)
            .await
            .unwrap()
            .expect("thread should exist");

        let scroll = loaded
            .ui_scroll_position
            .expect("scroll_position should be restored");
        assert_eq!(scroll.item_ix, 42);
        assert!((scroll.offset_in_item - 13.5).abs() < f32::EPSILON);
    }
}
