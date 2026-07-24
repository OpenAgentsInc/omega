//! Framed protocol types for `openagents.omega.effectd.v1`.

use serde::{Deserialize, Serialize};
use serde_json::Value;

pub const PROTOCOL_SCHEMA: &str = "openagents.omega.effectd.v1";
pub const PROTOCOL_VERSION: u32 = 1;
pub const SERVICE_VERSION: &str = "0.1.0";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ProtocolErrorCode {
    StaleGeneration,
    UnknownMethod,
    InvalidRequest,
    NotRunning,
    RunNotFound,
    HostUnavailable,
    HostTimeout,
    FrameTooLarge,
    Internal,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ProtocolError {
    pub code: ProtocolErrorCode,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RequestFrame {
    pub schema: String,
    pub kind: String,
    pub id: String,
    pub generation: u64,
    pub method: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub params: Option<Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResponseFrame {
    pub schema: String,
    pub kind: String,
    pub id: String,
    pub generation: u64,
    pub ok: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<ProtocolError>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum HostMethod {
    ResolveWorkspace,
    ResolveSyncSession,
    CreateThread,
    LaneReadiness,
    DispatchTurn,
    RefreshEvidence,
    InterruptTurn,
    AppendSystemNote,
    #[serde(other)]
    Unsupported,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HostRequestFrame {
    pub schema: String,
    pub kind: String,
    pub id: String,
    pub generation: u64,
    pub method: HostMethod,
    pub params: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum HostResponseErrorCode {
    StaleGeneration,
    InvalidRequest,
    Unsupported,
    Unavailable,
    Internal,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HostResponseError {
    pub code: HostResponseErrorCode,
    pub message: String,
}

impl HostResponseError {
    pub fn unavailable(message: impl Into<String>) -> Self {
        Self {
            code: HostResponseErrorCode::Unavailable,
            message: message.into(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HostResponseFrame {
    pub schema: String,
    pub kind: String,
    pub id: String,
    pub generation: u64,
    pub ok: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<HostResponseError>,
}

impl HostResponseFrame {
    pub fn success(request: &HostRequestFrame, result: Value) -> Self {
        Self {
            schema: PROTOCOL_SCHEMA.to_string(),
            kind: "host_response".to_string(),
            id: request.id.clone(),
            generation: request.generation,
            ok: true,
            result: Some(result),
            error: None,
        }
    }

    pub fn failure(request: &HostRequestFrame, error: HostResponseError) -> Self {
        Self {
            schema: PROTOCOL_SCHEMA.to_string(),
            kind: "host_response".to_string(),
            id: request.id.clone(),
            generation: request.generation,
            ok: false,
            result: None,
            error: Some(error),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct RunSnapshot {
    pub run_ref: String,
    #[serde(default)]
    pub thread_ref: Option<String>,
    pub state: String,
    pub title: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InitializeResult {
    pub schema: String,
    pub protocol_version: u32,
    pub service_version: String,
    pub generation: u64,
    pub capabilities: Vec<String>,
    pub data_root: String,
    pub active_run_limit: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HealthResult {
    pub ok: bool,
    pub status: String,
    pub generation: u64,
    pub data_root: String,
    pub active_run_count: u32,
}

pub fn request_frame(
    id: impl Into<String>,
    generation: u64,
    method: impl Into<String>,
    params: Option<Value>,
) -> RequestFrame {
    RequestFrame {
        schema: PROTOCOL_SCHEMA.to_string(),
        kind: "request".to_string(),
        id: id.into(),
        generation,
        method: method.into(),
        params,
    }
}
