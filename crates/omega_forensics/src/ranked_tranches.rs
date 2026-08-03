use super::{EntropyFileEligibility, EntropyManifest, ForensicsError};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{collections::BTreeSet, fs, path::Path};

pub const EVIDENCE_RANKED_SCHEDULE_SCHEMA_V1: &str = "openagents.omega.evidence-ranked-schedule.v1";
pub const EVIDENCE_RANKING_VERSION_V1: &str = "omega.forensics.boundary-ranking.v1";
pub const DETERMINISTIC_SCANNER_VERSION_V1: &str = "omega.forensics.cheap-scanners.v1";

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ForensicBoundaryClass {
    EntropyRandomness,
    ProviderSelection,
    KeyDerivation,
    SecretSink,
    AuthenticationAuthorization,
    Parser,
    UnsafeBoundary,
    ExternalInput,
    DependencyBuild,
    DomainInvariant,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DeterministicScannerReceipt {
    pub receipt_ref: String,
    pub scanner_ref: String,
    pub tool_version: String,
    pub config_digest: String,
    pub source_digest: String,
    pub matched_classes: Vec<ForensicBoundaryClass>,
    pub matched_features: Vec<String>,
    pub canonical_digest: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ForensicCoverageDisposition {
    Queued,
    Focal,
    Contextual,
    Completed,
    Excluded,
    Skipped,
    Oversized,
    Unreachable,
    NeverReached,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct EvidenceRankedUnit {
    pub path: String,
    pub manifest_sequence: u32,
    pub rank: u32,
    pub tranche: u32,
    pub evidence_score: u32,
    pub boundary_classes: Vec<ForensicBoundaryClass>,
    pub feature_refs: Vec<String>,
    pub rationale: String,
    pub scanner_receipt_refs: Vec<String>,
    pub focal_sessions: u32,
    pub contextual_reads: u32,
    pub disposition: ForensicCoverageDisposition,
    pub exclusion_reason_ref: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[derive(Default)]
pub struct EvidenceRankedSchedule {
    pub schema: String,
    pub ranking_version: String,
    pub scanner_version: String,
    pub manifest_digest: String,
    pub threat_model_ref: String,
    pub tranche_size: u32,
    pub units: Vec<EvidenceRankedUnit>,
    pub scanner_receipts: Vec<DeterministicScannerReceipt>,
    pub canonical_digest: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TrancheControlAction {
    Pause,
    Resume,
    ExtendBudget,
    Cancel,
    Restart,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TrancheControlEvent {
    pub sequence: u64,
    pub action: TrancheControlAction,
    pub budget_units: u32,
    pub reason_ref: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TrancheBudgetLedger {
    pub schedule_digest: String,
    pub budget_units: u32,
    pub consumed_units: u32,
    pub paused: bool,
    pub cancelled: bool,
    pub events: Vec<TrancheControlEvent>,
}

impl TrancheBudgetLedger {
    pub fn new(
        schedule: &EvidenceRankedSchedule,
        budget_units: u32,
    ) -> Result<Self, ForensicsError> {
        if budget_units == 0 {
            return Err(ForensicsError::InvalidEntropyRun(
                "tranche budget must be positive".into(),
            ));
        }
        Ok(Self {
            schedule_digest: schedule.canonical_digest.clone(),
            budget_units,
            consumed_units: 0,
            paused: false,
            cancelled: false,
            events: Vec::new(),
        })
    }
    pub fn can_start(&self) -> bool {
        !self.paused && !self.cancelled && self.consumed_units < self.budget_units
    }
    pub fn consume(&mut self) -> Result<(), ForensicsError> {
        if !self.can_start() {
            return Err(ForensicsError::InvalidEntropyRun(
                "budget, pause, or cancellation prevents another focal session".into(),
            ));
        }
        self.consumed_units += 1;
        Ok(())
    }
    pub fn control(
        &mut self,
        action: TrancheControlAction,
        budget_units: u32,
        reason_ref: String,
    ) -> Result<(), ForensicsError> {
        if reason_ref.trim().is_empty() {
            return Err(ForensicsError::InvalidEntropyRun(
                "tranche control requires an exact reason".into(),
            ));
        }
        match action {
            TrancheControlAction::Pause => self.paused = true,
            TrancheControlAction::Resume | TrancheControlAction::Restart => {
                if !self.cancelled {
                    self.paused = false;
                }
            }
            TrancheControlAction::ExtendBudget => {
                if budget_units <= self.budget_units {
                    return Err(ForensicsError::InvalidEntropyRun(
                        "budget extension must increase the admitted budget".into(),
                    ));
                }
                self.budget_units = budget_units;
            }
            TrancheControlAction::Cancel => self.cancelled = true,
        }
        self.events.push(TrancheControlEvent {
            sequence: self.events.len() as u64 + 1,
            action,
            budget_units: self.budget_units,
            reason_ref,
        });
        Ok(())
    }
}

impl EvidenceRankedSchedule {
    pub fn from_manifest(
        manifest: &EntropyManifest,
        threat_model_ref: String,
        tranche_size: u32,
    ) -> Result<Self, ForensicsError> {
        Self::build(None, manifest, threat_model_ref, tranche_size)
    }

    /// Runs bounded, deterministic byte inspection before any model session. Scanner output is
    /// ranking evidence only; it is never promoted into a finding.
    pub fn inspect(
        root: &Path,
        manifest: &EntropyManifest,
        threat_model_ref: String,
        tranche_size: u32,
    ) -> Result<Self, ForensicsError> {
        Self::build(Some(root), manifest, threat_model_ref, tranche_size)
    }

    fn build(
        root: Option<&Path>,
        manifest: &EntropyManifest,
        threat_model_ref: String,
        tranche_size: u32,
    ) -> Result<Self, ForensicsError> {
        manifest.validate()?;
        if threat_model_ref.trim().is_empty() || tranche_size == 0 {
            return Err(ForensicsError::InvalidEntropyRun(
                "ranking requires a threat model and positive tranche size".into(),
            ));
        }
        let config_digest = digest_bytes(
            format!("{EVIDENCE_RANKING_VERSION_V1}:{tranche_size}:{threat_model_ref}").as_bytes(),
        );
        let mut receipts = Vec::new();
        let mut units = Vec::new();
        for file in &manifest.files {
            let excluded = file.eligibility != EntropyFileEligibility::Eligible;
            let source = root.and_then(|root| fs::read(root.join(&file.path)).ok());
            let (classes, features) = classify(&file.path, source.as_deref());
            let score = classes.iter().map(class_weight).sum::<u32>()
                + u32::try_from(features.len()).unwrap_or(u32::MAX).min(20);
            let receipt_ref = format!(
                "scanner-receipt://{}",
                digest_bytes(
                    format!(
                        "{DETERMINISTIC_SCANNER_VERSION_V1}:{config_digest}:{}",
                        file.content_digest.as_deref().unwrap_or("unavailable")
                    )
                    .as_bytes()
                )
            );
            if let Some(source) = source {
                let mut receipt = DeterministicScannerReceipt {
                    receipt_ref: receipt_ref.clone(),
                    scanner_ref: "scanner://omega/forensics/bounded-boundary-map".into(),
                    tool_version: DETERMINISTIC_SCANNER_VERSION_V1.into(),
                    config_digest: config_digest.clone(),
                    source_digest: digest_bytes(&source),
                    matched_classes: classes.clone(),
                    matched_features: features.clone(),
                    canonical_digest: String::new(),
                };
                receipt.canonical_digest = digest_json(&receipt)?;
                receipts.push(receipt);
            }
            units.push(EvidenceRankedUnit {
                path: file.path.clone(),
                manifest_sequence: file.sequence,
                rank: 0,
                tranche: 0,
                evidence_score: score,
                boundary_classes: classes,
                feature_refs: features.clone(),
                rationale: if excluded {
                    format!(
                        "Not eligible: {}",
                        file.reason_ref.as_deref().unwrap_or("unspecified")
                    )
                } else if features.is_empty() {
                    "Eligible fallback coverage; no cheap boundary marker matched".into()
                } else {
                    format!(
                        "Cheap deterministic evidence matched: {}",
                        features.join(", ")
                    )
                },
                scanner_receipt_refs: if root.is_some() {
                    vec![receipt_ref]
                } else {
                    Vec::new()
                },
                focal_sessions: 0,
                contextual_reads: 0,
                disposition: match file.eligibility {
                    EntropyFileEligibility::Eligible => ForensicCoverageDisposition::Queued,
                    EntropyFileEligibility::Oversized => ForensicCoverageDisposition::Oversized,
                    _ => ForensicCoverageDisposition::Excluded,
                },
                exclusion_reason_ref: file.reason_ref.clone(),
            });
        }
        // Content digest is the final stable tie-breaker. Alphabetical path order cannot silently
        // become ranking policy; path remains source identity only.
        units.sort_by(|a, b| {
            b.evidence_score
                .cmp(&a.evidence_score)
                .then_with(|| {
                    manifest
                        .files
                        .iter()
                        .find(|f| f.path == a.path)
                        .and_then(|f| f.content_digest.as_ref())
                        .cmp(
                            &manifest
                                .files
                                .iter()
                                .find(|f| f.path == b.path)
                                .and_then(|f| f.content_digest.as_ref()),
                        )
                })
                .then_with(|| a.manifest_sequence.cmp(&b.manifest_sequence))
        });
        let mut eligible_rank = 0_u32;
        for unit in &mut units {
            if unit.disposition == ForensicCoverageDisposition::Queued {
                eligible_rank += 1;
                unit.rank = eligible_rank;
                unit.tranche = 1 + (eligible_rank - 1) / tranche_size;
            }
        }
        let mut schedule = Self {
            schema: EVIDENCE_RANKED_SCHEDULE_SCHEMA_V1.into(),
            ranking_version: EVIDENCE_RANKING_VERSION_V1.into(),
            scanner_version: DETERMINISTIC_SCANNER_VERSION_V1.into(),
            manifest_digest: manifest.canonical_digest.clone(),
            threat_model_ref,
            tranche_size,
            units,
            scanner_receipts: receipts,
            canonical_digest: String::new(),
        };
        schedule.canonical_digest = schedule.computed_digest()?;
        schedule.validate(manifest)?;
        Ok(schedule)
    }

    pub fn next_queued_path(&self) -> Option<&str> {
        self.units
            .iter()
            .filter(|u| u.disposition == ForensicCoverageDisposition::Queued)
            .min_by_key(|u| u.rank)
            .map(|u| u.path.as_str())
    }

    pub fn mark(
        &mut self,
        path: &str,
        disposition: ForensicCoverageDisposition,
    ) -> Result<(), ForensicsError> {
        let unit = self
            .units
            .iter_mut()
            .find(|u| u.path == path)
            .ok_or_else(|| {
                ForensicsError::InvalidEntropyRun(
                    "coverage path is absent from ranked schedule".into(),
                )
            })?;
        if disposition == ForensicCoverageDisposition::Focal {
            unit.focal_sessions = unit.focal_sessions.saturating_add(1);
        }
        unit.disposition = disposition;
        self.canonical_digest = self.computed_digest()?;
        Ok(())
    }

    pub fn mark_contextual_read(&mut self, path: &str) -> Result<(), ForensicsError> {
        let unit = self
            .units
            .iter_mut()
            .find(|unit| unit.path == path)
            .ok_or_else(|| {
                ForensicsError::InvalidEntropyRun(
                    "contextual path is absent from ranked schedule".into(),
                )
            })?;
        unit.contextual_reads = unit.contextual_reads.saturating_add(1);
        self.canonical_digest = self.computed_digest()?;
        Ok(())
    }

    pub fn validate(&self, manifest: &EntropyManifest) -> Result<(), ForensicsError> {
        if self.schema != EVIDENCE_RANKED_SCHEDULE_SCHEMA_V1
            || self.ranking_version != EVIDENCE_RANKING_VERSION_V1
            || self.scanner_version != DETERMINISTIC_SCANNER_VERSION_V1
            || self.manifest_digest != manifest.canonical_digest
            || self.tranche_size == 0
            || self.canonical_digest != self.computed_digest()?
        {
            return Err(ForensicsError::InvalidEntropyRun(
                "ranked schedule identity or digest is invalid".into(),
            ));
        }
        let paths = self
            .units
            .iter()
            .map(|u| u.path.as_str())
            .collect::<BTreeSet<_>>();
        if paths.len() != manifest.files.len()
            || manifest
                .files
                .iter()
                .any(|f| !paths.contains(f.path.as_str()))
        {
            return Err(ForensicsError::InvalidEntropyRun(
                "ranked schedule must account for every manifest file".into(),
            ));
        }
        let ranks = self
            .units
            .iter()
            .filter(|u| u.rank > 0)
            .map(|u| u.rank)
            .collect::<BTreeSet<_>>();
        if ranks
            .iter()
            .copied()
            .ne(1..=u32::try_from(ranks.len()).unwrap_or(u32::MAX))
        {
            return Err(ForensicsError::InvalidEntropyRun(
                "eligible ranks must be dense and unique".into(),
            ));
        }
        let receipt_refs = self
            .scanner_receipts
            .iter()
            .map(|receipt| receipt.receipt_ref.as_str())
            .collect::<BTreeSet<_>>();
        if receipt_refs.len() != self.scanner_receipts.len()
            || self
                .units
                .iter()
                .flat_map(|unit| &unit.scanner_receipt_refs)
                .any(|receipt_ref| !receipt_refs.contains(receipt_ref.as_str()))
        {
            return Err(ForensicsError::InvalidEntropyRun(
                "ranked units must reference unique retained scanner receipts".into(),
            ));
        }
        Ok(())
    }

    fn computed_digest(&self) -> Result<String, ForensicsError> {
        let mut copy = self.clone();
        copy.canonical_digest.clear();
        digest_json(&copy)
    }
}

fn classify(path: &str, source: Option<&[u8]>) -> (Vec<ForensicBoundaryClass>, Vec<String>) {
    let mut text = path.to_ascii_lowercase();
    if let Some(source) = source {
        text.push('\n');
        text.push_str(&String::from_utf8_lossy(source).to_ascii_lowercase());
    }
    let rules: &[(ForensicBoundaryClass, &str, &[&str])] = &[
        (
            ForensicBoundaryClass::EntropyRandomness,
            "boundary.entropy",
            &["entropy", "random", "rng", "urandom"],
        ),
        (
            ForensicBoundaryClass::ProviderSelection,
            "boundary.provider_selection",
            &["provider", "backend", "feature =", "cfg("],
        ),
        (
            ForensicBoundaryClass::KeyDerivation,
            "boundary.key_derivation",
            &["derive_key", "kdf", "hkdf", "pbkdf", "seed"],
        ),
        (
            ForensicBoundaryClass::SecretSink,
            "boundary.secret_sink",
            &["secret", "private_key", "credential", "token"],
        ),
        (
            ForensicBoundaryClass::AuthenticationAuthorization,
            "boundary.auth",
            &["authenticate", "authorize", "permission", "session"],
        ),
        (
            ForensicBoundaryClass::Parser,
            "boundary.parser",
            &["parse", "deserialize", "from_str", "json"],
        ),
        (
            ForensicBoundaryClass::UnsafeBoundary,
            "boundary.unsafe",
            &["unsafe", "extern \"c\"", "ffi"],
        ),
        (
            ForensicBoundaryClass::ExternalInput,
            "boundary.external_input",
            &["stdin", "socket", "request", "environment", "env::"],
        ),
        (
            ForensicBoundaryClass::DependencyBuild,
            "boundary.dependency_build",
            &["cargo.toml", "package.json", "build.rs", "dependency"],
        ),
        (
            ForensicBoundaryClass::DomainInvariant,
            "boundary.domain_invariant",
            &["invariant", "assert", "verify", "validate"],
        ),
    ];
    let mut classes = Vec::new();
    let mut features = Vec::new();
    for (class, feature, needles) in rules {
        if needles.iter().any(|needle| text.contains(needle)) {
            classes.push(*class);
            features.push((*feature).into());
        }
    }
    (classes, features)
}

const fn class_weight(class: &ForensicBoundaryClass) -> u32 {
    match class {
        ForensicBoundaryClass::EntropyRandomness
        | ForensicBoundaryClass::KeyDerivation
        | ForensicBoundaryClass::SecretSink => 100,
        ForensicBoundaryClass::ProviderSelection
        | ForensicBoundaryClass::AuthenticationAuthorization
        | ForensicBoundaryClass::UnsafeBoundary => 80,
        ForensicBoundaryClass::Parser | ForensicBoundaryClass::ExternalInput => 60,
        ForensicBoundaryClass::DependencyBuild | ForensicBoundaryClass::DomainInvariant => 40,
    }
}
fn digest_bytes(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
fn digest_json<T: Serialize>(value: &T) -> Result<String, ForensicsError> {
    serde_json::to_vec(value)
        .map(|v| digest_bytes(&v))
        .map_err(|e| {
            ForensicsError::InvalidEntropyRun(format!("cannot digest ranked schedule: {e}"))
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::EntropyRepositoryBinding;
    use tempfile::tempdir;

    #[test]
    fn cheap_evidence_ranks_before_alphabetical_and_preserves_eventual_coverage() {
        let root = tempdir().unwrap();
        fs::write(root.path().join("aaa.rs"), "pub fn ordinary() {}").unwrap();
        fs::write(
            root.path().join("zzz.rs"),
            "pub unsafe fn derive_key_from_entropy() {}",
        )
        .unwrap();
        let manifest = EntropyManifest::build(
            root.path(),
            "manifest://ranked".into(),
            EntropyRepositoryBinding {
                repository_ref: "repository://test".into(),
                display_name: "test".into(),
                revision: "a".repeat(40),
            },
            Vec::new(),
            10_000,
        )
        .unwrap();
        let schedule = EvidenceRankedSchedule::inspect(
            root.path(),
            &manifest,
            "threat-model://entropy".into(),
            1,
        )
        .unwrap();
        assert_eq!(schedule.next_queued_path(), Some("zzz.rs"));
        assert_eq!(schedule.units.iter().filter(|u| u.rank > 0).count(), 2);
        assert_eq!(
            schedule
                .units
                .iter()
                .find(|u| u.path == "aaa.rs")
                .unwrap()
                .tranche,
            2
        );
        assert!(!schedule.scanner_receipts.is_empty());
    }

    #[test]
    fn resume_uses_serialized_rank_without_duplicate_or_reorder() {
        let root = tempdir().unwrap();
        fs::write(root.path().join("a.rs"), "parse(request)").unwrap();
        fs::write(root.path().join("b.rs"), "random secret").unwrap();
        let manifest = EntropyManifest::build(
            root.path(),
            "manifest://resume".into(),
            EntropyRepositoryBinding {
                repository_ref: "repository://test".into(),
                display_name: "test".into(),
                revision: "b".repeat(40),
            },
            Vec::new(),
            10_000,
        )
        .unwrap();
        let mut schedule = EvidenceRankedSchedule::inspect(
            root.path(),
            &manifest,
            "threat-model://entropy".into(),
            2,
        )
        .unwrap();
        let first = schedule.next_queued_path().unwrap().to_string();
        schedule
            .mark(&first, ForensicCoverageDisposition::Completed)
            .unwrap();
        let restored: EvidenceRankedSchedule =
            serde_json::from_slice(&serde_json::to_vec(&schedule).unwrap()).unwrap();
        restored.validate(&manifest).unwrap();
        assert_ne!(restored.next_queued_path(), Some(first.as_str()));
    }

    #[test]
    fn budget_controls_append_without_rebuilding_the_schedule() {
        let root = tempdir().unwrap();
        fs::write(root.path().join("a.rs"), "random()").unwrap();
        let manifest = EntropyManifest::build(
            root.path(),
            "manifest://budget".into(),
            EntropyRepositoryBinding {
                repository_ref: "repository://test".into(),
                display_name: "test".into(),
                revision: "c".repeat(40),
            },
            Vec::new(),
            10_000,
        )
        .unwrap();
        let schedule = EvidenceRankedSchedule::inspect(
            root.path(),
            &manifest,
            "threat-model://entropy".into(),
            1,
        )
        .unwrap();
        let digest = schedule.canonical_digest.clone();
        let mut ledger = TrancheBudgetLedger::new(&schedule, 1).unwrap();
        ledger
            .control(TrancheControlAction::Pause, 1, "operator.pause".into())
            .unwrap();
        assert!(!ledger.can_start());
        ledger
            .control(TrancheControlAction::Resume, 1, "operator.resume".into())
            .unwrap();
        ledger.consume().unwrap();
        ledger
            .control(
                TrancheControlAction::ExtendBudget,
                2,
                "operator.extend".into(),
            )
            .unwrap();
        assert!(ledger.can_start());
        assert_eq!(ledger.schedule_digest, digest);
        assert_eq!(ledger.events.len(), 3);
    }
}
