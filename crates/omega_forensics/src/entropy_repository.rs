use super::{ForensicSourceCitation, ForensicsError};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeSet,
    fs,
    path::{Component, Path},
};
use walkdir::WalkDir;

pub const ENTROPY_MANIFEST_SCHEMA_V1: &str = "openagents.omega.entropy-manifest.v1";
pub const ENTROPY_RUN_SCHEMA_V1: &str = "openagents.omega.entropy-run.v1";
pub const ENTROPY_FILE_OUTPUT_SCHEMA_V1: &str = "openagents.omega.entropy-file-output.v1";
pub const ENTROPY_PROMPT_SNAPSHOT_SCHEMA_V1: &str = "openagents.omega.entropy-prompt-snapshot.v1";

pub const DEFAULT_ENTROPY_ANALYSIS_PROMPT: &str = r#"Inspect the supplied source file in the context of its pinned repository for entropy and secret-randomness risks only. Trace operating-system, hardware, secure-element, and library entropy sources; seeding and reseeding; provider-selection guards; deterministic fallbacks; dependency crossings; and secret consumers. Do not claim that a final artifact contains a source path unless artifact evidence is supplied. Return only the requested typed JSON. If required source, configuration, or a tool is unavailable, preserve that limitation instead of returning a clean result."#;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct EntropyRepositoryBinding {
    pub repository_ref: String,
    pub display_name: String,
    pub revision: String,
}

impl EntropyRepositoryBinding {
    pub fn validate(&self) -> Result<(), ForensicsError> {
        validate_public_ref("repository", &self.repository_ref)?;
        validate_revision(&self.revision)?;
        if self.display_name.trim().is_empty() || self.display_name.len() > 256 {
            return Err(ForensicsError::InvalidEntropyRun(
                "repository display name must contain 1 to 256 bytes".into(),
            ));
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EntropyDependencyAvailability {
    Available,
    Missing,
    WrongRevision,
    SourceUnavailable,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct EntropyDependencyBinding {
    pub path: String,
    pub expected_revision: Option<String>,
    pub observed_revision: Option<String>,
    pub availability: EntropyDependencyAvailability,
}

impl EntropyDependencyBinding {
    fn validate(&self) -> Result<(), ForensicsError> {
        validate_relative_path(&self.path)?;
        if let Some(revision) = &self.expected_revision {
            validate_revision(revision)?;
        }
        if let Some(revision) = &self.observed_revision {
            validate_revision(revision)?;
        }
        if self.availability == EntropyDependencyAvailability::Available
            && self.expected_revision != self.observed_revision
        {
            return Err(ForensicsError::InvalidEntropyRun(
                "available dependencies must match their expected revision".into(),
            ));
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EntropyFileEligibility {
    Eligible,
    UnsupportedLanguage,
    Oversized,
    Symlink,
    SourceUnavailable,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct EntropyManifestFile {
    pub sequence: u32,
    pub path: String,
    pub language: Option<String>,
    pub byte_length: u64,
    pub content_digest: Option<String>,
    pub eligibility: EntropyFileEligibility,
    pub reason_ref: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct EntropyManifest {
    pub schema: String,
    pub manifest_ref: String,
    pub repository: EntropyRepositoryBinding,
    pub maximum_file_bytes: u64,
    pub files: Vec<EntropyManifestFile>,
    pub dependencies: Vec<EntropyDependencyBinding>,
    pub canonical_digest: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct EntropyManifestDigestInput<'a> {
    schema: &'a str,
    manifest_ref: &'a str,
    repository: &'a EntropyRepositoryBinding,
    maximum_file_bytes: u64,
    files: &'a [EntropyManifestFile],
    dependencies: &'a [EntropyDependencyBinding],
}

impl EntropyManifest {
    pub fn build(
        root: &Path,
        manifest_ref: String,
        repository: EntropyRepositoryBinding,
        mut dependencies: Vec<EntropyDependencyBinding>,
        maximum_file_bytes: u64,
    ) -> Result<Self, ForensicsError> {
        repository.validate()?;
        validate_public_ref("manifest", &manifest_ref)?;
        if maximum_file_bytes == 0 {
            return Err(ForensicsError::InvalidEntropyRun(
                "maximum file bytes must be positive".into(),
            ));
        }
        let metadata = fs::metadata(root).map_err(|error| {
            ForensicsError::InvalidEntropyRun(format!("cannot inspect repository root: {error}"))
        })?;
        if !metadata.is_dir() {
            return Err(ForensicsError::InvalidEntropyRun(
                "repository root must be a folder".into(),
            ));
        }

        dependencies.sort_by(|left, right| left.path.cmp(&right.path));
        for dependency in &dependencies {
            dependency.validate()?;
        }
        if dependencies
            .windows(2)
            .any(|pair| pair[0].path == pair[1].path)
        {
            return Err(ForensicsError::InvalidEntropyRun(
                "dependency paths must be unique".into(),
            ));
        }

        let mut discovered = Vec::new();
        for entry in WalkDir::new(root)
            .follow_links(false)
            .into_iter()
            .filter_entry(|entry| entry.depth() == 0 || entry.file_name() != ".git")
        {
            match entry {
                Ok(entry) if entry.depth() == 0 || entry.file_type().is_dir() => continue,
                Ok(entry) => {
                    let relative = entry.path().strip_prefix(root).map_err(|error| {
                        ForensicsError::InvalidEntropyRun(format!(
                            "cannot bind a repository path: {error}"
                        ))
                    })?;
                    let path = normalized_relative_path(relative)?;
                    let file_type = entry.file_type();
                    let metadata = entry.metadata();
                    discovered.push((path, entry.path().to_path_buf(), file_type, metadata));
                }
                Err(error) => {
                    return Err(ForensicsError::InvalidEntropyRun(format!(
                        "cannot traverse repository: {error}"
                    )));
                }
            }
        }
        discovered.sort_by(|left, right| left.0.cmp(&right.0));

        let mut files = Vec::with_capacity(discovered.len());
        for (index, (path, absolute_path, file_type, metadata)) in
            discovered.into_iter().enumerate()
        {
            let (language, byte_length, content_digest, eligibility, reason_ref) =
                if file_type.is_symlink() {
                    (
                        source_language(&path),
                        0,
                        None,
                        EntropyFileEligibility::Symlink,
                        Some("limitation.entropy.symlink".into()),
                    )
                } else if let Ok(metadata) = metadata {
                    let language = source_language(&path);
                    if language.is_none() {
                        (
                            None,
                            metadata.len(),
                            None,
                            EntropyFileEligibility::UnsupportedLanguage,
                            Some("limitation.entropy.unsupported_language".into()),
                        )
                    } else if metadata.len() > maximum_file_bytes {
                        (
                            language,
                            metadata.len(),
                            None,
                            EntropyFileEligibility::Oversized,
                            Some("limitation.entropy.oversized".into()),
                        )
                    } else {
                        match fs::read(&absolute_path) {
                            Ok(content) => (
                                language,
                                metadata.len(),
                                Some(sha256_bytes(&content)),
                                EntropyFileEligibility::Eligible,
                                None,
                            ),
                            Err(_) => (
                                language,
                                metadata.len(),
                                None,
                                EntropyFileEligibility::SourceUnavailable,
                                Some("limitation.entropy.source_unavailable".into()),
                            ),
                        }
                    }
                } else {
                    (
                        source_language(&path),
                        0,
                        None,
                        EntropyFileEligibility::SourceUnavailable,
                        Some("limitation.entropy.source_unavailable".into()),
                    )
                };
            files.push(EntropyManifestFile {
                sequence: u32::try_from(index + 1).map_err(|_| {
                    ForensicsError::InvalidEntropyRun(
                        "repository contains too many manifest files".into(),
                    )
                })?,
                path,
                language,
                byte_length,
                content_digest,
                eligibility,
                reason_ref,
            });
        }

        let mut manifest = Self {
            schema: ENTROPY_MANIFEST_SCHEMA_V1.into(),
            manifest_ref,
            repository,
            maximum_file_bytes,
            files,
            dependencies,
            canonical_digest: String::new(),
        };
        manifest.canonical_digest = manifest.computed_digest()?;
        manifest.validate()?;
        Ok(manifest)
    }

    pub fn validate(&self) -> Result<(), ForensicsError> {
        if self.schema != ENTROPY_MANIFEST_SCHEMA_V1 {
            return Err(ForensicsError::InvalidSchema);
        }
        validate_public_ref("manifest", &self.manifest_ref)?;
        self.repository.validate()?;
        if self.maximum_file_bytes == 0 || self.canonical_digest != self.computed_digest()? {
            return Err(ForensicsError::InvalidEntropyRun(
                "entropy manifest digest or file policy is invalid".into(),
            ));
        }
        let mut paths = BTreeSet::new();
        for (index, file) in self.files.iter().enumerate() {
            if file.sequence as usize != index + 1 || !paths.insert(&file.path) {
                return Err(ForensicsError::InvalidEntropyRun(
                    "manifest files must use unique paths in dense deterministic order".into(),
                ));
            }
            validate_relative_path(&file.path)?;
            if file.eligibility == EntropyFileEligibility::Eligible
                && (file.language.is_none() || file.content_digest.is_none())
            {
                return Err(ForensicsError::InvalidEntropyRun(
                    "eligible files require language and content digests".into(),
                ));
            }
        }
        for dependency in &self.dependencies {
            dependency.validate()?;
        }
        Ok(())
    }

    pub fn computed_digest(&self) -> Result<String, ForensicsError> {
        sha256_json(&EntropyManifestDigestInput {
            schema: &self.schema,
            manifest_ref: &self.manifest_ref,
            repository: &self.repository,
            maximum_file_bytes: self.maximum_file_bytes,
            files: &self.files,
            dependencies: &self.dependencies,
        })
    }

    pub fn is_incomplete(&self) -> bool {
        self.dependencies
            .iter()
            .any(|dependency| dependency.availability != EntropyDependencyAvailability::Available)
            || self.files.iter().any(|file| {
                matches!(
                    file.eligibility,
                    EntropyFileEligibility::SourceUnavailable
                        | EntropyFileEligibility::Oversized
                        | EntropyFileEligibility::Symlink
                )
            })
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct EntropyPromptSnapshot {
    pub schema: String,
    pub prompt_ref: String,
    pub parent_prompt_ref: Option<String>,
    pub source_run_ref: Option<String>,
    pub text: String,
    pub canonical_digest: String,
    pub created_at: String,
}

impl EntropyPromptSnapshot {
    pub fn new(
        prompt_ref: String,
        parent_prompt_ref: Option<String>,
        source_run_ref: Option<String>,
        text: String,
        created_at: String,
    ) -> Result<Self, ForensicsError> {
        let canonical_digest = entropy_prompt_digest(&text)?;
        let snapshot = Self {
            schema: ENTROPY_PROMPT_SNAPSHOT_SCHEMA_V1.into(),
            prompt_ref,
            parent_prompt_ref,
            source_run_ref,
            text,
            canonical_digest,
            created_at,
        };
        snapshot.validate()?;
        Ok(snapshot)
    }

    pub fn validate(&self) -> Result<(), ForensicsError> {
        if self.schema != ENTROPY_PROMPT_SNAPSHOT_SCHEMA_V1 {
            return Err(ForensicsError::InvalidSchema);
        }
        validate_public_ref("entropy prompt", &self.prompt_ref)?;
        if let Some(parent_prompt_ref) = &self.parent_prompt_ref {
            validate_public_ref("parent entropy prompt", parent_prompt_ref)?;
            if parent_prompt_ref == &self.prompt_ref {
                return Err(ForensicsError::InvalidEntropyRun(
                    "an entropy prompt cannot parent itself".into(),
                ));
            }
        }
        if let Some(source_run_ref) = &self.source_run_ref {
            validate_public_ref("source entropy run", source_run_ref)?;
        }
        if self.canonical_digest != entropy_prompt_digest(&self.text)? {
            return Err(ForensicsError::InvalidEntropyRun(
                "entropy prompt snapshot digest does not match its immutable text".into(),
            ));
        }
        if self.created_at.trim().is_empty() || self.created_at.len() > 64 {
            return Err(ForensicsError::InvalidEntropyRun(
                "prompt creation time must be present and bounded".into(),
            ));
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct EntropyModelParameters {
    pub temperature_millis: u16,
    pub thinking_allowed: bool,
    pub reasoning_effort_ref: Option<String>,
}

impl EntropyModelParameters {
    fn validate(&self) -> Result<(), ForensicsError> {
        if self.temperature_millis > 2_000 {
            return Err(ForensicsError::InvalidEntropyRun(
                "model temperature must stay between 0 and 2.0".into(),
            ));
        }
        if let Some(reasoning_effort_ref) = &self.reasoning_effort_ref {
            validate_public_ref("reasoning effort", reasoning_effort_ref)?;
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct EntropyRunBinding {
    pub run_ref: String,
    pub repository: EntropyRepositoryBinding,
    pub manifest_ref: String,
    pub manifest_digest: String,
    pub prompt_snapshot: EntropyPromptSnapshot,
    pub prompt_digest: String,
    pub model_route_ref: String,
    pub model_parameters: EntropyModelParameters,
    pub tool_surface_refs: Vec<String>,
    pub started_at: String,
}

impl EntropyRunBinding {
    pub fn validate(&self) -> Result<(), ForensicsError> {
        validate_public_ref("entropy run", &self.run_ref)?;
        self.repository.validate()?;
        validate_public_ref("manifest", &self.manifest_ref)?;
        validate_sha256("manifest", &self.manifest_digest)?;
        validate_sha256("prompt", &self.prompt_digest)?;
        self.prompt_snapshot.validate()?;
        if self.prompt_digest != self.prompt_snapshot.canonical_digest {
            return Err(ForensicsError::InvalidEntropyRun(
                "run prompt digest does not match its immutable prompt snapshot".into(),
            ));
        }
        validate_public_ref("model route", &self.model_route_ref)?;
        self.model_parameters.validate()?;
        if self.started_at.trim().is_empty() || self.started_at.len() > 64 {
            return Err(ForensicsError::InvalidEntropyRun(
                "run start time must be present and bounded".into(),
            ));
        }
        if self.tool_surface_refs.is_empty() || self.tool_surface_refs.len() > 32 {
            return Err(ForensicsError::InvalidEntropyRun(
                "run tool surface must contain 1 to 32 refs".into(),
            ));
        }
        let mut tools = BTreeSet::new();
        for tool_ref in &self.tool_surface_refs {
            validate_public_ref("tool", tool_ref)?;
            if !tools.insert(tool_ref) {
                return Err(ForensicsError::InvalidEntropyRun(
                    "run tool surface refs must be unique".into(),
                ));
            }
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EntropyRunPhase {
    Ready,
    Running,
    CancelRequested,
    Completed,
    CompletedWithLimitations,
    Cancelled,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EntropyFileState {
    Queued,
    Reading,
    Analyzed,
    Candidate,
    Skipped,
    Failed,
    Cancelled,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EntropyLimitationClass {
    SourceUnavailable,
    UnsupportedLanguage,
    IncompleteDependency,
    Oversized,
    Symlink,
    ToolFailure,
    RequestSchemaFailure,
    ModelFailure,
    InvalidOutput,
    Cancelled,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct EntropyLimitation {
    pub class: EntropyLimitationClass,
    pub reason_ref: String,
    pub message: String,
    pub file_path: Option<String>,
}

impl EntropyLimitation {
    pub fn validate(&self) -> Result<(), ForensicsError> {
        validate_public_ref("limitation reason", &self.reason_ref)?;
        if self.message.trim().is_empty() || self.message.len() > 2_048 {
            return Err(ForensicsError::InvalidEntropyRun(
                "limitation messages must be present and bounded".into(),
            ));
        }
        if let Some(path) = &self.file_path {
            validate_relative_path(path)?;
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct EntropyCausalLink {
    pub sequence: u32,
    pub claim: String,
    pub source_refs: Vec<ForensicSourceCitation>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct EntropyObservation {
    pub observation_ref: String,
    pub title: String,
    pub analyzed_file: String,
    pub symbols: Vec<String>,
    pub suspected_mechanism: String,
    pub secret_consumers: Vec<String>,
    pub source_refs: Vec<ForensicSourceCitation>,
    pub confidence_boundary: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct EntropyHypothesis {
    pub hypothesis_ref: String,
    pub title: String,
    pub analyzed_file: String,
    pub symbols: Vec<String>,
    pub suspected_mechanism: String,
    pub secret_consumers: Vec<String>,
    pub causal_links: Vec<EntropyCausalLink>,
    pub missing_evidence: Vec<String>,
    pub next_check: String,
    pub confidence_boundary: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct EntropyFileAnalysisOutput {
    pub schema: String,
    pub run_ref: String,
    pub file_path: String,
    pub observations: Vec<EntropyObservation>,
    pub hypotheses: Vec<EntropyHypothesis>,
    pub limitations: Vec<EntropyLimitation>,
}

impl EntropyFileAnalysisOutput {
    pub fn validate(&self, binding: &EntropyRunBinding) -> Result<(), ForensicsError> {
        if self.schema != ENTROPY_FILE_OUTPUT_SCHEMA_V1 || self.run_ref != binding.run_ref {
            return Err(ForensicsError::InvalidEntropyRun(
                "file output is not bound to this entropy run".into(),
            ));
        }
        validate_relative_path(&self.file_path)?;
        for observation in &self.observations {
            validate_public_ref("observation", &observation.observation_ref)?;
            validate_candidate_fields(
                &self.file_path,
                &observation.analyzed_file,
                &observation.title,
                &observation.symbols,
                &observation.suspected_mechanism,
                &observation.secret_consumers,
                &observation.confidence_boundary,
            )?;
            if observation.source_refs.is_empty() {
                return Err(ForensicsError::InvalidEntropyRun(
                    "source observations require at least one exact source ref".into(),
                ));
            }
            for source in &observation.source_refs {
                source.validate(&binding.repository.revision)?;
            }
        }
        for hypothesis in &self.hypotheses {
            validate_public_ref("hypothesis", &hypothesis.hypothesis_ref)?;
            validate_candidate_fields(
                &self.file_path,
                &hypothesis.analyzed_file,
                &hypothesis.title,
                &hypothesis.symbols,
                &hypothesis.suspected_mechanism,
                &hypothesis.secret_consumers,
                &hypothesis.confidence_boundary,
            )?;
            if hypothesis.causal_links.is_empty()
                || hypothesis.missing_evidence.is_empty()
                || hypothesis.next_check.trim().is_empty()
            {
                return Err(ForensicsError::InvalidEntropyRun(
                    "hypotheses require causal links, missing evidence, and a next check".into(),
                ));
            }
            for (index, link) in hypothesis.causal_links.iter().enumerate() {
                if link.sequence as usize != index + 1
                    || link.claim.trim().is_empty()
                    || link.source_refs.is_empty()
                {
                    return Err(ForensicsError::InvalidEntropyRun(
                        "causal links must be dense and source referenced".into(),
                    ));
                }
                for source in &link.source_refs {
                    source.validate(&binding.repository.revision)?;
                }
            }
        }
        for limitation in &self.limitations {
            limitation.validate()?;
            if limitation.file_path.as_deref() != Some(self.file_path.as_str()) {
                return Err(ForensicsError::InvalidEntropyRun(
                    "file output limitations must name the analyzed file".into(),
                ));
            }
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct EntropyFileProgress {
    pub sequence: u32,
    pub path: String,
    pub state: EntropyFileState,
    pub observations: Vec<EntropyObservation>,
    pub hypotheses: Vec<EntropyHypothesis>,
    pub limitations: Vec<EntropyLimitation>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct EntropyRunEvent {
    pub sequence: u64,
    pub file_path: Option<String>,
    pub state: EntropyFileState,
    pub observed_at: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct EntropyFileTask {
    pub run_ref: String,
    pub file_path: String,
    pub file_digest: String,
    pub prompt_digest: String,
    pub model_route_ref: String,
    pub tool_surface_refs: Vec<String>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct EntropyRunCounts {
    pub queued: u32,
    pub reading: u32,
    pub analyzed: u32,
    pub candidate: u32,
    pub skipped: u32,
    pub failed: u32,
    pub cancelled: u32,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct EntropyRunProjection {
    pub schema: String,
    pub binding: EntropyRunBinding,
    pub manifest: EntropyManifest,
    pub phase: EntropyRunPhase,
    pub files: Vec<EntropyFileProgress>,
    pub limitations: Vec<EntropyLimitation>,
    pub events: Vec<EntropyRunEvent>,
}

impl EntropyRunProjection {
    pub fn new(
        binding: EntropyRunBinding,
        manifest: EntropyManifest,
    ) -> Result<Self, ForensicsError> {
        binding.validate()?;
        manifest.validate()?;
        if binding.repository != manifest.repository
            || binding.manifest_ref != manifest.manifest_ref
            || binding.manifest_digest != manifest.canonical_digest
        {
            return Err(ForensicsError::InvalidEntropyRun(
                "run binding does not match the immutable manifest".into(),
            ));
        }

        let mut limitations = Vec::new();
        for dependency in &manifest.dependencies {
            if dependency.availability != EntropyDependencyAvailability::Available {
                limitations.push(EntropyLimitation {
                    class: EntropyLimitationClass::IncompleteDependency,
                    reason_ref: "limitation.entropy.incomplete_dependency".into(),
                    message: format!(
                        "Dependency {} is {:?}",
                        dependency.path, dependency.availability
                    ),
                    file_path: Some(dependency.path.clone()),
                });
            }
        }
        let files = manifest
            .files
            .iter()
            .map(|file| {
                let limitation = manifest_limitation(file);
                EntropyFileProgress {
                    sequence: file.sequence,
                    path: file.path.clone(),
                    state: if limitation.is_some() {
                        EntropyFileState::Skipped
                    } else {
                        EntropyFileState::Queued
                    },
                    observations: Vec::new(),
                    hypotheses: Vec::new(),
                    limitations: limitation.into_iter().collect(),
                }
            })
            .collect::<Vec<_>>();
        for file in &files {
            limitations.extend(file.limitations.clone());
        }
        Ok(Self {
            schema: ENTROPY_RUN_SCHEMA_V1.into(),
            binding,
            manifest,
            phase: EntropyRunPhase::Ready,
            files,
            limitations,
            events: Vec::new(),
        })
    }

    pub fn validate(&self) -> Result<(), ForensicsError> {
        if self.schema != ENTROPY_RUN_SCHEMA_V1 {
            return Err(ForensicsError::InvalidSchema);
        }
        self.binding.validate()?;
        self.manifest.validate()?;
        if self.binding.repository != self.manifest.repository
            || self.binding.manifest_ref != self.manifest.manifest_ref
            || self.binding.manifest_digest != self.manifest.canonical_digest
            || self.files.len() != self.manifest.files.len()
        {
            return Err(ForensicsError::InvalidEntropyRun(
                "restored entropy run does not match its immutable manifest".into(),
            ));
        }
        for (index, (file, manifest_file)) in
            self.files.iter().zip(&self.manifest.files).enumerate()
        {
            if file.sequence as usize != index + 1
                || file.sequence != manifest_file.sequence
                || file.path != manifest_file.path
            {
                return Err(ForensicsError::InvalidEntropyRun(
                    "restored file progress drifted from manifest order".into(),
                ));
            }
            for limitation in &file.limitations {
                limitation.validate()?;
            }
            EntropyFileAnalysisOutput {
                schema: ENTROPY_FILE_OUTPUT_SCHEMA_V1.into(),
                run_ref: self.binding.run_ref.clone(),
                file_path: file.path.clone(),
                observations: file.observations.clone(),
                hypotheses: file.hypotheses.clone(),
                limitations: file.limitations.clone(),
            }
            .validate(&self.binding)?;
            if file.state == EntropyFileState::Candidate
                && file.observations.is_empty()
                && file.hypotheses.is_empty()
            {
                return Err(ForensicsError::InvalidEntropyRun(
                    "candidate file state requires a typed candidate".into(),
                ));
            }
        }
        for limitation in &self.limitations {
            limitation.validate()?;
        }
        for (index, event) in self.events.iter().enumerate() {
            if event.sequence as usize != index + 1
                || event.observed_at.trim().is_empty()
                || event.observed_at.len() > 64
            {
                return Err(ForensicsError::InvalidEntropyRun(
                    "restored entropy events must be dense and timestamped".into(),
                ));
            }
            if let Some(path) = &event.file_path {
                validate_relative_path(path)?;
            }
        }
        Ok(())
    }

    pub fn start_next_file(
        &mut self,
        observed_at: String,
    ) -> Result<Option<EntropyFileTask>, ForensicsError> {
        if matches!(
            self.phase,
            EntropyRunPhase::CancelRequested
                | EntropyRunPhase::Cancelled
                | EntropyRunPhase::Completed
                | EntropyRunPhase::CompletedWithLimitations
        ) {
            return Ok(None);
        }
        if self
            .files
            .iter()
            .any(|file| file.state == EntropyFileState::Reading)
        {
            return Err(ForensicsError::InvalidEntropyRun(
                "the sequential entropy runner already has a reading file".into(),
            ));
        }
        let Some(index) = self
            .files
            .iter()
            .position(|file| file.state == EntropyFileState::Queued)
        else {
            self.finish();
            return Ok(None);
        };
        let manifest_file =
            self.manifest.files.get(index).ok_or_else(|| {
                ForensicsError::InvalidEntropyRun("manifest order drifted".into())
            })?;
        let file_digest = manifest_file.content_digest.clone().ok_or_else(|| {
            ForensicsError::InvalidEntropyRun("queued files require a content digest".into())
        })?;
        self.phase = EntropyRunPhase::Running;
        self.files[index].state = EntropyFileState::Reading;
        let file_path = self.files[index].path.clone();
        self.push_event(
            Some(file_path.clone()),
            EntropyFileState::Reading,
            observed_at,
        )?;
        Ok(Some(EntropyFileTask {
            run_ref: self.binding.run_ref.clone(),
            file_path,
            file_digest,
            prompt_digest: self.binding.prompt_digest.clone(),
            model_route_ref: self.binding.model_route_ref.clone(),
            tool_surface_refs: self.binding.tool_surface_refs.clone(),
        }))
    }

    pub fn apply_output(
        &mut self,
        output: EntropyFileAnalysisOutput,
        observed_at: String,
    ) -> Result<(), ForensicsError> {
        output.validate(&self.binding)?;
        let file = self
            .files
            .iter_mut()
            .find(|file| file.path == output.file_path)
            .ok_or_else(|| {
                ForensicsError::InvalidEntropyRun("file output is absent from manifest".into())
            })?;
        if file.state != EntropyFileState::Reading {
            return Err(ForensicsError::InvalidEntropyRun(
                "file output requires the reading state".into(),
            ));
        }
        file.state = if output.observations.is_empty() && output.hypotheses.is_empty() {
            if output.limitations.is_empty() {
                EntropyFileState::Analyzed
            } else {
                EntropyFileState::Failed
            }
        } else {
            EntropyFileState::Candidate
        };
        file.observations = output.observations;
        file.hypotheses = output.hypotheses;
        file.limitations = output.limitations;
        self.limitations.extend(file.limitations.clone());
        let state = file.state;
        let path = file.path.clone();
        self.push_event(Some(path), state, observed_at)?;
        self.finish_if_exhausted();
        Ok(())
    }

    pub fn fail_reading_file(
        &mut self,
        limitation: EntropyLimitation,
        observed_at: String,
    ) -> Result<(), ForensicsError> {
        limitation.validate()?;
        let path = limitation.file_path.clone().ok_or_else(|| {
            ForensicsError::InvalidEntropyRun("file failures require a file path".into())
        })?;
        let file = self
            .files
            .iter_mut()
            .find(|file| file.path == path)
            .ok_or_else(|| {
                ForensicsError::InvalidEntropyRun("failed file is absent from manifest".into())
            })?;
        if file.state != EntropyFileState::Reading {
            return Err(ForensicsError::InvalidEntropyRun(
                "only a reading file can fail".into(),
            ));
        }
        file.state = EntropyFileState::Failed;
        file.limitations.push(limitation.clone());
        self.limitations.push(limitation);
        self.push_event(Some(path), EntropyFileState::Failed, observed_at)?;
        self.finish_if_exhausted();
        Ok(())
    }

    pub fn cancel(&mut self, observed_at: String) -> Result<(), ForensicsError> {
        if matches!(
            self.phase,
            EntropyRunPhase::Completed
                | EntropyRunPhase::CompletedWithLimitations
                | EntropyRunPhase::Cancelled
        ) {
            return Ok(());
        }
        self.phase = EntropyRunPhase::CancelRequested;
        let mut cancelled_paths = Vec::new();
        for file in &mut self.files {
            if matches!(
                file.state,
                EntropyFileState::Queued | EntropyFileState::Reading
            ) {
                file.state = EntropyFileState::Cancelled;
                let limitation = EntropyLimitation {
                    class: EntropyLimitationClass::Cancelled,
                    reason_ref: "limitation.entropy.cancelled".into(),
                    message: "Analysis stopped at the operator cancellation boundary".into(),
                    file_path: Some(file.path.clone()),
                };
                file.limitations.push(limitation.clone());
                self.limitations.push(limitation);
                cancelled_paths.push(file.path.clone());
            }
        }
        for path in cancelled_paths {
            self.push_event(Some(path), EntropyFileState::Cancelled, observed_at.clone())?;
        }
        self.phase = EntropyRunPhase::Cancelled;
        Ok(())
    }

    pub fn counts(&self) -> EntropyRunCounts {
        let mut counts = EntropyRunCounts::default();
        for file in &self.files {
            match file.state {
                EntropyFileState::Queued => counts.queued += 1,
                EntropyFileState::Reading => counts.reading += 1,
                EntropyFileState::Analyzed => counts.analyzed += 1,
                EntropyFileState::Candidate => counts.candidate += 1,
                EntropyFileState::Skipped => counts.skipped += 1,
                EntropyFileState::Failed => counts.failed += 1,
                EntropyFileState::Cancelled => counts.cancelled += 1,
            }
        }
        counts
    }

    fn finish_if_exhausted(&mut self) {
        if !self.files.iter().any(|file| {
            matches!(
                file.state,
                EntropyFileState::Queued | EntropyFileState::Reading
            )
        }) {
            self.finish();
        }
    }

    fn finish(&mut self) {
        self.phase = if self.limitations.is_empty() {
            EntropyRunPhase::Completed
        } else {
            EntropyRunPhase::CompletedWithLimitations
        };
    }

    fn push_event(
        &mut self,
        file_path: Option<String>,
        state: EntropyFileState,
        observed_at: String,
    ) -> Result<(), ForensicsError> {
        if observed_at.trim().is_empty() || observed_at.len() > 64 {
            return Err(ForensicsError::InvalidEntropyRun(
                "event time must be present and bounded".into(),
            ));
        }
        self.events.push(EntropyRunEvent {
            sequence: u64::try_from(self.events.len() + 1).map_err(|_| {
                ForensicsError::InvalidEntropyRun("too many entropy run events".into())
            })?,
            file_path,
            state,
            observed_at,
        });
        Ok(())
    }
}

pub fn entropy_prompt_digest(prompt: &str) -> Result<String, ForensicsError> {
    if prompt.trim().is_empty() || prompt.len() > 64 * 1024 {
        return Err(ForensicsError::InvalidEntropyRun(
            "entropy prompt must contain 1 to 65536 bytes".into(),
        ));
    }
    Ok(sha256_bytes(prompt.as_bytes()))
}

pub fn parse_entropy_file_output(text: &str) -> Result<EntropyFileAnalysisOutput, ForensicsError> {
    let trimmed = text.trim();
    let json = trimmed
        .strip_prefix("```json")
        .and_then(|value| value.strip_suffix("```"))
        .unwrap_or(trimmed)
        .trim();
    serde_json::from_str(json).map_err(|error| {
        ForensicsError::InvalidEntropyRun(format!(
            "model output is not typed entropy JSON: {error}"
        ))
    })
}

fn validate_candidate_fields(
    expected_file: &str,
    analyzed_file: &str,
    title: &str,
    symbols: &[String],
    suspected_mechanism: &str,
    secret_consumers: &[String],
    confidence_boundary: &str,
) -> Result<(), ForensicsError> {
    if analyzed_file != expected_file
        || title.trim().is_empty()
        || symbols.is_empty()
        || suspected_mechanism.trim().is_empty()
        || secret_consumers.is_empty()
        || confidence_boundary.trim().is_empty()
    {
        return Err(ForensicsError::InvalidEntropyRun(
            "candidates require the analyzed file, symbols, mechanism, consumers, and confidence boundary"
                .into(),
        ));
    }
    Ok(())
}

fn manifest_limitation(file: &EntropyManifestFile) -> Option<EntropyLimitation> {
    let class = match file.eligibility {
        EntropyFileEligibility::Eligible => return None,
        EntropyFileEligibility::UnsupportedLanguage => EntropyLimitationClass::UnsupportedLanguage,
        EntropyFileEligibility::Oversized => EntropyLimitationClass::Oversized,
        EntropyFileEligibility::Symlink => EntropyLimitationClass::Symlink,
        EntropyFileEligibility::SourceUnavailable => EntropyLimitationClass::SourceUnavailable,
    };
    Some(EntropyLimitation {
        class,
        reason_ref: file
            .reason_ref
            .clone()
            .unwrap_or_else(|| "limitation.entropy.source_unavailable".into()),
        message: format!("{} is not eligible for this entropy run", file.path),
        file_path: Some(file.path.clone()),
    })
}

fn normalized_relative_path(path: &Path) -> Result<String, ForensicsError> {
    if path.as_os_str().is_empty()
        || path
            .components()
            .any(|component| !matches!(component, Component::Normal(_)))
    {
        return Err(ForensicsError::InvalidEntropyRun(
            "manifest paths must be normalized and relative".into(),
        ));
    }
    let value = path.to_str().ok_or_else(|| {
        ForensicsError::InvalidEntropyRun("manifest paths must be valid UTF-8".into())
    })?;
    validate_relative_path(value)?;
    Ok(value.replace(std::path::MAIN_SEPARATOR, "/"))
}

fn validate_relative_path(value: &str) -> Result<(), ForensicsError> {
    if value.is_empty()
        || value.len() > 4_096
        || value.starts_with('/')
        || value.contains('\\')
        || value
            .split('/')
            .any(|component| component.is_empty() || component == "." || component == "..")
    {
        return Err(ForensicsError::InvalidEntropyRun(
            "source paths must be normalized relative paths".into(),
        ));
    }
    Ok(())
}

fn validate_public_ref(label: &str, value: &str) -> Result<(), ForensicsError> {
    if value.len() < 3
        || value.len() > 512
        || value.chars().any(char::is_whitespace)
        || value
            .chars()
            .any(|character| matches!(character, '?' | '&' | '='))
    {
        return Err(ForensicsError::InvalidEntropyRun(format!(
            "{label} ref is not bounded and public-safe"
        )));
    }
    Ok(())
}

fn validate_revision(value: &str) -> Result<(), ForensicsError> {
    if value.len() != 40 || !value.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(ForensicsError::InvalidEntropyRun(
            "repository revisions must contain 40 hexadecimal characters".into(),
        ));
    }
    Ok(())
}

fn validate_sha256(label: &str, value: &str) -> Result<(), ForensicsError> {
    let Some(hex) = value.strip_prefix("sha256:") else {
        return Err(ForensicsError::InvalidEntropyRun(format!(
            "{label} digest must use sha256"
        )));
    };
    if hex.len() != 64 || !hex.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(ForensicsError::InvalidEntropyRun(format!(
            "{label} digest must contain 64 hexadecimal characters"
        )));
    }
    Ok(())
}

fn source_language(path: &str) -> Option<String> {
    let file_name = path.rsplit('/').next().unwrap_or(path);
    let language = match file_name {
        "CMakeLists.txt" => "CMake",
        "Makefile" | "GNUmakefile" => "Make",
        "Kconfig" => "Kconfig",
        _ => match file_name.rsplit_once('.').map(|(_, extension)| extension) {
            Some("c" | "h") => "C",
            Some("cc" | "cpp" | "cxx" | "hh" | "hpp" | "hxx") => "C++",
            Some("rs") => "Rust",
            Some("py" | "pyi") => "Python",
            Some("js" | "jsx" | "mjs" | "cjs") => "JavaScript",
            Some("ts" | "tsx" | "mts" | "cts") => "TypeScript",
            Some("go") => "Go",
            Some("java") => "Java",
            Some("kt" | "kts") => "Kotlin",
            Some("swift") => "Swift",
            Some("m" | "mm") => "Objective-C",
            Some("cs") => "C#",
            Some("rb") => "Ruby",
            Some("php") => "PHP",
            Some("lua") => "Lua",
            Some("zig") => "Zig",
            Some("sol") => "Solidity",
            Some("move") => "Move",
            Some("s" | "S" | "asm") => "Assembly",
            Some("sh" | "bash" | "zsh") => "Shell",
            Some("cmake") => "CMake",
            Some("mk") => "Make",
            Some("ld") => "Linker script",
            Some("json") => "JSON",
            Some("toml") => "TOML",
            Some("yaml" | "yml") => "YAML",
            Some("xml") => "XML",
            Some("gradle") => "Gradle",
            Some("properties") => "Properties",
            Some("ino") => "Arduino",
            _ => return None,
        },
    };
    Some(language.into())
}

fn sha256_bytes(value: &[u8]) -> String {
    format!("sha256:{:x}", Sha256::digest(value))
}

fn sha256_json<Value: Serialize>(value: &Value) -> Result<String, ForensicsError> {
    let encoded = serde_json::to_vec(value).map_err(|error| {
        ForensicsError::InvalidEntropyRun(format!("cannot encode entropy contract: {error}"))
    })?;
    Ok(sha256_bytes(&encoded))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    const REVISION: &str = "bcc2c382a324690a2fcf972c0bac3b79bf923f7b";

    fn repository() -> EntropyRepositoryBinding {
        EntropyRepositoryBinding {
            repository_ref: "repository.coldcard.firmware".into(),
            display_name: "Coldcard firmware".into(),
            revision: REVISION.into(),
        }
    }

    fn build_manifest(root: &Path, dependencies: Vec<EntropyDependencyBinding>) -> EntropyManifest {
        EntropyManifest::build(
            root,
            "manifest.entropy.coldcard.fixture".into(),
            repository(),
            dependencies,
            1_024,
        )
        .expect("fixture manifest should be valid")
    }

    fn run_binding(manifest: &EntropyManifest) -> EntropyRunBinding {
        let prompt_snapshot = EntropyPromptSnapshot::new(
            "prompt.entropy.coldcard.fixture".into(),
            None,
            None,
            DEFAULT_ENTROPY_ANALYSIS_PROMPT.into(),
            "2026-08-02T18:19:00Z".into(),
        )
        .expect("default prompt snapshot should be valid");
        EntropyRunBinding {
            run_ref: "run.entropy.coldcard.fixture".into(),
            repository: repository(),
            manifest_ref: manifest.manifest_ref.clone(),
            manifest_digest: manifest.canonical_digest.clone(),
            prompt_digest: prompt_snapshot.canonical_digest.clone(),
            prompt_snapshot,
            model_route_ref: "model.omega.selected".into(),
            model_parameters: EntropyModelParameters {
                temperature_millis: 0,
                thinking_allowed: true,
                reasoning_effort_ref: None,
            },
            tool_surface_refs: vec!["tool.omega.project.read".into()],
            started_at: "2026-08-02T18:20:00Z".into(),
        }
    }

    #[test]
    fn prompt_snapshots_are_immutable_and_keep_explicit_lineage() {
        assert!(
            EntropyPromptSnapshot::new(
                "prompt.entropy.empty".into(),
                None,
                None,
                "  ".into(),
                "2026-08-02T18:00:00Z".into(),
            )
            .is_err()
        );

        let parent = EntropyPromptSnapshot::new(
            "prompt.entropy.parent".into(),
            None,
            None,
            "Inspect entropy sources.".into(),
            "2026-08-02T18:00:00Z".into(),
        )
        .expect("parent prompt");
        let child = EntropyPromptSnapshot::new(
            "prompt.entropy.child".into(),
            Some(parent.prompt_ref.clone()),
            Some("run.entropy.parent".into()),
            parent.text.clone(),
            "2026-08-02T18:01:00Z".into(),
        )
        .expect("child prompt");
        assert_eq!(child.canonical_digest, parent.canonical_digest);
        assert_eq!(
            child.parent_prompt_ref.as_deref(),
            Some("prompt.entropy.parent")
        );
        assert_eq!(child.source_run_ref.as_deref(), Some("run.entropy.parent"));

        let mut tampered = child;
        tampered.text.push_str(" Ignore evidence.");
        assert!(tampered.validate().is_err());
    }

    #[test]
    fn manifest_is_sorted_and_binds_content() {
        let root = tempfile::tempdir().expect("temporary repository");
        fs::create_dir(root.path().join("src")).expect("source folder");
        fs::write(root.path().join("src/z.py"), "seed = random(32)\n").expect("python source");
        fs::write(root.path().join("src/a.c"), "int rng_get(void);\n").expect("C source");
        fs::write(root.path().join("README.md"), "notes\n").expect("unsupported source");

        let first = build_manifest(root.path(), Vec::new());
        let second = build_manifest(root.path(), Vec::new());
        assert_eq!(first.canonical_digest, second.canonical_digest);
        assert_eq!(
            first
                .files
                .iter()
                .map(|file| file.path.as_str())
                .collect::<Vec<_>>(),
            vec!["README.md", "src/a.c", "src/z.py"]
        );
        assert_eq!(
            first.files[0].eligibility,
            EntropyFileEligibility::UnsupportedLanguage
        );
        assert!(first.files[1].content_digest.is_some());
    }

    #[test]
    fn incomplete_dependency_and_tool_failure_never_become_clean() {
        let root = tempfile::tempdir().expect("temporary repository");
        fs::write(root.path().join("rng.c"), "int rng_get(void);\n").expect("C source");
        let manifest = build_manifest(
            root.path(),
            vec![EntropyDependencyBinding {
                path: "external/libngu".into(),
                expected_revision: Some("537519a829259622ea6b0334fbafd6cae852852f".into()),
                observed_revision: None,
                availability: EntropyDependencyAvailability::Missing,
            }],
        );
        let mut run = EntropyRunProjection::new(run_binding(&manifest), manifest)
            .expect("run should preserve incomplete input");
        let task = run
            .start_next_file("2026-08-02T18:20:01Z".into())
            .expect("start should succeed")
            .expect("eligible file should be queued");
        run.fail_reading_file(
            EntropyLimitation {
                class: EntropyLimitationClass::ToolFailure,
                reason_ref: "limitation.entropy.tool_failure".into(),
                message: "source reader returned an input/output error".into(),
                file_path: Some(task.file_path),
            },
            "2026-08-02T18:20:02Z".into(),
        )
        .expect("failure should remain explicit");

        assert_eq!(run.phase, EntropyRunPhase::CompletedWithLimitations);
        assert_eq!(run.counts().failed, 1);
        assert!(run.limitations.iter().any(|limitation| {
            limitation.class == EntropyLimitationClass::IncompleteDependency
        }));
        assert!(
            run.limitations
                .iter()
                .any(|limitation| { limitation.class == EntropyLimitationClass::ToolFailure })
        );
    }

    #[test]
    fn cancellation_is_ordered_and_terminal() {
        let root = tempfile::tempdir().expect("temporary repository");
        fs::write(root.path().join("a.c"), "int a;\n").expect("first source");
        fs::write(root.path().join("b.c"), "int b;\n").expect("second source");
        let manifest = build_manifest(root.path(), Vec::new());
        let mut run = EntropyRunProjection::new(run_binding(&manifest), manifest)
            .expect("run should be valid");
        run.start_next_file("2026-08-02T18:20:01Z".into())
            .expect("start should succeed");
        run.cancel("2026-08-02T18:20:02Z".into())
            .expect("cancel should succeed");

        assert_eq!(run.phase, EntropyRunPhase::Cancelled);
        assert_eq!(run.counts().cancelled, 2);
        assert_eq!(run.events[0].state, EntropyFileState::Reading);
        assert_eq!(run.events[1].state, EntropyFileState::Cancelled);
        assert_eq!(run.events[2].state, EntropyFileState::Cancelled);
    }

    #[test]
    fn coldcard_fixture_is_a_six_link_source_hypothesis() {
        let output: EntropyFileAnalysisOutput = serde_json::from_str(include_str!(
            "../fixtures/coldcard-entropy-hypothesis.v1.json"
        ))
        .expect("Coldcard fixture should decode");
        let root = tempfile::tempdir().expect("temporary repository");
        let mut source = fs::File::create(root.path().join("mpconfigboard.h"))
            .expect("fixture source should be created");
        writeln!(source, "#define MICROPY_HW_ENABLE_RNG (0)")
            .expect("fixture source should be written");
        let manifest = build_manifest(root.path(), Vec::new());
        let binding = run_binding(&manifest);
        output.validate(&binding).expect("fixture should validate");
        let hypothesis = output.hypotheses.first().expect("fixture hypothesis");
        assert_eq!(hypothesis.causal_links.len(), 6);
        assert!(
            hypothesis
                .missing_evidence
                .iter()
                .any(|value| value.contains("linked artifact"))
        );
    }
}
