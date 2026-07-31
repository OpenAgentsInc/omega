use agent_client_protocol::schema::v1::{
    AgentCapabilities, ContentBlock, ContentChunk, InitializeRequest, InitializeResponse,
    NewSessionRequest, NewSessionResponse, PromptRequest, PromptResponse, SessionNotification,
    SessionUpdate, StopReason,
};
use agent_client_protocol::{Agent, Result, Stdio};
use anyhow::Context as _;
use serde::Deserialize;
use std::sync::atomic::{AtomicU64, Ordering};

const DEFAULT_READ_LIMIT: usize = 2_000;

#[derive(Debug, Deserialize, PartialEq, Eq)]
pub struct ReadInput {
    pub path: String,
    #[serde(default)]
    pub start_line: Option<usize>,
    #[serde(default)]
    pub end_line: Option<usize>,
    #[serde(default)]
    pub limit: Option<usize>,
}

pub fn read(input: &ReadInput) -> anyhow::Result<String> {
    let text = std::fs::read_to_string(&input.path)
        .with_context(|| format!("failed to read {}", input.path))?;
    Ok(format_lines(&text, input))
}

pub fn format_lines(text: &str, input: &ReadInput) -> String {
    let start_line = input.start_line.unwrap_or(1).max(1);
    let end_line = input.end_line.unwrap_or(usize::MAX);
    let limit = input.limit.unwrap_or(DEFAULT_READ_LIMIT);

    text.lines()
        .enumerate()
        .filter_map(|(index, line)| {
            let line_number = index + 1;
            (line_number >= start_line && line_number <= end_line)
                .then_some(format!("{line_number}: {line}"))
        })
        .take(limit)
        .collect::<Vec<_>>()
        .join("\n")
}

pub fn initialize(request: InitializeRequest) -> InitializeResponse {
    InitializeResponse::new(request.protocol_version).agent_capabilities(AgentCapabilities::new())
}

pub fn new_session(request: NewSessionRequest, sequence: &AtomicU64) -> NewSessionResponse {
    let session_number = sequence.fetch_add(1, Ordering::Relaxed);
    NewSessionResponse::new(format!("scv-{session_number}")).meta(request.meta)
}

pub fn prompt_response(request: &PromptRequest) -> (String, PromptResponse) {
    let response = request
        .prompt
        .iter()
        .find_map(|block| match block {
            ContentBlock::Text(text) => Some(text.text.as_str()),
            _ => None,
        })
        .map(handle_prompt_text)
        .unwrap_or_else(|| {
            "SCV accepts a JSON read request, for example: {\"path\": \"Cargo.toml\"}.".to_owned()
        });
    (response, PromptResponse::new(StopReason::EndTurn))
}

fn handle_prompt_text(text: &str) -> String {
    match serde_json::from_str::<ReadInput>(text) {
        Ok(input) => match read(&input) {
            Ok(output) => output,
            Err(error) => format!("read failed: {error:#}"),
        },
        Err(_) => {
            "SCV accepts a JSON read request, for example: {\"path\": \"Cargo.toml\"}.".to_owned()
        }
    }
}

pub async fn serve() -> Result<()> {
    let sequence = AtomicU64::new(1);
    Agent
        .builder()
        .name("scv")
        .on_receive_request(
            async move |request: InitializeRequest, responder, _connection| {
                responder.respond(initialize(request))
            },
            agent_client_protocol::on_receive_request!(),
        )
        .on_receive_request(
            async move |request: NewSessionRequest, responder, _connection| {
                responder.respond(new_session(request, &sequence))
            },
            agent_client_protocol::on_receive_request!(),
        )
        .on_receive_request(
            async move |request: PromptRequest, responder, connection| {
                let session_id = request.session_id.clone();
                let (output, response) = prompt_response(&request);
                connection.send_notification(SessionNotification::new(
                    session_id,
                    SessionUpdate::AgentMessageChunk(ContentChunk::new(output.into())),
                ))?;
                responder.respond(response)
            },
            agent_client_protocol::on_receive_request!(),
        )
        .connect_to(Stdio::new())
        .await
}

#[cfg(test)]
mod tests {
    use super::*;
    use agent_client_protocol::schema::{
        ProtocolVersion,
        v1::{SessionId, TextContent},
    };
    use std::fs;
    use tempfile::tempdir;

    #[test]
    fn read_formats_requested_lines_with_one_based_numbers() {
        let directory = tempdir().expect("temp directory");
        let path = directory.path().join("sample.txt");
        fs::write(&path, "one\ntwo\nthree\nfour\n").expect("write sample");
        let input = ReadInput {
            path: path.display().to_string(),
            start_line: Some(2),
            end_line: Some(4),
            limit: Some(2),
        };

        assert_eq!(read(&input).expect("read sample"), "2: two\n3: three");
    }

    #[test]
    fn handlers_initialize_create_sessions_and_answer_read_prompts() {
        let initialized = initialize(InitializeRequest::new(ProtocolVersion::V1));
        assert_eq!(initialized.protocol_version, ProtocolVersion::V1);

        let sequence = AtomicU64::new(1);
        let session = new_session(NewSessionRequest::new("/tmp"), &sequence);
        assert_eq!(session.session_id, SessionId::new("scv-1"));

        let request = PromptRequest::new(
            session.session_id,
            vec![ContentBlock::Text(TextContent::new("not a read request"))],
        );
        let (output, response) = prompt_response(&request);
        assert!(output.contains("JSON read request"));
        assert_eq!(response.stop_reason, StopReason::EndTurn);
    }

    #[test]
    fn prompt_handler_returns_read_output() {
        let directory = tempdir().expect("temp directory");
        let path = directory.path().join("sample.txt");
        fs::write(&path, "alpha\nbeta\n").expect("write sample");
        let request = PromptRequest::new(
            SessionId::new("scv-1"),
            vec![ContentBlock::Text(TextContent::new(format!(
                r#"{{"path": {:?}, "start_line": 2}}"#,
                path.to_string_lossy()
            )))],
        );

        let (output, response) = prompt_response(&request);
        assert_eq!(output, "2: beta");
        assert_eq!(response.stop_reason, StopReason::EndTurn);
    }
}
