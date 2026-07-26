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

// ---------------------------------------------------------------------------
// Emitter
// ---------------------------------------------------------------------------

/// The role this host grants the reader for one capability.
///
/// An active role must name the grant that made it active. That is enforced at
/// decode, and this type keeps the emitter from constructing the violation by
/// accident: `Active` carries its grant, and the other two cannot carry one.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Issue31RoleInput<'a> {
    Active {
        kind: Issue31RoleKind,
        grant_ref: &'a str,
    },
    Revoked {
        kind: Issue31RoleKind,
        grant_ref: &'a str,
    },
    /// No grant is known. A role nobody granted cannot cite one.
    Unknown {
        kind: Issue31RoleKind,
    },
}

/// The state of the reader's most recent command against one capability.
///
/// `Terminal` requires the outcome reference that settled it, for the same
/// reason `completed` does in the Full Auto contract: a caller cannot say
/// finished without naming what finished it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Issue31CommandStateInput<'a> {
    Idle,
    Pending {
        intent_ref: &'a str,
        action_ref: &'a str,
    },
    Refused {
        intent_ref: &'a str,
        action_ref: &'a str,
        reason_class: &'a str,
        decision_ref: &'a str,
        receipt_ref: Option<&'a str>,
    },
    Terminal {
        intent_ref: &'a str,
        action_ref: &'a str,
        state: Issue31TerminalState,
        outcome_ref: &'a str,
        reason_ref: Option<&'a str>,
        receipt_ref: Option<&'a str>,
    },
}

/// How completely the host observed one capability, when it did observe it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Issue31ObservedGap {
    Complete,
    Partial,
}

/// Why the host could not observe one capability at all.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Issue31AbsentGap {
    /// The capability exists on this host but produced no record.
    Missing,
    /// The capability could not be reached.
    Unavailable,
}

/// One capability's live host state.
///
/// The two variants are the whole point of this type. An `Absent` projection
/// structurally cannot carry record references, permitted actions, or a command
/// state, so the emitter cannot produce the incoherent shape the decoder
/// rejects as `InvalidProjectionState` — a capability the host could not read,
/// yet which somehow offers the reader a pending action. The decoder still
/// guards it; this makes writing it impossible one layer earlier.
pub enum Issue31HostProjectionInput<'a> {
    Observed {
        source_ref: &'a str,
        observed_at_ms: u64,
        freshness: ProjectionFreshness,
        gap: Issue31ObservedGap,
        role: Issue31RoleInput<'a>,
        record_refs: &'a [&'a str],
        permitted_action_refs: &'a [&'a str],
        command_state: Issue31CommandStateInput<'a>,
    },
    Absent {
        source_ref: &'a str,
        observed_at_ms: u64,
        gap: Issue31AbsentGap,
        role: Issue31RoleInput<'a>,
    },
}

/// The four capabilities this snapshot projects, named rather than listed.
///
/// A `Vec` would let a caller omit one or repeat one, which the decoder refuses
/// as `MissingCapability` / `DuplicateCapability`. Naming each field makes both
/// unrepresentable: exactly four projections exist and each carries its own
/// capability tag, assigned here rather than by the caller.
pub struct Issue31HostSources<'a> {
    pub connection_identity: Issue31HostProjectionInput<'a>,
    pub full_auto_runs: Issue31HostProjectionInput<'a>,
    pub provider_accounts: Issue31HostProjectionInput<'a>,
    pub evidence_chain: Issue31HostProjectionInput<'a>,
}

/// Build the `host.v1` snapshot the omega#47 detail projection is published
/// beside.
///
/// Like `build_issue31_full_auto_adjunct`, this deliberately builds a JSON
/// document and hands it to `decode_issue31_host_adjunct` rather than
/// constructing the typed value directly. There is then exactly one place where
/// the contract's boundaries live, and the emitter is structurally incapable of
/// producing something the reader would refuse: every refusal a phone could
/// raise against this snapshot is raised here first, against the same bytes.
pub fn build_issue31_host_adjunct(
    host_ref: &str,
    snapshot_ref: &str,
    generated_at_ms: u64,
    sources: &Issue31HostSources<'_>,
) -> Result<Issue31HostAdjunct, Issue31HostAdjunctError> {
    build_issue31_host_adjunct_document(host_ref, snapshot_ref, generated_at_ms, sources)
        .map(|(adjunct, _)| adjunct)
}

/// The same snapshot, plus the exact bytes the decoder accepted.
///
/// A caller that has to put the snapshot on a wire would otherwise have to
/// re-encode the typed value, which is a second serializer that can disagree
/// with the one the contract validated. Returning the validated document means
/// what is published is what was checked.
pub fn build_issue31_host_adjunct_document(
    host_ref: &str,
    snapshot_ref: &str,
    generated_at_ms: u64,
    sources: &Issue31HostSources<'_>,
) -> Result<(Issue31HostAdjunct, serde_json::Value), Issue31HostAdjunctError> {
    let document = serde_json::json!({
        "schema": ISSUE31_HOST_ADJUNCT_SCHEMA,
        "hostRef": host_ref,
        "snapshotRef": snapshot_ref,
        "generatedAtMs": generated_at_ms,
        "projections": [
            build_projection(
                Issue31ProjectionCapability::ConnectionIdentity,
                &sources.connection_identity,
            ),
            build_projection(
                Issue31ProjectionCapability::FullAutoRuns,
                &sources.full_auto_runs,
            ),
            build_projection(
                Issue31ProjectionCapability::ProviderAccounts,
                &sources.provider_accounts,
            ),
            build_projection(
                Issue31ProjectionCapability::EvidenceChain,
                &sources.evidence_chain,
            ),
        ],
    });
    let serialized =
        serde_json::to_string(&document).map_err(|_| Issue31HostAdjunctError::InvalidJson)?;
    let adjunct = decode_issue31_host_adjunct(&serialized)?;
    Ok((adjunct, document))
}

fn build_projection(
    capability: Issue31ProjectionCapability,
    input: &Issue31HostProjectionInput<'_>,
) -> serde_json::Value {
    match input {
        Issue31HostProjectionInput::Observed {
            source_ref,
            observed_at_ms,
            freshness,
            gap,
            role,
            record_refs,
            permitted_action_refs,
            command_state,
        } => serde_json::json!({
            "capability": capability,
            "source": {
                "kind": Issue31SourceKind::OmegaHost,
                "sourceRef": source_ref,
                "observedAtMs": observed_at_ms,
            },
            "freshness": freshness,
            "gap": match gap {
                Issue31ObservedGap::Complete => Issue31Gap::Complete,
                Issue31ObservedGap::Partial => Issue31Gap::Partial,
            },
            "role": build_role(role),
            "recordRefs": record_refs,
            "permittedActionRefs": permitted_action_refs,
            "commandState": build_command_state(command_state),
        }),
        // A capability the host could not read offers nothing and claims
        // nothing: unknown freshness, no records, no actions, an idle command
        // state. These are not defaults the caller may override — there is no
        // field to override them with.
        Issue31HostProjectionInput::Absent {
            source_ref,
            observed_at_ms,
            gap,
            role,
        } => serde_json::json!({
            "capability": capability,
            "source": {
                "kind": Issue31SourceKind::OmegaHost,
                "sourceRef": source_ref,
                "observedAtMs": observed_at_ms,
            },
            "freshness": ProjectionFreshness::Unknown,
            "gap": match gap {
                Issue31AbsentGap::Missing => Issue31Gap::Missing,
                Issue31AbsentGap::Unavailable => Issue31Gap::Unavailable,
            },
            "role": build_role(role),
            "recordRefs": Vec::<&str>::new(),
            "permittedActionRefs": Vec::<&str>::new(),
            "commandState": { "kind": "idle" },
        }),
    }
}

fn build_role(role: &Issue31RoleInput<'_>) -> serde_json::Value {
    match role {
        Issue31RoleInput::Active { kind, grant_ref } => serde_json::json!({
            "kind": kind,
            "status": Issue31RoleStatus::Active,
            "grantRef": grant_ref,
        }),
        Issue31RoleInput::Revoked { kind, grant_ref } => serde_json::json!({
            "kind": kind,
            "status": Issue31RoleStatus::Revoked,
            "grantRef": grant_ref,
        }),
        Issue31RoleInput::Unknown { kind } => serde_json::json!({
            "kind": kind,
            "status": Issue31RoleStatus::Unknown,
            "grantRef": serde_json::Value::Null,
        }),
    }
}

fn build_command_state(state: &Issue31CommandStateInput<'_>) -> serde_json::Value {
    match state {
        Issue31CommandStateInput::Idle => serde_json::json!({ "kind": "idle" }),
        Issue31CommandStateInput::Pending {
            intent_ref,
            action_ref,
        } => serde_json::json!({
            "kind": "pending",
            "intentRef": intent_ref,
            "actionRef": action_ref,
        }),
        Issue31CommandStateInput::Refused {
            intent_ref,
            action_ref,
            reason_class,
            decision_ref,
            receipt_ref,
        } => serde_json::json!({
            "kind": "refused",
            "intentRef": intent_ref,
            "actionRef": action_ref,
            "reasonClass": reason_class,
            "decisionRef": decision_ref,
            "receiptRef": receipt_ref,
        }),
        Issue31CommandStateInput::Terminal {
            intent_ref,
            action_ref,
            state,
            outcome_ref,
            reason_ref,
            receipt_ref,
        } => serde_json::json!({
            "kind": "terminal",
            "intentRef": intent_ref,
            "actionRef": action_ref,
            "state": state,
            "outcomeRef": outcome_ref,
            "reasonRef": reason_ref,
            "receiptRef": receipt_ref,
        }),
    }
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

    // -----------------------------------------------------------------
    // Emitter (omega#47): the `host.v1` producer the detail projection is
    // published beside.
    // -----------------------------------------------------------------

    const HOST: &str = "host.omega.device-alpha";
    const SNAPSHOT: &str = "snapshot.omega.issue31.000042";
    const GENERATED_AT: u64 = 1_784_894_400_000;
    const OWNER_GRANT: &str = "grant.omega.mobile.owner-01";

    fn owner() -> Issue31RoleInput<'static> {
        Issue31RoleInput::Active {
            kind: Issue31RoleKind::Owner,
            grant_ref: OWNER_GRANT,
        }
    }

    /// The exact host state the byte-shared canonical fixture describes.
    fn canonical_sources() -> Issue31HostSources<'static> {
        Issue31HostSources {
            connection_identity: Issue31HostProjectionInput::Observed {
                source_ref: "source.omega.identity-binding",
                observed_at_ms: 1_784_894_399_000,
                freshness: ProjectionFreshness::Current,
                gap: Issue31ObservedGap::Complete,
                role: owner(),
                record_refs: &[
                    "record.omega.host-announcement.01",
                    "record.omega.owner-binding.01",
                ],
                permitted_action_refs: &[
                    "action.omega.device.pair",
                    "action.omega.device.renew",
                    "action.omega.device.revoke",
                ],
                command_state: Issue31CommandStateInput::Idle,
            },
            full_auto_runs: Issue31HostProjectionInput::Observed {
                source_ref: "source.omega.full-auto-registry",
                observed_at_ms: 1_784_894_340_000,
                freshness: ProjectionFreshness::Stale,
                gap: Issue31ObservedGap::Partial,
                role: owner(),
                record_refs: &["record.full-auto.run.run-01"],
                permitted_action_refs: &[
                    "action.full-auto.pause",
                    "action.full-auto.resume",
                    "action.full-auto.stop",
                    "action.full-auto.ask-sarah",
                ],
                command_state: Issue31CommandStateInput::Pending {
                    intent_ref: "intent.full-auto.pause.01",
                    action_ref: "action.full-auto.pause",
                },
            },
            provider_accounts: Issue31HostProjectionInput::Observed {
                source_ref: "source.omega.provider-roster",
                observed_at_ms: 1_784_894_398_000,
                freshness: ProjectionFreshness::Current,
                gap: Issue31ObservedGap::Complete,
                role: owner(),
                record_refs: &["record.provider.roster.01"],
                permitted_action_refs: &["action.provider.request-connect-handoff"],
                command_state: Issue31CommandStateInput::Refused {
                    intent_ref: "intent.provider.connect.01",
                    action_ref: "action.provider.request-connect-handoff",
                    reason_class: "provider_login_requires_host",
                    decision_ref: "decision.provider.connect.01",
                    receipt_ref: Some("receipt.provider.connect.01"),
                },
            },
            evidence_chain: Issue31HostProjectionInput::Observed {
                source_ref: "source.omega.evidence-inspector",
                observed_at_ms: 1_784_894_397_000,
                freshness: ProjectionFreshness::Current,
                gap: Issue31ObservedGap::Complete,
                role: Issue31RoleInput::Active {
                    kind: Issue31RoleKind::Verifier,
                    grant_ref: "grant.omega.mobile.verifier-01",
                },
                record_refs: &["record.evidence.chain.run-01"],
                permitted_action_refs: &[],
                command_state: Issue31CommandStateInput::Terminal {
                    intent_ref: "intent.evidence.verify.01",
                    action_ref: "action.evidence.verify",
                    state: Issue31TerminalState::Succeeded,
                    outcome_ref: "outcome.evidence.verify.01",
                    reason_ref: Some("reason.evidence.chain-valid"),
                    receipt_ref: Some("receipt.evidence.verify.01"),
                },
            },
        }
    }

    fn build(sources: &Issue31HostSources<'_>) -> Result<Issue31HostAdjunct, Issue31HostAdjunctError>
    {
        build_issue31_host_adjunct(HOST, SNAPSHOT, GENERATED_AT, sources)
    }

    /// The strongest cross-check available: what this host emits must be the
    /// same value the shared fixture decodes to, and that fixture is
    /// byte-identical to its `packages/sarah` peer. The producer and the phone
    /// therefore cannot drift without a test going red on one side.
    #[test]
    fn the_producer_emits_exactly_what_the_shared_fixture_decodes_to() {
        let emitted = build(&canonical_sources()).expect("canonical host state emits");
        let fixture = decode_issue31_host_adjunct(CANONICAL).expect("canonical fixture decodes");
        assert_eq!(emitted, fixture);
    }

    #[test]
    fn every_emitted_snapshot_carries_all_four_capabilities_exactly_once() {
        let emitted = build(&canonical_sources()).expect("emits");
        let mut seen: Vec<_> = emitted
            .projections
            .iter()
            .map(|projection| projection.capability)
            .collect();
        assert_eq!(seen.len(), ISSUE31_PROJECTION_COUNT);
        for capability in Issue31ProjectionCapability::ALL {
            let before = seen.len();
            seen.retain(|value| *value != capability);
            assert_eq!(before - seen.len(), 1, "{capability:?} appears exactly once");
        }
        assert!(seen.is_empty());
    }

    /// A capability the host could not read has no way to offer the reader an
    /// action. `Absent` has no field for one.
    #[test]
    fn an_unreadable_capability_claims_nothing_and_offers_nothing() {
        let mut sources = canonical_sources();
        sources.provider_accounts = Issue31HostProjectionInput::Absent {
            source_ref: "source.omega.provider-roster",
            observed_at_ms: 1_784_894_398_000,
            gap: Issue31AbsentGap::Unavailable,
            role: owner(),
        };
        let emitted = build(&sources).expect("an unreadable capability is still a valid snapshot");
        let projection = emitted
            .projections
            .iter()
            .find(|projection| {
                projection.capability == Issue31ProjectionCapability::ProviderAccounts
            })
            .expect("provider accounts projection");
        assert_eq!(projection.gap, Issue31Gap::Unavailable);
        assert_eq!(projection.freshness, ProjectionFreshness::Unknown);
        assert!(projection.record_refs.is_empty());
        assert!(projection.permitted_action_refs.is_empty());
        assert_eq!(projection.command_state, Issue31CommandState::Idle);
    }

    #[test]
    fn a_private_path_cannot_be_emitted_and_the_error_does_not_echo_it() {
        let mut sources = canonical_sources();
        sources.connection_identity = Issue31HostProjectionInput::Observed {
            source_ref: "/Users/owner/.codex/auth.json",
            observed_at_ms: 1_784_894_399_000,
            freshness: ProjectionFreshness::Current,
            gap: Issue31ObservedGap::Complete,
            role: owner(),
            record_refs: &["record.omega.host-announcement.01"],
            permitted_action_refs: &[],
            command_state: Issue31CommandStateInput::Idle,
        };
        let error = build(&sources).expect_err("a private path must fail closed");
        assert_eq!(error, Issue31HostAdjunctError::UnsafeReference);
        assert!(!error.to_string().contains("/Users/"));
        assert!(!error.to_string().contains("auth.json"));
    }

    #[test]
    fn a_pending_command_the_host_does_not_permit_is_refused() {
        let mut sources = canonical_sources();
        sources.full_auto_runs = Issue31HostProjectionInput::Observed {
            source_ref: "source.omega.full-auto-registry",
            observed_at_ms: 1_784_894_340_000,
            freshness: ProjectionFreshness::Stale,
            gap: Issue31ObservedGap::Partial,
            role: owner(),
            record_refs: &["record.full-auto.run.run-01"],
            permitted_action_refs: &["action.full-auto.pause"],
            command_state: Issue31CommandStateInput::Pending {
                intent_ref: "intent.full-auto.stop.01",
                action_ref: "action.full-auto.stop",
            },
        };
        assert_eq!(
            build(&sources),
            Err(Issue31HostAdjunctError::InvalidCommandState)
        );
    }

    #[test]
    fn a_revoked_role_cannot_be_emitted_with_permitted_actions() {
        let mut sources = canonical_sources();
        sources.provider_accounts = Issue31HostProjectionInput::Observed {
            source_ref: "source.omega.provider-roster",
            observed_at_ms: 1_784_894_398_000,
            freshness: ProjectionFreshness::Current,
            gap: Issue31ObservedGap::Complete,
            role: Issue31RoleInput::Revoked {
                kind: Issue31RoleKind::Owner,
                grant_ref: OWNER_GRANT,
            },
            record_refs: &["record.provider.roster.01"],
            permitted_action_refs: &["action.provider.request-connect-handoff"],
            command_state: Issue31CommandStateInput::Idle,
        };
        assert_eq!(build(&sources), Err(Issue31HostAdjunctError::InvalidRoleState));
    }

    #[test]
    fn a_projection_observed_after_the_snapshot_was_generated_is_refused() {
        let mut sources = canonical_sources();
        sources.evidence_chain = Issue31HostProjectionInput::Observed {
            source_ref: "source.omega.evidence-inspector",
            observed_at_ms: GENERATED_AT + 1,
            freshness: ProjectionFreshness::Current,
            gap: Issue31ObservedGap::Complete,
            role: owner(),
            record_refs: &["record.evidence.chain.run-01"],
            permitted_action_refs: &[],
            command_state: Issue31CommandStateInput::Idle,
        };
        assert_eq!(build(&sources), Err(Issue31HostAdjunctError::InvalidTimestamp));
    }

    #[test]
    fn a_duplicate_record_reference_is_refused_rather_than_deduplicated() {
        let mut sources = canonical_sources();
        sources.connection_identity = Issue31HostProjectionInput::Observed {
            source_ref: "source.omega.identity-binding",
            observed_at_ms: 1_784_894_399_000,
            freshness: ProjectionFreshness::Current,
            gap: Issue31ObservedGap::Complete,
            role: owner(),
            record_refs: &[
                "record.omega.host-announcement.01",
                "record.omega.host-announcement.01",
            ],
            permitted_action_refs: &[],
            command_state: Issue31CommandStateInput::Idle,
        };
        assert_eq!(
            build(&sources),
            Err(Issue31HostAdjunctError::DuplicateReference)
        );
    }

    #[test]
    fn the_reference_bound_holds_at_the_producer_too() {
        let refs: Vec<String> = (0..=MAX_ISSUE31_PROJECTION_REFS)
            .map(|index| format!("record.omega.extra.{index}"))
            .collect();
        let borrowed: Vec<&str> = refs.iter().map(String::as_str).collect();
        let mut sources = canonical_sources();
        sources.connection_identity = Issue31HostProjectionInput::Observed {
            source_ref: "source.omega.identity-binding",
            observed_at_ms: 1_784_894_399_000,
            freshness: ProjectionFreshness::Current,
            gap: Issue31ObservedGap::Complete,
            role: owner(),
            record_refs: &borrowed,
            permitted_action_refs: &[],
            command_state: Issue31CommandStateInput::Idle,
        };
        assert_eq!(
            build(&sources),
            Err(Issue31HostAdjunctError::ReferenceBoundExceeded)
        );
    }

    /// Nothing the producer emits may carry owner-private material, whatever
    /// the host record around it looked like.
    #[test]
    fn an_emitted_snapshot_serializes_to_admitted_fields_only() {
        let emitted = build(&canonical_sources()).expect("emits");
        let encoded = serde_json::to_string(&emitted).expect("emitted snapshot encodes");
        assert!(!encoded.contains("/Users/"));
        assert!(!encoded.contains("credential"));
        assert!(!encoded.contains("Bearer "));
        assert!(!encoded.contains("prompt"));
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
