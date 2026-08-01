use std::sync::Arc;

use anyhow::Context as _;
use futures::AsyncReadExt as _;
use http_client::{AsyncBody, HttpClient, Method, Request, StatusCode};
use omega_forensics::{
    ForensicWorkerEvent, ForensicWorkerObservation, ForensicWorkerPlacement, ForensicsFailureClass,
    ForensicsFailureProjection,
};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use serde_json::Value;
use url::Url;

pub const FORENSICS_CLOUD_PATH: &str = "/api/forensics/workers";
const MAX_RESPONSE_BYTES: u64 = 512 * 1024;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ForensicsCommandIdentity {
    pub command_ref: String,
    pub idempotency_ref: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ForensicsAdmitRequest {
    pub command_ref: String,
    pub idempotency_ref: String,
    pub work_unit_ref: String,
    pub attachment_ref: String,
    pub placement_ref: String,
    pub requested_at: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ForensicsSourceBinding {
    pub run_ref: String,
    pub authority_ref: String,
    pub bundle_ref: String,
    pub coverage_ref: String,
    pub coverage_digest: String,
    pub source_digest: String,
    pub materialization_receipt_ref: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ForensicsDispatchRequest {
    pub identity: ForensicsCommandIdentity,
    pub turn_ref: String,
    pub capability_ref: String,
    pub prompt: String,
    pub source_binding: ForensicsSourceBinding,
    pub requested_at: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ForensicsCancelRequest {
    pub identity: ForensicsCommandIdentity,
    pub inspect_identity: ForensicsCommandIdentity,
    pub turn_ref: String,
    pub reason_ref: String,
    pub requested_at: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ForensicsDeleteRequest {
    pub identity: ForensicsCommandIdentity,
    pub reason_ref: String,
    pub requested_at: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ForensicsObserveRequest {
    pub identity: ForensicsCommandIdentity,
    pub after_sequence: u64,
    pub limit: u16,
    pub requested_at: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ForensicsCommandReceipt {
    pub receipt_ref: String,
    pub sandbox_ref: String,
    pub resource_generation: u64,
    pub turn_ref: Option<String>,
    pub turn_status: Option<String>,
    pub events: Vec<ForensicWorkerEvent>,
}

#[derive(Debug, thiserror::Error)]
pub enum ForensicsCloudError {
    #[error("the OpenAgents forensic route is not an admitted HTTPS origin")]
    InvalidEndpoint,
    #[error("the OpenAgents forensic request could not be sent")]
    Transport(#[source] anyhow::Error),
    #[error("the OpenAgents forensic response was invalid")]
    InvalidResponse(#[source] anyhow::Error),
    #[error("the forensic command was refused: {code}: {message}")]
    Refused { code: String, message: String },
    #[error("the forensic command failed: {code}: {message}")]
    Failed { code: String, message: String },
    #[error("the forensic run requires recovery: {code}: {message}")]
    RecoveryRequired { code: String, message: String },
}

impl ForensicsCloudError {
    pub fn failure_projection(&self, observed_at: String) -> ForensicsFailureProjection {
        let (class, code, message, retryable) = match self {
            Self::RecoveryRequired { code, message } => (
                ForensicsFailureClass::RecoveryRequired,
                code.clone(),
                message.clone(),
                true,
            ),
            Self::Refused { code, message } => (
                ForensicsFailureClass::Refused,
                code.clone(),
                message.clone(),
                false,
            ),
            Self::Failed { code, message } => (
                ForensicsFailureClass::Failed,
                code.clone(),
                message.clone(),
                false,
            ),
            Self::InvalidEndpoint | Self::Transport(_) | Self::InvalidResponse(_) => (
                ForensicsFailureClass::RecoveryRequired,
                "forensics.transport_unavailable".into(),
                self.to_string(),
                true,
            ),
        };
        ForensicsFailureProjection {
            class,
            reason_ref: format!("reason.{code}"),
            message,
            retryable,
            observed_at,
        }
    }
}

#[derive(Clone)]
pub struct ForensicsCloudClient {
    http_client: Arc<dyn HttpClient>,
    endpoint: Url,
}

impl ForensicsCloudClient {
    pub fn new(
        http_client: Arc<dyn HttpClient>,
        base_url: &str,
    ) -> Result<Self, ForensicsCloudError> {
        let base_url = Url::parse(base_url).map_err(|_| ForensicsCloudError::InvalidEndpoint)?;
        if base_url.scheme() != "https"
            || base_url.host_str() != Some("openagents.com")
            || !base_url.username().is_empty()
            || base_url.password().is_some()
            || base_url.query().is_some()
            || base_url.fragment().is_some()
        {
            return Err(ForensicsCloudError::InvalidEndpoint);
        }
        let endpoint = base_url
            .join(FORENSICS_CLOUD_PATH)
            .map_err(|_| ForensicsCloudError::InvalidEndpoint)?;
        Ok(Self {
            http_client,
            endpoint,
        })
    }

    pub async fn admit(
        &self,
        access_token: &str,
        request: ForensicsAdmitRequest,
    ) -> Result<ForensicWorkerPlacement, ForensicsCloudError> {
        self.post(access_token, &AdmittedCommand::Admit(request))
            .await
    }

    pub async fn dispatch(
        &self,
        access_token: &str,
        placement: &ForensicWorkerPlacement,
        request: ForensicsDispatchRequest,
    ) -> Result<ForensicsCommandReceipt, ForensicsCloudError> {
        let value: Value = self
            .post(
                access_token,
                &AdmittedCommand::Dispatch {
                    placement,
                    command_ref: &request.identity.command_ref,
                    idempotency_ref: &request.identity.idempotency_ref,
                    turn_ref: &request.turn_ref,
                    capability_ref: &request.capability_ref,
                    prompt: &request.prompt,
                    source_binding: &request.source_binding,
                    requested_at: &request.requested_at,
                },
            )
            .await?;
        project_broker_result(value, false)
    }

    pub async fn observe(
        &self,
        access_token: &str,
        placement: &ForensicWorkerPlacement,
        request: ForensicsObserveRequest,
    ) -> Result<ForensicWorkerObservation, ForensicsCloudError> {
        self.post(
            access_token,
            &AdmittedCommand::Observe {
                placement,
                command_ref: &request.identity.command_ref,
                idempotency_ref: &request.identity.idempotency_ref,
                after_sequence: request.after_sequence,
                limit: request.limit,
                requested_at: &request.requested_at,
            },
        )
        .await
    }

    pub async fn cancel(
        &self,
        access_token: &str,
        placement: &ForensicWorkerPlacement,
        request: ForensicsCancelRequest,
    ) -> Result<ForensicsCommandReceipt, ForensicsCloudError> {
        let value: Value = self
            .post(
                access_token,
                &AdmittedCommand::Cancel {
                    placement,
                    command_ref: &request.identity.command_ref,
                    inspect_command_ref: &request.inspect_identity.command_ref,
                    idempotency_ref: &request.identity.idempotency_ref,
                    inspect_idempotency_ref: &request.inspect_identity.idempotency_ref,
                    turn_ref: &request.turn_ref,
                    reason_ref: &request.reason_ref,
                    requested_at: &request.requested_at,
                },
            )
            .await?;
        project_broker_result(value, true)
    }

    pub async fn delete(
        &self,
        access_token: &str,
        placement: &ForensicWorkerPlacement,
        request: ForensicsDeleteRequest,
    ) -> Result<ForensicWorkerPlacement, ForensicsCloudError> {
        self.post(
            access_token,
            &AdmittedCommand::Delete {
                placement,
                command_ref: &request.identity.command_ref,
                idempotency_ref: &request.identity.idempotency_ref,
                reason_ref: &request.reason_ref,
                requested_at: &request.requested_at,
            },
        )
        .await
    }

    async fn post<Response: DeserializeOwned>(
        &self,
        access_token: &str,
        command: &impl Serialize,
    ) -> Result<Response, ForensicsCloudError> {
        if access_token.is_empty() || access_token.len() > 16 * 1024 {
            return Err(ForensicsCloudError::Refused {
                code: "authentication_required".into(),
                message: "a bounded verified OpenAgents session is required".into(),
            });
        }
        let body = serde_json::to_vec(command)
            .context("encode forensic command")
            .map_err(ForensicsCloudError::InvalidResponse)?;
        let request = Request::builder()
            .method(Method::POST)
            .uri(self.endpoint.as_str())
            .header("authorization", format!("Bearer {access_token}"))
            .header("content-type", "application/json")
            .header("content-length", body.len().to_string())
            .body(AsyncBody::from(body))
            .context("build forensic request")
            .map_err(ForensicsCloudError::InvalidResponse)?;
        let mut response = self
            .http_client
            .send(request)
            .await
            .context("send forensic request")
            .map_err(ForensicsCloudError::Transport)?;
        let status = response.status();
        let mut body = Vec::new();
        response
            .body_mut()
            .take(MAX_RESPONSE_BYTES)
            .read_to_end(&mut body)
            .await
            .context("read forensic response")
            .map_err(ForensicsCloudError::Transport)?;
        if !status.is_success() {
            return Err(route_error(status, &body));
        }
        let envelope: RouteEnvelope<Response> = serde_json::from_slice(&body)
            .context("decode forensic response")
            .map_err(ForensicsCloudError::InvalidResponse)?;
        Ok(envelope.result)
    }
}

#[derive(Serialize)]
#[serde(tag = "_tag")]
enum AdmittedCommand<'a> {
    Admit(ForensicsAdmitRequest),
    Dispatch {
        placement: &'a ForensicWorkerPlacement,
        #[serde(rename = "commandRef")]
        command_ref: &'a str,
        #[serde(rename = "idempotencyRef")]
        idempotency_ref: &'a str,
        #[serde(rename = "turnRef")]
        turn_ref: &'a str,
        #[serde(rename = "capabilityRef")]
        capability_ref: &'a str,
        prompt: &'a str,
        #[serde(rename = "sourceBinding")]
        source_binding: &'a ForensicsSourceBinding,
        #[serde(rename = "requestedAt")]
        requested_at: &'a str,
    },
    Observe {
        placement: &'a ForensicWorkerPlacement,
        #[serde(rename = "commandRef")]
        command_ref: &'a str,
        #[serde(rename = "idempotencyRef")]
        idempotency_ref: &'a str,
        #[serde(rename = "afterSequence")]
        after_sequence: u64,
        limit: u16,
        #[serde(rename = "requestedAt")]
        requested_at: &'a str,
    },
    Cancel {
        placement: &'a ForensicWorkerPlacement,
        #[serde(rename = "commandRef")]
        command_ref: &'a str,
        #[serde(rename = "inspectCommandRef")]
        inspect_command_ref: &'a str,
        #[serde(rename = "idempotencyRef")]
        idempotency_ref: &'a str,
        #[serde(rename = "inspectIdempotencyRef")]
        inspect_idempotency_ref: &'a str,
        #[serde(rename = "turnRef")]
        turn_ref: &'a str,
        #[serde(rename = "reasonRef")]
        reason_ref: &'a str,
        #[serde(rename = "requestedAt")]
        requested_at: &'a str,
    },
    Delete {
        placement: &'a ForensicWorkerPlacement,
        #[serde(rename = "commandRef")]
        command_ref: &'a str,
        #[serde(rename = "idempotencyRef")]
        idempotency_ref: &'a str,
        #[serde(rename = "reasonRef")]
        reason_ref: &'a str,
        #[serde(rename = "requestedAt")]
        requested_at: &'a str,
    },
}

#[derive(Deserialize)]
struct RouteEnvelope<Response> {
    result: Response,
}

#[derive(Deserialize)]
struct RouteError {
    error: String,
    message: Option<String>,
    retryable: Option<bool>,
}

fn route_error(status: StatusCode, body: &[u8]) -> ForensicsCloudError {
    let decoded = serde_json::from_slice::<RouteError>(body).unwrap_or(RouteError {
        error: format!("http_{}", status.as_u16()),
        message: None,
        retryable: None,
    });
    let message = decoded
        .message
        .unwrap_or_else(|| "OpenAgents did not admit the forensic command".into());
    if status == StatusCode::SERVICE_UNAVAILABLE || decoded.retryable == Some(true) {
        ForensicsCloudError::RecoveryRequired {
            code: decoded.error,
            message,
        }
    } else if status == StatusCode::CONFLICT
        || status == StatusCode::BAD_REQUEST
        || status == StatusCode::UNAUTHORIZED
        || status == StatusCode::FORBIDDEN
    {
        ForensicsCloudError::Refused {
            code: decoded.error,
            message,
        }
    } else {
        ForensicsCloudError::Failed {
            code: decoded.error,
            message,
        }
    }
}

fn project_broker_result(
    value: Value,
    require_settlement: bool,
) -> Result<ForensicsCommandReceipt, ForensicsCloudError> {
    let result: BrokerResult = serde_json::from_value(value)
        .context("decode forensic broker result")
        .map_err(ForensicsCloudError::InvalidResponse)?;
    if require_settlement
        && !result.turn.as_ref().is_some_and(|turn| {
            matches!(turn.status.as_str(), "settled" | "failed" | "interrupted")
        })
    {
        return Err(ForensicsCloudError::RecoveryRequired {
            code: "cancellation_not_settled".into(),
            message: "forensic cancellation did not structurally settle".into(),
        });
    }
    Ok(ForensicsCommandReceipt {
        receipt_ref: result.receipt.receipt_ref,
        sandbox_ref: result.resource.sandbox_ref,
        resource_generation: result.resource.resource_generation,
        turn_ref: result.turn.as_ref().map(|turn| turn.turn_ref.clone()),
        turn_status: result.turn.as_ref().map(|turn| turn.status.clone()),
        events: result
            .events
            .into_iter()
            .map(|event| ForensicWorkerEvent {
                event_ref: event.event_ref,
                kind: event.kind,
                sequence: event.sequence,
                resource_generation: event.resource_generation,
                observed_at: event.observed_at,
                turn_ref: event.turn_ref,
            })
            .collect(),
    })
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct BrokerResult {
    resource: BrokerResource,
    receipt: BrokerReceipt,
    turn: Option<BrokerTurn>,
    events: Vec<BrokerEvent>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct BrokerResource {
    sandbox_ref: String,
    resource_generation: u64,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct BrokerReceipt {
    receipt_ref: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct BrokerTurn {
    turn_ref: String,
    status: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct BrokerEvent {
    event_ref: String,
    #[serde(rename = "_tag")]
    kind: String,
    sequence: u64,
    resource_generation: u64,
    observed_at: String,
    turn_ref: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use http_client::{FakeHttpClient, Response};

    fn placement_json() -> Value {
        serde_json::json!({
            "schema": "openagents.forensic_worker_placement.v1",
            "placementRef": "placement.forensic.fixture",
            "ownerRef": "owner.forensic.fixture",
            "tenantRef": "owner.forensic.fixture",
            "workUnitRef": "work.forensic.fixture",
            "sandboxRef": "sandbox.forensic.fixture",
            "attachmentGeneration": 1,
            "resourceGeneration": 1,
            "targetClass": "openagents_managed",
            "provider": "google_cloud",
            "adapterRef": "adapter.oa-codex-control.gce.v1",
            "isolation": "gce_vm",
            "regionRef": "region.google-cloud.us-central1",
            "imageDigest": format!("sha256:{}", "a".repeat(64)),
            "profileDigest": format!("sha256:{}", "b".repeat(64)),
            "networkPolicyRef": "network-policy-ref://openagents/managed-sandbox/broker-only-v1",
            "leaseRef": "lease.forensic.fixture",
            "budgetRef": "budget.forensic.worker.initial.v1",
            "capabilityRefs": ["capability.forensic.fixture.agent_turn"],
            "state": "worker_ready",
            "admissionReceiptRef": "receipt.forensic.admission",
            "readinessReceiptRef": "receipt.forensic.readiness",
            "updatedAt": "2026-08-01T10:00:00.000Z"
        })
    }

    fn placement() -> ForensicWorkerPlacement {
        serde_json::from_value(placement_json()).expect("fixture placement")
    }

    #[test]
    fn client_refuses_substitute_cloud_origins() {
        let http = FakeHttpClient::with_404_response();
        assert!(ForensicsCloudClient::new(http, "https://example.com").is_err());
    }

    #[test]
    fn client_posts_admission_only_to_the_native_forensic_route() {
        smol::block_on(async {
            let response = serde_json::to_vec(&serde_json::json!({
                "result": placement_json()
            }))
            .expect("fixture response");
            let http = FakeHttpClient::create(move |mut request| {
                let response = response.clone();
                async move {
                    assert_eq!(request.method(), Method::POST);
                    assert_eq!(request.uri().path(), FORENSICS_CLOUD_PATH);
                    assert_eq!(
                        request
                            .headers()
                            .get("authorization")
                            .and_then(|value| value.to_str().ok()),
                        Some("Bearer test-session")
                    );
                    let mut request_body = Vec::new();
                    request
                        .body_mut()
                        .read_to_end(&mut request_body)
                        .await
                        .expect("request body");
                    let command: Value =
                        serde_json::from_slice(&request_body).expect("forensic command");
                    assert_eq!(command["_tag"], "Admit");
                    assert_eq!(command["commandRef"], "command.forensic.fixture");
                    Ok(Response::builder()
                        .status(200)
                        .body(AsyncBody::from(response))?)
                }
            });
            let client = ForensicsCloudClient::new(http, "https://openagents.com")
                .expect("admitted endpoint");
            let placement = client
                .admit(
                    "test-session",
                    ForensicsAdmitRequest {
                        command_ref: "command.forensic.fixture".into(),
                        idempotency_ref: "idempotency.forensic.fixture".into(),
                        work_unit_ref: "work.forensic.fixture".into(),
                        attachment_ref: "attachment.forensic.fixture".into(),
                        placement_ref: "placement.forensic.fixture".into(),
                        requested_at: "2026-08-01T10:00:00.000Z".into(),
                    },
                )
                .await
                .expect("admitted placement");
            placement.validate().expect("valid placement");
        });
    }

    #[test]
    fn broker_outage_and_unmeasured_budget_are_recovery_or_refusal() {
        let recovery = route_error(
            StatusCode::SERVICE_UNAVAILABLE,
            br#"{"error":"upstream_unavailable","message":"broker offline","retryable":true}"#,
        );
        assert!(matches!(
            recovery,
            ForensicsCloudError::RecoveryRequired { .. }
        ));
        let refused = route_error(
            StatusCode::CONFLICT,
            br#"{"error":"conflict","message":"measured cost required","retryable":false}"#,
        );
        assert!(matches!(refused, ForensicsCloudError::Refused { .. }));
    }

    #[test]
    fn observation_resumes_from_the_exact_native_cursor() {
        smol::block_on(async {
            let response = serde_json::to_vec(&serde_json::json!({
                "result": {
                    "schema": "openagents.forensic_worker_observation.v1",
                    "placementRef": "placement.forensic.fixture",
                    "sandboxRef": "sandbox.forensic.fixture",
                    "resourceGeneration": 1,
                    "lifecycle": "ready",
                    "cleanupComplete": false,
                    "turn": null,
                    "events": [],
                    "afterSequence": 7,
                    "nextSequence": 7,
                    "terminalSequence": 7,
                    "hasMore": false,
                    "silenceIsTerminal": false
                }
            }))
            .expect("fixture response");
            let http = FakeHttpClient::create(move |mut request| {
                let response = response.clone();
                async move {
                    let mut request_body = Vec::new();
                    request
                        .body_mut()
                        .read_to_end(&mut request_body)
                        .await
                        .expect("request body");
                    let command: Value =
                        serde_json::from_slice(&request_body).expect("forensic command");
                    assert_eq!(command["_tag"], "Observe");
                    assert_eq!(command["afterSequence"], 7);
                    assert_eq!(command["limit"], 128);
                    Ok(Response::builder()
                        .status(200)
                        .body(AsyncBody::from(response))?)
                }
            });
            let client = ForensicsCloudClient::new(http, "https://openagents.com")
                .expect("admitted endpoint");
            let observation = client
                .observe(
                    "test-session",
                    &placement(),
                    ForensicsObserveRequest {
                        identity: ForensicsCommandIdentity {
                            command_ref: "command.forensic.observe.fixture".into(),
                            idempotency_ref: "idempotency.forensic.observe.fixture".into(),
                        },
                        after_sequence: 7,
                        limit: 128,
                        requested_at: "2026-08-01T10:00:07.000Z".into(),
                    },
                )
                .await
                .expect("observation");
            assert_eq!(observation.after_sequence, 7);
            assert!(!observation.silence_is_terminal);
        });
    }

    #[test]
    fn cancellation_requires_structural_settlement() {
        let value = serde_json::json!({
            "resource": {
                "sandboxRef": "sandbox.forensic.fixture",
                "resourceGeneration": 1
            },
            "receipt": { "receiptRef": "receipt.forensic.cancel.fixture" },
            "turn": {
                "turnRef": "turn.forensic.fixture",
                "status": "running"
            },
            "events": []
        });
        assert!(matches!(
            project_broker_result(value, true),
            Err(ForensicsCloudError::RecoveryRequired { code, .. })
                if code == "cancellation_not_settled"
        ));
    }
}
