//! ACP lifecycle and JSON-RPC dispatch for SCV.

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use agent_client_protocol::schema::v1::{
    CancelNotification, InitializeRequest, InitializeResponse, NewSessionRequest,
    NewSessionResponse, PromptRequest, PromptResponse, SessionId, StopReason,
};
use agent_client_protocol::{Agent, Error, Result, Stdio};
use tracing::{debug, warn};

use crate::error::not_initialized_error;
use crate::protocol::{
    PromptToolRequest, available_commands_notification, build_initialize_response,
    build_new_session_response, extract_prompt_text, session_tool_call, session_tool_call_update,
    tool_call_completed, tool_call_failed, tool_call_started,
};
use crate::read::execute_read;
use crate::roots::ReadRoots;

/// Shared server state for one SCV process.
pub struct ScvServer {
    roots: ReadRoots,
    initialized: AtomicBool,
    session_sequence: AtomicU64,
    tool_call_sequence: AtomicU64,
}

impl ScvServer {
    pub fn new(roots: ReadRoots) -> Arc<Self> {
        Arc::new(Self {
            roots,
            initialized: AtomicBool::new(false),
            session_sequence: AtomicU64::new(1),
            tool_call_sequence: AtomicU64::new(1),
        })
    }

    pub fn roots(&self) -> &ReadRoots {
        &self.roots
    }

    pub fn is_initialized(&self) -> bool {
        self.initialized.load(Ordering::SeqCst)
    }

    fn require_initialized(&self) -> Result<()> {
        if self.is_initialized() {
            Ok(())
        } else {
            Err(not_initialized_error())
        }
    }

    pub fn handle_initialize(&self, request: InitializeRequest) -> Result<InitializeResponse> {
        let response = build_initialize_response(request);
        self.initialized.store(true, Ordering::SeqCst);
        Ok(response)
    }

    pub fn handle_new_session(
        &self,
        request: NewSessionRequest,
    ) -> Result<(NewSessionResponse, SessionId)> {
        self.require_initialized()?;
        let number = self.session_sequence.fetch_add(1, Ordering::Relaxed);
        let response = build_new_session_response(request, number);
        let session_id = response.session_id.clone();
        Ok((response, session_id))
    }

    pub fn handle_prompt(&self, request: PromptRequest) -> Result<PromptTurn> {
        self.require_initialized()?;

        let text = extract_prompt_text(&request).ok_or_else(|| {
            Error::invalid_params().data(serde_json::json!({
                "code": "invalid_params",
                "message": "prompt must include a text content block with a JSON tool request",
            }))
        })?;

        let tool_request = PromptToolRequest::parse(text).map_err(|error| error.to_jsonrpc())?;
        let input = tool_request.into_read_input()?;

        let call_number = self.tool_call_sequence.fetch_add(1, Ordering::Relaxed);
        let tool_call_id = format!("scv-read-{call_number}");

        let started = tool_call_started(&tool_call_id, &input)?;
        match execute_read(&self.roots, &input) {
            Ok(output) => {
                let completed = tool_call_completed(&tool_call_id, &output)?;
                Ok(PromptTurn {
                    session_id: request.session_id,
                    started,
                    finished: completed,
                    response: PromptResponse::new(StopReason::EndTurn),
                    error: None,
                })
            }
            Err(tool_error) => {
                let failed = tool_call_failed(&tool_call_id, &tool_error)?;
                Ok(PromptTurn {
                    session_id: request.session_id,
                    started,
                    finished: failed,
                    response: PromptResponse::new(StopReason::EndTurn),
                    error: Some(tool_error.to_jsonrpc()),
                })
            }
        }
    }
}

/// Outcome of a prompt turn: tool-call notifications plus final response or error.
#[derive(Debug)]
pub struct PromptTurn {
    pub session_id: SessionId,
    pub started: agent_client_protocol::schema::v1::ToolCall,
    pub finished: agent_client_protocol::schema::v1::ToolCallUpdate,
    pub response: PromptResponse,
    pub error: Option<Error>,
}

/// Parse CLI arguments for SCV. Supports repeated `--read-root <PATH>`.
pub fn parse_cli_args(args: impl IntoIterator<Item = String>) -> Result<Vec<PathBuf>, String> {
    let mut roots = Vec::new();
    let mut items = args.into_iter();
    // Skip argv0 when present.
    let _binary = items.next();
    while let Some(arg) = items.next() {
        match arg.as_str() {
            "--read-root" => {
                let path = items
                    .next()
                    .ok_or_else(|| "missing value for --read-root".to_owned())?;
                roots.push(PathBuf::from(path));
            }
            "--help" | "-h" => {
                return Err(help_text());
            }
            other => {
                return Err(format!("unknown argument: {other}\n{}", help_text()));
            }
        }
    }
    Ok(roots)
}

pub fn help_text() -> String {
    "SCV v0.1 — Space Construction Vehicle (read-only ACP agent)\n\n\
     Usage: scv [--read-root <PATH>]...\n\n\
     Options:\n\
       --read-root <PATH>  Allow reading regular files under this directory\n\
                           (repeatable). Defaults to the process current directory.\n\
       -h, --help          Show this help\n"
        .to_owned()
}

/// Serve ACP over stdio until the connection closes.
pub async fn serve(roots: ReadRoots) -> Result<()> {
    let server = ScvServer::new(roots);
    let initialize_server = Arc::clone(&server);
    let session_server = Arc::clone(&server);
    let prompt_server = Arc::clone(&server);

    Agent
        .builder()
        .name(crate::protocol::AGENT_NAME)
        .on_receive_request(
            async move |request: InitializeRequest, responder, _connection| {
                debug!("initialize");
                let response = initialize_server.handle_initialize(request)?;
                responder.respond(response)
            },
            agent_client_protocol::on_receive_request!(),
        )
        .on_receive_request(
            async move |request: NewSessionRequest, responder, connection| {
                debug!("session/new");
                let (response, session_id) = session_server.handle_new_session(request)?;
                responder.respond(response)?;
                connection.send_notification(available_commands_notification(session_id))?;
                Ok(())
            },
            agent_client_protocol::on_receive_request!(),
        )
        .on_receive_request(
            async move |request: PromptRequest, responder, connection| {
                debug!(session = %request.session_id, "session/prompt");
                let turn = prompt_server.handle_prompt(request)?;
                connection
                    .send_notification(session_tool_call(turn.session_id.clone(), turn.started))?;
                connection.send_notification(session_tool_call_update(
                    turn.session_id.clone(),
                    turn.finished,
                ))?;
                if let Some(error) = turn.error {
                    responder.respond_with_error(error)?;
                } else {
                    responder.respond(turn.response)?;
                }
                Ok(())
            },
            agent_client_protocol::on_receive_request!(),
        )
        .on_receive_notification(
            async move |_notification: CancelNotification, _connection| {
                debug!("session/cancel");
                Ok(())
            },
            agent_client_protocol::on_receive_notification!(),
        )
        .connect_to(Stdio::new())
        .await
        .inspect_err(|error| {
            warn!(?error, "ACP connection ended with error");
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::{ToolErrorCode, error_code_i32};
    use agent_client_protocol::schema::ProtocolVersion;
    use agent_client_protocol::schema::v1::{ContentBlock, TextContent};
    use std::fs;
    use tempfile::tempdir;

    fn server_with_root(path: &std::path::Path) -> Arc<ScvServer> {
        let roots = ReadRoots::new([path.to_path_buf()], path).expect("roots");
        ScvServer::new(roots)
    }

    #[test]
    fn rejects_prompt_before_initialize() {
        let directory = tempdir().expect("temp");
        let server = server_with_root(directory.path());
        let request = PromptRequest::new(
            "scv-1",
            vec![ContentBlock::Text(TextContent::new(r#"{"path":"/x"}"#))],
        );
        let error = server.handle_prompt(request).expect_err("pre-init");
        assert_eq!(error_code_i32(&error), -32600);
    }

    #[test]
    fn successful_read_prompt() {
        let directory = tempdir().expect("temp");
        let file = directory.path().join("sample.txt");
        fs::write(&file, "alpha\nbeta\n").expect("write");
        let server = server_with_root(directory.path());
        server
            .handle_initialize(InitializeRequest::new(ProtocolVersion::V1))
            .expect("init");
        let request = PromptRequest::new(
            "scv-1",
            vec![ContentBlock::Text(TextContent::new(format!(
                r#"{{"path":{}}}"#,
                serde_json::to_string(&file.to_string_lossy()).expect("json")
            )))],
        );
        let turn = server.handle_prompt(request).expect("prompt");
        assert!(turn.error.is_none());
        assert_eq!(turn.response.stop_reason, StopReason::EndTurn);
    }

    #[test]
    fn path_not_allowed_returns_structured_error() {
        let directory = tempdir().expect("temp");
        let outside = tempdir().expect("outside");
        let file = outside.path().join("x.txt");
        fs::write(&file, "x\n").expect("write");
        let server = server_with_root(directory.path());
        server
            .handle_initialize(InitializeRequest::new(ProtocolVersion::V1))
            .expect("init");
        let request = PromptRequest::new(
            "scv-1",
            vec![ContentBlock::Text(TextContent::new(format!(
                r#"{{"tool":"read","arguments":{{"path":{}}}}}"#,
                serde_json::to_string(&file.to_string_lossy()).expect("json")
            )))],
        );
        let turn = server.handle_prompt(request).expect("prompt");
        let error = turn.error.expect("tool error");
        let data = error.data.expect("data");
        assert_eq!(data["code"], "path_not_allowed");
    }

    #[test]
    fn unknown_tool_is_minus_32601() {
        let directory = tempdir().expect("temp");
        let server = server_with_root(directory.path());
        server
            .handle_initialize(InitializeRequest::new(ProtocolVersion::V1))
            .expect("init");
        let request = PromptRequest::new(
            "scv-1",
            vec![ContentBlock::Text(TextContent::new(
                r#"{"tool":"shell","arguments":{}}"#,
            ))],
        );
        let error = server.handle_prompt(request).expect_err("unknown");
        assert_eq!(error_code_i32(&error), -32601);
    }

    #[test]
    fn invalid_read_params_is_minus_32602() {
        let directory = tempdir().expect("temp");
        let server = server_with_root(directory.path());
        server
            .handle_initialize(InitializeRequest::new(ProtocolVersion::V1))
            .expect("init");
        let request = PromptRequest::new(
            "scv-1",
            vec![ContentBlock::Text(TextContent::new(
                r#"{"path":"/tmp/x","limit":5000}"#,
            ))],
        );
        let error = server.handle_prompt(request).expect_err("params");
        assert_eq!(error_code_i32(&error), -32602);
        let data = error.data.expect("data");
        assert_eq!(data["code"], ToolErrorCode::InvalidParams.as_str());
    }

    #[test]
    fn parse_cli_read_roots() {
        let roots = parse_cli_args([
            "scv".into(),
            "--read-root".into(),
            "/tmp/a".into(),
            "--read-root".into(),
            "/tmp/b".into(),
        ])
        .expect("args");
        assert_eq!(roots.len(), 2);
    }
}
