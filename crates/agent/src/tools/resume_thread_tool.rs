use agent_client_protocol::schema::v1 as acp;
use anyhow::Result;
use gpui::{App, SharedString, Task};
use language_model::LanguageModelToolResultContent;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::rc::Rc;
use std::sync::Arc;

use crate::{AgentTool, ResumeThreadRequest, ThreadEnvironment, ToolCallEventStream, ToolInput};

/// Continue a persisted Omega thread and return its next response.
///
/// Use this when the user asks to return to earlier work, continue a task by
/// session ID, or branch from an earlier point in a conversation. The target
/// thread keeps its original model, prompt prefix, tool order, and saved tool
/// result references.
///
/// Set `fork` when the original thread must remain at its current cursor. A
/// fork receives a new session ID and preserves the selected history prefix.
/// Without `fork`, selecting an earlier event or message moves the original
/// thread's active cursor while retaining the abandoned branch in its event
/// log.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub struct ResumeThreadToolInput {
    /// Harness that owns the session. Use `omega` for native Omega threads, or
    /// an installed ACP agent ID such as `codex-acp`, `claude-acp`, or `grok`.
    pub harness: String,

    /// Native session ID assigned by the selected harness.
    pub session_id: String,

    /// New instruction to send after restoring the selected thread state.
    pub prompt: String,

    /// Fork into a new session instead of continuing the original session.
    #[serde(default)]
    pub fork: bool,

    /// Optional append-only event sequence to resume or fork from.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub event_sequence: Option<u64>,

    /// Optional zero-based visible message index to resume or fork from.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message_index: Option<usize>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum ResumeThreadToolOutput {
    Success {
        harness: String,
        session_id: String,
        source_session_id: String,
        forked: bool,
        response: String,
    },
    Error {
        error: String,
    },
}

impl From<ResumeThreadToolOutput> for LanguageModelToolResultContent {
    fn from(output: ResumeThreadToolOutput) -> Self {
        serde_json::to_string(&output)
            .unwrap_or_else(|error| format!("Failed to serialize resume_thread output: {error}"))
            .into()
    }
}

pub struct ResumeThreadTool {
    environment: Rc<dyn ThreadEnvironment>,
}

impl ResumeThreadTool {
    pub fn new(environment: Rc<dyn ThreadEnvironment>) -> Self {
        Self { environment }
    }
}

impl AgentTool for ResumeThreadTool {
    type Input = ResumeThreadToolInput;
    type Output = ResumeThreadToolOutput;

    const NAME: &'static str = "resume_thread";

    fn kind() -> acp::ToolKind {
        acp::ToolKind::Other
    }

    fn initial_title(
        &self,
        input: Result<Self::Input, serde_json::Value>,
        _cx: &mut App,
    ) -> SharedString {
        match input {
            Ok(input) if input.fork => format!("Fork thread {}", input.session_id).into(),
            Ok(input) => format!("Resume thread {}", input.session_id).into(),
            Err(_) => "Resume thread".into(),
        }
    }

    fn run(
        self: Arc<Self>,
        input: ToolInput<Self::Input>,
        _event_stream: ToolCallEventStream,
        cx: &mut App,
    ) -> Task<Result<Self::Output, Self::Output>> {
        cx.spawn(async move |cx| {
            let input = input
                .recv()
                .await
                .map_err(|error| ResumeThreadToolOutput::Error {
                    error: format!("Failed to receive tool input: {error}"),
                })?;

            if input.event_sequence.is_some() && input.message_index.is_some() {
                return Err(ResumeThreadToolOutput::Error {
                    error: "Specify either event_sequence or message_index, not both".into(),
                });
            }

            let source_session_id = input.session_id.clone();
            let harness = input.harness.clone();
            let request = ResumeThreadRequest {
                harness: input.harness,
                session_id: acp::SessionId::new(input.session_id),
                prompt: input.prompt,
                fork: input.fork,
                event_sequence: input.event_sequence,
                message_index: input.message_index,
            };

            match self.environment.resume_thread(request, cx).await {
                Ok(result) => Ok(ResumeThreadToolOutput::Success {
                    harness,
                    session_id: result.session_id.to_string(),
                    source_session_id,
                    forked: result.forked,
                    response: result.response,
                }),
                Err(error) => Err(ResumeThreadToolOutput::Error {
                    error: error.to_string(),
                }),
            }
        })
    }
}
