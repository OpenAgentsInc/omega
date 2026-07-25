use std::collections::HashSet;

use serde::{Deserialize, Serialize};

use crate::{PublicRef, sanitize_public_ref};

pub const ISSUE31_HOST_ADJUNCT_SCHEMA: &str = "openagents.omega.issue31.host.v1";
pub const MAX_ISSUE31_PROJECTION_REFS: usize = 16;
pub const MAX_ISSUE31_TIMESTAMP_MS: u64 = 8_640_000_000_000_000;
const ISSUE31_PROJECTION_COUNT: usize = 4;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Issue31ProjectionCapability {
    ConnectionIdentity,
    FullAutoRuns,
    ProviderAccounts,
    EvidenceChain,
}

impl Issue31ProjectionCapability {
    const ALL: [Self; ISSUE31_PROJECTION_COUNT] = [
        Self::ConnectionIdentity,
        Self::FullAutoRuns,
        Self::ProviderAccounts,
        Self::EvidenceChain,
    ];
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Issue31SourceKind {
    OmegaHost,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProjectionFreshness {
    Current,
    Stale,
    Unknown,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Issue31Gap {
    Complete,
    Partial,
    Missing,
    Unavailable,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Issue31RoleKind {
    Owner,
    Member,
    Verifier,
    Observer,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Issue31RoleStatus {
    Active,
    Revoked,
    Unknown,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Issue31TerminalState {
    Succeeded,
    Failed,
    Stopped,
    Unavailable,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Issue31ProjectionSource {
    pub kind: Issue31SourceKind,
    pub source_ref: PublicRef,
    pub observed_at_ms: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Issue31Role {
    pub kind: Issue31RoleKind,
    pub status: Issue31RoleStatus,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub grant_ref: Option<PublicRef>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Issue31CommandState {
    Idle,
    Pending {
        #[serde(rename = "intentRef")]
        intent_ref: PublicRef,
        #[serde(rename = "actionRef")]
        action_ref: PublicRef,
    },
    Refused {
        #[serde(rename = "intentRef")]
        intent_ref: PublicRef,
        #[serde(rename = "actionRef")]
        action_ref: PublicRef,
        #[serde(rename = "reasonClass")]
        reason_class: PublicRef,
        #[serde(rename = "decisionRef")]
        decision_ref: PublicRef,
        #[serde(rename = "receiptRef", skip_serializing_if = "Option::is_none")]
        receipt_ref: Option<PublicRef>,
    },
    Terminal {
        #[serde(rename = "intentRef")]
        intent_ref: PublicRef,
        #[serde(rename = "actionRef")]
        action_ref: PublicRef,
        state: Issue31TerminalState,
        #[serde(rename = "outcomeRef")]
        outcome_ref: PublicRef,
        #[serde(rename = "reasonRef", skip_serializing_if = "Option::is_none")]
        reason_ref: Option<PublicRef>,
        #[serde(rename = "receiptRef", skip_serializing_if = "Option::is_none")]
        receipt_ref: Option<PublicRef>,
    },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Issue31HostProjection {
    pub capability: Issue31ProjectionCapability,
    pub source: Issue31ProjectionSource,
    pub freshness: ProjectionFreshness,
    pub gap: Issue31Gap,
    pub role: Issue31Role,
    pub record_refs: Vec<PublicRef>,
    pub permitted_action_refs: Vec<PublicRef>,
    pub command_state: Issue31CommandState,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Issue31HostAdjunct {
    pub schema: &'static str,
    pub host_ref: PublicRef,
    pub snapshot_ref: PublicRef,
    pub generated_at_ms: u64,
    pub projections: Vec<Issue31HostProjection>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Issue31HostAdjunctError {
    InvalidJson,
    InvalidSchema,
    UnsafeReference,
    ReferenceBoundExceeded,
    DuplicateReference,
    MissingCapability,
    DuplicateCapability,
    InvalidTimestamp,
    InvalidProjectionState,
    InvalidRoleState,
    InvalidCommandState,
}

impl std::fmt::Display for Issue31HostAdjunctError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let message = match self {
            Self::InvalidJson => "issue 31 host adjunct is not valid contract JSON",
            Self::InvalidSchema => "issue 31 host adjunct schema is not supported",
            Self::UnsafeReference => "issue 31 host adjunct contains an unsafe reference",
            Self::ReferenceBoundExceeded => "issue 31 host adjunct reference bound was exceeded",
            Self::DuplicateReference => "issue 31 host adjunct contains a duplicate reference",
            Self::MissingCapability => "issue 31 host adjunct is missing a required capability",
            Self::DuplicateCapability => "issue 31 host adjunct repeats a capability",
            Self::InvalidTimestamp => "issue 31 host adjunct timestamp order is invalid",
            Self::InvalidProjectionState => "issue 31 host adjunct projection state is invalid",
            Self::InvalidRoleState => "issue 31 host adjunct role state is invalid",
            Self::InvalidCommandState => "issue 31 host adjunct command state is invalid",
        };
        formatter.write_str(message)
    }
}

impl std::error::Error for Issue31HostAdjunctError {}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RawIssue31HostAdjunct {
    schema: String,
    host_ref: String,
    snapshot_ref: String,
    generated_at_ms: u64,
    projections: Vec<RawIssue31HostProjection>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RawIssue31HostProjection {
    capability: Issue31ProjectionCapability,
    source: RawIssue31ProjectionSource,
    freshness: ProjectionFreshness,
    gap: Issue31Gap,
    role: RawIssue31Role,
    record_refs: Vec<String>,
    permitted_action_refs: Vec<String>,
    command_state: RawIssue31CommandState,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RawIssue31ProjectionSource {
    kind: Issue31SourceKind,
    source_ref: String,
    observed_at_ms: u64,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RawIssue31Role {
    kind: Issue31RoleKind,
    status: Issue31RoleStatus,
    grant_ref: Option<String>,
}

#[derive(Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
enum RawIssue31CommandState {
    Idle,
    Pending {
        #[serde(rename = "intentRef")]
        intent_ref: String,
        #[serde(rename = "actionRef")]
        action_ref: String,
    },
    Refused {
        #[serde(rename = "intentRef")]
        intent_ref: String,
        #[serde(rename = "actionRef")]
        action_ref: String,
        #[serde(rename = "reasonClass")]
        reason_class: String,
        #[serde(rename = "decisionRef")]
        decision_ref: String,
        #[serde(rename = "receiptRef")]
        receipt_ref: Option<String>,
    },
    Terminal {
        #[serde(rename = "intentRef")]
        intent_ref: String,
        #[serde(rename = "actionRef")]
        action_ref: String,
        state: Issue31TerminalState,
        #[serde(rename = "outcomeRef")]
        outcome_ref: String,
        #[serde(rename = "reasonRef")]
        reason_ref: Option<String>,
        #[serde(rename = "receiptRef")]
        receipt_ref: Option<String>,
    },
}

pub fn decode_issue31_host_adjunct(
    input: &str,
) -> Result<Issue31HostAdjunct, Issue31HostAdjunctError> {
    let raw: RawIssue31HostAdjunct =
        serde_json::from_str(input).map_err(|_| Issue31HostAdjunctError::InvalidJson)?;
    if raw.schema != ISSUE31_HOST_ADJUNCT_SCHEMA {
        return Err(Issue31HostAdjunctError::InvalidSchema);
    }
    let host_ref = public_ref(raw.host_ref)?;
    let snapshot_ref = public_ref(raw.snapshot_ref)?;
    if raw.generated_at_ms > MAX_ISSUE31_TIMESTAMP_MS {
        return Err(Issue31HostAdjunctError::InvalidTimestamp);
    }
    if raw.projections.len() != ISSUE31_PROJECTION_COUNT {
        return Err(Issue31HostAdjunctError::MissingCapability);
    }

    let mut capabilities = HashSet::with_capacity(ISSUE31_PROJECTION_COUNT);
    let mut projections = Vec::with_capacity(ISSUE31_PROJECTION_COUNT);
    for projection in raw.projections {
        if !capabilities.insert(projection.capability) {
            return Err(Issue31HostAdjunctError::DuplicateCapability);
        }
        projections.push(project_projection(projection, raw.generated_at_ms)?);
    }
    if Issue31ProjectionCapability::ALL
        .iter()
        .any(|capability| !capabilities.contains(capability))
    {
        return Err(Issue31HostAdjunctError::MissingCapability);
    }

    Ok(Issue31HostAdjunct {
        schema: ISSUE31_HOST_ADJUNCT_SCHEMA,
        host_ref,
        snapshot_ref,
        generated_at_ms: raw.generated_at_ms,
        projections,
    })
}

fn project_projection(
    raw: RawIssue31HostProjection,
    generated_at_ms: u64,
) -> Result<Issue31HostProjection, Issue31HostAdjunctError> {
    if raw.source.observed_at_ms > generated_at_ms
        || raw.source.observed_at_ms > MAX_ISSUE31_TIMESTAMP_MS
    {
        return Err(Issue31HostAdjunctError::InvalidTimestamp);
    }

    let source = Issue31ProjectionSource {
        kind: raw.source.kind,
        source_ref: public_ref(raw.source.source_ref)?,
        observed_at_ms: raw.source.observed_at_ms,
    };
    let role = Issue31Role {
        kind: raw.role.kind,
        status: raw.role.status,
        grant_ref: raw.role.grant_ref.map(public_ref).transpose()?,
    };
    let record_refs = project_ref_list(raw.record_refs)?;
    let permitted_action_refs = project_ref_list(raw.permitted_action_refs)?;
    let command_state = project_command_state(raw.command_state)?;

    validate_source_state(
        raw.freshness,
        raw.gap,
        &record_refs,
        &permitted_action_refs,
        &command_state,
    )?;
    validate_role_state(&role, &permitted_action_refs, &command_state)?;
    validate_command_state(&permitted_action_refs, &command_state)?;

    Ok(Issue31HostProjection {
        capability: raw.capability,
        source,
        freshness: raw.freshness,
        gap: raw.gap,
        role,
        record_refs,
        permitted_action_refs,
        command_state,
    })
}

fn project_command_state(
    raw: RawIssue31CommandState,
) -> Result<Issue31CommandState, Issue31HostAdjunctError> {
    match raw {
        RawIssue31CommandState::Idle => Ok(Issue31CommandState::Idle),
        RawIssue31CommandState::Pending {
            intent_ref,
            action_ref,
        } => Ok(Issue31CommandState::Pending {
            intent_ref: public_ref(intent_ref)?,
            action_ref: public_ref(action_ref)?,
        }),
        RawIssue31CommandState::Refused {
            intent_ref,
            action_ref,
            reason_class,
            decision_ref,
            receipt_ref,
        } => Ok(Issue31CommandState::Refused {
            intent_ref: public_ref(intent_ref)?,
            action_ref: public_ref(action_ref)?,
            reason_class: public_ref(reason_class)?,
            decision_ref: public_ref(decision_ref)?,
            receipt_ref: receipt_ref.map(public_ref).transpose()?,
        }),
        RawIssue31CommandState::Terminal {
            intent_ref,
            action_ref,
            state,
            outcome_ref,
            reason_ref,
            receipt_ref,
        } => Ok(Issue31CommandState::Terminal {
            intent_ref: public_ref(intent_ref)?,
            action_ref: public_ref(action_ref)?,
            state,
            outcome_ref: public_ref(outcome_ref)?,
            reason_ref: reason_ref.map(public_ref).transpose()?,
            receipt_ref: receipt_ref.map(public_ref).transpose()?,
        }),
    }
}

fn validate_source_state(
    freshness: ProjectionFreshness,
    gap: Issue31Gap,
    record_refs: &[PublicRef],
    permitted_action_refs: &[PublicRef],
    command_state: &Issue31CommandState,
) -> Result<(), Issue31HostAdjunctError> {
    match gap {
        Issue31Gap::Complete | Issue31Gap::Partial => {
            if freshness == ProjectionFreshness::Unknown {
                return Err(Issue31HostAdjunctError::InvalidProjectionState);
            }
        }
        Issue31Gap::Missing | Issue31Gap::Unavailable => {
            if freshness != ProjectionFreshness::Unknown
                || !record_refs.is_empty()
                || !permitted_action_refs.is_empty()
                || !matches!(command_state, Issue31CommandState::Idle)
            {
                return Err(Issue31HostAdjunctError::InvalidProjectionState);
            }
        }
    }
    Ok(())
}

fn validate_role_state(
    role: &Issue31Role,
    permitted_action_refs: &[PublicRef],
    command_state: &Issue31CommandState,
) -> Result<(), Issue31HostAdjunctError> {
    if role.status == Issue31RoleStatus::Active && role.grant_ref.is_none() {
        return Err(Issue31HostAdjunctError::InvalidRoleState);
    }
    if role.status == Issue31RoleStatus::Unknown && role.grant_ref.is_some() {
        return Err(Issue31HostAdjunctError::InvalidRoleState);
    }
    if role.status != Issue31RoleStatus::Active
        && (!permitted_action_refs.is_empty()
            || matches!(command_state, Issue31CommandState::Pending { .. }))
    {
        return Err(Issue31HostAdjunctError::InvalidRoleState);
    }
    Ok(())
}

fn validate_command_state(
    permitted_action_refs: &[PublicRef],
    command_state: &Issue31CommandState,
) -> Result<(), Issue31HostAdjunctError> {
    if let Issue31CommandState::Pending { action_ref, .. } = command_state {
        if !permitted_action_refs.contains(action_ref) {
            return Err(Issue31HostAdjunctError::InvalidCommandState);
        }
    }
    Ok(())
}

fn project_ref_list(values: Vec<String>) -> Result<Vec<PublicRef>, Issue31HostAdjunctError> {
    if values.len() > MAX_ISSUE31_PROJECTION_REFS {
        return Err(Issue31HostAdjunctError::ReferenceBoundExceeded);
    }
    let mut unique = HashSet::with_capacity(values.len());
    let mut projected = Vec::with_capacity(values.len());
    for value in values {
        let value = public_ref(value)?;
        if !unique.insert(value.clone()) {
            return Err(Issue31HostAdjunctError::DuplicateReference);
        }
        projected.push(value);
    }
    Ok(projected)
}

fn public_ref(raw: String) -> Result<PublicRef, Issue31HostAdjunctError> {
    if raw != raw.trim() {
        return Err(Issue31HostAdjunctError::UnsafeReference);
    }
    sanitize_public_ref(&raw).ok_or(Issue31HostAdjunctError::UnsafeReference)
}

#[cfg(test)]
mod tests {
    use super::*;

    const CANONICAL: &str =
        include_str!("../fixtures/openagents.omega.issue31.host.v1.canonical.json");
    const NEGATIVE_PRIVATE_FIELD: &str =
        include_str!("../fixtures/openagents.omega.issue31.host.v1.negative-private-field.json");
    const NEGATIVE_UNSAFE_REF: &str =
        include_str!("../fixtures/openagents.omega.issue31.host.v1.negative-unsafe-ref.json");
    const NEGATIVE_INVALID_STATE: &str =
        include_str!("../fixtures/openagents.omega.issue31.host.v1.negative-invalid-state.json");

    #[test]
    fn decodes_canonical_issue31_host_adjunct() {
        let adjunct = decode_issue31_host_adjunct(CANONICAL).expect("canonical fixture decodes");
        assert_eq!(adjunct.schema, ISSUE31_HOST_ADJUNCT_SCHEMA);
        assert_eq!(adjunct.projections.len(), ISSUE31_PROJECTION_COUNT);
        assert!(adjunct.projections.iter().any(|projection| matches!(
            projection.command_state,
            Issue31CommandState::Pending { .. }
        )));
        assert!(adjunct.projections.iter().any(|projection| matches!(
            projection.command_state,
            Issue31CommandState::Refused { .. }
        )));
        assert!(adjunct.projections.iter().any(|projection| matches!(
            projection.command_state,
            Issue31CommandState::Terminal { .. }
        )));
    }

    #[test]
    fn serialized_projection_contains_only_admitted_contract_fields() {
        let adjunct = decode_issue31_host_adjunct(CANONICAL).expect("canonical fixture decodes");
        let value = serde_json::to_value(adjunct).expect("safe adjunct serializes");
        let encoded = serde_json::to_string(&value).expect("safe adjunct encodes");
        assert!(!encoded.contains("prompt"));
        assert!(!encoded.contains("credential"));
        assert!(!encoded.contains("/Users/"));
        assert!(value.get("hostRef").is_some());
        assert!(value.get("generatedAtMs").is_some());
    }

    #[test]
    fn rejects_private_payload_field_without_echoing_it() {
        let error = decode_issue31_host_adjunct(NEGATIVE_PRIVATE_FIELD)
            .expect_err("private payload field must fail closed");
        assert_eq!(error, Issue31HostAdjunctError::InvalidJson);
        assert!(!error.to_string().contains("owner-private prompt"));
    }

    #[test]
    fn rejects_unsafe_ref_without_echoing_it() {
        let error = decode_issue31_host_adjunct(NEGATIVE_UNSAFE_REF)
            .expect_err("private path must fail closed");
        assert_eq!(error, Issue31HostAdjunctError::UnsafeReference);
        assert!(!error.to_string().contains("/Users/"));

        let mut value: serde_json::Value =
            serde_json::from_str(CANONICAL).expect("canonical JSON value");
        value["hostRef"] = serde_json::Value::String(" host.omega.device-alpha".into());
        assert_eq!(
            decode_issue31_host_adjunct(&value.to_string()),
            Err(Issue31HostAdjunctError::UnsafeReference)
        );
    }

    #[test]
    fn rejects_incoherent_unavailable_projection() {
        let error = decode_issue31_host_adjunct(NEGATIVE_INVALID_STATE)
            .expect_err("unavailable source cannot expose a pending action");
        assert_eq!(error, Issue31HostAdjunctError::InvalidProjectionState);
    }

    #[test]
    fn rejects_missing_or_duplicate_capabilities() {
        let mut value: serde_json::Value =
            serde_json::from_str(CANONICAL).expect("canonical JSON value");
        let projections = value
            .get_mut("projections")
            .and_then(serde_json::Value::as_array_mut)
            .expect("projection array");
        projections.pop();
        assert_eq!(
            decode_issue31_host_adjunct(&value.to_string()),
            Err(Issue31HostAdjunctError::MissingCapability)
        );

        let mut value: serde_json::Value =
            serde_json::from_str(CANONICAL).expect("canonical JSON value");
        let projections = value
            .get_mut("projections")
            .and_then(serde_json::Value::as_array_mut)
            .expect("projection array");
        projections[1]["capability"] = serde_json::Value::String("connection_identity".into());
        assert_eq!(
            decode_issue31_host_adjunct(&value.to_string()),
            Err(Issue31HostAdjunctError::DuplicateCapability)
        );
    }

    #[test]
    fn enforces_reference_and_role_bounds() {
        let mut value: serde_json::Value =
            serde_json::from_str(CANONICAL).expect("canonical JSON value");
        let refs = value["projections"][0]["recordRefs"]
            .as_array_mut()
            .expect("record refs");
        for index in 0..=MAX_ISSUE31_PROJECTION_REFS {
            refs.push(serde_json::Value::String(format!("record.extra.{index}")));
        }
        assert_eq!(
            decode_issue31_host_adjunct(&value.to_string()),
            Err(Issue31HostAdjunctError::ReferenceBoundExceeded)
        );

        let mut value: serde_json::Value =
            serde_json::from_str(CANONICAL).expect("canonical JSON value");
        value["projections"][1]["role"]["status"] = serde_json::Value::String("revoked".into());
        assert_eq!(
            decode_issue31_host_adjunct(&value.to_string()),
            Err(Issue31HostAdjunctError::InvalidRoleState)
        );
    }

    #[test]
    fn rejects_timestamp_outside_the_shared_javascript_date_bound() {
        let mut value: serde_json::Value =
            serde_json::from_str(CANONICAL).expect("canonical JSON value");
        value["generatedAtMs"] = serde_json::Value::from(MAX_ISSUE31_TIMESTAMP_MS + 1);
        assert_eq!(
            decode_issue31_host_adjunct(&value.to_string()),
            Err(Issue31HostAdjunctError::InvalidTimestamp)
        );

        let mut value: serde_json::Value =
            serde_json::from_str(CANONICAL).expect("canonical JSON value");
        value["projections"][0]["source"]["observedAtMs"] =
            serde_json::Value::from(MAX_ISSUE31_TIMESTAMP_MS + 1);
        assert_eq!(
            decode_issue31_host_adjunct(&value.to_string()),
            Err(Issue31HostAdjunctError::InvalidTimestamp)
        );
    }
}
