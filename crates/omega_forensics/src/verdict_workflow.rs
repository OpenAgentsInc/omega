use super::{ForensicEvidenceTier, ForensicFindingProjection, ForensicsError};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeSet;

pub const INDEPENDENT_VERIFICATION_CASE_SCHEMA_V1: &str =
    "openagents.omega.independent-verification-case.v1";

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ForensicPocIdentity {
    pub poc_ref: String,
    pub content_digest: String,
    pub supersedes_poc_ref: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ForensicModelProvenance {
    pub provider_ref: String,
    pub model_ref: String,
    pub route_ref: String,
    pub configuration_digest: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct IndependentVerifierEnvelope {
    pub request_ref: String,
    pub finding: ForensicFindingProjection,
    pub finding_digest: String,
    pub assumptions: Vec<String>,
    pub occurrence_refs: Vec<String>,
    pub root_cause_ref: String,
    pub source_bundle_ref: String,
    pub source_bundle_digest: String,
    pub coverage_manifest_ref: String,
    pub coverage_manifest_digest: String,
    pub original_poc: ForensicPocIdentity,
    pub discovery_actor_ref: String,
    pub prompt_digest: String,
    pub prompt_lineage_refs: Vec<String>,
    pub model_provenance: ForensicModelProvenance,
    pub tool_surface_refs: Vec<String>,
    pub evidence_refs: Vec<String>,
    pub verifier_actor_ref: String,
    pub verifier_capability_refs: Vec<String>,
    pub vulnerable_revision_digest: String,
    pub fixed_revision_digest: String,
    pub requested_at: String,
    pub canonical_digest: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IndependentVerificationOutcome {
    Confirmed,
    Dismissed,
    Inconclusive,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VerificationEvidenceKind {
    SourceValidation,
    DependencyValidation,
    PocApplied,
    VulnerableControl,
    FixedControl,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct IndependentVerificationEvidence {
    pub receipt_ref: String,
    pub kind: VerificationEvidenceKind,
    pub evidence_tier: ForensicEvidenceTier,
    pub revision_digest: String,
    pub command_digest: String,
    pub environment_digest: String,
    pub output_digest: String,
    pub outcome: String,
    pub observed_test_outcome: Option<String>,
    pub observed_at: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ImmutableInitialVerdict {
    pub verdict_ref: String,
    pub outcome: IndependentVerificationOutcome,
    pub verifier_actor_ref: String,
    pub rationale_digest: String,
    pub evidence_refs: Vec<String>,
    pub stored_at: String,
    pub canonical_digest: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IndependentVerificationState {
    Requested,
    Running,
    VerdictStored,
    Refused,
    WorkerUnavailable,
    SourceUnavailable,
    Inconclusive,
    Interrupted,
    FailedControl,
    StaleSource,
    RecoveryRequired,
    CleanupFailed,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct VerificationSettlement {
    pub settlement_ref: String,
    pub outcome: IndependentVerificationOutcome,
    pub verifier_actor_ref: String,
    pub rationale_digest: String,
    pub evidence: Vec<IndependentVerificationEvidence>,
    pub settled_at: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct IndependentVerificationEvent {
    pub sequence: u64,
    pub state: IndependentVerificationState,
    pub event_ref: String,
    pub payload_digest: String,
    pub observed_at: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct IndependentVerificationCase {
    pub schema: String,
    pub envelope: IndependentVerifierEnvelope,
    pub state: IndependentVerificationState,
    pub initial_verdict: Option<ImmutableInitialVerdict>,
    pub evidence: Vec<IndependentVerificationEvidence>,
    pub poc_history: Vec<ForensicPocIdentity>,
    pub events: Vec<IndependentVerificationEvent>,
    pub remediation_enabled: bool,
    pub completed: bool,
    pub canonical_digest: String,
}

impl IndependentVerifierEnvelope {
    pub fn seal(mut self) -> Result<Self, ForensicsError> {
        self.canonical_digest.clear();
        self.validate_unsealed()?;
        self.finding_digest = digest_json(&self.finding)?;
        self.canonical_digest = digest_json(&self)?;
        Ok(self)
    }
    fn validate_unsealed(&self) -> Result<(), ForensicsError> {
        if [
            &self.request_ref,
            &self.root_cause_ref,
            &self.source_bundle_ref,
            &self.coverage_manifest_ref,
            &self.original_poc.poc_ref,
            &self.discovery_actor_ref,
            &self.model_provenance.provider_ref,
            &self.model_provenance.model_ref,
            &self.model_provenance.route_ref,
            &self.verifier_actor_ref,
            &self.requested_at,
        ]
        .iter()
        .any(|value| value.trim().is_empty())
            || self.assumptions.iter().any(|value| value.trim().is_empty())
            || self.original_poc.supersedes_poc_ref.is_some()
            || self.discovery_actor_ref == self.verifier_actor_ref
            || self.assumptions.is_empty()
            || self.occurrence_refs.is_empty()
            || self.prompt_lineage_refs.is_empty()
            || self.tool_surface_refs.is_empty()
            || self.evidence_refs.is_empty()
            || self.verifier_capability_refs.is_empty()
            || self.finding.poc_ref.as_deref() != Some(self.original_poc.poc_ref.as_str())
            || self.vulnerable_revision_digest == self.fixed_revision_digest
        {
            return invalid(
                "verification envelope is incomplete, self-confirming, or target-ambiguous",
            );
        }
        for digest in [
            &self.source_bundle_digest,
            &self.coverage_manifest_digest,
            &self.original_poc.content_digest,
            &self.prompt_digest,
            &self.model_provenance.configuration_digest,
            &self.vulnerable_revision_digest,
            &self.fixed_revision_digest,
        ] {
            validate_digest(digest)?;
        }
        for values in [
            &self.occurrence_refs,
            &self.prompt_lineage_refs,
            &self.tool_surface_refs,
            &self.evidence_refs,
            &self.verifier_capability_refs,
        ] {
            if values.iter().any(|value| value.trim().is_empty())
                || values.iter().collect::<BTreeSet<_>>().len() != values.len()
            {
                return invalid("verification envelope refs must be present and unique");
            }
        }
        Ok(())
    }
    pub fn validate(&self) -> Result<(), ForensicsError> {
        let mut copy = self.clone();
        copy.canonical_digest.clear();
        let digest = digest_json(&copy)?;
        copy.validate_unsealed()?;
        if self.finding_digest != digest_json(&self.finding)? || self.canonical_digest != digest {
            return invalid("verification envelope digest drifted");
        }
        Ok(())
    }
}

impl IndependentVerificationCase {
    pub fn request(envelope: IndependentVerifierEnvelope) -> Result<Self, ForensicsError> {
        envelope.validate()?;
        let payload_digest = envelope.canonical_digest.clone();
        let event = IndependentVerificationEvent {
            sequence: 1,
            state: IndependentVerificationState::Requested,
            event_ref: format!("event.{}.1", envelope.request_ref),
            payload_digest,
            observed_at: envelope.requested_at.clone(),
        };
        let mut case = Self {
            schema: INDEPENDENT_VERIFICATION_CASE_SCHEMA_V1.into(),
            poc_history: vec![envelope.original_poc.clone()],
            envelope,
            state: IndependentVerificationState::Requested,
            initial_verdict: None,
            evidence: Vec::new(),
            events: vec![event],
            remediation_enabled: false,
            completed: false,
            canonical_digest: String::new(),
        };
        case.refresh()?;
        Ok(case)
    }
    pub fn begin(&mut self, observed_at: String) -> Result<(), ForensicsError> {
        self.transition(
            IndependentVerificationState::Running,
            "verification.running",
            observed_at,
        )
    }
    pub fn settle(&mut self, settlement: VerificationSettlement) -> Result<(), ForensicsError> {
        if settlement.verifier_actor_ref != self.envelope.verifier_actor_ref {
            return invalid("only the admitted independent verifier can settle");
        }
        let verdict = ImmutableInitialVerdict {
            verdict_ref: format!("verdict.{}", settlement.settlement_ref),
            outcome: settlement.outcome,
            verifier_actor_ref: settlement.verifier_actor_ref.clone(),
            rationale_digest: settlement.rationale_digest.clone(),
            evidence_refs: settlement
                .evidence
                .iter()
                .map(|e| e.receipt_ref.clone())
                .collect(),
            stored_at: settlement.settled_at.clone(),
            canonical_digest: String::new(),
        };
        let mut verdict = verdict;
        verdict.canonical_digest = digest_json(&verdict)?;
        if let Some(existing) = &self.initial_verdict {
            if existing == &verdict {
                return Ok(());
            }
            return invalid("the immutable initial verdict is already stored");
        }
        validate_settlement(&self.envelope, &settlement)?;
        self.evidence = settlement.evidence;
        self.initial_verdict = Some(verdict.clone());
        self.state = match settlement.outcome {
            IndependentVerificationOutcome::Inconclusive => {
                IndependentVerificationState::Inconclusive
            }
            _ => IndependentVerificationState::VerdictStored,
        };
        self.remediation_enabled = true;
        self.completed = true;
        self.push_event(self.state, verdict.canonical_digest, settlement.settled_at)?;
        self.refresh()
    }
    pub fn add_superseding_poc(&mut self, poc: ForensicPocIdentity) -> Result<(), ForensicsError> {
        if !self.remediation_enabled || self.initial_verdict.is_none() {
            return invalid(
                "patch and superseding PoC work requires a durably stored initial verdict",
            );
        }
        if poc.supersedes_poc_ref.as_deref() != self.poc_history.last().map(|p| p.poc_ref.as_str())
        {
            return invalid("superseding PoC lineage must name the prior immutable PoC");
        }
        validate_digest(&poc.content_digest)?;
        self.poc_history.push(poc);
        self.refresh()
    }
    pub fn fail(
        &mut self,
        state: IndependentVerificationState,
        reason_ref: String,
        observed_at: String,
    ) -> Result<(), ForensicsError> {
        if matches!(
            state,
            IndependentVerificationState::Requested
                | IndependentVerificationState::Running
                | IndependentVerificationState::VerdictStored
        ) || self.initial_verdict.is_some()
        {
            return invalid("failure cannot overwrite a stored verdict");
        }
        self.state = state;
        self.completed = false;
        self.remediation_enabled = false;
        self.push_event(state, digest_bytes(reason_ref.as_bytes()), observed_at)?;
        self.refresh()
    }
    pub fn validate(&self) -> Result<(), ForensicsError> {
        if self.schema != INDEPENDENT_VERIFICATION_CASE_SCHEMA_V1 {
            return invalid("verification case schema is invalid");
        }
        self.envelope.validate()?;
        if self
            .events
            .iter()
            .enumerate()
            .any(|(i, e)| e.sequence != i as u64 + 1)
            || self.events.is_empty()
            || self.events.last().map(|event| event.state) != Some(self.state)
            || self.events.iter().any(|event| {
                event.event_ref.trim().is_empty()
                    || event.observed_at.trim().is_empty()
                    || validate_digest(&event.payload_digest).is_err()
            })
            || self.poc_history.first() != Some(&self.envelope.original_poc)
        {
            return invalid("verification event or PoC history drifted");
        }
        let mut previous_poc_ref = None;
        let mut poc_refs = BTreeSet::new();
        for poc in &self.poc_history {
            if poc.poc_ref.trim().is_empty()
                || !poc_refs.insert(poc.poc_ref.as_str())
                || validate_digest(&poc.content_digest).is_err()
                || poc.supersedes_poc_ref.as_deref() != previous_poc_ref
            {
                return invalid("verification PoC lineage drifted");
            }
            previous_poc_ref = Some(poc.poc_ref.as_str());
        }
        if self.completed != self.initial_verdict.is_some()
            || self.remediation_enabled != self.initial_verdict.is_some()
        {
            return invalid(
                "successful verification requires one stored verdict before remediation",
            );
        }
        match &self.initial_verdict {
            Some(verdict) => {
                let expected_state =
                    if verdict.outcome == IndependentVerificationOutcome::Inconclusive {
                        IndependentVerificationState::Inconclusive
                    } else {
                        IndependentVerificationState::VerdictStored
                    };
                let mut unsealed_verdict = verdict.clone();
                unsealed_verdict.canonical_digest.clear();
                if self.state != expected_state
                    || verdict.verifier_actor_ref != self.envelope.verifier_actor_ref
                    || verdict.evidence_refs
                        != self
                            .evidence
                            .iter()
                            .map(|evidence| evidence.receipt_ref.clone())
                            .collect::<Vec<_>>()
                    || verdict.canonical_digest != digest_json(&unsealed_verdict)?
                {
                    return invalid("stored verification verdict drifted");
                }
            }
            None if matches!(
                self.state,
                IndependentVerificationState::VerdictStored
                    | IndependentVerificationState::Inconclusive
            ) =>
            {
                return invalid("verdict state requires the immutable initial verdict");
            }
            None => {}
        }
        let mut copy = self.clone();
        copy.canonical_digest.clear();
        if self.canonical_digest != digest_json(&copy)? {
            return invalid("verification case digest drifted");
        }
        Ok(())
    }
    fn transition(
        &mut self,
        state: IndependentVerificationState,
        payload: &str,
        at: String,
    ) -> Result<(), ForensicsError> {
        if self.initial_verdict.is_some() {
            return invalid("stored verdict state is immutable");
        }
        self.state = state;
        self.push_event(state, digest_bytes(payload.as_bytes()), at)?;
        self.refresh()
    }
    fn push_event(
        &mut self,
        state: IndependentVerificationState,
        payload_digest: String,
        observed_at: String,
    ) -> Result<(), ForensicsError> {
        self.events.push(IndependentVerificationEvent {
            sequence: self.events.len() as u64 + 1,
            state,
            event_ref: format!(
                "event.{}.{}",
                self.envelope.request_ref,
                self.events.len() + 1
            ),
            payload_digest,
            observed_at,
        });
        Ok(())
    }
    fn refresh(&mut self) -> Result<(), ForensicsError> {
        self.canonical_digest.clear();
        self.canonical_digest = digest_json(self)?;
        self.validate()
    }
}

fn validate_settlement(
    envelope: &IndependentVerifierEnvelope,
    settlement: &VerificationSettlement,
) -> Result<(), ForensicsError> {
    validate_digest(&settlement.rationale_digest)?;
    if settlement.evidence.is_empty()
        || settlement
            .evidence
            .iter()
            .map(|e| &e.receipt_ref)
            .collect::<BTreeSet<_>>()
            .len()
            != settlement.evidence.len()
    {
        return invalid("verdict settlement requires unique evidence receipts");
    }
    for evidence in &settlement.evidence {
        for digest in [
            &evidence.revision_digest,
            &evidence.command_digest,
            &evidence.environment_digest,
            &evidence.output_digest,
        ] {
            validate_digest(digest)?;
        }
    }
    if settlement.outcome == IndependentVerificationOutcome::Confirmed {
        let source = settlement.evidence.iter().any(|e| {
            e.kind == VerificationEvidenceKind::SourceValidation && e.outcome == "succeeded"
        });
        let dependency = settlement.evidence.iter().any(|e| {
            e.kind == VerificationEvidenceKind::DependencyValidation && e.outcome == "succeeded"
        });
        let poc = settlement.evidence.iter().any(|e| {
            e.kind == VerificationEvidenceKind::PocApplied
                && e.evidence_tier == ForensicEvidenceTier::ArtifactObserved
                && e.outcome == "succeeded"
        });
        let vulnerable = settlement.evidence.iter().any(|e| {
            e.kind == VerificationEvidenceKind::VulnerableControl
                && e.evidence_tier == ForensicEvidenceTier::Executed
                && e.revision_digest == envelope.vulnerable_revision_digest
                && e.observed_test_outcome.as_deref() == Some("failure")
                && e.outcome == "succeeded"
        });
        let fixed = settlement.evidence.iter().any(|e| {
            e.kind == VerificationEvidenceKind::FixedControl
                && e.evidence_tier == ForensicEvidenceTier::Executed
                && e.revision_digest == envelope.fixed_revision_digest
                && e.observed_test_outcome.as_deref() == Some("success")
                && e.outcome == "succeeded"
        });
        if !(source && dependency && poc && vulnerable && fixed) {
            return invalid(
                "confirmation requires source/dependency validation, PoC application, vulnerable failure, and fixed success",
            );
        }
    }
    Ok(())
}
fn validate_digest(value: &str) -> Result<(), ForensicsError> {
    if value
        .strip_prefix("sha256:")
        .is_some_and(|v| v.len() == 64 && v.bytes().all(|b| b.is_ascii_hexdigit()))
    {
        Ok(())
    } else {
        invalid("verification digest must be sha256")
    }
}
fn digest_bytes(value: &[u8]) -> String {
    format!("sha256:{:x}", Sha256::digest(value))
}
fn digest_json<T: Serialize>(value: &T) -> Result<String, ForensicsError> {
    serde_json::to_vec(value)
        .map(|v| digest_bytes(&v))
        .map_err(|_| ForensicsError::InvalidReview("cannot digest verification state".into()))
}
fn invalid<T>(message: &str) -> Result<T, ForensicsError> {
    Err(ForensicsError::InvalidReview(message.into()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ForensicCausalLink, ForensicSourceCitation};
    fn d(c: char) -> String {
        format!("sha256:{}", c.to_string().repeat(64))
    }
    fn envelope() -> IndependentVerifierEnvelope {
        IndependentVerifierEnvelope {
            request_ref: "verify.1".into(),
            finding: ForensicFindingProjection {
                finding_ref: "finding.1".into(),
                claim_ref: "claim.1".into(),
                title: "Finding".into(),
                impact: "Impact".into(),
                severity: "high".into(),
                claim_state: "unverified".into(),
                evidence_tier: ForensicEvidenceTier::SourceObserved,
                duplicate_group_ref: None,
                source_refs: vec![ForensicSourceCitation {
                    source_ref: "source.1".into(),
                    path: "src/lib.rs".into(),
                    symbol: None,
                    start_line: 1,
                    end_line: 1,
                    commit: "a".repeat(40),
                }],
                causal_path: vec![ForensicCausalLink {
                    sequence: 1,
                    proposition: "cause".into(),
                    evidence_refs: vec!["source.1".into()],
                    supported: true,
                }],
                evidence_receipts: vec![],
                poc_ref: Some("poc.1".into()),
                submitted_at: "2026-08-03T00:00:00Z".into(),
            },
            finding_digest: String::new(),
            assumptions: vec!["assumption".into()],
            occurrence_refs: vec!["occurrence.1".into()],
            root_cause_ref: "root.1".into(),
            source_bundle_ref: "bundle.1".into(),
            source_bundle_digest: d('a'),
            coverage_manifest_ref: "coverage.1".into(),
            coverage_manifest_digest: d('b'),
            original_poc: ForensicPocIdentity {
                poc_ref: "poc.1".into(),
                content_digest: d('c'),
                supersedes_poc_ref: None,
            },
            discovery_actor_ref: "actor.discovery".into(),
            prompt_digest: d('d'),
            prompt_lineage_refs: vec!["prompt.1".into()],
            model_provenance: ForensicModelProvenance {
                provider_ref: "provider.1".into(),
                model_ref: "model.1".into(),
                route_ref: "route.1".into(),
                configuration_digest: d('e'),
            },
            tool_surface_refs: vec!["tool.1".into()],
            evidence_refs: vec!["source.1".into()],
            verifier_actor_ref: "actor.verifier".into(),
            verifier_capability_refs: vec!["capability.verify".into()],
            vulnerable_revision_digest: d('f'),
            fixed_revision_digest: d('0'),
            requested_at: "2026-08-03T00:01:00Z".into(),
            canonical_digest: String::new(),
        }
        .seal()
        .unwrap()
    }
    fn evidence(
        kind: VerificationEvidenceKind,
        tier: ForensicEvidenceTier,
        rev: String,
        test: Option<&str>,
    ) -> IndependentVerificationEvidence {
        IndependentVerificationEvidence {
            receipt_ref: format!("receipt.{kind:?}"),
            kind,
            evidence_tier: tier,
            revision_digest: rev,
            command_digest: d('1'),
            environment_digest: d('2'),
            output_digest: d('3'),
            outcome: "succeeded".into(),
            observed_test_outcome: test.map(str::to_string),
            observed_at: "2026-08-03T00:02:00Z".into(),
        }
    }
    #[test]
    fn verdict_precedes_patch_and_confirmation_requires_both_controls() {
        let env = envelope();
        let mut case = IndependentVerificationCase::request(env.clone()).unwrap();
        assert!(
            case.add_superseding_poc(ForensicPocIdentity {
                poc_ref: "poc.2".into(),
                content_digest: d('4'),
                supersedes_poc_ref: Some("poc.1".into())
            })
            .is_err()
        );
        case.begin("2026-08-03T00:01:30Z".into()).unwrap();
        let weak = VerificationSettlement {
            settlement_ref: "settle.weak".into(),
            outcome: IndependentVerificationOutcome::Confirmed,
            verifier_actor_ref: "actor.verifier".into(),
            rationale_digest: d('5'),
            evidence: vec![evidence(
                VerificationEvidenceKind::PocApplied,
                ForensicEvidenceTier::ArtifactObserved,
                env.vulnerable_revision_digest.clone(),
                None,
            )],
            settled_at: "2026-08-03T00:03:00Z".into(),
        };
        assert!(case.settle(weak).is_err());
        let receipts = vec![
            evidence(
                VerificationEvidenceKind::SourceValidation,
                ForensicEvidenceTier::SourceObserved,
                env.vulnerable_revision_digest.clone(),
                None,
            ),
            evidence(
                VerificationEvidenceKind::DependencyValidation,
                ForensicEvidenceTier::SourceObserved,
                env.vulnerable_revision_digest.clone(),
                None,
            ),
            evidence(
                VerificationEvidenceKind::PocApplied,
                ForensicEvidenceTier::ArtifactObserved,
                env.vulnerable_revision_digest.clone(),
                None,
            ),
            evidence(
                VerificationEvidenceKind::VulnerableControl,
                ForensicEvidenceTier::Executed,
                env.vulnerable_revision_digest.clone(),
                Some("failure"),
            ),
            evidence(
                VerificationEvidenceKind::FixedControl,
                ForensicEvidenceTier::Executed,
                env.fixed_revision_digest,
                Some("success"),
            ),
        ];
        case.settle(VerificationSettlement {
            settlement_ref: "settle.1".into(),
            outcome: IndependentVerificationOutcome::Confirmed,
            verifier_actor_ref: "actor.verifier".into(),
            rationale_digest: d('6'),
            evidence: receipts,
            settled_at: "2026-08-03T00:04:00Z".into(),
        })
        .unwrap();
        assert!(case.completed && case.remediation_enabled);
        let restored: IndependentVerificationCase =
            serde_json::from_slice(&serde_json::to_vec(&case).unwrap()).unwrap();
        restored.validate().unwrap();
    }
    #[test]
    fn self_verification_and_applicable_diff_cannot_confirm() {
        let mut env = envelope();
        env.verifier_actor_ref = env.discovery_actor_ref.clone();
        assert!(env.seal().is_err());
    }
    #[test]
    fn historical_132_findings_zero_verdicts_remain_visibly_unsettled() {
        let cases = (0..132)
            .map(|_| IndependentVerificationCase::request(envelope()).unwrap())
            .collect::<Vec<_>>();
        assert_eq!(cases.len(), 132);
        assert_eq!(
            cases.iter().filter(|c| c.initial_verdict.is_some()).count(),
            0
        );
        assert!(cases.iter().all(|c| !c.completed && !c.remediation_enabled));
    }
}
