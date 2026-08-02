//! Framed protocol types for `openagents.omega.effectd.v1`.

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::all_work::generated::ProtocolInitializeResult as AllWorkProtocolInitializeResult;

pub const PROTOCOL_SCHEMA: &str = "openagents.omega.effectd.v1";
pub const PROTOCOL_VERSION: u32 = 1;
pub const SERVICE_VERSION: &str = "0.1.0";
/// Newline-framed JSON frames must stay under this byte budget (spec §8).
pub const MAX_FRAME_BYTES: usize = 64 * 1024;

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
    IncompatibleVersion,
    NotFound,
    Unavailable,
    StaleCursor,
    Gap,
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
    SarahSessionStatus,
    SarahBootstrap,
    SarahRoomSnapshot,
    SarahSendMessage,
    SarahInterruptTurn,
    SarahDeviceGrants,
    SarahRenewDeviceGrant,
    SarahRevokeDeviceGrant,
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
    /// Formatted for display. Never re-parsed to derive a duration.
    pub updated_at: String,
    /// OMEGA-MOB-31-03 (omega#47): the host's own numeric record of when the
    /// run began, in epoch milliseconds. `None` when this host never recorded
    /// one — a run that predates the field, or one that has not started. The
    /// mobile projection refuses such a run rather than reporting a zero
    /// unattended duration.
    #[serde(default)]
    pub started_at_ms: Option<u64>,
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
    #[serde(default)]
    pub all_work: Option<AllWorkProtocolInitializeResult>,
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
