use std::sync::Arc;

use agent_client_protocol::schema::v1 as acp;
use gpui::{App, SharedString, Task};
use language_model::LanguageModelToolResultContent;
use schemars::JsonSchema;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

use super::{
    ReadFileTool, ReadFileToolInput, ReadSubagentTranscriptTool, ReadSubagentTranscriptToolInput,
    ReadToolResultArtifactTool, ReadToolResultArtifactToolInput, SkillBodyResolver, SkillTool,
    SkillToolInput, SkillsResolver, TranscriptDetail,
};
use crate::{AgentTool, ToolCallEventStream, ToolInput};

const DEFAULT_FILE_LINE_LIMIT: usize = 2_000;
const MAX_FILE_LINE_LIMIT: usize = 2_000;

/// Read a file, image, tool-result artifact, thread transcript, or
/// skill from an address already present in your context.
///
/// File output is line-numbered. Large files return an outline. Use `offset`
/// and `limit` to page through a file; file offsets are 1-based. Artifact and
/// transcript offsets are 0-based because their printed continuation addresses
/// use result-line and message indexes.
///
/// Address forms:
/// - Project or global-skill file path: `root/src/main.rs`
/// - Tool-result artifact: copy `tool:...@v...` or `terminal:...@v...` exactly
/// - Thread transcript: `thread:<session_id>`, `session:<session_id>`,
///   `agent:<session_id>`, or `delegate:<session_id>`
/// - Skill: copy its catalog `location` exactly, or use `skill:<name>`
///
/// Directories are not readable. Use `bash` with `ls` to inspect them.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ReadToolInput {
    /// The file path or other readable address.
    pub path: String,
    /// Where to begin. File offsets are 1-based; artifact and transcript
    /// offsets are 0-based. Defaults to the beginning.
    #[serde(default)]
    pub offset: Option<usize>,
    /// Maximum file lines, artifact lines, or transcript messages to return.
    #[serde(default)]
    pub limit: Option<usize>,
    /// Transcript detail. Ignored for other address types.
    #[serde(default)]
    pub detail: TranscriptDetail,
}

pub struct ReadTool {
    files: Arc<ReadFileTool>,
    artifacts: ReadToolResultArtifactTool,
    transcripts: ReadSubagentTranscriptTool,
    skills: SkillsResolver,
    skill_tool: Arc<SkillTool>,
}

impl ReadTool {
    pub fn new(
        files: ReadFileTool,
        artifacts: ReadToolResultArtifactTool,
        transcripts: ReadSubagentTranscriptTool,
        skills: SkillsResolver,
        skill_bodies: SkillBodyResolver,
    ) -> Self {
        Self {
            files: Arc::new(files),
            artifacts,
            transcripts,
            skills: skills.clone(),
            skill_tool: Arc::new(SkillTool::new(skills, skill_bodies)),
        }
    }

    fn tool_input<T: DeserializeOwned>(input: impl Serialize) -> Result<ToolInput<T>, String> {
        serde_json::to_value(input)
            .map(ToolInput::ready)
            .map_err(|error| format!("Could not prepare read request: {error}"))
    }
}

impl AgentTool for ReadTool {
    type Input = ReadToolInput;
    type Output = LanguageModelToolResultContent;

    const NAME: &'static str = "read";

    fn kind() -> acp::ToolKind {
        acp::ToolKind::Read
    }

    fn initial_title(
        &self,
        input: Result<Self::Input, serde_json::Value>,
        _cx: &mut App,
    ) -> SharedString {
        input
            .map(|input| format!("Read `{}`", input.path).into())
            .unwrap_or_else(|_| "Read".into())
    }

    fn run(
        self: Arc<Self>,
        input: ToolInput<Self::Input>,
        event_stream: ToolCallEventStream,
        cx: &mut App,
    ) -> Task<Result<Self::Output, Self::Output>> {
        cx.spawn(async move |cx| {
            let input = input
                .recv()
                .await
                .map_err(|error| LanguageModelToolResultContent::from(error.to_string()))?;

            let matching_skill = cx.update(|cx| {
                (self.skills)(cx)
                    .iter()
                    .find(|skill| {
                        !skill.disable_model_invocation
                            && (input.path == skill.skill_file_path.to_string_lossy()
                                || input.path == format!("skill:{}", skill.name))
                    })
                    .map(|skill| skill.name.clone())
            });
            if let Some(name) = matching_skill {
                let value = Self::tool_input(SkillToolInput { name })
                    .map_err(LanguageModelToolResultContent::from)?;
                let task = cx.update(|cx| self.skill_tool.clone().run(value, event_stream, cx));
                return task.await.map(Into::into).map_err(Into::into);
            }

            if input.path.starts_with("tool:") || input.path.starts_with("terminal:") {
                return self
                    .artifacts
                    .read(
                        ReadToolResultArtifactToolInput {
                            artifact: input.path,
                            offset: input.offset,
                            limit: input.limit,
                        },
                        &event_stream,
                    )
                    .map(Into::into)
                    .map_err(Into::into);
            }

            if let Some(session_id) = input
                .path
                .strip_prefix("thread:")
                .or_else(|| input.path.strip_prefix("session:"))
                .or_else(|| input.path.strip_prefix("agent:"))
                .or_else(|| input.path.strip_prefix("delegate:"))
            {
                let task = cx.update(|cx| {
                    self.transcripts.read(
                        ReadSubagentTranscriptToolInput {
                            session_id: acp::SessionId::from(session_id.to_owned()),
                            offset: input.offset,
                            limit: input.limit,
                            detail: input.detail,
                        },
                        &event_stream,
                        cx,
                    )
                });
                return task.await.map(Into::into).map_err(Into::into);
            }

            let start_line = input.offset.map(|offset| offset.max(1));
            let limit = input
                .limit
                .map(|limit| limit.clamp(1, MAX_FILE_LINE_LIMIT))
                .or_else(|| start_line.map(|_| DEFAULT_FILE_LINE_LIMIT));
            let end_line = limit.map(|limit| {
                start_line
                    .unwrap_or(1)
                    .saturating_add(limit)
                    .saturating_sub(1)
                    .min(u32::MAX as usize) as u32
            });
            let value = Self::tool_input(ReadFileToolInput {
                path: input.path,
                start_line: start_line.map(|line| line.min(u32::MAX as usize) as u32),
                end_line,
            })
            .map_err(LanguageModelToolResultContent::from)?;
            let task = cx.update(|cx| self.files.clone().run(value, event_stream, cx));
            task.await
        })
    }
}
