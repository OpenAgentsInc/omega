use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::{
    ForensicEvidenceReceiptProjection, ForensicEvidenceTier, ForensicFindingProjection,
    ForensicHypothesisProjection, ForensicsError, validate_finding, validate_hypothesis,
};

pub const FORENSIC_TOOL_JOURNAL_SCHEMA_V1: &str = "openagents.omega.forensic_tool_journal.v1";
pub const FORENSIC_TOOL_EVENT_SCHEMA_V1: &str = "openagents.omega.forensic_tool_event.v1";
pub const FORENSIC_TOOL_VERSION_V1: &str = "openagents.forensic_tools.v1";

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ForensicToolName {
    QueryPriorForensicWork,
    GetForensicWorkByRef,
    SubmitForensicHypothesis,
    SubmitForensicFinding,
    SubmitForensicLimitation,
    ValidateCandidateDiffApplicability,
    ExecuteIndependentControl,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ForensicToolActorRole {
    Discovery,
    IndependentVerifier,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ForensicToolCallBinding {
    pub run_ref: String,
    pub task_ref: String,
    pub actor_ref: String,
    pub actor_role: ForensicToolActorRole,
    pub audience_ref: String,
    pub source_bundle_ref: String,
    pub source_bundle_digest: String,
    pub coverage_generation: u64,
    pub prompt_digest: String,
    pub model_route_ref: String,
    pub tool_version: String,
    pub budget_ref: String,
    pub expected_event_cursor: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ForensicSourceFile {
    pub path: String,
    pub revision: String,
    pub content_digest: String,
    pub bytes: Vec<u8>,
    pub symbols: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ForensicSourceCatalog {
    pub audience_ref: String,
    pub source_bundle_ref: String,
    pub source_bundle_digest: String,
    pub coverage_generation: u64,
    pub missing_dependency_paths: Vec<String>,
    pub files: Vec<ForensicSourceFile>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ForensicFindingToolInput {
    pub finding: ForensicFindingProjection,
    pub source_window_digests: BTreeMap<String, String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ForensicLimitationProjection {
    pub limitation_ref: String,
    pub class_ref: String,
    pub message: String,
    pub affected_source_refs: Vec<String>,
    pub required_next_check: String,
    pub submitted_at: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ForensicDiffApplicabilityProjection {
    pub applicability_ref: String,
    pub diff_digest: String,
    pub target_revision: String,
    pub applicable: bool,
    pub evidence_tier: ForensicEvidenceTier,
    pub executed: bool,
    pub test_outcome: String,
    pub observed_at: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ForensicExecutedControlProjection {
    pub control_ref: String,
    pub finding_ref: String,
    pub receipt: ForensicEvidenceReceiptProjection,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "tool", content = "input", rename_all = "snake_case")]
pub enum ForensicToolPayload {
    QueryPriorForensicWork { query_ref: String },
    GetForensicWorkByRef { work_ref: String },
    SubmitForensicHypothesis(ForensicHypothesisProjection),
    SubmitForensicFinding(ForensicFindingToolInput),
    SubmitForensicLimitation(ForensicLimitationProjection),
    ValidateCandidateDiffApplicability(ForensicDiffApplicabilityProjection),
    ExecuteIndependentControl(ForensicExecutedControlProjection),
}

impl ForensicToolPayload {
    pub fn tool_name(&self) -> ForensicToolName {
        match self {
            Self::QueryPriorForensicWork { .. } => ForensicToolName::QueryPriorForensicWork,
            Self::GetForensicWorkByRef { .. } => ForensicToolName::GetForensicWorkByRef,
            Self::SubmitForensicHypothesis(_) => ForensicToolName::SubmitForensicHypothesis,
            Self::SubmitForensicFinding(_) => ForensicToolName::SubmitForensicFinding,
            Self::SubmitForensicLimitation(_) => ForensicToolName::SubmitForensicLimitation,
            Self::ValidateCandidateDiffApplicability(_) => {
                ForensicToolName::ValidateCandidateDiffApplicability
            }
            Self::ExecuteIndependentControl(_) => ForensicToolName::ExecuteIndependentControl,
        }
    }
}

impl ForensicToolName {
    pub fn canonical_name(self) -> &'static str {
        match self {
            Self::QueryPriorForensicWork => "query_prior_forensic_work",
            Self::GetForensicWorkByRef => "get_forensic_work_by_ref",
            Self::SubmitForensicHypothesis => "submit_forensic_hypothesis",
            Self::SubmitForensicFinding => "submit_forensic_finding",
            Self::SubmitForensicLimitation => "submit_forensic_limitation",
            Self::ValidateCandidateDiffApplicability => "validate_candidate_diff_applicability",
            Self::ExecuteIndependentControl => "execute_independent_control",
        }
    }

    pub fn from_canonical_name(value: &str) -> Option<Self> {
        Some(match value {
            "query_prior_forensic_work" => Self::QueryPriorForensicWork,
            "get_forensic_work_by_ref" => Self::GetForensicWorkByRef,
            "submit_forensic_hypothesis" => Self::SubmitForensicHypothesis,
            "submit_forensic_finding" => Self::SubmitForensicFinding,
            "submit_forensic_limitation" => Self::SubmitForensicLimitation,
            "validate_candidate_diff_applicability" => Self::ValidateCandidateDiffApplicability,
            "execute_independent_control" => Self::ExecuteIndependentControl,
            _ => return None,
        })
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ForensicToolCall {
    pub call_ref: String,
    pub idempotency_ref: String,
    pub binding: ForensicToolCallBinding,
    pub payload: ForensicToolPayload,
    pub observed_at: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ForensicToolEventStatus {
    Accepted,
    Rejected,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ForensicToolEvent {
    pub schema: String,
    pub event_ref: String,
    pub sequence: u64,
    pub call_ref: String,
    pub tool: ForensicToolName,
    pub status: ForensicToolEventStatus,
    pub result_ref: Option<String>,
    pub refusal_ref: Option<String>,
    pub call_digest: String,
    pub observed_at: String,
    pub call: ForensicToolCall,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ForensicToolJournal {
    pub schema: String,
    pub binding: ForensicToolCallBinding,
    pub verifier_actor_ref: String,
    pub discovery_tools: BTreeSet<ForensicToolName>,
    pub verifier_tools: BTreeSet<ForensicToolName>,
    pub events: Vec<ForensicToolEvent>,
    pub findings: Vec<ForensicFindingProjection>,
    pub hypotheses: Vec<ForensicHypothesisProjection>,
    pub limitations: Vec<ForensicLimitationProjection>,
    pub diff_applicability: Vec<ForensicDiffApplicabilityProjection>,
    pub executed_controls: Vec<ForensicExecutedControlProjection>,
    pub command_digests: BTreeMap<String, String>,
}

impl ForensicToolJournal {
    pub fn new(
        mut binding: ForensicToolCallBinding,
        verifier_actor_ref: String,
    ) -> Result<Self, ForensicsError> {
        binding.expected_event_cursor = 0;
        validate_binding(&binding)?;
        if binding.actor_role != ForensicToolActorRole::Discovery
            || verifier_actor_ref == binding.actor_ref
            || verifier_actor_ref.trim().is_empty()
        {
            return invalid("discovery and verifier identities must be distinct");
        }
        Ok(Self {
            schema: FORENSIC_TOOL_JOURNAL_SCHEMA_V1.into(),
            binding,
            verifier_actor_ref,
            discovery_tools: BTreeSet::from([
                ForensicToolName::QueryPriorForensicWork,
                ForensicToolName::GetForensicWorkByRef,
                ForensicToolName::SubmitForensicHypothesis,
                ForensicToolName::SubmitForensicFinding,
                ForensicToolName::SubmitForensicLimitation,
                ForensicToolName::ValidateCandidateDiffApplicability,
            ]),
            verifier_tools: BTreeSet::from([ForensicToolName::ExecuteIndependentControl]),
            events: Vec::new(),
            findings: Vec::new(),
            hypotheses: Vec::new(),
            limitations: Vec::new(),
            diff_applicability: Vec::new(),
            executed_controls: Vec::new(),
            command_digests: BTreeMap::new(),
        })
    }

    pub fn event_cursor(&self) -> u64 {
        self.events.last().map_or(0, |event| event.sequence)
    }

    pub fn validate(&self) -> Result<(), ForensicsError> {
        if self.schema != FORENSIC_TOOL_JOURNAL_SCHEMA_V1 {
            return invalid("forensic tool journal schema is invalid");
        }
        validate_binding(&self.binding)?;
        if self.binding.actor_role != ForensicToolActorRole::Discovery
            || self.verifier_actor_ref.trim().is_empty()
            || self.verifier_actor_ref == self.binding.actor_ref
        {
            return invalid("restored discovery and verifier identities are invalid");
        }
        let expected_discovery = BTreeSet::from([
            ForensicToolName::QueryPriorForensicWork,
            ForensicToolName::GetForensicWorkByRef,
            ForensicToolName::SubmitForensicHypothesis,
            ForensicToolName::SubmitForensicFinding,
            ForensicToolName::SubmitForensicLimitation,
            ForensicToolName::ValidateCandidateDiffApplicability,
        ]);
        let expected_verifier = BTreeSet::from([ForensicToolName::ExecuteIndependentControl]);
        if self.discovery_tools != expected_discovery || self.verifier_tools != expected_verifier {
            return invalid("restored forensic tool capabilities drifted");
        }
        let mut findings = Vec::new();
        let mut hypotheses = Vec::new();
        let mut limitations = Vec::new();
        let mut diff_applicability = Vec::new();
        let mut executed_controls = Vec::new();
        for (index, event) in self.events.iter().enumerate() {
            if event.schema != FORENSIC_TOOL_EVENT_SCHEMA_V1
                || event.sequence != index as u64 + 1
                || event.event_ref != format!("event:forensic-tool:{}", index + 1)
                || !valid_digest(&event.call_digest)
                || digest(&event.call)? != event.call_digest
                || event.call_ref != event.call.call_ref
                || event.tool != event.call.payload.tool_name()
                || event.observed_at != event.call.observed_at
                || (event.status == ForensicToolEventStatus::Accepted
                    && (event.result_ref.is_none() || event.refusal_ref.is_some()))
                || (event.status == ForensicToolEventStatus::Rejected
                    && (event.result_ref.is_some() || event.refusal_ref.is_none()))
            {
                return invalid("restored forensic tool event is invalid");
            }
            if event.status == ForensicToolEventStatus::Accepted {
                match &event.call.payload {
                    ForensicToolPayload::SubmitForensicFinding(value) => {
                        findings.push(value.finding.clone())
                    }
                    ForensicToolPayload::SubmitForensicHypothesis(value) => {
                        hypotheses.push(value.clone())
                    }
                    ForensicToolPayload::SubmitForensicLimitation(value) => {
                        limitations.push(value.clone())
                    }
                    ForensicToolPayload::ValidateCandidateDiffApplicability(value) => {
                        diff_applicability.push(value.clone())
                    }
                    ForensicToolPayload::ExecuteIndependentControl(value) => {
                        executed_controls.push(value.clone())
                    }
                    ForensicToolPayload::QueryPriorForensicWork { .. }
                    | ForensicToolPayload::GetForensicWorkByRef { .. } => {}
                }
            }
        }
        if self
            .command_digests
            .values()
            .any(|value| !valid_digest(value))
        {
            return invalid("restored forensic command digest is invalid");
        }
        if self.command_digests.iter().any(|(idempotency_ref, value)| {
            !self.events.iter().any(|event| {
                &event.call.idempotency_ref == idempotency_ref && &event.call_digest == value
            })
        }) || self.findings != findings
            || self.hypotheses != hypotheses
            || self.limitations != limitations
            || self.diff_applicability != diff_applicability
            || self.executed_controls != executed_controls
        {
            return invalid("restored forensic journal projection drifted from its events");
        }
        Ok(())
    }

    pub fn resume_after(&self, cursor: u64) -> Result<&[ForensicToolEvent], ForensicsError> {
        if cursor > self.event_cursor() {
            return invalid("resume cursor is ahead of the journal");
        }
        Ok(&self.events[cursor as usize..])
    }

    pub fn observe_transcript_prose(&self, _markdown: &str) {
        // Transcript prose is diagnostic only. It has no mutation path.
    }

    pub fn ingest(
        &mut self,
        call: ForensicToolCall,
        sources: &ForensicSourceCatalog,
    ) -> Result<ForensicToolEvent, ForensicsError> {
        let call_digest = digest(&call)?;
        if let Some(prior) = self.command_digests.get(&call.idempotency_ref) {
            if prior == &call_digest {
                return self
                    .events
                    .iter()
                    .find(|event| event.call_digest == call_digest)
                    .cloned()
                    .ok_or_else(|| {
                        ForensicsError::InvalidToolEvent("replay event is missing".into())
                    });
            }
            if let Some(event) = self
                .events
                .iter()
                .find(|event| event.call_digest == call_digest)
            {
                return Ok(event.clone());
            }
            return Ok(self.reject(&call, call_digest, "idempotency_conflict"));
        }
        let outcome = self.admit(&call, sources);
        self.command_digests
            .insert(call.idempotency_ref.clone(), call_digest.clone());
        match outcome {
            Ok(result_ref) => {
                self.apply_payload(&call.payload);
                Ok(self.append_event(
                    &call,
                    call_digest,
                    ForensicToolEventStatus::Accepted,
                    Some(result_ref),
                    None,
                ))
            }
            Err(reason) => Ok(self.reject(&call, call_digest, reason)),
        }
    }

    /// Ingests the canonical typed fallback result. Provider adapters must
    /// normalize their transport envelope into the same call before invoking
    /// this method; transcript Markdown is never an input.
    pub fn ingest_typed_fallback(
        &mut self,
        call: ForensicToolCall,
        sources: &ForensicSourceCatalog,
    ) -> Result<ForensicToolEvent, ForensicsError> {
        self.ingest(call, sources)
    }

    /// Ingests structured arguments from a visible provider/ACP tool event.
    /// Supported provider envelopes contain exactly one `input` or
    /// `arguments` member. String arguments must themselves be JSON. This API
    /// cannot accept transcript prose.
    pub fn ingest_visible_tool_json(
        &mut self,
        tool_name: &str,
        raw_input: &serde_json::Value,
        sources: &ForensicSourceCatalog,
    ) -> Result<Option<ForensicToolEvent>, ForensicsError> {
        let Some((expected_tool, call)) = Self::decode_visible_tool_json(tool_name, raw_input)?
        else {
            return Ok(None);
        };
        if call.payload.tool_name() != expected_tool {
            return invalid("visible tool name does not match its typed forensic payload");
        }
        self.ingest(call, sources).map(Some)
    }

    pub fn decode_visible_tool_json(
        tool_name: &str,
        raw_input: &serde_json::Value,
    ) -> Result<Option<(ForensicToolName, ForensicToolCall)>, ForensicsError> {
        let Some(expected_tool) = ForensicToolName::from_canonical_name(tool_name) else {
            return Ok(None);
        };
        let normalized = normalize_structured_arguments(raw_input)?;
        let call = serde_json::from_value(normalized)
            .map_err(|error| ForensicsError::InvalidToolEvent(error.to_string()))?;
        Ok(Some((expected_tool, call)))
    }

    fn admit(
        &self,
        call: &ForensicToolCall,
        sources: &ForensicSourceCatalog,
    ) -> Result<String, &'static str> {
        if !binding_matches(&self.binding, &call.binding) {
            return Err("binding_mismatch");
        }
        if call.binding.expected_event_cursor != self.event_cursor() {
            return Err("event_cursor_conflict");
        }
        let admitted = match call.binding.actor_role {
            ForensicToolActorRole::Discovery
                if call.binding.actor_ref == self.binding.actor_ref =>
            {
                &self.discovery_tools
            }
            ForensicToolActorRole::IndependentVerifier
                if call.binding.actor_ref == self.verifier_actor_ref =>
            {
                &self.verifier_tools
            }
            _ => return Err("actor_identity_forbidden"),
        };
        if !admitted.contains(&call.payload.tool_name()) {
            return Err("tool_capability_forbidden");
        }
        if sources.source_bundle_ref != call.binding.source_bundle_ref
            || sources.source_bundle_digest != call.binding.source_bundle_digest
            || sources.coverage_generation != call.binding.coverage_generation
            || sources.audience_ref != call.binding.audience_ref
        {
            return Err("source_bundle_mismatch");
        }
        match &call.payload {
            ForensicToolPayload::QueryPriorForensicWork { query_ref } => Ok(query_ref.clone()),
            ForensicToolPayload::GetForensicWorkByRef { work_ref } => Ok(work_ref.clone()),
            ForensicToolPayload::SubmitForensicFinding(input) => {
                if !sources.missing_dependency_paths.is_empty() {
                    return Err("incomplete_dependency_requires_hypothesis");
                }
                let Some(expected_commit) = input
                    .finding
                    .source_refs
                    .first()
                    .map(|source| source.commit.as_str())
                else {
                    return Err("finding_requires_source");
                };
                validate_finding(&input.finding, expected_commit)
                    .map_err(|_| "finding_contract_invalid")?;
                validate_finding_sources(input, sources)?;
                Ok(input.finding.finding_ref.clone())
            }
            ForensicToolPayload::SubmitForensicHypothesis(hypothesis) => {
                validate_hypothesis(hypothesis).map_err(|_| "hypothesis_contract_invalid")?;
                Ok(hypothesis.hypothesis_ref.clone())
            }
            ForensicToolPayload::SubmitForensicLimitation(limitation) => {
                if limitation.message.trim().is_empty()
                    || limitation.required_next_check.trim().is_empty()
                {
                    return Err("limitation_missing_next_check");
                }
                Ok(limitation.limitation_ref.clone())
            }
            ForensicToolPayload::ValidateCandidateDiffApplicability(applicability) => {
                if applicability.evidence_tier != ForensicEvidenceTier::ArtifactObserved
                    || applicability.executed
                    || applicability.test_outcome != "not_run"
                {
                    return Err("applicability_cannot_claim_execution");
                }
                Ok(applicability.applicability_ref.clone())
            }
            ForensicToolPayload::ExecuteIndependentControl(control) => {
                if control.receipt.evidence_tier != ForensicEvidenceTier::Executed
                    || control.receipt.artifact_ref.is_none()
                    || control.receipt.outcome != "succeeded"
                {
                    return Err("executed_control_requires_receipt");
                }
                Ok(control.control_ref.clone())
            }
        }
    }

    fn apply_payload(&mut self, payload: &ForensicToolPayload) {
        match payload {
            ForensicToolPayload::SubmitForensicFinding(input) => {
                self.findings.push(input.finding.clone())
            }
            ForensicToolPayload::SubmitForensicHypothesis(value) => {
                self.hypotheses.push(value.clone())
            }
            ForensicToolPayload::SubmitForensicLimitation(value) => {
                self.limitations.push(value.clone())
            }
            ForensicToolPayload::ValidateCandidateDiffApplicability(value) => {
                self.diff_applicability.push(value.clone())
            }
            ForensicToolPayload::ExecuteIndependentControl(value) => {
                self.executed_controls.push(value.clone())
            }
            ForensicToolPayload::QueryPriorForensicWork { .. }
            | ForensicToolPayload::GetForensicWorkByRef { .. } => {}
        }
    }

    fn reject(
        &mut self,
        call: &ForensicToolCall,
        call_digest: String,
        reason: &'static str,
    ) -> ForensicToolEvent {
        self.append_event(
            call,
            call_digest,
            ForensicToolEventStatus::Rejected,
            None,
            Some(format!("refusal:forensic-tool:{reason}")),
        )
    }

    fn append_event(
        &mut self,
        call: &ForensicToolCall,
        call_digest: String,
        status: ForensicToolEventStatus,
        result_ref: Option<String>,
        refusal_ref: Option<String>,
    ) -> ForensicToolEvent {
        let sequence = self.event_cursor() + 1;
        let event = ForensicToolEvent {
            schema: FORENSIC_TOOL_EVENT_SCHEMA_V1.into(),
            event_ref: format!("event:forensic-tool:{sequence}"),
            sequence,
            call_ref: call.call_ref.clone(),
            tool: call.payload.tool_name(),
            status,
            result_ref,
            refusal_ref,
            call_digest,
            observed_at: call.observed_at.clone(),
            call: call.clone(),
        };
        self.events.push(event.clone());
        event
    }
}

fn normalize_structured_arguments(
    value: &serde_json::Value,
) -> Result<serde_json::Value, ForensicsError> {
    let Some(object) = value.as_object() else {
        return invalid("forensic tool arguments must be a JSON object");
    };
    if object.len() == 1 {
        if let Some(arguments) = object
            .get("arguments")
            .or_else(|| object.get("input"))
            .or_else(|| object.get("call"))
        {
            return match arguments {
                serde_json::Value::String(encoded) => serde_json::from_str(encoded)
                    .map_err(|error| ForensicsError::InvalidToolEvent(error.to_string())),
                structured => Ok(structured.clone()),
            };
        }
    }
    Ok(value.clone())
}

fn validate_finding_sources(
    input: &ForensicFindingToolInput,
    catalog: &ForensicSourceCatalog,
) -> Result<(), &'static str> {
    if input.finding.source_refs.is_empty() {
        return Err("finding_requires_source");
    }
    for citation in &input.finding.source_refs {
        if citation.path.starts_with('/')
            || citation
                .path
                .split('/')
                .any(|part| matches!(part, "" | "." | ".."))
        {
            return Err("source_path_traversal");
        }
        let Some(file) = catalog.files.iter().find(|file| file.path == citation.path) else {
            return Err("source_file_missing");
        };
        if file.revision != citation.commit {
            return Err("source_revision_mismatch");
        }
        if digest_bytes(&file.bytes) != file.content_digest {
            return Err("source_bytes_changed");
        }
        let lines = file.bytes.split(|byte| *byte == b'\n').collect::<Vec<_>>();
        if citation.start_line == 0
            || citation.end_line < citation.start_line
            || citation.end_line as usize > lines.len()
        {
            return Err("source_line_bounds_invalid");
        }
        if let Some(symbol) = &citation.symbol
            && (!file.symbols.contains(symbol)
                || !String::from_utf8_lossy(&file.bytes).contains(symbol))
        {
            return Err("source_symbol_unsupported");
        }
        let window = lines[(citation.start_line - 1) as usize..citation.end_line as usize]
            .iter()
            .flat_map(|line| line.iter().copied().chain(std::iter::once(b'\n')))
            .collect::<Vec<_>>();
        if input.source_window_digests.get(&citation.source_ref) != Some(&digest_bytes(&window)) {
            return Err("source_window_changed");
        }
    }
    Ok(())
}

fn validate_binding(binding: &ForensicToolCallBinding) -> Result<(), ForensicsError> {
    if binding.coverage_generation == 0
        || binding.tool_version != FORENSIC_TOOL_VERSION_V1
        || binding.audience_ref.trim().is_empty()
    {
        return invalid("binding generation or tool version is invalid");
    }
    for digest in [&binding.source_bundle_digest, &binding.prompt_digest] {
        if !valid_digest(digest) {
            return invalid("binding digest is invalid");
        }
    }
    Ok(())
}

fn binding_matches(expected: &ForensicToolCallBinding, actual: &ForensicToolCallBinding) -> bool {
    expected.run_ref == actual.run_ref
        && expected.task_ref == actual.task_ref
        && expected.audience_ref == actual.audience_ref
        && expected.source_bundle_ref == actual.source_bundle_ref
        && expected.source_bundle_digest == actual.source_bundle_digest
        && expected.coverage_generation == actual.coverage_generation
        && expected.prompt_digest == actual.prompt_digest
        && expected.model_route_ref == actual.model_route_ref
        && expected.tool_version == actual.tool_version
        && expected.budget_ref == actual.budget_ref
}

fn digest<T: Serialize>(value: &T) -> Result<String, ForensicsError> {
    serde_json::to_vec(value)
        .map(|bytes| digest_bytes(&bytes))
        .map_err(|error| ForensicsError::InvalidToolEvent(error.to_string()))
}

fn digest_bytes(bytes: &[u8]) -> String {
    format!("sha256:{:x}", Sha256::digest(bytes))
}

fn valid_digest(value: &str) -> bool {
    value
        .strip_prefix("sha256:")
        .is_some_and(|hex| hex.len() == 64 && hex.bytes().all(|byte| byte.is_ascii_hexdigit()))
}

fn invalid<T>(message: impl Into<String>) -> Result<T, ForensicsError> {
    Err(ForensicsError::InvalidToolEvent(message.into()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ForensicCausalLink, ForensicSourceCitation};

    const REVISION: &str = "bcc2c382a324690a2fcf972c0bac3b79bf923f7b";

    fn binding(cursor: u64) -> ForensicToolCallBinding {
        ForensicToolCallBinding {
            run_ref: "run:forensic:1".into(),
            task_ref: "task:forensic:1".into(),
            actor_ref: "actor:discovery:1".into(),
            actor_role: ForensicToolActorRole::Discovery,
            audience_ref: "audience:private:owner".into(),
            source_bundle_ref: "source-bundle:forensic:1".into(),
            source_bundle_digest: format!("sha256:{}", "a".repeat(64)),
            coverage_generation: 7,
            prompt_digest: format!("sha256:{}", "b".repeat(64)),
            model_route_ref: "model-route:forensic:1".into(),
            tool_version: FORENSIC_TOOL_VERSION_V1.into(),
            budget_ref: "budget:forensic:1".into(),
            expected_event_cursor: cursor,
        }
    }

    fn source() -> ForensicSourceCatalog {
        let bytes = b"fn vulnerable() {\n    consume_secret();\n}\n".to_vec();
        ForensicSourceCatalog {
            audience_ref: "audience:private:owner".into(),
            source_bundle_ref: "source-bundle:forensic:1".into(),
            source_bundle_digest: format!("sha256:{}", "a".repeat(64)),
            coverage_generation: 7,
            missing_dependency_paths: vec![],
            files: vec![ForensicSourceFile {
                path: "src/wallet.rs".into(),
                revision: REVISION.into(),
                content_digest: digest_bytes(&bytes),
                bytes,
                symbols: vec!["vulnerable".into()],
            }],
        }
    }

    fn finding(source: &ForensicSourceCatalog) -> ForensicFindingToolInput {
        let file = &source.files[0];
        let lines = file.bytes.split(|byte| *byte == b'\n').collect::<Vec<_>>();
        let window = lines[0..2]
            .iter()
            .flat_map(|line| line.iter().copied().chain(std::iter::once(b'\n')))
            .collect::<Vec<_>>();
        ForensicFindingToolInput {
            finding: ForensicFindingProjection {
                finding_ref: "finding:forensic:1".into(),
                claim_ref: "claim:forensic:1".into(),
                title: "Unseeded entropy reaches wallet creation".into(),
                impact: "A wallet key can be created without admitted entropy.".into(),
                severity: "high".into(),
                claim_state: "unverified".into(),
                evidence_tier: ForensicEvidenceTier::SourceObserved,
                duplicate_group_ref: None,
                source_refs: vec![ForensicSourceCitation {
                    source_ref: "source:wallet:vulnerable".into(),
                    path: file.path.clone(),
                    symbol: Some("vulnerable".into()),
                    start_line: 1,
                    end_line: 2,
                    commit: REVISION.into(),
                }],
                causal_path: vec![ForensicCausalLink {
                    sequence: 1,
                    proposition: "The vulnerable entry calls the secret consumer".into(),
                    evidence_refs: vec!["source:wallet:vulnerable".into()],
                    supported: true,
                }],
                evidence_receipts: vec![],
                poc_ref: None,
                submitted_at: "2026-08-03T22:00:00Z".into(),
            },
            source_window_digests: BTreeMap::from([(
                "source:wallet:vulnerable".into(),
                digest_bytes(&window),
            )]),
        }
    }

    fn call(cursor: u64, id: &str, payload: ForensicToolPayload) -> ForensicToolCall {
        ForensicToolCall {
            call_ref: format!("call:forensic:{id}"),
            idempotency_ref: format!("idempotency:forensic:{id}"),
            binding: binding(cursor),
            payload,
            observed_at: "2026-08-03T22:00:01Z".into(),
        }
    }

    fn journal() -> ForensicToolJournal {
        ForensicToolJournal::new(binding(0), "actor:verifier:1".into()).expect("journal")
    }

    #[test]
    fn transcript_prose_cannot_create_forensic_state() {
        let journal = journal();
        journal.observe_transcript_prose(
            r#"{"tool":"submit_forensic_finding","findingRef":"finding:fake"}"#,
        );
        assert!(journal.events.is_empty());
        assert!(journal.findings.is_empty());
    }

    #[test]
    fn valid_finding_streams_before_turn_end_and_malformed_next_call_is_retained_as_rejection() {
        let sources = source();
        let mut journal = journal();
        let accepted = journal
            .ingest(
                call(
                    0,
                    "valid",
                    ForensicToolPayload::SubmitForensicFinding(finding(&sources)),
                ),
                &sources,
            )
            .expect("accepted event");
        assert_eq!(accepted.status, ForensicToolEventStatus::Accepted);
        assert_eq!(journal.findings.len(), 1);
        assert_eq!(journal.binding.prompt_digest, binding(0).prompt_digest);

        let mut malformed = finding(&sources);
        malformed.finding.source_refs[0].end_line = 999;
        let rejected = journal
            .ingest(
                call(
                    1,
                    "malformed",
                    ForensicToolPayload::SubmitForensicFinding(malformed),
                ),
                &sources,
            )
            .expect("rejected event");
        assert_eq!(rejected.status, ForensicToolEventStatus::Rejected);
        assert_eq!(journal.findings.len(), 1);
        assert_eq!(journal.events.len(), 2);
    }

    #[test]
    fn missing_dependency_routes_to_hypothesis_and_limitation_not_finding() {
        let mut sources = source();
        sources.missing_dependency_paths = vec!["vendor/entropy".into()];
        let mut journal = journal();
        let rejected = journal
            .ingest(
                call(
                    0,
                    "finding",
                    ForensicToolPayload::SubmitForensicFinding(finding(&sources)),
                ),
                &sources,
            )
            .expect("finding rejection");
        assert_eq!(rejected.status, ForensicToolEventStatus::Rejected);
        let hypothesis = ForensicHypothesisProjection {
            hypothesis_ref: "hypothesis:dependency:1".into(),
            suspected_mechanism: "The missing provider may return unseeded bytes".into(),
            supporting_refs: vec!["source:wallet:vulnerable".into()],
            missing_evidence: vec!["vendor/entropy source".into()],
            next_check: "Materialize and inspect vendor/entropy".into(),
            consequence_if_true: "Wallet keys can be predictable".into(),
            state: "unverified".into(),
            submitted_at: "2026-08-03T22:00:02Z".into(),
        };
        assert_eq!(
            journal
                .ingest(
                    call(
                        1,
                        "hypothesis",
                        ForensicToolPayload::SubmitForensicHypothesis(hypothesis),
                    ),
                    &sources,
                )
                .expect("hypothesis")
                .status,
            ForensicToolEventStatus::Accepted
        );
        assert_eq!(
            journal
                .ingest(
                    call(
                        2,
                        "limitation",
                        ForensicToolPayload::SubmitForensicLimitation(
                            ForensicLimitationProjection {
                                limitation_ref: "limitation:dependency:1".into(),
                                class_ref: "limitation:missing-dependency".into(),
                                message: "vendor/entropy is not materialized".into(),
                                affected_source_refs: vec!["source:wallet:vulnerable".into()],
                                required_next_check: "Materialize vendor/entropy".into(),
                                submitted_at: "2026-08-03T22:00:03Z".into(),
                            },
                        ),
                    ),
                    &sources,
                )
                .expect("limitation")
                .status,
            ForensicToolEventStatus::Accepted
        );
        assert!(journal.findings.is_empty());
        assert_eq!(
            (journal.hypotheses.len(), journal.limitations.len()),
            (1, 1)
        );
    }

    #[test]
    fn source_validation_rejects_changed_bytes_traversal_revision_lines_and_symbols() {
        let base_sources = source();
        for (index, mutation) in [
            "missing_file",
            "traversal",
            "revision",
            "lines",
            "changed_bytes",
            "symbol",
        ]
        .into_iter()
        .enumerate()
        {
            let mut sources = base_sources.clone();
            let mut input = finding(&sources);
            match mutation {
                "missing_file" => input.finding.source_refs[0].path = "src/missing.rs".into(),
                "traversal" => input.finding.source_refs[0].path = "../wallet.rs".into(),
                "revision" => input.finding.source_refs[0].commit = "0".repeat(40),
                "lines" => input.finding.source_refs[0].end_line = 999,
                "changed_bytes" => sources.files[0].bytes.push(b'!'),
                "symbol" => input.finding.source_refs[0].symbol = Some("not_present".into()),
                _ => unreachable!(),
            }
            let mut journal = journal();
            let event = journal
                .ingest(
                    call(
                        0,
                        &format!("source-{index}"),
                        ForensicToolPayload::SubmitForensicFinding(input),
                    ),
                    &sources,
                )
                .expect("rejection event");
            assert_eq!(
                event.status,
                ForensicToolEventStatus::Rejected,
                "{mutation}"
            );
            assert!(journal.findings.is_empty());
        }
    }

    #[test]
    fn applicability_is_artifact_evidence_and_discovery_cannot_execute_controls() {
        let sources = source();
        let mut journal = journal();
        let applicability = ForensicDiffApplicabilityProjection {
            applicability_ref: "applicability:diff:1".into(),
            diff_digest: format!("sha256:{}", "d".repeat(64)),
            target_revision: REVISION.into(),
            applicable: true,
            evidence_tier: ForensicEvidenceTier::ArtifactObserved,
            executed: false,
            test_outcome: "not_run".into(),
            observed_at: "2026-08-03T22:00:04Z".into(),
        };
        assert_eq!(
            journal
                .ingest(
                    call(
                        0,
                        "applicability",
                        ForensicToolPayload::ValidateCandidateDiffApplicability(applicability),
                    ),
                    &sources,
                )
                .expect("applicability")
                .status,
            ForensicToolEventStatus::Accepted
        );
        let control = ForensicExecutedControlProjection {
            control_ref: "control:forensic:1".into(),
            finding_ref: "finding:forensic:1".into(),
            receipt: ForensicEvidenceReceiptProjection {
                receipt_ref: "receipt:control:1".into(),
                evidence_tier: ForensicEvidenceTier::Executed,
                outcome: "succeeded".into(),
                artifact_ref: Some("artifact:control:1".into()),
                verifier_verdict: Some("confirmed".into()),
                observed_at: "2026-08-03T22:00:05Z".into(),
            },
        };
        let denied = journal
            .ingest(
                call(
                    1,
                    "discovery-control",
                    ForensicToolPayload::ExecuteIndependentControl(control.clone()),
                ),
                &sources,
            )
            .expect("denied discovery control");
        assert_eq!(denied.status, ForensicToolEventStatus::Rejected);

        let mut verifier_call = call(
            2,
            "verifier-control",
            ForensicToolPayload::ExecuteIndependentControl(control),
        );
        verifier_call.binding.actor_ref = "actor:verifier:1".into();
        verifier_call.binding.actor_role = ForensicToolActorRole::IndependentVerifier;
        assert_eq!(
            journal
                .ingest(verifier_call, &sources)
                .expect("verified control")
                .status,
            ForensicToolEventStatus::Accepted
        );
        assert_eq!(journal.executed_controls.len(), 1);
    }

    #[test]
    fn replay_and_restore_preserve_one_event_and_cursor() {
        let sources = source();
        let mut journal = journal();
        let call = call(
            0,
            "replay",
            ForensicToolPayload::SubmitForensicFinding(finding(&sources)),
        );
        let first = journal.ingest(call.clone(), &sources).expect("first");
        let replay = journal.ingest(call, &sources).expect("replay");
        assert_eq!(replay, first);
        assert_eq!(journal.events.len(), 1);
        let encoded = serde_json::to_vec(&journal).expect("serialize journal");
        let restored: ForensicToolJournal = serde_json::from_slice(&encoded).expect("restore");
        restored.validate().expect("validate restored journal");
        assert_eq!(restored.resume_after(0).expect("resume"), journal.events);
        assert!(restored.resume_after(1).expect("tail").is_empty());
    }

    #[test]
    fn typed_fallback_and_visible_tool_paths_have_one_canonical_projection() {
        let sources = source();
        let submitted = call(
            0,
            "conformance",
            ForensicToolPayload::SubmitForensicFinding(finding(&sources)),
        );
        let mut visible = journal();
        let mut fallback = journal();
        visible
            .ingest(submitted.clone(), &sources)
            .expect("visible tool path");
        fallback
            .ingest_typed_fallback(submitted, &sources)
            .expect("typed fallback path");
        assert_eq!(fallback, visible);
    }

    #[test]
    fn provider_argument_envelopes_have_one_visible_projection() {
        let sources = source();
        let submitted = call(
            0,
            "provider",
            ForensicToolPayload::QueryPriorForensicWork {
                query_ref: "query:forensic:provider".into(),
            },
        );
        let direct = serde_json::to_value(submitted).expect("canonical call json");
        let encoded = direct.to_string();
        let variants = [
            direct.clone(),
            serde_json::json!({ "input": direct }),
            serde_json::json!({ "arguments": encoded }),
        ];
        let mut projections = Vec::new();
        for variant in variants {
            let mut journal = journal();
            let event = journal
                .ingest_visible_tool_json("query_prior_forensic_work", &variant, &sources)
                .expect("visible tool")
                .expect("forensic tool");
            projections.push((event, journal));
        }
        assert!(projections.windows(2).all(|pair| pair[0] == pair[1]));
    }

    #[test]
    fn restore_rejects_capability_and_cursor_drift() {
        let sources = source();
        let mut journal = journal();
        journal
            .ingest(
                call(
                    0,
                    "restore",
                    ForensicToolPayload::SubmitForensicFinding(finding(&sources)),
                ),
                &sources,
            )
            .expect("event");
        journal
            .discovery_tools
            .insert(ForensicToolName::ExecuteIndependentControl);
        assert!(journal.validate().is_err());
        journal
            .discovery_tools
            .remove(&ForensicToolName::ExecuteIndependentControl);
        journal.events[0].sequence = 9;
        assert!(journal.validate().is_err());
    }
}
