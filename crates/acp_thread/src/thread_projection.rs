use crate::{
    AgentThreadEntry, ContentBlock, ContextCompactionStatus, ElicitationStatus, ToolCallContent,
    ToolCallStatus,
};
use agent_client_protocol::schema::v1 as acp;
use gpui::App;
use std::{
    collections::hash_map::DefaultHasher,
    hash::{Hash, Hasher},
    path::PathBuf,
    sync::Arc,
};

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct ThreadProjectionBinding {
    pub thread_id: Arc<str>,
    pub work_dirs: Arc<[PathBuf]>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ThreadEntryId(pub u64);

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ThreadArtifactId(pub u64);

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ThreadEventKind {
    UserMessage,
    AssistantMessage,
    ToolCall,
    Elicitation,
    CompletedPlan,
    ContextCompaction,
    SystemNote,
    Reasoning,
    ToolResult,
    Approval,
    PlanUpdate,
    Checkpoint,
    Error,
    Retry,
    ReplayBoundary,
    Completion,
    Cancellation,
    Refusal,
    Unknown,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ThreadEventOwner {
    User,
    Agent,
    Tool,
    Host,
    System,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum ThreadEventSource {
    Session,
    ToolCall(Arc<str>),
    Elicitation(Arc<str>),
    SystemNote(Arc<str>),
    Unknown,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ThreadEventStatus {
    Pending,
    WaitingForConfirmation,
    InProgress,
    Completed,
    Failed,
    Rejected,
    Canceled,
    Unknown,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum ThreadActionTarget {
    Entry(ThreadEntryId),
    ToolCall(Arc<str>),
    File { path: PathBuf, line: Option<u32> },
    Uri(Arc<str>),
    Terminal(Arc<str>),
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum ThreadArtifact {
    File {
        path: PathBuf,
        line: Option<u32>,
    },
    Diff {
        path: Option<PathBuf>,
        /// Stable while the GPUI diff entity is retained. ACP does not provide
        /// an identity that can survive reconstructing the thread entity.
        entity_id: u64,
    },
    Resource {
        uri: Arc<str>,
        mime_type: Option<Arc<str>>,
    },
    Link {
        uri: Arc<str>,
        name: Arc<str>,
        mime_type: Option<Arc<str>>,
    },
    Image {
        /// Stable while the decoded image is retained. ACP does not provide an
        /// image identity that can survive reconstructing the thread entity.
        image_id: u64,
        width: Option<u32>,
        height: Option<u32>,
    },
    TerminalResult {
        terminal_id: Arc<str>,
        retained_artifact: Option<Arc<str>>,
        byte_count: Option<usize>,
        line_count: Option<usize>,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ThreadArtifactRevision {
    pub revision: u64,
    pub source_events: Vec<ThreadEntryId>,
    pub owner: ThreadEventOwner,
    pub status: ThreadEventStatus,
    pub artifact: ThreadArtifact,
    pub action_target: Option<ThreadActionTarget>,
    pub is_current: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ThreadArtifactProjection {
    /// Captured when this artifact is first projected and never rewritten.
    pub binding: ThreadProjectionBinding,
    pub id: ThreadArtifactId,
    pub revision: u64,
    pub source_events: Vec<ThreadEntryId>,
    pub owner: ThreadEventOwner,
    pub status: ThreadEventStatus,
    pub artifact: ThreadArtifact,
    pub action_target: Option<ThreadActionTarget>,
    pub is_current: bool,
    pub history: Vec<ThreadArtifactRevision>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ThreadEventProjection {
    /// Captured when this event is first projected and never rewritten.
    pub binding: ThreadProjectionBinding,
    pub id: ThreadEntryId,
    pub parent_id: Option<ThreadEntryId>,
    pub revision: u64,
    pub entry_index: Option<usize>,
    pub kind: ThreadEventKind,
    pub owner: ThreadEventOwner,
    pub source: ThreadEventSource,
    pub status: ThreadEventStatus,
    pub related_kinds: Vec<ThreadEventKind>,
    pub artifacts: Vec<ThreadArtifactProjection>,
    pub action_targets: Vec<ThreadActionTarget>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ThreadProjectionSnapshot {
    pub binding: ThreadProjectionBinding,
    pub thread_id: Arc<str>,
    pub work_dirs: Arc<[PathBuf]>,
    pub revision: u64,
    pub entries: Vec<ThreadEventProjection>,
    pub artifacts: Vec<ThreadArtifactProjection>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct ThreadEntryProjectionState {
    pub id: ThreadEntryId,
    pub revision: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct ThreadChildProjectionState {
    pub id: ThreadEntryId,
    pub revision: u64,
    pub kind: ThreadEventKind,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ThreadArtifactProjectionState {
    pub binding: ThreadProjectionBinding,
    pub id: ThreadArtifactId,
    pub revision: u64,
    pub source_events: Vec<ThreadEntryId>,
    pub owner: ThreadEventOwner,
    pub status: ThreadEventStatus,
    pub artifact: ThreadArtifact,
    pub action_target: Option<ThreadActionTarget>,
    pub is_current: bool,
    pub history: Vec<ThreadArtifactRevision>,
}

pub(crate) fn project_entry(
    entry: &AgentThreadEntry,
    state: ThreadEntryProjectionState,
    artifact_states: &[ThreadArtifactProjectionState],
    entry_index: usize,
    binding: &ThreadProjectionBinding,
) -> ThreadEventProjection {
    let (kind, owner, source, status) = event_metadata(entry);
    let mut action_targets = vec![ThreadActionTarget::Entry(state.id)];

    if let AgentThreadEntry::ToolCall(tool_call) = entry {
        let tool_call_id: Arc<str> = tool_call.id.to_string().into();
        action_targets.push(ThreadActionTarget::ToolCall(tool_call_id));

        for artifact in artifact_states {
            if artifact.source_events.contains(&state.id)
                && let Some(target) = &artifact.action_target
            {
                push_unique(&mut action_targets, target.clone());
            }
        }
    }

    let artifacts = artifact_states
        .iter()
        .filter(|artifact| artifact.source_events.contains(&state.id))
        .map(project_artifact)
        .collect();

    ThreadEventProjection {
        binding: binding.clone(),
        id: state.id,
        parent_id: None,
        revision: state.revision,
        entry_index: Some(entry_index),
        kind,
        owner,
        source,
        status,
        related_kinds: related_kinds(entry),
        artifacts,
        action_targets,
    }
}

pub(crate) fn related_kinds(entry: &AgentThreadEntry) -> Vec<ThreadEventKind> {
    let mut kinds = Vec::new();
    match entry {
        AgentThreadEntry::UserMessage(message) => {
            if message.checkpoint.is_some() {
                kinds.push(ThreadEventKind::Checkpoint);
            }
        }
        AgentThreadEntry::AssistantMessage(message) => {
            if message
                .chunks
                .iter()
                .any(|chunk| matches!(chunk, crate::AssistantMessageChunk::Thought { .. }))
            {
                kinds.push(ThreadEventKind::Reasoning);
            }
        }
        AgentThreadEntry::ToolCall(tool_call) => {
            if matches!(
                tool_call.status,
                ToolCallStatus::WaitingForConfirmation { .. }
            ) {
                kinds.push(ThreadEventKind::Approval);
            }
            if !tool_call.content.is_empty() || tool_call.raw_output.is_some() {
                kinds.push(ThreadEventKind::ToolResult);
            }
            match tool_call.status {
                ToolCallStatus::Completed => kinds.push(ThreadEventKind::Completion),
                ToolCallStatus::Failed => kinds.push(ThreadEventKind::Error),
                ToolCallStatus::Rejected => kinds.push(ThreadEventKind::Refusal),
                ToolCallStatus::Canceled => kinds.push(ThreadEventKind::Cancellation),
                ToolCallStatus::Pending
                | ToolCallStatus::WaitingForConfirmation { .. }
                | ToolCallStatus::InProgress => {}
            }
        }
        AgentThreadEntry::Elicitation(_) => kinds.push(ThreadEventKind::Approval),
        AgentThreadEntry::CompletedPlan(_) => kinds.push(ThreadEventKind::PlanUpdate),
        AgentThreadEntry::ContextCompaction(compaction) => match compaction.status {
            ContextCompactionStatus::Completed => kinds.push(ThreadEventKind::Completion),
            ContextCompactionStatus::Canceled => kinds.push(ThreadEventKind::Cancellation),
            ContextCompactionStatus::InProgress => {}
        },
        AgentThreadEntry::SystemNote(_) => {}
    }
    kinds
}

pub(crate) fn project_artifact(
    artifact: &ThreadArtifactProjectionState,
) -> ThreadArtifactProjection {
    ThreadArtifactProjection {
        binding: artifact.binding.clone(),
        id: artifact.id,
        revision: artifact.revision,
        source_events: artifact.source_events.clone(),
        owner: artifact.owner,
        status: artifact.status,
        artifact: artifact.artifact.clone(),
        action_target: artifact.action_target.clone(),
        is_current: artifact.is_current,
        history: artifact.history.clone(),
    }
}

pub(crate) fn project_child_event(
    parent: &ThreadEventProjection,
    id: ThreadEntryId,
    revision: u64,
    kind: ThreadEventKind,
) -> ThreadEventProjection {
    ThreadEventProjection {
        binding: parent.binding.clone(),
        id,
        parent_id: Some(parent.id),
        revision,
        entry_index: parent.entry_index,
        kind,
        owner: parent.owner,
        source: parent.source.clone(),
        status: parent.status,
        related_kinds: Vec::new(),
        artifacts: parent.artifacts.clone(),
        action_targets: vec![ThreadActionTarget::Entry(parent.id)],
    }
}

pub(crate) fn status_for_entry(entry: &AgentThreadEntry) -> ThreadEventStatus {
    event_metadata(entry).3
}

pub(crate) fn artifact_candidates(
    entry: &AgentThreadEntry,
    cx: &App,
) -> Vec<(ThreadArtifact, Option<ThreadActionTarget>)> {
    let AgentThreadEntry::ToolCall(tool_call) = entry else {
        return Vec::new();
    };
    let mut artifacts = Vec::new();
    for location in &tool_call.locations {
        push_candidate(
            &mut artifacts,
            ThreadArtifact::File {
                path: location.path.clone(),
                line: location.line,
            },
            Some(ThreadActionTarget::File {
                path: location.path.clone(),
                line: location.line,
            }),
        );
    }
    for content in &tool_call.content {
        match content {
            ToolCallContent::ContentBlock(content) => {
                project_content_block(content, &mut artifacts)
            }
            ToolCallContent::Diff(diff) => {
                let entity_id = diff.entity_id().as_u64();
                let diff = diff.read(cx);
                let path = diff.file_path(cx).map(PathBuf::from);
                push_candidate(
                    &mut artifacts,
                    ThreadArtifact::Diff {
                        path: path.clone(),
                        entity_id,
                    },
                    path.map(|path| ThreadActionTarget::File { path, line: None }),
                );
            }
            ToolCallContent::Terminal(terminal) => {
                let terminal = terminal.read(cx);
                let terminal_id: Arc<str> = terminal.id().to_string().into();
                let retained_artifact = terminal
                    .result_artifacts()
                    .latest()
                    .map(|artifact| Arc::from(artifact.id().to_string()));
                let output = terminal.output();
                push_candidate(
                    &mut artifacts,
                    ThreadArtifact::TerminalResult {
                        terminal_id: terminal_id.clone(),
                        retained_artifact,
                        byte_count: output.map(|output| output.original_content_len),
                        line_count: output.map(|output| output.content_line_count),
                    },
                    Some(ThreadActionTarget::Terminal(terminal_id)),
                );
            }
        }
    }
    artifacts
}

fn event_metadata(
    entry: &AgentThreadEntry,
) -> (
    ThreadEventKind,
    ThreadEventOwner,
    ThreadEventSource,
    ThreadEventStatus,
) {
    match entry {
        AgentThreadEntry::UserMessage(_) => (
            ThreadEventKind::UserMessage,
            ThreadEventOwner::User,
            ThreadEventSource::Session,
            ThreadEventStatus::Completed,
        ),
        AgentThreadEntry::AssistantMessage(_) => (
            ThreadEventKind::AssistantMessage,
            ThreadEventOwner::Agent,
            ThreadEventSource::Session,
            ThreadEventStatus::Completed,
        ),
        AgentThreadEntry::ToolCall(tool_call) => (
            ThreadEventKind::ToolCall,
            ThreadEventOwner::Tool,
            ThreadEventSource::ToolCall(tool_call.id.to_string().into()),
            tool_status(&tool_call.status),
        ),
        AgentThreadEntry::Elicitation(id) => (
            ThreadEventKind::Elicitation,
            ThreadEventOwner::Agent,
            ThreadEventSource::Elicitation(id.0.clone()),
            ThreadEventStatus::Pending,
        ),
        AgentThreadEntry::CompletedPlan(_) => (
            ThreadEventKind::CompletedPlan,
            ThreadEventOwner::Agent,
            ThreadEventSource::Session,
            ThreadEventStatus::Completed,
        ),
        AgentThreadEntry::ContextCompaction(compaction) => (
            ThreadEventKind::ContextCompaction,
            ThreadEventOwner::System,
            ThreadEventSource::Session,
            match compaction.status {
                ContextCompactionStatus::InProgress => ThreadEventStatus::InProgress,
                ContextCompactionStatus::Completed => ThreadEventStatus::Completed,
                ContextCompactionStatus::Canceled => ThreadEventStatus::Canceled,
            },
        ),
        AgentThreadEntry::SystemNote(note) => (
            ThreadEventKind::SystemNote,
            ThreadEventOwner::Host,
            ThreadEventSource::SystemNote(note.id.0.clone()),
            ThreadEventStatus::Completed,
        ),
    }
}

fn tool_status(status: &ToolCallStatus) -> ThreadEventStatus {
    match status {
        ToolCallStatus::Pending => ThreadEventStatus::Pending,
        ToolCallStatus::WaitingForConfirmation { .. } => ThreadEventStatus::WaitingForConfirmation,
        ToolCallStatus::InProgress => ThreadEventStatus::InProgress,
        ToolCallStatus::Completed => ThreadEventStatus::Completed,
        ToolCallStatus::Failed => ThreadEventStatus::Failed,
        ToolCallStatus::Rejected => ThreadEventStatus::Rejected,
        ToolCallStatus::Canceled => ThreadEventStatus::Canceled,
    }
}

fn project_content_block(
    content: &ContentBlock,
    artifacts: &mut Vec<(ThreadArtifact, Option<ThreadActionTarget>)>,
) {
    match content {
        ContentBlock::EmbeddedResource { resource, .. } => {
            let (uri, mime_type) = match &resource.resource {
                acp::EmbeddedResourceResource::TextResourceContents(resource) => {
                    (&resource.uri, resource.mime_type.as_deref())
                }
                acp::EmbeddedResourceResource::BlobResourceContents(resource) => {
                    (&resource.uri, resource.mime_type.as_deref())
                }
                _ => return,
            };
            let uri: Arc<str> = uri.clone().into();
            push_candidate(
                artifacts,
                ThreadArtifact::Resource {
                    uri: uri.clone(),
                    mime_type: mime_type.map(Arc::from),
                },
                Some(ThreadActionTarget::Uri(uri)),
            );
        }
        ContentBlock::ResourceLink { resource_link } => {
            let uri: Arc<str> = resource_link.uri.clone().into();
            push_candidate(
                artifacts,
                ThreadArtifact::Link {
                    uri: uri.clone(),
                    name: resource_link.name.clone().into(),
                    mime_type: resource_link.mime_type.as_deref().map(Arc::from),
                },
                Some(ThreadActionTarget::Uri(uri)),
            );
        }
        ContentBlock::Image { image, dimensions } => push_candidate(
            artifacts,
            ThreadArtifact::Image {
                image_id: image.id,
                width: dimensions.map(|size| size.width),
                height: dimensions.map(|size| size.height),
            },
            None,
        ),
        ContentBlock::Empty | ContentBlock::Markdown { .. } => {}
    }
}

fn push_candidate(
    items: &mut Vec<(ThreadArtifact, Option<ThreadActionTarget>)>,
    artifact: ThreadArtifact,
    action_target: Option<ThreadActionTarget>,
) {
    let key = artifact_logical_key(&artifact);
    if !items
        .iter()
        .any(|(existing, _)| artifact_logical_key(existing) == key)
    {
        items.push((artifact, action_target));
    }
}

pub(crate) fn same_logical_artifact(left: &ThreadArtifact, right: &ThreadArtifact) -> bool {
    artifact_logical_key(left) == artifact_logical_key(right)
}

#[derive(PartialEq, Eq)]
enum ArtifactLogicalKey<'a> {
    File(&'a PathBuf),
    DiffPath(&'a PathBuf),
    DiffRuntime(u64),
    Resource(&'a str),
    Link(&'a str),
    Image(u64),
    Terminal(&'a str),
}

fn artifact_logical_key(artifact: &ThreadArtifact) -> ArtifactLogicalKey<'_> {
    match artifact {
        ThreadArtifact::File { path, .. } => ArtifactLogicalKey::File(path),
        ThreadArtifact::Diff {
            path: Some(path), ..
        } => ArtifactLogicalKey::DiffPath(path),
        ThreadArtifact::Diff {
            path: None,
            entity_id,
        } => ArtifactLogicalKey::DiffRuntime(*entity_id),
        ThreadArtifact::Resource { uri, .. } => ArtifactLogicalKey::Resource(uri),
        ThreadArtifact::Link { uri, .. } => ArtifactLogicalKey::Link(uri),
        ThreadArtifact::Image { image_id, .. } => ArtifactLogicalKey::Image(*image_id),
        ThreadArtifact::TerminalResult { terminal_id, .. } => {
            ArtifactLogicalKey::Terminal(terminal_id)
        }
    }
}

pub(crate) fn entry_projection_fingerprint(
    entry: &AgentThreadEntry,
    status_override: Option<ThreadEventStatus>,
    cx: &App,
) -> u64 {
    let (kind, owner, source, status) = event_metadata(entry);
    let mut hasher = DefaultHasher::new();
    kind.hash(&mut hasher);
    owner.hash(&mut hasher);
    source.hash(&mut hasher);
    status_override.unwrap_or(status).hash(&mut hasher);
    entry.to_markdown(cx).hash(&mut hasher);
    related_kinds(entry).hash(&mut hasher);
    artifact_candidates(entry, cx).hash(&mut hasher);
    hasher.finish()
}

fn push_unique<T: PartialEq>(items: &mut Vec<T>, item: T) {
    if !items.contains(&item) {
        items.push(item);
    }
}

pub(crate) fn elicitation_status(status: &ElicitationStatus) -> ThreadEventStatus {
    match status {
        ElicitationStatus::Pending { .. } => ThreadEventStatus::Pending,
        ElicitationStatus::Accepted => ThreadEventStatus::InProgress,
        ElicitationStatus::Declined => ThreadEventStatus::Rejected,
        ElicitationStatus::Canceled => ThreadEventStatus::Canceled,
        ElicitationStatus::Completed => ThreadEventStatus::Completed,
    }
}
