//! JSON-RPC and structured tool-error mapping for SCV.

use agent_client_protocol::schema::v1::{Error, ErrorCode};
use serde::Serialize;
use thiserror::Error;

/// Stable machine-readable tool error codes from the SCV v0.1 contract.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolErrorCode {
    InvalidParams,
    PathNotAllowed,
    NotFound,
    NotRegularFile,
    InvalidText,
    ReadFailed,
    ResponseTooLarge,
}

impl ToolErrorCode {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::InvalidParams => "invalid_params",
            Self::PathNotAllowed => "path_not_allowed",
            Self::NotFound => "not_found",
            Self::NotRegularFile => "not_regular_file",
            Self::InvalidText => "invalid_text",
            Self::ReadFailed => "read_failed",
            Self::ResponseTooLarge => "response_too_large",
        }
    }
}

/// Public-safe structured tool failure. Never includes file content or OS detail.
#[derive(Debug, Clone, PartialEq, Eq, Error, Serialize)]
#[error("{message}")]
pub struct ToolError {
    pub code: ToolErrorCode,
    pub message: String,
    pub path: String,
}

impl ToolError {
    pub fn new(code: ToolErrorCode, message: impl Into<String>, path: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
            path: path.into(),
        }
    }

    pub fn invalid_params(message: impl Into<String>, path: impl Into<String>) -> Self {
        Self::new(ToolErrorCode::InvalidParams, message, path)
    }

    pub fn path_not_allowed(path: impl Into<String>) -> Self {
        let path = path.into();
        Self::new(
            ToolErrorCode::PathNotAllowed,
            "Path is outside configured read roots",
            path,
        )
    }

    pub fn not_found(path: impl Into<String>) -> Self {
        let path = path.into();
        Self::new(ToolErrorCode::NotFound, "File not found", path)
    }

    pub fn not_regular_file(path: impl Into<String>) -> Self {
        let path = path.into();
        Self::new(
            ToolErrorCode::NotRegularFile,
            "Path is not a regular file",
            path,
        )
    }

    pub fn invalid_text(path: impl Into<String>) -> Self {
        let path = path.into();
        Self::new(
            ToolErrorCode::InvalidText,
            "File is not valid UTF-8 text",
            path,
        )
    }

    pub fn read_failed(path: impl Into<String>) -> Self {
        let path = path.into();
        Self::new(ToolErrorCode::ReadFailed, "Failed to read file", path)
    }

    pub fn response_too_large(path: impl Into<String>) -> Self {
        let path = path.into();
        Self::new(
            ToolErrorCode::ResponseTooLarge,
            "No complete line fits within the response size bound",
            path,
        )
    }

    /// Wire shape for tool-call `rawOutput` and JSON-RPC `error.data`.
    pub fn to_json(&self) -> serde_json::Value {
        serde_json::json!({
            "code": self.code.as_str(),
            "message": self.message,
            "path": self.path,
        })
    }

    /// Map a tool error onto a JSON-RPC error response.
    ///
    /// - `invalid_params` → `-32602`
    /// - all other tool codes → application error `-32001` with structured `data`
    pub fn to_jsonrpc(&self) -> Error {
        let error = match self.code {
            ToolErrorCode::InvalidParams => Error::invalid_params(),
            _ => Error::new(APPLICATION_TOOL_ERROR_CODE, self.message.clone()),
        };
        error.data(self.to_json())
    }
}

/// Application-level JSON-RPC code used for non-params tool failures.
pub const APPLICATION_TOOL_ERROR_CODE: i32 = -32001;

/// Lifecycle error when a request arrives before successful `initialize`.
pub fn not_initialized_error() -> Error {
    Error::invalid_request().data(serde_json::json!({
        "code": "not_initialized",
        "message": "Request received before successful initialize",
    }))
}

/// JSON-RPC `-32600` invalid envelope.
pub fn invalid_envelope_error() -> Error {
    Error::invalid_request()
}

/// JSON-RPC `-32601` method / unknown tool.
pub fn method_not_found_error() -> Error {
    Error::method_not_found()
}

/// JSON-RPC `-32602` invalid params.
pub fn invalid_params_error(data: impl Into<Option<serde_json::Value>>) -> Error {
    let error = Error::invalid_params();
    match data.into() {
        Some(value) => error.data(value),
        None => error,
    }
}

pub fn error_code_i32(error: &Error) -> i32 {
    match error.code {
        ErrorCode::ParseError => -32700,
        ErrorCode::InvalidRequest => -32600,
        ErrorCode::MethodNotFound => -32601,
        ErrorCode::InvalidParams => -32602,
        ErrorCode::InternalError => -32603,
        ErrorCode::RequestCancelled => -32800,
        ErrorCode::AuthRequired => -32000,
        ErrorCode::ResourceNotFound => -32002,
        ErrorCode::Other(code) => code,
        // ErrorCode is non_exhaustive; preserve unknown codes as-is when possible.
        other => i32::from(other),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_each_tool_error_code_to_jsonrpc() {
        let cases = [
            (
                ToolError::invalid_params("bad", "/a"),
                -32602,
                "invalid_params",
            ),
            (
                ToolError::path_not_allowed("/a"),
                APPLICATION_TOOL_ERROR_CODE,
                "path_not_allowed",
            ),
            (
                ToolError::not_found("/a"),
                APPLICATION_TOOL_ERROR_CODE,
                "not_found",
            ),
            (
                ToolError::not_regular_file("/a"),
                APPLICATION_TOOL_ERROR_CODE,
                "not_regular_file",
            ),
            (
                ToolError::invalid_text("/a"),
                APPLICATION_TOOL_ERROR_CODE,
                "invalid_text",
            ),
            (
                ToolError::read_failed("/a"),
                APPLICATION_TOOL_ERROR_CODE,
                "read_failed",
            ),
            (
                ToolError::response_too_large("/a"),
                APPLICATION_TOOL_ERROR_CODE,
                "response_too_large",
            ),
        ];

        for (tool_error, expected_code, expected_name) in cases {
            let jsonrpc = tool_error.to_jsonrpc();
            assert_eq!(error_code_i32(&jsonrpc), expected_code);
            let data = jsonrpc.data.expect("tool error data");
            assert_eq!(data["code"], expected_name);
            assert_eq!(data["path"], "/a");
            assert!(data.get("message").is_some());
            let message = jsonrpc.message;
            assert!(!message.contains('\0'));
            assert!(!message.contains("Permission denied"));
        }
    }

    #[test]
    fn standard_jsonrpc_codes() {
        assert_eq!(error_code_i32(&invalid_envelope_error()), -32600);
        assert_eq!(error_code_i32(&method_not_found_error()), -32601);
        assert_eq!(error_code_i32(&invalid_params_error(None)), -32602);
        assert_eq!(error_code_i32(&not_initialized_error()), -32600);
    }
}
