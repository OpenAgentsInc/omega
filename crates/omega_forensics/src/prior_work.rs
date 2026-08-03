use serde::{Deserialize, Serialize};

use crate::ForensicsError;

pub const FORENSIC_OCCURRENCE_IDENTITY_SCHEMA_V1: &str =
    "openagents.forensic_occurrence_identity.v1";
pub const FORENSIC_ROOT_CAUSE_IDENTITY_SCHEMA_V1: &str =
    "openagents.forensic_root_cause_identity.v1";
pub const FORENSIC_PRIOR_WORK_QUERY_RECEIPT_SCHEMA_V1: &str =
    "openagents.forensic_prior_work_query_receipt.v1";
pub const FORENSIC_DUPLICATE_CONTINUATION_SCHEMA_V1: &str =
    "openagents.omega.forensic_duplicate_continuation.v1";
pub const FORENSIC_OCCURRENCE_ALGORITHM_V1: &str = "forensic-occurrence-sha256-v1";
pub const FORENSIC_ROOT_CAUSE_ALGORITHM_V1: &str = "forensic-root-cause-semantic-v1";

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ForensicWorkDisposition {
    Confirmed,
    Dismissed,
    Rejected,
    Inconclusive,
    Expired,
    Superseded,
    Corrected,
    Duplicate,
    Retained,
}

impl ForensicWorkDisposition {
    pub const ALL: [Self; 9] = [
        Self::Confirmed,
        Self::Dismissed,
        Self::Rejected,
        Self::Inconclusive,
        Self::Expired,
        Self::Superseded,
        Self::Corrected,
        Self::Duplicate,
        Self::Retained,
    ];
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ForensicAudienceVisibility {
    Public,
    Organization,
    Private,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ForensicWorkAudience {
    pub visibility: ForensicAudienceVisibility,
    pub organization_ref: Option<String>,
    pub principal_ref: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ForensicOccurrenceIdentity {
    pub schema: String,
    pub algorithm_version: String,
    pub occurrence_ref: String,
    pub repository_ref: String,
    pub revision: String,
    pub path: String,
    pub symbol: Option<String>,
    pub start_line: u64,
    pub end_line: u64,
    pub source_window_digest: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ForensicRootCauseIdentity {
    pub schema: String,
    pub algorithm_version: String,
    pub root_cause_ref: String,
    pub mechanism_class: String,
    pub causal_mechanism: String,
    pub affected_behavior: String,
    pub security_boundary: String,
    pub normalized_mechanism_digest: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ForensicDispositionEvent {
    pub event_ref: String,
    pub work_ref: String,
    pub disposition: ForensicWorkDisposition,
    pub reason: String,
    pub actor_ref: String,
    pub occurred_at: String,
    pub idempotency_ref: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ForensicWorkRelationKind {
    Duplicate,
    Related,
    Supersedes,
    SplitFrom,
    MergedInto,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ForensicRelationConfidence {
    Exact,
    Probable,
    Possible,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ForensicWorkRelationEvent {
    pub relation_ref: String,
    pub from_work_ref: String,
    pub kind: ForensicWorkRelationKind,
    pub target_work_ref: String,
    pub confidence: ForensicRelationConfidence,
    pub reason: String,
    pub actor_ref: String,
    pub occurred_at: String,
    pub idempotency_ref: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ForensicPriorWorkRecord {
    pub record_ref: String,
    pub root_cause: ForensicRootCauseIdentity,
    pub primary_work_ref: String,
    pub work_refs: Vec<String>,
    pub occurrences: Vec<ForensicOccurrenceIdentity>,
    pub audience: ForensicWorkAudience,
    pub causal_chain_summary: String,
    pub prompt_refs: Vec<String>,
    pub source_refs: Vec<String>,
    pub evidence_refs: Vec<String>,
    pub dispositions: Vec<ForensicDispositionEvent>,
    pub relations: Vec<ForensicWorkRelationEvent>,
    pub first_identified_at: String,
    pub updated_at: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ForensicPriorWorkQueryMode {
    Exact,
    Semantic,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ForensicPriorWorkQuery {
    pub query_ref: String,
    pub principal_ref: String,
    pub organization_refs: Vec<String>,
    pub include_public: bool,
    pub mode: ForensicPriorWorkQueryMode,
    pub exact_ref: Option<String>,
    pub text: Option<String>,
    pub disposition_filter: Vec<ForensicWorkDisposition>,
    pub cursor: Option<String>,
    pub limit: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ForensicPriorWorkMatch {
    pub record: ForensicPriorWorkRecord,
    pub matched_work_refs: Vec<String>,
    pub matched_occurrence_refs: Vec<String>,
    pub score_basis_points: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ForensicPriorWorkQueryReceipt {
    pub schema: String,
    pub receipt_ref: String,
    pub query_ref: String,
    pub state_revision: u64,
    pub query_digest: String,
    pub result_digest: String,
    pub authorized_population_complete: bool,
    pub loss_refs: Vec<String>,
    pub searched_authorized_count: u64,
    pub returned_count: u64,
    pub observed_at: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ForensicPriorWorkQueryResult {
    pub matches: Vec<ForensicPriorWorkMatch>,
    pub next_cursor: Option<String>,
    pub receipt: ForensicPriorWorkQueryReceipt,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ForensicPriorWorkSubmission {
    pub work_ref: String,
    pub repository_ref: String,
    pub revision: String,
    pub path: String,
    pub symbol: Option<String>,
    pub start_line: u64,
    pub end_line: u64,
    pub source_window_digest: String,
    pub mechanism_class: String,
    pub causal_mechanism: String,
    pub affected_behavior: String,
    pub security_boundary: String,
    pub causal_chain_summary: String,
    pub prompt_refs: Vec<String>,
    pub source_refs: Vec<String>,
    pub evidence_refs: Vec<String>,
    pub audience: ForensicWorkAudience,
    pub disposition: ForensicWorkDisposition,
    pub actor_ref: String,
    pub submitted_at: String,
    pub idempotency_ref: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ForensicRelationCommand {
    pub from_work_ref: String,
    pub target_work_ref: String,
    pub kind: ForensicWorkRelationKind,
    pub confidence: ForensicRelationConfidence,
    pub reason: String,
    pub actor_ref: String,
    pub occurred_at: String,
    pub idempotency_ref: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ForensicDispositionCommand {
    pub work_ref: String,
    pub disposition: ForensicWorkDisposition,
    pub reason: String,
    pub actor_ref: String,
    pub occurred_at: String,
    pub idempotency_ref: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ForensicCandidateDecision {
    Duplicate,
    Submitted,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RankedForensicCandidate {
    pub rank: u64,
    pub search_query: ForensicPriorWorkQuery,
    pub submission: ForensicPriorWorkSubmission,
    pub started_at_milliseconds: u64,
    pub decided_at_milliseconds: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ForensicCandidateContinuationStep {
    pub candidate_rank: u64,
    pub query: ForensicPriorWorkQuery,
    pub query_receipt_ref: String,
    pub returned_work_refs: Vec<String>,
    pub decision: ForensicCandidateDecision,
    pub resulting_record_ref: String,
    pub decided_at_milliseconds: u64,
    pub time_to_next_candidate_milliseconds: Option<u64>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ForensicDuplicateContinuationReceipt {
    pub schema: String,
    pub task_ref: String,
    pub steps: Vec<ForensicCandidateContinuationStep>,
    pub completed_at_milliseconds: u64,
}

pub trait ForensicPriorWorkService {
    type Error;

    fn query(
        &mut self,
        query: &ForensicPriorWorkQuery,
    ) -> Result<ForensicPriorWorkQueryResult, Self::Error>;
    fn submit(
        &mut self,
        submission: &ForensicPriorWorkSubmission,
    ) -> Result<ForensicPriorWorkRecord, Self::Error>;
}

pub fn continue_ranked_forensic_discovery<Service: ForensicPriorWorkService>(
    service: &mut Service,
    task_ref: String,
    candidates: &[RankedForensicCandidate],
) -> Result<ForensicDuplicateContinuationReceipt, Service::Error> {
    let mut steps = Vec::with_capacity(candidates.len());
    for (index, candidate) in candidates.iter().enumerate() {
        let result = service.query(&candidate.search_query)?;
        let returned_work_refs = result
            .matches
            .iter()
            .flat_map(|prior| prior.record.work_refs.iter().cloned())
            .collect::<Vec<_>>();
        let decision = if result.matches.is_empty() {
            ForensicCandidateDecision::Submitted
        } else {
            ForensicCandidateDecision::Duplicate
        };
        // Submission is append-only in both cases. The authority records a
        // same-root-cause submission as a duplicate relation and disposition.
        let record = service.submit(&candidate.submission)?;
        let time_to_next_candidate_milliseconds = candidates.get(index + 1).map(|next| {
            next.started_at_milliseconds
                .saturating_sub(candidate.decided_at_milliseconds)
        });
        steps.push(ForensicCandidateContinuationStep {
            candidate_rank: candidate.rank,
            query: candidate.search_query.clone(),
            query_receipt_ref: result.receipt.receipt_ref,
            returned_work_refs,
            decision,
            resulting_record_ref: record.record_ref,
            decided_at_milliseconds: candidate.decided_at_milliseconds,
            time_to_next_candidate_milliseconds,
        });
    }
    Ok(ForensicDuplicateContinuationReceipt {
        schema: FORENSIC_DUPLICATE_CONTINUATION_SCHEMA_V1.into(),
        task_ref,
        steps,
        completed_at_milliseconds: candidates
            .last()
            .map_or(0, |candidate| candidate.decided_at_milliseconds),
    })
}

impl ForensicPriorWorkQuery {
    pub fn validate(&self) -> Result<(), ForensicsError> {
        valid_ref("query", &self.query_ref)?;
        valid_ref("principal", &self.principal_ref)?;
        if self.organization_refs.len() > 128 || self.disposition_filter.is_empty() {
            return invalid("query bounds are invalid");
        }
        if self.limit == 0 || self.limit > 100 {
            return invalid("query limit is outside 1..=100");
        }
        match self.mode {
            ForensicPriorWorkQueryMode::Exact
                if self.exact_ref.is_none() || self.text.is_some() =>
            {
                invalid("exact query requires only exactRef")
            }
            ForensicPriorWorkQueryMode::Semantic
                if self
                    .text
                    .as_deref()
                    .is_none_or(|text| text.trim().len() < 2)
                    || self.exact_ref.is_some() =>
            {
                invalid("semantic query requires only bounded text")
            }
            _ => Ok(()),
        }
    }
}

impl ForensicPriorWorkQueryResult {
    pub fn validate(&self) -> Result<(), ForensicsError> {
        if self.matches.len() > 100 || self.receipt.returned_count != self.matches.len() as u64 {
            return invalid("query result count does not match its receipt");
        }
        if self.receipt.schema != FORENSIC_PRIOR_WORK_QUERY_RECEIPT_SCHEMA_V1 {
            return invalid("query receipt schema is unsupported");
        }
        valid_digest("query", &self.receipt.query_digest)?;
        valid_digest("result", &self.receipt.result_digest)?;
        for matched in &self.matches {
            if matched.score_basis_points > 10_000 {
                return invalid("match score exceeds 10000 basis points");
            }
            matched.record.validate()?;
        }
        Ok(())
    }
}

impl ForensicPriorWorkRecord {
    pub fn validate(&self) -> Result<(), ForensicsError> {
        valid_ref("record", &self.record_ref)?;
        valid_ref("primary Work", &self.primary_work_ref)?;
        if self.work_refs.is_empty() || self.occurrences.is_empty() || self.dispositions.is_empty()
        {
            return invalid("record omits Work, occurrence, or disposition history");
        }
        self.root_cause.validate()?;
        self.audience.validate()?;
        for occurrence in &self.occurrences {
            occurrence.validate()?;
        }
        Ok(())
    }
}

impl ForensicOccurrenceIdentity {
    pub fn validate(&self) -> Result<(), ForensicsError> {
        if self.schema != FORENSIC_OCCURRENCE_IDENTITY_SCHEMA_V1
            || self.algorithm_version != FORENSIC_OCCURRENCE_ALGORITHM_V1
        {
            return invalid("occurrence identity version is unsupported");
        }
        if self.revision.len() != 40
            || !self.revision.bytes().all(|byte| byte.is_ascii_hexdigit())
            || self.start_line == 0
            || self.end_line < self.start_line
            || self.path.is_empty()
        {
            return invalid("occurrence coordinates are invalid");
        }
        valid_ref("occurrence", &self.occurrence_ref)?;
        valid_ref("repository", &self.repository_ref)?;
        valid_digest("source window", &self.source_window_digest)
    }
}

impl ForensicRootCauseIdentity {
    pub fn validate(&self) -> Result<(), ForensicsError> {
        if self.schema != FORENSIC_ROOT_CAUSE_IDENTITY_SCHEMA_V1
            || self.algorithm_version != FORENSIC_ROOT_CAUSE_ALGORITHM_V1
        {
            return invalid("root-cause identity version is unsupported");
        }
        valid_ref("root cause", &self.root_cause_ref)?;
        valid_ref("mechanism class", &self.mechanism_class)?;
        valid_digest("normalized mechanism", &self.normalized_mechanism_digest)?;
        if [
            &self.causal_mechanism,
            &self.affected_behavior,
            &self.security_boundary,
        ]
        .iter()
        .any(|value| value.trim().is_empty())
        {
            return invalid("root-cause semantics are incomplete");
        }
        Ok(())
    }
}

impl ForensicWorkAudience {
    pub fn validate(&self) -> Result<(), ForensicsError> {
        let valid = match self.visibility {
            ForensicAudienceVisibility::Public => {
                self.organization_ref.is_none() && self.principal_ref.is_none()
            }
            ForensicAudienceVisibility::Organization => {
                self.organization_ref.is_some() && self.principal_ref.is_none()
            }
            ForensicAudienceVisibility::Private => {
                self.organization_ref.is_none() && self.principal_ref.is_some()
            }
        };
        if valid {
            Ok(())
        } else {
            invalid("audience coordinates do not match visibility")
        }
    }
}

impl ForensicDuplicateContinuationReceipt {
    pub fn validate(&self) -> Result<(), ForensicsError> {
        if self.schema != FORENSIC_DUPLICATE_CONTINUATION_SCHEMA_V1 || self.steps.is_empty() {
            return invalid("duplicate-continuation receipt is empty or unsupported");
        }
        valid_ref("task", &self.task_ref)?;
        for (index, step) in self.steps.iter().enumerate() {
            if step.candidate_rank != index as u64 + 1 {
                return invalid("candidate ranks are not exact and ordered");
            }
            step.query.validate()?;
            if let Some(elapsed) = step.time_to_next_candidate_milliseconds
                && elapsed == 0
            {
                return invalid("time to next candidate must be positive when present");
            }
        }
        Ok(())
    }
}

fn valid_ref(label: &str, value: &str) -> Result<(), ForensicsError> {
    if value.len() < 3 || value.len() > 512 || value.chars().any(char::is_whitespace) {
        invalid(format!("{label} ref is invalid"))
    } else {
        Ok(())
    }
}

fn valid_digest(label: &str, value: &str) -> Result<(), ForensicsError> {
    let Some(hex) = value.strip_prefix("sha256:") else {
        return invalid(format!("{label} digest does not use sha256"));
    };
    if hex.len() != 64 || !hex.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return invalid(format!("{label} digest is invalid"));
    }
    Ok(())
}

fn invalid<T>(message: impl Into<String>) -> Result<T, ForensicsError> {
    Err(ForensicsError::InvalidPriorWork(message.into()))
}

#[cfg(test)]
mod tests {
    use std::collections::VecDeque;

    use super::*;

    fn query(query_ref: &str) -> ForensicPriorWorkQuery {
        ForensicPriorWorkQuery {
            query_ref: query_ref.into(),
            principal_ref: "principal:omega:owner".into(),
            organization_refs: vec!["organization:openagents".into()],
            include_public: true,
            mode: ForensicPriorWorkQueryMode::Semantic,
            exact_ref: None,
            text: Some("unseeded entropy fallback".into()),
            disposition_filter: ForensicWorkDisposition::ALL.into(),
            cursor: None,
            limit: 25,
        }
    }

    fn occurrence(
        path: &str,
        revision: &str,
        digest_character: char,
    ) -> ForensicOccurrenceIdentity {
        ForensicOccurrenceIdentity {
            schema: FORENSIC_OCCURRENCE_IDENTITY_SCHEMA_V1.into(),
            algorithm_version: FORENSIC_OCCURRENCE_ALGORITHM_V1.into(),
            occurrence_ref: format!("occurrence:{digest_character}"),
            repository_ref: "repository:coldcard:firmware".into(),
            revision: revision.into(),
            path: path.into(),
            symbol: Some("rng_get_bytes".into()),
            start_line: 10,
            end_line: 24,
            source_window_digest: format!("sha256:{}", digest_character.to_string().repeat(64)),
        }
    }

    fn record(
        work_ref: &str,
        occurrences: Vec<ForensicOccurrenceIdentity>,
    ) -> ForensicPriorWorkRecord {
        ForensicPriorWorkRecord {
            record_ref: format!("forensic-record:{work_ref}"),
            root_cause: ForensicRootCauseIdentity {
                schema: FORENSIC_ROOT_CAUSE_IDENTITY_SCHEMA_V1.into(),
                algorithm_version: FORENSIC_ROOT_CAUSE_ALGORITHM_V1.into(),
                root_cause_ref: "root-cause:entropy-fallback".into(),
                mechanism_class: "mechanism:entropy:unseeded-fallback".into(),
                causal_mechanism: "an unseeded fallback reaches key generation".into(),
                affected_behavior: "keys can use insufficient entropy".into(),
                security_boundary: "entropy provider to key generator".into(),
                normalized_mechanism_digest: format!("sha256:{}", "c".repeat(64)),
            },
            primary_work_ref: work_ref.into(),
            work_refs: vec![work_ref.into()],
            occurrences,
            audience: ForensicWorkAudience {
                visibility: ForensicAudienceVisibility::Organization,
                organization_ref: Some("organization:openagents".into()),
                principal_ref: None,
            },
            causal_chain_summary: "provider -> fallback -> key".into(),
            prompt_refs: vec!["prompt:entropy:v1".into()],
            source_refs: vec!["source:coldcard:rng".into()],
            evidence_refs: vec!["evidence:coldcard:trace".into()],
            dispositions: vec![ForensicDispositionEvent {
                event_ref: "event:disposition:1".into(),
                work_ref: work_ref.into(),
                disposition: ForensicWorkDisposition::Confirmed,
                reason: "fixture".into(),
                actor_ref: "principal:reviewer".into(),
                occurred_at: "2026-08-03T20:00:00Z".into(),
                idempotency_ref: "idempotency:disposition:1".into(),
            }],
            relations: vec![],
            first_identified_at: "2026-08-03T20:00:00Z".into(),
            updated_at: "2026-08-03T20:00:00Z".into(),
        }
    }

    fn result(
        query_ref: &str,
        matches: Vec<ForensicPriorWorkMatch>,
    ) -> ForensicPriorWorkQueryResult {
        ForensicPriorWorkQueryResult {
            receipt: ForensicPriorWorkQueryReceipt {
                schema: FORENSIC_PRIOR_WORK_QUERY_RECEIPT_SCHEMA_V1.into(),
                receipt_ref: format!("receipt:{query_ref}"),
                query_ref: query_ref.into(),
                state_revision: 2,
                query_digest: format!("sha256:{}", "d".repeat(64)),
                result_digest: format!("sha256:{}", "e".repeat(64)),
                authorized_population_complete: true,
                loss_refs: vec![],
                searched_authorized_count: matches.len() as u64,
                returned_count: matches.len() as u64,
                observed_at: "2026-08-03T20:00:01Z".into(),
            },
            matches,
            next_cursor: None,
        }
    }

    fn submission(work_ref: &str, path: &str, id: &str) -> ForensicPriorWorkSubmission {
        ForensicPriorWorkSubmission {
            work_ref: work_ref.into(),
            repository_ref: "repository:coldcard:firmware".into(),
            revision: "bcc2c382a324690a2fcf972c0bac3b79bf923f7b".into(),
            path: path.into(),
            symbol: Some("rng_get_bytes".into()),
            start_line: 10,
            end_line: 24,
            source_window_digest: format!("sha256:{}", id.repeat(64)),
            mechanism_class: "mechanism:entropy:unseeded-fallback".into(),
            causal_mechanism: "An unseeded fallback reaches key generation".into(),
            affected_behavior: "Keys can use insufficient entropy".into(),
            security_boundary: "entropy provider to key generator".into(),
            causal_chain_summary: "provider -> fallback -> key".into(),
            prompt_refs: vec!["prompt:entropy:v1".into()],
            source_refs: vec!["source:coldcard:rng".into()],
            evidence_refs: vec!["evidence:coldcard:trace".into()],
            audience: ForensicWorkAudience {
                visibility: ForensicAudienceVisibility::Organization,
                organization_ref: Some("organization:openagents".into()),
                principal_ref: None,
            },
            disposition: ForensicWorkDisposition::Confirmed,
            actor_ref: "principal:reviewer".into(),
            submitted_at: "2026-08-03T20:00:00Z".into(),
            idempotency_ref: format!("idempotency:{id}"),
        }
    }

    struct FixtureService {
        queries: VecDeque<ForensicPriorWorkQueryResult>,
        submitted: Vec<String>,
    }

    impl ForensicPriorWorkService for FixtureService {
        type Error = String;

        fn query(
            &mut self,
            _: &ForensicPriorWorkQuery,
        ) -> Result<ForensicPriorWorkQueryResult, Self::Error> {
            self.queries
                .pop_front()
                .ok_or_else(|| "missing query fixture".into())
        }

        fn submit(
            &mut self,
            submission: &ForensicPriorWorkSubmission,
        ) -> Result<ForensicPriorWorkRecord, Self::Error> {
            self.submitted.push(submission.work_ref.clone());
            Ok(record(
                &submission.work_ref,
                vec![occurrence(&submission.path, &submission.revision, 'a')],
            ))
        }
    }

    #[test]
    fn cross_file_and_revision_occurrences_remain_distinct_under_one_root_cause() {
        let record = record(
            "work:forensic:coldcard:root",
            vec![
                occurrence(
                    "shared/hmac.c",
                    "bcc2c382a324690a2fcf972c0bac3b79bf923f7b",
                    'a',
                ),
                occurrence(
                    "unix/random.c",
                    "ca72463709f4e3f8964952039d5caf955f566a87",
                    'b',
                ),
            ],
        );
        record.validate().expect("valid cross-revision record");
        assert_eq!(record.occurrences.len(), 2);
        assert_eq!(
            record.root_cause.root_cause_ref,
            "root-cause:entropy-fallback"
        );
        assert_ne!(
            record.occurrences[0].occurrence_ref,
            record.occurrences[1].occurrence_ref
        );
    }

    #[test]
    fn every_retained_disposition_round_trips_through_the_query_contract() {
        let encoded = serde_json::to_string(&query("query:all-dispositions")).expect("encode");
        let decoded: ForensicPriorWorkQuery = serde_json::from_str(&encoded).expect("decode");
        assert_eq!(decoded.disposition_filter, ForensicWorkDisposition::ALL);
        decoded.validate().expect("valid query");
    }

    #[test]
    fn duplicate_first_candidate_does_not_end_discovery() {
        let prior = record(
            "work:forensic:known",
            vec![occurrence(
                "shared/hmac.c",
                "bcc2c382a324690a2fcf972c0bac3b79bf923f7b",
                'a',
            )],
        );
        let mut service = FixtureService {
            queries: VecDeque::from([
                result(
                    "query:rank:1",
                    vec![ForensicPriorWorkMatch {
                        record: prior,
                        matched_work_refs: vec!["work:forensic:known".into()],
                        matched_occurrence_refs: vec![],
                        score_basis_points: 10_000,
                    }],
                ),
                result("query:rank:2", vec![]),
            ]),
            submitted: vec![],
        };
        let receipt = continue_ranked_forensic_discovery(
            &mut service,
            "task:forensic:coldcard".into(),
            &[
                RankedForensicCandidate {
                    rank: 1,
                    search_query: query("query:rank:1"),
                    submission: submission("work:forensic:duplicate", "unix/random.c", "a"),
                    started_at_milliseconds: 100,
                    decided_at_milliseconds: 130,
                },
                RankedForensicCandidate {
                    rank: 2,
                    search_query: query("query:rank:2"),
                    submission: submission("work:forensic:new", "stm32/rng.c", "b"),
                    started_at_milliseconds: 150,
                    decided_at_milliseconds: 210,
                },
            ],
        )
        .expect("continuation");
        receipt.validate().expect("valid receipt");
        assert_eq!(
            service.submitted,
            ["work:forensic:duplicate", "work:forensic:new"]
        );
        assert_eq!(
            receipt.steps[0].decision,
            ForensicCandidateDecision::Duplicate
        );
        assert_eq!(
            receipt.steps[0].time_to_next_candidate_milliseconds,
            Some(20)
        );
        assert_eq!(
            receipt.steps[1].decision,
            ForensicCandidateDecision::Submitted
        );
    }

    #[test]
    fn audience_shape_rejects_self_inconsistent_visibility() {
        let audience = ForensicWorkAudience {
            visibility: ForensicAudienceVisibility::Private,
            organization_ref: Some("organization:forbidden".into()),
            principal_ref: None,
        };
        assert!(matches!(
            audience.validate(),
            Err(ForensicsError::InvalidPriorWork(_))
        ));
    }
}
