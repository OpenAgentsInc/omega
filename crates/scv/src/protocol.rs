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
pub const AGENT_TITLE: &str = "Space Construction Vehicle";
pub const AGENT_VERSION: &str = env!("CARGO_PKG_VERSION");
pub const READ_TOOL_NAME: &str = "read";

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
    pub fn parse(text: &str) -> Result<Self, ToolError> {
        serde_json::from_str(text).map_err(|error| {
            ToolError::invalid_params(format!("invalid tool request: {error}"), "")
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
        UnstructuredCommandInput::new(
            r#"{"path":"/absolute/path","offset":1,"limit":2000}"#,
        ),
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

    #[test]
    fn only_read_in_available_commands() {
        let commands = read_available_commands();
        assert_eq!(commands.len(), 1);
        assert_eq!(commands[0].name, "read");
    }
}
