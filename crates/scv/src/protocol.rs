//! Pinned ACP envelope and types adapter for SCV v0.1.

use agent_client_protocol::schema::ProtocolVersion;
use agent_client_protocol::schema::v1::{
    AgentCapabilities, AvailableCommand, AvailableCommandInput, AvailableCommandsUpdate,
    ContentBlock, Implementation, InitializeRequest, InitializeResponse, NewSessionRequest,
    NewSessionResponse, PromptRequest, SessionId, SessionNotification, SessionUpdate, TextContent,
    ToolCall, ToolCallContent, ToolCallStatus, ToolCallUpdate, ToolCallUpdateFields, ToolKind,
    UnstructuredCommandInput,
};
use serde::Deserialize;
use serde_json::Value;

use crate::error::{ToolError, method_not_found_error};
use crate::read::{ReadInput, ReadOutput, parse_read_input};

pub const AGENT_NAME: &str = "scv";
pub const AGENT_TITLE: &str = "SCV";
pub const AGENT_VERSION: &str = env!("CARGO_PKG_VERSION");
pub const READ_TOOL_NAME: &str = "read";

/// The request SCV answers, as a caller must write it.
///
/// OMEGA-DELTA-0209. One string, used three times: it is the hint on the
/// `available_commands` update a client sees at `session/new`, it is what a
/// refusal names, and `omega_agent_detect::SCV_REQUEST` is checked against it
/// so the shape Omega tells a delegating model to emit is this one and not a
/// copy that drifted.
pub const PROMPT_REQUEST_SHAPE: &str =
    r#"{"tool":"read","arguments":{"path":"/absolute/path","offset":1,"limit":2000}}"#;

/// Tool request envelope accepted in `session/prompt` text blocks.
///
/// ACP v1 has no client→agent tool-call method. SCV accepts JSON text of either:
/// - `{"tool":"read","arguments":{...}}` (or `"name"` instead of `"tool"`)
/// - a bare `read` input object `{"path":...,"offset":...,"limit":...}`
#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
pub enum PromptToolRequest {
    Named(NamedToolRequest),
    BareRead(ReadInput),
}

#[derive(Debug, Clone, Deserialize)]
pub struct NamedToolRequest {
    #[serde(alias = "name")]
    pub tool: String,
    #[serde(default)]
    pub arguments: Value,
}

impl PromptToolRequest {
    /// OMEGA-DELTA-0209. Parse the prompt text as a tool request.
    ///
    /// One Markdown code fence around the whole text is removed first. A caller
    /// with a model behind it emits JSON in a fence often enough that refusing
    /// it would be refusing the right request over its wrapping, and removing a
    /// fence is an unwrapping rather than a reading — SCV has no model and
    /// interprets nothing.
    ///
    /// A refusal names the shape. `invalid tool request: expected value at line
    /// 1 column 1` was what a prose prompt produced, and it tells the reader
    /// neither what SCV wanted nor that it wanted anything in particular.
    pub fn parse(text: &str) -> Result<Self, ToolError> {
        let text = strip_one_code_fence(text.trim()).trim();
        serde_json::from_str(text).map_err(|error| {
            ToolError::invalid_params(
                format!(
                    "invalid tool request: {error}. SCV has no model and takes a \
                     JSON tool request, not prose. Send: {PROMPT_REQUEST_SHAPE}"
                ),
                "",
            )
        })
    }

    pub fn into_read_input(self) -> Result<ReadInput, agent_client_protocol::Error> {
        match self {
            Self::BareRead(input) => {
                input.validate().map_err(|error| error.to_jsonrpc())?;
                Ok(input)
            }
            Self::Named(named) => {
                if named.tool != READ_TOOL_NAME {
                    return Err(method_not_found_error());
                }
                parse_read_input(&named.arguments).map_err(|error| error.to_jsonrpc())
            }
        }
    }
}

/// The body of a single Markdown code fence, or the input unchanged.
///
/// Only a fence that opens the text and closes it is removed. Text that merely
/// *contains* a fenced block is left alone: taking a block out of the middle of
/// a message would be deciding which part of it was the request, and deciding
/// is exactly what an agent with no model must not do.
fn strip_one_code_fence(text: &str) -> &str {
    let Some(rest) = text.strip_prefix("```") else {
        return text;
    };
    let Some((_language, body)) = rest.split_once('\n') else {
        return text;
    };
    match body.trim_end().strip_suffix("```") {
        Some(body) => body,
        None => text,
    }
}

/// Negotiate protocol version and advertise SCV identity + minimal capabilities.
pub fn build_initialize_response(request: InitializeRequest) -> InitializeResponse {
    let protocol_version = negotiate_protocol_version(request.protocol_version);
    let agent_info = Implementation::new(AGENT_NAME, AGENT_VERSION).title(AGENT_TITLE);
    // Default AgentCapabilities: load_session=false, prompt image/audio/embedded=false,
    // mcp http/sse=false. That is the only capability surface required to accept
    // text prompts that invoke the advertised `read` tool.
    InitializeResponse::new(protocol_version)
        .agent_capabilities(AgentCapabilities::new())
        .agent_info(agent_info)
}

fn negotiate_protocol_version(requested: ProtocolVersion) -> ProtocolVersion {
    if requested == ProtocolVersion::V1 {
        ProtocolVersion::V1
    } else {
        // Latest version SCV implements handlers for.
        ProtocolVersion::V1
    }
}

pub fn build_new_session_response(
    request: NewSessionRequest,
    session_number: u64,
) -> NewSessionResponse {
    NewSessionResponse::new(format!("scv-{session_number}")).meta(request.meta)
}

pub fn read_available_commands() -> Vec<AvailableCommand> {
    vec![AvailableCommand::new(
        READ_TOOL_NAME,
        "Read a bounded range of lines from a regular UTF-8 text file below configured read roots.",
    )
    .input(AvailableCommandInput::Unstructured(
        // OMEGA-DELTA-0209. The whole request rather than only its arguments.
        // The hint is what a client reads to learn what to send, and what it
        // sends is a tool request — a hint showing the inner object described
        // the half that is not the envelope.
        UnstructuredCommandInput::new(PROMPT_REQUEST_SHAPE),
    ))]
}

pub fn available_commands_notification(session_id: SessionId) -> SessionNotification {
    SessionNotification::new(
        session_id,
        SessionUpdate::AvailableCommandsUpdate(AvailableCommandsUpdate::new(
            read_available_commands(),
        )),
    )
}

pub fn extract_prompt_text(request: &PromptRequest) -> Option<&str> {
    request.prompt.iter().find_map(|block| match block {
        ContentBlock::Text(TextContent { text, .. }) => Some(text.as_str()),
        _ => None,
    })
}

pub fn tool_call_started(
    tool_call_id: impl Into<String>,
    input: &ReadInput,
) -> Result<ToolCall, agent_client_protocol::Error> {
    let raw_input =
        serde_json::to_value(input).map_err(agent_client_protocol::Error::into_internal_error)?;
    Ok(
        ToolCall::new(tool_call_id.into(), format!("read {}", input.path))
            .kind(ToolKind::Read)
            .status(ToolCallStatus::InProgress)
            .raw_input(raw_input),
    )
}

pub fn tool_call_completed(
    tool_call_id: impl Into<String>,
    output: &ReadOutput,
) -> Result<ToolCallUpdate, agent_client_protocol::Error> {
    let raw_output =
        serde_json::to_value(output).map_err(agent_client_protocol::Error::into_internal_error)?;
    let fields = ToolCallUpdateFields::new()
        .status(ToolCallStatus::Completed)
        .content(vec![ToolCallContent::from(output.content.clone())])
        .raw_output(raw_output);
    Ok(ToolCallUpdate::new(tool_call_id.into(), fields))
}

pub fn tool_call_failed(
    tool_call_id: impl Into<String>,
    error: &ToolError,
) -> Result<ToolCallUpdate, agent_client_protocol::Error> {
    let fields = ToolCallUpdateFields::new()
        .status(ToolCallStatus::Failed)
        .content(vec![ToolCallContent::from(error.message.clone())])
        .raw_output(error.to_json());
    Ok(ToolCallUpdate::new(tool_call_id.into(), fields))
}

pub fn session_tool_call(session_id: SessionId, tool_call: ToolCall) -> SessionNotification {
    SessionNotification::new(session_id, SessionUpdate::ToolCall(tool_call))
}

pub fn session_tool_call_update(
    session_id: SessionId,
    update: ToolCallUpdate,
) -> SessionNotification {
    SessionNotification::new(session_id, SessionUpdate::ToolCallUpdate(update))
}

#[cfg(test)]
mod tests {
    use super::*;
    use agent_client_protocol::schema::v1::ErrorCode;

    #[test]
    fn initialize_identifies_scv_and_minimal_capabilities() {
        let response = build_initialize_response(InitializeRequest::new(ProtocolVersion::V1));
        assert_eq!(response.protocol_version, ProtocolVersion::V1);
        let info = response.agent_info.expect("agent info");
        assert_eq!(info.name, "scv");
        assert_eq!(info.version, AGENT_VERSION);
        assert!(!response.agent_capabilities.load_session);
        assert!(!response.agent_capabilities.prompt_capabilities.image);
        assert!(!response.agent_capabilities.prompt_capabilities.audio);
        assert!(!response.agent_capabilities.mcp_capabilities.http);
        assert!(!response.agent_capabilities.mcp_capabilities.sse);
    }

    #[test]
    fn bare_read_and_named_tool_requests_parse() {
        let bare = PromptToolRequest::parse(r#"{"path":"/tmp/a","offset":2,"limit":3}"#)
            .expect("bare")
            .into_read_input()
            .expect("input");
        assert_eq!(bare.path, "/tmp/a");
        assert_eq!(bare.offset, 2);
        assert_eq!(bare.limit, 3);

        let named = PromptToolRequest::parse(
            r#"{"tool":"read","arguments":{"path":"/tmp/b","offset":1,"limit":1}}"#,
        )
        .expect("named")
        .into_read_input()
        .expect("input");
        assert_eq!(named.path, "/tmp/b");
    }

    #[test]
    fn unknown_tool_is_method_not_found() {
        let error = PromptToolRequest::parse(r#"{"tool":"write","arguments":{}}"#)
            .expect("parse")
            .into_read_input()
            .expect_err("unknown");
        assert!(matches!(error.code, ErrorCode::MethodNotFound));
    }

    /// OMEGA-DELTA-0209. The owner's defect, at SCV's own edge.
    ///
    /// A prose prompt still fails — SCV has no model and cannot be given one by
    /// a better error message. What changes is that the failure says what SCV
    /// wanted, so a reader (a person or the model that wrote the task) can fix
    /// it. `expected value at line 1 column 1` alone could not be acted on.
    #[test]
    fn a_prose_prompt_is_refused_by_naming_the_shape() {
        let error = PromptToolRequest::parse(
            "Perform a read-only test delegation: report the project root path \
             and list one or two top-level entries. Do not modify files.",
        )
        .expect_err("SCV cannot read prose");

        assert!(
            error.message.contains(PROMPT_REQUEST_SHAPE),
            "{}",
            error.message
        );
    }

    /// A caller with a model behind it emits JSON in a fence often enough that
    /// refusing it would be refusing the right request over its wrapping.
    #[test]
    fn one_surrounding_code_fence_is_unwrapped() {
        for fenced in [
            "```json\n{\"tool\":\"read\",\"arguments\":{\"path\":\"/tmp/a\"}}\n```",
            "```\n{\"tool\":\"read\",\"arguments\":{\"path\":\"/tmp/a\"}}\n```",
            "  ```json\n{\"tool\":\"read\",\"arguments\":{\"path\":\"/tmp/a\"}}\n```\n",
        ] {
            let input = PromptToolRequest::parse(fenced)
                .expect("a fenced request is the request")
                .into_read_input()
                .expect("input");
            assert_eq!(input.path, "/tmp/a", "{fenced}");
        }
    }

    /// And nothing is dug out of the middle of a message.
    #[test]
    fn a_request_inside_prose_is_not_extracted() {
        PromptToolRequest::parse(
            "Please run this:\n```json\n{\"tool\":\"read\",\"arguments\":{\"path\":\"/tmp/a\"}}\n```\nThanks.",
        )
        .expect_err(
            "choosing which part of a message was the request is interpretation, \
             and an agent with no model must not interpret",
        );
    }

    /// The hint a client is given is the request it must send.
    #[test]
    fn the_advertised_hint_is_the_whole_request() {
        let commands = read_available_commands();
        let AvailableCommandInput::Unstructured(input) = commands[0]
            .input
            .clone()
            .expect("the read command has a hint")
        else {
            panic!("the hint is unstructured text");
        };
        assert_eq!(input.hint, PROMPT_REQUEST_SHAPE);
        // The hint is a request SCV itself accepts, not a sketch of one.
        assert!(PromptToolRequest::parse(PROMPT_REQUEST_SHAPE).is_ok());
    }

    #[test]
    fn only_read_in_available_commands() {
        let commands = read_available_commands();
        assert_eq!(commands.len(), 1);
        assert_eq!(commands[0].name, "read");
    }
}
