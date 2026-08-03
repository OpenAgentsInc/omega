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
pub const ENTROPY_RUN_SUMMARY_SCHEMA_V1: &str = "openagents.omega.entropy-run-summary.v1";
pub const ENTROPY_FILE_OUTPUT_SCHEMA_V1: &str = "openagents.omega.entropy-file-output.v1";
pub const ENTROPY_PROMPT_SNAPSHOT_SCHEMA_V1: &str = "openagents.omega.entropy-prompt-snapshot.v1";
pub const ENTROPY_SOURCE_INSPECTION_SCHEMA_V1: &str =
    "openagents.omega.entropy-source-inspection.v1";

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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub materialization_error: Option<String>,
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
        if self
            .materialization_error
            .as_ref()
            .is_some_and(|error| error.trim().is_empty() || error.len() > 4_096)
        {
            return Err(ForensicsError::InvalidEntropyRun(
                "dependency materialization errors must contain 1 to 4096 bytes".into(),
            ));
        }
        if self.availability == EntropyDependencyAvailability::Available
            && self.materialization_error.is_some()
        {
            return Err(ForensicsError::InvalidEntropyRun(
                "available dependencies cannot retain materialization errors".into(),
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

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EntropySourceInspectionState {
    Pending,
    Complete,
    Incomplete,
    Denied,
    Stale,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EntropySourceInspectionInput {
    pub inspection_ref: String,
    pub generation: u64,
    pub observed_revision: String,
    pub top_level_tree: String,
    pub focal_paths: Vec<String>,
    pub reached_paths: Vec<String>,
    pub required_generated_input_paths: Vec<String>,
    pub missing_generated_input_paths: Vec<String>,
    pub required_excluded_paths: Vec<String>,
    pub dirty_excluded_paths: Vec<String>,
    pub observed_at: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct EntropySourceInspection {
    pub schema: String,
    pub inspection_ref: String,
    pub generation: u64,
    pub state: EntropySourceInspectionState,
    pub repository: EntropyRepositoryBinding,
    pub observed_revision: String,
    pub top_level_tree: String,
    pub manifest_ref: String,
    pub manifest_digest: String,
    pub focal_paths: Vec<String>,
    pub contextual_paths: Vec<String>,
    pub reached_paths: Vec<String>,
    pub not_reached_paths: Vec<String>,
    pub dependency_paths: Vec<String>,
    pub dependency_facts: Vec<EntropyDependencyBinding>,
    pub required_generated_input_paths: Vec<String>,
    pub missing_generated_input_paths: Vec<String>,
    pub excluded_paths: Vec<String>,
    pub required_excluded_paths: Vec<String>,
    pub oversized_paths: Vec<String>,
    pub dirty_excluded_paths: Vec<String>,
    pub reason_refs: Vec<String>,
    pub observed_at: String,
    pub canonical_digest: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct EntropySourceInspectionDigestInput<'a> {
    schema: &'a str,
    inspection_ref: &'a str,
    generation: u64,
    state: EntropySourceInspectionState,
    repository: &'a EntropyRepositoryBinding,
    observed_revision: &'a str,
    top_level_tree: &'a str,
    manifest_ref: &'a str,
    manifest_digest: &'a str,
    focal_paths: &'a [String],
    contextual_paths: &'a [String],
    reached_paths: &'a [String],
    not_reached_paths: &'a [String],
    dependency_paths: &'a [String],
    dependency_facts: &'a [EntropyDependencyBinding],
    required_generated_input_paths: &'a [String],
    missing_generated_input_paths: &'a [String],
    excluded_paths: &'a [String],
    required_excluded_paths: &'a [String],
    oversized_paths: &'a [String],
    dirty_excluded_paths: &'a [String],
    reason_refs: &'a [String],
    observed_at: &'a str,
}

impl EntropySourceInspection {
    pub fn from_manifest(
        manifest: &EntropyManifest,
        mut input: EntropySourceInspectionInput,
    ) -> Result<Self, ForensicsError> {
        manifest.validate()?;
        validate_public_ref("source inspection", &input.inspection_ref)?;
        if input.generation == 0 {
            return Err(ForensicsError::InvalidEntropyRun(
                "source inspection generation must be positive".into(),
            ));
        }
        validate_revision(&input.top_level_tree)?;
        validate_revision(&input.observed_revision)?;
        for paths in [
            &mut input.focal_paths,
            &mut input.reached_paths,
            &mut input.required_generated_input_paths,
            &mut input.missing_generated_input_paths,
            &mut input.required_excluded_paths,
            &mut input.dirty_excluded_paths,
        ] {
            paths.sort();
            paths.dedup();
            for path in paths.iter() {
                validate_relative_path(path)?;
            }
        }
        if input.observed_at.trim().is_empty() || input.observed_at.len() > 64 {
            return Err(ForensicsError::InvalidEntropyRun(
                "source inspection time must be present and bounded".into(),
            ));
        }

        let all_paths = manifest
            .files
            .iter()
            .map(|file| file.path.clone())
            .collect::<BTreeSet<_>>();
        if input
            .focal_paths
            .iter()
            .chain(input.reached_paths.iter())
            .any(|path| !all_paths.contains(path))
        {
            return Err(ForensicsError::InvalidEntropyRun(
                "focal and reached paths must belong to the immutable manifest".into(),
            ));
        }
        if input
            .missing_generated_input_paths
            .iter()
            .any(|path| !input.required_generated_input_paths.contains(path))
        {
            return Err(ForensicsError::InvalidEntropyRun(
                "missing generated inputs must be declared as required".into(),
            ));
        }

        let contextual_paths = all_paths
            .iter()
            .filter(|path| !input.focal_paths.contains(path))
            .cloned()
            .collect::<Vec<_>>();
        let not_reached_paths = all_paths
            .iter()
            .filter(|path| !input.reached_paths.contains(path))
            .cloned()
            .collect::<Vec<_>>();
        let dependency_paths = manifest
            .dependencies
            .iter()
            .map(|dependency| dependency.path.clone())
            .collect::<Vec<_>>();
        let excluded_paths = manifest
            .files
            .iter()
            .filter(|file| file.eligibility != EntropyFileEligibility::Eligible)
            .map(|file| file.path.clone())
            .collect::<Vec<_>>();
        let oversized_paths = manifest
            .files
            .iter()
            .filter(|file| file.eligibility == EntropyFileEligibility::Oversized)
            .map(|file| file.path.clone())
            .collect::<Vec<_>>();
        if input
            .required_excluded_paths
            .iter()
            .any(|path| !excluded_paths.contains(path))
        {
            return Err(ForensicsError::InvalidEntropyRun(
                "required excluded paths must name an ineligible manifest row".into(),
            ));
        }
        let mut reason_refs = manifest
            .dependencies
            .iter()
            .filter(|dependency| {
                dependency.availability != EntropyDependencyAvailability::Available
            })
            .map(|dependency| match dependency.availability {
                EntropyDependencyAvailability::Missing => "source.dependency.missing",
                EntropyDependencyAvailability::WrongRevision => "source.dependency.wrong_revision",
                EntropyDependencyAvailability::SourceUnavailable => "source.dependency.unavailable",
                EntropyDependencyAvailability::Available => unreachable!(),
            })
            .map(str::to_string)
            .collect::<Vec<_>>();
        if input.observed_revision != manifest.repository.revision {
            reason_refs.push("source.top_level.wrong_revision".into());
        }
        if !input.required_excluded_paths.is_empty() {
            reason_refs.push("source.required_path.excluded".into());
        }
        if manifest
            .files
            .iter()
            .any(|file| file.eligibility == EntropyFileEligibility::Oversized)
        {
            reason_refs.push("source.path.oversized".into());
        }
        if manifest
            .files
            .iter()
            .any(|file| file.eligibility == EntropyFileEligibility::SourceUnavailable)
        {
            reason_refs.push("source.path.unavailable".into());
        }
        if manifest
            .files
            .iter()
            .any(|file| file.eligibility == EntropyFileEligibility::Symlink)
        {
            reason_refs.push("source.path.symlink".into());
        }
        if !input.missing_generated_input_paths.is_empty() {
            reason_refs.push("source.generated_input.missing".into());
        }
        if !input.dirty_excluded_paths.is_empty() {
            reason_refs.push("source.dirty_bytes.excluded".into());
        }
        reason_refs.sort();
        reason_refs.dedup();
        let state = if reason_refs.is_empty() {
            EntropySourceInspectionState::Complete
        } else {
            EntropySourceInspectionState::Incomplete
        };
        let mut inspection = Self {
            schema: ENTROPY_SOURCE_INSPECTION_SCHEMA_V1.into(),
            inspection_ref: input.inspection_ref,
            generation: input.generation,
            state,
            repository: manifest.repository.clone(),
            observed_revision: input.observed_revision,
            top_level_tree: input.top_level_tree,
            manifest_ref: manifest.manifest_ref.clone(),
            manifest_digest: manifest.canonical_digest.clone(),
            focal_paths: input.focal_paths,
            contextual_paths,
            reached_paths: input.reached_paths,
            not_reached_paths,
            dependency_paths,
            dependency_facts: manifest.dependencies.clone(),
            required_generated_input_paths: input.required_generated_input_paths,
            missing_generated_input_paths: input.missing_generated_input_paths,
            excluded_paths,
            required_excluded_paths: input.required_excluded_paths,
            oversized_paths,
            dirty_excluded_paths: input.dirty_excluded_paths,
            reason_refs,
            observed_at: input.observed_at,
            canonical_digest: String::new(),
        };
        inspection.canonical_digest = inspection.computed_digest()?;
        inspection.validate()?;
        Ok(inspection)
    }

    pub fn mark_stale(&self, generation: u64, observed_at: String) -> Result<Self, ForensicsError> {
        if generation <= self.generation {
            return Err(ForensicsError::InvalidEntropyRun(
                "stale source generations must advance monotonically".into(),
            ));
        }
        let mut stale = self.clone();
        stale.generation = generation;
        stale.state = EntropySourceInspectionState::Stale;
        stale.observed_at = observed_at;
        stale.reason_refs.push("source.generation.changed".into());
        stale.reason_refs.sort();
        stale.reason_refs.dedup();
        stale.canonical_digest = stale.computed_digest()?;
        stale.validate()?;
        Ok(stale)
    }

    pub fn qualified_miss_eligible(&self) -> bool {
        self.state == EntropySourceInspectionState::Complete && self.reason_refs.is_empty()
    }

    pub fn validate(&self) -> Result<(), ForensicsError> {
        if self.schema != ENTROPY_SOURCE_INSPECTION_SCHEMA_V1 {
            return Err(ForensicsError::InvalidSchema);
        }
        validate_public_ref("source inspection", &self.inspection_ref)?;
        self.repository.validate()?;
        validate_revision(&self.top_level_tree)?;
        validate_revision(&self.observed_revision)?;
        validate_sha256("source manifest", &self.manifest_digest)?;
        validate_sha256("source inspection", &self.canonical_digest)?;
        if self.generation == 0
            || self.observed_at.trim().is_empty()
            || self.observed_at.len() > 64
            || self.canonical_digest != self.computed_digest()?
        {
            return Err(ForensicsError::InvalidEntropyRun(
                "source inspection identity, time, or digest is invalid".into(),
            ));
        }
        for paths in [
            &self.focal_paths,
            &self.contextual_paths,
            &self.reached_paths,
            &self.not_reached_paths,
            &self.dependency_paths,
            &self.required_generated_input_paths,
            &self.missing_generated_input_paths,
            &self.excluded_paths,
            &self.required_excluded_paths,
            &self.oversized_paths,
            &self.dirty_excluded_paths,
        ] {
            let mut prior: Option<&str> = None;
            for path in paths {
                validate_relative_path(path)?;
                if prior.is_some_and(|prior| prior >= path.as_str()) {
                    return Err(ForensicsError::InvalidEntropyRun(
                        "source inspection paths must be sorted and unique".into(),
                    ));
                }
                prior = Some(path);
            }
        }
        if self.dependency_facts.len() != self.dependency_paths.len()
            || self
                .dependency_facts
                .iter()
                .zip(self.dependency_paths.iter())
                .any(|(fact, path)| fact.path != *path || fact.validate().is_err())
        {
            return Err(ForensicsError::InvalidEntropyRun(
                "source dependency facts must account for every declared path in order".into(),
            ));
        }
        if self.state == EntropySourceInspectionState::Complete && !self.reason_refs.is_empty() {
            return Err(ForensicsError::InvalidEntropyRun(
                "complete source inspection cannot retain incomplete reasons".into(),
            ));
        }
        if matches!(
            self.state,
            EntropySourceInspectionState::Incomplete | EntropySourceInspectionState::Stale
        ) && self.reason_refs.is_empty()
        {
            return Err(ForensicsError::InvalidEntropyRun(
                "incomplete or stale source inspection requires an exact reason".into(),
            ));
        }
        for reason_ref in &self.reason_refs {
            validate_public_ref("source inspection reason", reason_ref)?;
        }
        Ok(())
    }

    fn computed_digest(&self) -> Result<String, ForensicsError> {
        sha256_json(&EntropySourceInspectionDigestInput {
            schema: &self.schema,
            inspection_ref: &self.inspection_ref,
            generation: self.generation,
            state: self.state,
            repository: &self.repository,
            observed_revision: &self.observed_revision,
            top_level_tree: &self.top_level_tree,
            manifest_ref: &self.manifest_ref,
            manifest_digest: &self.manifest_digest,
            focal_paths: &self.focal_paths,
            contextual_paths: &self.contextual_paths,
            reached_paths: &self.reached_paths,
            not_reached_paths: &self.not_reached_paths,
            dependency_paths: &self.dependency_paths,
            dependency_facts: &self.dependency_facts,
            required_generated_input_paths: &self.required_generated_input_paths,
            missing_generated_input_paths: &self.missing_generated_input_paths,
            excluded_paths: &self.excluded_paths,
            required_excluded_paths: &self.required_excluded_paths,
            oversized_paths: &self.oversized_paths,
            dirty_excluded_paths: &self.dirty_excluded_paths,
            reason_refs: &self.reason_refs,
            observed_at: &self.observed_at,
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
    pub fn validate(&self) -> Result<(), ForensicsError> {
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
    AwaitingCleanup,
    Completed,
    CompletedWithLimitations,
    Failed,
    FailedWithPartialOutput,
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
    TimedOut,
    Refused,
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
    ToolUnavailable,
    ToolDenied,
    ToolTimedOut,
    SessionRefused,
    SessionTimedOut,
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
    pub timed_out: u32,
    pub refused: u32,
    pub cancelled: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EntropyToolState {
    Requested,
    Available,
    Unavailable,
    Denied,
    TimedOut,
    Failed,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct EntropyToolFact {
    pub tool_ref: String,
    pub state: EntropyToolState,
    pub reason_ref: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EntropyAccountingOutcome {
    Active,
    Completed,
    CompletedIncomplete,
    Failed,
    FailedWithPartialOutput,
    Cancelled,
    RecoveryRequired,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EntropyRunUsageExactness {
    Exact,
    Estimated,
    Unavailable,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct EntropyUsageValue {
    pub value: Option<u64>,
    pub exactness: EntropyRunUsageExactness,
}

impl EntropyUsageValue {
    pub const fn unavailable() -> Self {
        Self {
            value: None,
            exactness: EntropyRunUsageExactness::Unavailable,
        }
    }

    fn validate(&self) -> Result<(), ForensicsError> {
        if self.value.is_some() == (self.exactness == EntropyRunUsageExactness::Unavailable) {
            return Err(ForensicsError::InvalidEntropyRun(
                "usage values and exactness must agree; unavailable is never numeric zero".into(),
            ));
        }
        Ok(())
    }
}

impl Default for EntropyUsageValue {
    fn default() -> Self {
        Self::unavailable()
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct EntropySessionCounts {
    pub queued: u32,
    pub attempted: u32,
    pub settled: u32,
    pub timed_out: u32,
    pub cancelled: u32,
    pub refused: u32,
    pub failed: u32,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct EntropyToolCounts {
    pub requested: u32,
    pub available: u32,
    pub unavailable: u32,
    pub denied: u32,
    pub timed_out: u32,
    pub failed: u32,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct EntropyOutputCounts {
    pub findings: u32,
    pub hypotheses: u32,
    pub duplicates: u32,
    pub limitations: u32,
    pub rejected_malformed: u32,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct EntropySourceCounts {
    pub eligible_focal_units: u32,
    pub focal_used: u32,
    pub contextual_read: u32,
    pub reached: u32,
    pub excluded: u32,
    pub skipped: u32,
    pub oversized: u32,
    pub never_reached: u32,
    pub dependency_trees_total: u32,
    pub dependency_trees_reached: u32,
    pub manifest_generation: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EntropyCleanupState {
    Pending,
    Observed,
    Failed,
    RecoveryRequired,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct EntropyCleanupTruth {
    pub state: EntropyCleanupState,
    pub receipt_ref: Option<String>,
    #[serde(default)]
    pub reason_ref: Option<String>,
    pub observed_at: Option<String>,
}

impl Default for EntropyCleanupTruth {
    fn default() -> Self {
        Self {
            state: EntropyCleanupState::Pending,
            receipt_ref: None,
            reason_ref: None,
            observed_at: None,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct EntropyRunSummary {
    pub schema: String,
    pub run_ref: String,
    pub outcome: EntropyAccountingOutcome,
    pub source: EntropySourceCounts,
    pub sessions: EntropySessionCounts,
    pub tools: EntropyToolCounts,
    pub outputs: EntropyOutputCounts,
    pub elapsed_milliseconds: EntropyUsageValue,
    pub total_tokens: EntropyUsageValue,
    pub cost_micros: EntropyUsageValue,
    pub network_bytes: EntropyUsageValue,
    pub cleanup: EntropyCleanupTruth,
    pub output_refs: Vec<String>,
    pub failure_refs: Vec<String>,
    pub canonical_digest: String,
}

impl Default for EntropyRunSummary {
    fn default() -> Self {
        Self {
            schema: String::new(),
            run_ref: String::new(),
            outcome: EntropyAccountingOutcome::Active,
            source: EntropySourceCounts::default(),
            sessions: EntropySessionCounts::default(),
            tools: EntropyToolCounts::default(),
            outputs: EntropyOutputCounts::default(),
            elapsed_milliseconds: EntropyUsageValue::unavailable(),
            total_tokens: EntropyUsageValue::unavailable(),
            cost_micros: EntropyUsageValue::unavailable(),
            network_bytes: EntropyUsageValue::unavailable(),
            cleanup: EntropyCleanupTruth::default(),
            output_refs: Vec::new(),
            failure_refs: Vec::new(),
            canonical_digest: String::new(),
        }
    }
}

impl EntropyRunSummary {
    fn computed_digest(&self) -> Result<String, ForensicsError> {
        let mut value = self.clone();
        value.canonical_digest.clear();
        sha256_json(&value)
    }

    pub fn validate(&self) -> Result<(), ForensicsError> {
        if self.schema != ENTROPY_RUN_SUMMARY_SCHEMA_V1 {
            return Err(ForensicsError::InvalidSchema);
        }
        validate_public_ref("entropy run", &self.run_ref)?;
        validate_sha256("entropy run summary", &self.canonical_digest)?;
        for usage in [
            &self.elapsed_milliseconds,
            &self.total_tokens,
            &self.cost_micros,
            &self.network_bytes,
        ] {
            usage.validate()?;
        }
        if self.canonical_digest != self.computed_digest()? {
            return Err(ForensicsError::InvalidEntropyRun(
                "entropy run summary digest is invalid".into(),
            ));
        }
        if self.sessions.settled > self.source.eligible_focal_units
            || self.sessions.attempted > self.source.eligible_focal_units
            || self.tools.available
                + self.tools.unavailable
                + self.tools.denied
                + self.tools.timed_out
                + self.tools.failed
                > self.tools.requested
        {
            return Err(ForensicsError::InvalidEntropyRun(
                "entropy run summary denominators are inconsistent".into(),
            ));
        }
        let clean = self.source.eligible_focal_units > 0
            && self.sessions.attempted == self.source.eligible_focal_units
            && self.sessions.settled == self.sessions.attempted
            && self.sessions.failed == 0
            && self.sessions.timed_out == 0
            && self.sessions.refused == 0
            && self.sessions.cancelled == 0
            && self.tools.available == self.tools.requested
            && self.outputs.rejected_malformed == 0
            && self.failure_refs.is_empty()
            && self.cleanup.state == EntropyCleanupState::Observed
            && self.cleanup.receipt_ref.is_some();
        if self.outcome == EntropyAccountingOutcome::Completed && !clean {
            return Err(ForensicsError::InvalidEntropyRun(
                "ordinary completion requires complete session, tool, source, and cleanup truth"
                    .into(),
            ));
        }
        if (self.cleanup.state == EntropyCleanupState::Observed)
            != self.cleanup.receipt_ref.is_some()
        {
            return Err(ForensicsError::InvalidEntropyRun(
                "cleanup state and receipt disagree".into(),
            ));
        }
        if matches!(
            self.cleanup.state,
            EntropyCleanupState::Observed
                | EntropyCleanupState::Failed
                | EntropyCleanupState::RecoveryRequired
        ) != self.cleanup.observed_at.is_some()
        {
            return Err(ForensicsError::InvalidEntropyRun(
                "cleanup state and observation time disagree".into(),
            ));
        }
        if (self.cleanup.state == EntropyCleanupState::Observed
            && self.cleanup.reason_ref.is_some())
            || (matches!(
                self.cleanup.state,
                EntropyCleanupState::Failed | EntropyCleanupState::RecoveryRequired
            ) != self.cleanup.reason_ref.is_some())
        {
            return Err(ForensicsError::InvalidEntropyRun(
                "cleanup failure state and exact reason disagree".into(),
            ));
        }
        for value in self.output_refs.iter().chain(&self.failure_refs) {
            validate_public_ref("entropy accounting ref", value)?;
        }
        Ok(())
    }

    pub fn qualified_miss_eligible(&self) -> bool {
        self.outcome == EntropyAccountingOutcome::Completed
            && self.outputs.findings == 0
            && self.outputs.hypotheses == 0
            && self.outputs.rejected_malformed == 0
    }
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
    #[serde(default)]
    pub tool_facts: Vec<EntropyToolFact>,
    #[serde(default)]
    pub cleanup: EntropyCleanupTruth,
    #[serde(default)]
    pub summary: EntropyRunSummary,
    #[serde(default)]
    pub duplicate_terminal_events: u32,
}

impl EntropyRunProjection {
    pub fn migrate_legacy_accounting(&mut self) -> Result<bool, ForensicsError> {
        if self.summary.schema == ENTROPY_RUN_SUMMARY_SCHEMA_V1 {
            return Ok(false);
        }
        if !self.summary.schema.is_empty() || !self.tool_facts.is_empty() {
            return Err(ForensicsError::InvalidEntropyRun(
                "unknown entropy accounting schema cannot be migrated".into(),
            ));
        }
        self.tool_facts = self
            .binding
            .tool_surface_refs
            .iter()
            .cloned()
            .map(|tool_ref| EntropyToolFact {
                tool_ref,
                state: EntropyToolState::Requested,
                reason_ref: None,
            })
            .collect();
        self.cleanup = EntropyCleanupTruth::default();
        self.summary = EntropyRunSummary {
            run_ref: self.binding.run_ref.clone(),
            ..EntropyRunSummary::default()
        };
        if matches!(
            self.phase,
            EntropyRunPhase::Completed | EntropyRunPhase::CompletedWithLimitations
        ) {
            self.phase = EntropyRunPhase::AwaitingCleanup;
        }
        self.refresh_summary()?;
        self.apply_summary_phase();
        Ok(true)
    }

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
                    message: dependency.materialization_error.as_ref().map_or_else(
                        || {
                            format!(
                                "Dependency {} is {:?}",
                                dependency.path, dependency.availability
                            )
                        },
                        |error| {
                            format!(
                                "Dependency {} is {:?}: {error}",
                                dependency.path, dependency.availability
                            )
                        },
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
        let tool_facts = binding
            .tool_surface_refs
            .iter()
            .cloned()
            .map(|tool_ref| EntropyToolFact {
                tool_ref,
                state: EntropyToolState::Requested,
                reason_ref: None,
            })
            .collect();
        let cleanup = EntropyCleanupTruth {
            state: EntropyCleanupState::Pending,
            receipt_ref: None,
            reason_ref: None,
            observed_at: None,
        };
        let summary = EntropyRunSummary {
            schema: ENTROPY_RUN_SUMMARY_SCHEMA_V1.into(),
            run_ref: binding.run_ref.clone(),
            outcome: EntropyAccountingOutcome::Active,
            source: EntropySourceCounts::default(),
            sessions: EntropySessionCounts::default(),
            tools: EntropyToolCounts::default(),
            outputs: EntropyOutputCounts::default(),
            elapsed_milliseconds: EntropyUsageValue::unavailable(),
            total_tokens: EntropyUsageValue::unavailable(),
            cost_micros: EntropyUsageValue::unavailable(),
            network_bytes: EntropyUsageValue::unavailable(),
            cleanup: cleanup.clone(),
            output_refs: Vec::new(),
            failure_refs: Vec::new(),
            canonical_digest: String::new(),
        };
        let mut run = Self {
            schema: ENTROPY_RUN_SCHEMA_V1.into(),
            binding,
            manifest,
            phase: EntropyRunPhase::Ready,
            files,
            limitations,
            events: Vec::new(),
            tool_facts,
            cleanup,
            summary,
            duplicate_terminal_events: 0,
        };
        run.refresh_summary()?;
        Ok(run)
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
        if self.tool_facts.len() != self.binding.tool_surface_refs.len() {
            return Err(ForensicsError::InvalidEntropyRun(
                "tool accounting must preserve every requested tool".into(),
            ));
        }
        for (fact, requested_ref) in self.tool_facts.iter().zip(&self.binding.tool_surface_refs) {
            validate_public_ref("tool", &fact.tool_ref)?;
            if fact.tool_ref != *requested_ref
                || fact
                    .reason_ref
                    .as_ref()
                    .is_some_and(|reason| validate_public_ref("tool reason", reason).is_err())
                || (fact.state == EntropyToolState::Available && fact.reason_ref.is_some())
                || (matches!(
                    fact.state,
                    EntropyToolState::Unavailable
                        | EntropyToolState::Denied
                        | EntropyToolState::TimedOut
                        | EntropyToolState::Failed
                ) && fact.reason_ref.is_none())
            {
                return Err(ForensicsError::InvalidEntropyRun(
                    "tool accounting fact is invalid or out of request order".into(),
                ));
            }
        }
        self.summary.validate()?;
        if self.summary != self.rebuilt_summary()? {
            return Err(ForensicsError::InvalidEntropyRun(
                "persisted entropy accounting drifted from the canonical run facts".into(),
            ));
        }
        let phase_matches_summary = match self.summary.outcome {
            EntropyAccountingOutcome::Active => matches!(
                self.phase,
                EntropyRunPhase::Ready
                    | EntropyRunPhase::Running
                    | EntropyRunPhase::CancelRequested
            ),
            EntropyAccountingOutcome::RecoveryRequired => {
                self.phase == EntropyRunPhase::AwaitingCleanup
            }
            EntropyAccountingOutcome::Completed => self.phase == EntropyRunPhase::Completed,
            EntropyAccountingOutcome::CompletedIncomplete => {
                self.phase == EntropyRunPhase::CompletedWithLimitations
            }
            EntropyAccountingOutcome::Failed => self.phase == EntropyRunPhase::Failed,
            EntropyAccountingOutcome::FailedWithPartialOutput => {
                self.phase == EntropyRunPhase::FailedWithPartialOutput
            }
            EntropyAccountingOutcome::Cancelled => self.phase == EntropyRunPhase::Cancelled,
        };
        if !phase_matches_summary {
            return Err(ForensicsError::InvalidEntropyRun(
                "run phase and canonical accounting outcome disagree".into(),
            ));
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

    pub fn observe_tool(
        &mut self,
        tool_ref: &str,
        state: EntropyToolState,
        reason_ref: Option<String>,
    ) -> Result<(), ForensicsError> {
        if let Some(reason) = &reason_ref {
            validate_public_ref("tool reason", reason)?;
        }
        if (state == EntropyToolState::Available && reason_ref.is_some())
            || (matches!(
                state,
                EntropyToolState::Unavailable
                    | EntropyToolState::Denied
                    | EntropyToolState::TimedOut
                    | EntropyToolState::Failed
            ) && reason_ref.is_none())
        {
            return Err(ForensicsError::InvalidEntropyRun(
                "tool outcome and exact reason must agree".into(),
            ));
        }
        let fact = self
            .tool_facts
            .iter_mut()
            .find(|fact| fact.tool_ref == tool_ref)
            .ok_or_else(|| ForensicsError::InvalidEntropyRun("tool was not requested".into()))?;
        if fact.state != EntropyToolState::Requested {
            if fact.state == state && fact.reason_ref == reason_ref {
                return Ok(());
            }
            return Err(ForensicsError::InvalidEntropyRun(
                "settled tool facts are immutable".into(),
            ));
        }
        if state == EntropyToolState::Requested {
            return Ok(());
        }
        fact.state = state;
        fact.reason_ref = reason_ref;
        self.refresh_summary()?;
        Ok(())
    }

    pub fn observe_all_tools_available(&mut self) -> Result<(), ForensicsError> {
        let refs = self
            .tool_facts
            .iter()
            .map(|fact| fact.tool_ref.clone())
            .collect::<Vec<_>>();
        for tool_ref in refs {
            self.observe_tool(&tool_ref, EntropyToolState::Available, None)?;
        }
        Ok(())
    }

    pub fn record_usage(
        &mut self,
        elapsed_milliseconds: EntropyUsageValue,
        total_tokens: EntropyUsageValue,
        cost_micros: EntropyUsageValue,
        network_bytes: EntropyUsageValue,
    ) -> Result<(), ForensicsError> {
        for value in [
            &elapsed_milliseconds,
            &total_tokens,
            &cost_micros,
            &network_bytes,
        ] {
            value.validate()?;
        }
        self.summary.elapsed_milliseconds = elapsed_milliseconds;
        self.summary.total_tokens = total_tokens;
        self.summary.cost_micros = cost_micros;
        self.summary.network_bytes = network_bytes;
        self.refresh_summary()?;
        Ok(())
    }

    pub fn observe_cleanup(
        &mut self,
        receipt_ref: String,
        observed_at: String,
    ) -> Result<(), ForensicsError> {
        validate_public_ref("cleanup receipt", &receipt_ref)?;
        if observed_at.trim().is_empty() || observed_at.len() > 64 {
            return Err(ForensicsError::InvalidEntropyRun(
                "cleanup observation time must be present and bounded".into(),
            ));
        }
        if self.files.iter().any(|file| {
            matches!(
                file.state,
                EntropyFileState::Queued | EntropyFileState::Reading
            )
        }) {
            return Err(ForensicsError::InvalidEntropyRun(
                "cleanup cannot settle while file sessions remain active".into(),
            ));
        }
        if self.cleanup.state == EntropyCleanupState::Observed {
            if self.cleanup.receipt_ref.as_deref() == Some(&receipt_ref)
                && self.cleanup.observed_at.as_deref() == Some(&observed_at)
            {
                return Ok(());
            }
            return Err(ForensicsError::InvalidEntropyRun(
                "cleanup settlement is immutable".into(),
            ));
        }
        self.cleanup = EntropyCleanupTruth {
            state: EntropyCleanupState::Observed,
            receipt_ref: Some(receipt_ref),
            reason_ref: None,
            observed_at: Some(observed_at),
        };
        self.refresh_summary()?;
        self.apply_summary_phase();
        Ok(())
    }

    pub fn fail_cleanup(
        &mut self,
        reason_ref: String,
        observed_at: String,
    ) -> Result<(), ForensicsError> {
        validate_public_ref("cleanup failure", &reason_ref)?;
        if observed_at.trim().is_empty() || observed_at.len() > 64 {
            return Err(ForensicsError::InvalidEntropyRun(
                "cleanup failure time must be present and bounded".into(),
            ));
        }
        if self.cleanup.state == EntropyCleanupState::Observed {
            return Err(ForensicsError::InvalidEntropyRun(
                "observed cleanup cannot regress to failure".into(),
            ));
        }
        self.cleanup = EntropyCleanupTruth {
            state: EntropyCleanupState::Failed,
            receipt_ref: None,
            reason_ref: Some(reason_ref),
            observed_at: Some(observed_at),
        };
        self.phase = EntropyRunPhase::AwaitingCleanup;
        self.refresh_summary()?;
        Ok(())
    }

    pub fn start_next_file(
        &mut self,
        observed_at: String,
    ) -> Result<Option<EntropyFileTask>, ForensicsError> {
        if matches!(
            self.phase,
            EntropyRunPhase::CancelRequested
                | EntropyRunPhase::AwaitingCleanup
                | EntropyRunPhase::Cancelled
                | EntropyRunPhase::Completed
                | EntropyRunPhase::CompletedWithLimitations
                | EntropyRunPhase::Failed
                | EntropyRunPhase::FailedWithPartialOutput
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
        self.refresh_summary()?;
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
        if let Some(file) = self.files.iter().find(|file| file.path == output.file_path)
            && matches!(
                file.state,
                EntropyFileState::Analyzed | EntropyFileState::Candidate
            )
        {
            if file.observations == output.observations
                && file.hypotheses == output.hypotheses
                && file.limitations == output.limitations
            {
                self.duplicate_terminal_events = self.duplicate_terminal_events.saturating_add(1);
                self.refresh_summary()?;
                return Ok(());
            }
            return Err(ForensicsError::InvalidEntropyRun(
                "settled file output is immutable".into(),
            ));
        }
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
        self.refresh_summary()?;
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
        if matches!(
            file.state,
            EntropyFileState::Failed | EntropyFileState::TimedOut | EntropyFileState::Refused
        ) && file.limitations.contains(&limitation)
        {
            self.duplicate_terminal_events = self.duplicate_terminal_events.saturating_add(1);
            self.refresh_summary()?;
            return Ok(());
        }
        if file.state != EntropyFileState::Reading {
            return Err(ForensicsError::InvalidEntropyRun(
                "only a reading file can fail".into(),
            ));
        }
        file.state = match limitation.class {
            EntropyLimitationClass::SessionTimedOut | EntropyLimitationClass::ToolTimedOut => {
                EntropyFileState::TimedOut
            }
            EntropyLimitationClass::SessionRefused | EntropyLimitationClass::ToolDenied => {
                EntropyFileState::Refused
            }
            _ => EntropyFileState::Failed,
        };
        file.limitations.push(limitation.clone());
        self.limitations.push(limitation);
        let state = file.state;
        self.push_event(Some(path), state, observed_at)?;
        self.refresh_summary()?;
        self.finish_if_exhausted();
        Ok(())
    }

    pub fn cancel(&mut self, observed_at: String) -> Result<(), ForensicsError> {
        if matches!(
            self.phase,
            EntropyRunPhase::Completed
                | EntropyRunPhase::CompletedWithLimitations
                | EntropyRunPhase::Failed
                | EntropyRunPhase::FailedWithPartialOutput
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
        self.refresh_summary()?;
        self.apply_summary_phase();
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
                EntropyFileState::TimedOut => counts.timed_out += 1,
                EntropyFileState::Refused => counts.refused += 1,
                EntropyFileState::Cancelled => counts.cancelled += 1,
            }
        }
        counts
    }

    pub fn qualified_miss_eligible(&self, inspection: &EntropySourceInspection) -> bool {
        self.phase == EntropyRunPhase::Completed
            && self.binding.repository == inspection.repository
            && self.binding.manifest_ref == inspection.manifest_ref
            && self.binding.manifest_digest == inspection.manifest_digest
            && inspection.qualified_miss_eligible()
            && self.summary.qualified_miss_eligible()
            && self
                .files
                .iter()
                .all(|file| file.state == EntropyFileState::Analyzed)
            && self
                .files
                .iter()
                .all(|file| file.observations.is_empty() && file.hypotheses.is_empty())
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
        self.phase = EntropyRunPhase::AwaitingCleanup;
        let _ = self.refresh_summary();
    }

    fn apply_summary_phase(&mut self) {
        self.phase = match self.summary.outcome {
            EntropyAccountingOutcome::Active => self.phase,
            EntropyAccountingOutcome::Completed => EntropyRunPhase::Completed,
            EntropyAccountingOutcome::CompletedIncomplete => {
                EntropyRunPhase::CompletedWithLimitations
            }
            EntropyAccountingOutcome::Failed => EntropyRunPhase::Failed,
            EntropyAccountingOutcome::FailedWithPartialOutput => {
                EntropyRunPhase::FailedWithPartialOutput
            }
            EntropyAccountingOutcome::Cancelled => EntropyRunPhase::Cancelled,
            EntropyAccountingOutcome::RecoveryRequired => EntropyRunPhase::AwaitingCleanup,
        };
    }

    fn refresh_summary(&mut self) -> Result<(), ForensicsError> {
        self.summary = self.rebuilt_summary()?;
        Ok(())
    }

    fn rebuilt_summary(&self) -> Result<EntropyRunSummary, ForensicsError> {
        let mut sessions = EntropySessionCounts::default();
        let mut outputs = EntropyOutputCounts::default();
        let mut output_refs = Vec::new();
        let mut reached_paths = BTreeSet::new();
        let mut contextual_paths = BTreeSet::new();
        for file in &self.files {
            match file.state {
                EntropyFileState::Queued => sessions.queued += 1,
                EntropyFileState::Reading => sessions.attempted += 1,
                EntropyFileState::Analyzed | EntropyFileState::Candidate => {
                    sessions.attempted += 1;
                    sessions.settled += 1;
                }
                EntropyFileState::Failed => {
                    sessions.attempted += 1;
                    sessions.settled += 1;
                    sessions.failed += 1;
                }
                EntropyFileState::TimedOut => {
                    sessions.attempted += 1;
                    sessions.settled += 1;
                    sessions.timed_out += 1;
                }
                EntropyFileState::Refused => {
                    sessions.attempted += 1;
                    sessions.settled += 1;
                    sessions.refused += 1;
                }
                EntropyFileState::Cancelled => {
                    sessions.settled += 1;
                    sessions.cancelled += 1;
                    if self.events.iter().any(|event| {
                        event.file_path.as_deref() == Some(file.path.as_str())
                            && event.state == EntropyFileState::Reading
                    }) {
                        sessions.attempted += 1;
                    }
                }
                EntropyFileState::Skipped => {}
            }
            if matches!(
                file.state,
                EntropyFileState::Reading
                    | EntropyFileState::Analyzed
                    | EntropyFileState::Candidate
                    | EntropyFileState::Failed
                    | EntropyFileState::TimedOut
                    | EntropyFileState::Refused
            ) || (file.state == EntropyFileState::Cancelled
                && self.events.iter().any(|event| {
                    event.file_path.as_deref() == Some(file.path.as_str())
                        && event.state == EntropyFileState::Reading
                }))
            {
                reached_paths.insert(file.path.clone());
            }
            outputs.findings = outputs
                .findings
                .saturating_add(u32::try_from(file.observations.len()).unwrap_or(u32::MAX));
            outputs.hypotheses = outputs
                .hypotheses
                .saturating_add(u32::try_from(file.hypotheses.len()).unwrap_or(u32::MAX));
            output_refs.extend(
                file.observations
                    .iter()
                    .map(|observation| observation.observation_ref.clone()),
            );
            for source in file
                .observations
                .iter()
                .flat_map(|observation| observation.source_refs.iter())
                .chain(
                    file.hypotheses
                        .iter()
                        .flat_map(|hypothesis| hypothesis.causal_links.iter())
                        .flat_map(|link| link.source_refs.iter()),
                )
            {
                reached_paths.insert(source.path.clone());
                if source.path != file.path {
                    contextual_paths.insert(source.path.clone());
                }
            }
            output_refs.extend(
                file.hypotheses
                    .iter()
                    .map(|hypothesis| hypothesis.hypothesis_ref.clone()),
            );
        }
        outputs.limitations = u32::try_from(self.limitations.len()).unwrap_or(u32::MAX);
        outputs.duplicates = self.duplicate_terminal_events;
        outputs.rejected_malformed = u32::try_from(
            self.limitations
                .iter()
                .filter(|limitation| {
                    matches!(
                        limitation.class,
                        EntropyLimitationClass::RequestSchemaFailure
                            | EntropyLimitationClass::InvalidOutput
                    )
                })
                .count(),
        )
        .unwrap_or(u32::MAX);
        output_refs.sort();
        output_refs.dedup();
        let mut failure_refs = self
            .limitations
            .iter()
            .map(|limitation| limitation.reason_ref.clone())
            .collect::<Vec<_>>();
        failure_refs.extend(
            self.tool_facts
                .iter()
                .filter_map(|fact| fact.reason_ref.clone()),
        );
        failure_refs.extend(self.cleanup.reason_ref.iter().cloned());
        failure_refs.sort();
        failure_refs.dedup();

        let mut tools = EntropyToolCounts {
            requested: u32::try_from(self.tool_facts.len()).unwrap_or(u32::MAX),
            ..EntropyToolCounts::default()
        };
        for fact in &self.tool_facts {
            match fact.state {
                EntropyToolState::Requested => {}
                EntropyToolState::Available => tools.available += 1,
                EntropyToolState::Unavailable => tools.unavailable += 1,
                EntropyToolState::Denied => tools.denied += 1,
                EntropyToolState::TimedOut => tools.timed_out += 1,
                EntropyToolState::Failed => tools.failed += 1,
            }
        }
        let eligible_focal_units = u32::try_from(
            self.manifest
                .files
                .iter()
                .filter(|file| file.eligibility == EntropyFileEligibility::Eligible)
                .count(),
        )
        .unwrap_or(u32::MAX);
        let reached = u32::try_from(reached_paths.len()).unwrap_or(u32::MAX);
        let dependency_trees_reached = u32::try_from(
            self.manifest
                .dependencies
                .iter()
                .filter(|dependency| {
                    reached_paths.iter().any(|path| {
                        path == &dependency.path
                            || path
                                .strip_prefix(&dependency.path)
                                .is_some_and(|suffix| suffix.starts_with('/'))
                    })
                })
                .count(),
        )
        .unwrap_or(u32::MAX);
        let source = EntropySourceCounts {
            eligible_focal_units,
            focal_used: sessions.attempted,
            contextual_read: u32::try_from(contextual_paths.len()).unwrap_or(u32::MAX),
            reached,
            excluded: u32::try_from(self.manifest.files.len())
                .unwrap_or(u32::MAX)
                .saturating_sub(eligible_focal_units),
            skipped: self.counts().skipped,
            oversized: u32::try_from(
                self.manifest
                    .files
                    .iter()
                    .filter(|file| file.eligibility == EntropyFileEligibility::Oversized)
                    .count(),
            )
            .unwrap_or(u32::MAX),
            never_reached: eligible_focal_units.saturating_sub(sessions.attempted),
            dependency_trees_total: u32::try_from(self.manifest.dependencies.len())
                .unwrap_or(u32::MAX),
            dependency_trees_reached,
            manifest_generation: 1,
        };
        let all_settled = sessions.queued == 0
            && self
                .files
                .iter()
                .all(|file| file.state != EntropyFileState::Reading);
        let unsuccessful = sessions.failed + sessions.timed_out + sessions.refused;
        let outcome = if !all_settled || self.phase == EntropyRunPhase::Ready {
            EntropyAccountingOutcome::Active
        } else if sessions.cancelled > 0 {
            EntropyAccountingOutcome::Cancelled
        } else if self.cleanup.state != EntropyCleanupState::Observed {
            EntropyAccountingOutcome::RecoveryRequired
        } else if sessions.attempted > 0 && unsuccessful == sessions.attempted {
            if output_refs.is_empty() {
                EntropyAccountingOutcome::Failed
            } else {
                EntropyAccountingOutcome::FailedWithPartialOutput
            }
        } else if unsuccessful > 0 && !output_refs.is_empty() {
            EntropyAccountingOutcome::FailedWithPartialOutput
        } else if unsuccessful > 0
            || !failure_refs.is_empty()
            || tools.available != tools.requested
            || sessions.attempted != eligible_focal_units
        {
            EntropyAccountingOutcome::CompletedIncomplete
        } else {
            EntropyAccountingOutcome::Completed
        };
        let mut summary = EntropyRunSummary {
            schema: ENTROPY_RUN_SUMMARY_SCHEMA_V1.into(),
            run_ref: self.binding.run_ref.clone(),
            outcome,
            source,
            sessions,
            tools,
            outputs,
            elapsed_milliseconds: self.summary.elapsed_milliseconds.clone(),
            total_tokens: self.summary.total_tokens.clone(),
            cost_micros: self.summary.cost_micros.clone(),
            network_bytes: self.summary.network_bytes.clone(),
            cleanup: self.cleanup.clone(),
            output_refs,
            failure_refs,
            canonical_digest: String::new(),
        };
        summary.canonical_digest = summary.computed_digest()?;
        summary.validate()?;
        Ok(summary)
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
        assert!(!first.is_incomplete());
    }

    fn source_inspection(
        manifest: &EntropyManifest,
        dirty_excluded_paths: Vec<String>,
    ) -> EntropySourceInspection {
        EntropySourceInspection::from_manifest(
            manifest,
            EntropySourceInspectionInput {
                inspection_ref: "inspection.entropy.coldcard.fixture".into(),
                generation: 1,
                observed_revision: REVISION.into(),
                top_level_tree: "7abc9a4c680b5623fc8a64f70555dd2d3802e488".into(),
                focal_paths: vec!["rng.c".into()],
                reached_paths: vec!["rng.c".into()],
                required_generated_input_paths: Vec::new(),
                missing_generated_input_paths: Vec::new(),
                required_excluded_paths: Vec::new(),
                dirty_excluded_paths,
                observed_at: "2026-08-03T20:00:00Z".into(),
            },
        )
        .expect("source inspection")
    }

    #[test]
    fn mechanical_source_inspection_separates_path_classes_and_gates_qualified_misses() {
        let root = tempfile::tempdir().expect("temporary repository");
        fs::write(root.path().join("rng.c"), "int rng_get(void);\n").expect("C source");
        fs::write(root.path().join("consumer.c"), "void make_seed(void);\n")
            .expect("neighbor source");
        let manifest = build_manifest(root.path(), Vec::new());
        let complete = source_inspection(&manifest, Vec::new());

        assert_eq!(complete.state, EntropySourceInspectionState::Complete);
        assert_eq!(complete.focal_paths, vec!["rng.c"]);
        assert_eq!(complete.contextual_paths, vec!["consumer.c"]);
        assert_eq!(complete.reached_paths, vec!["rng.c"]);
        assert_eq!(complete.not_reached_paths, vec!["consumer.c"]);
        assert!(complete.qualified_miss_eligible());

        let dirty = source_inspection(&manifest, vec!["rng.c".into()]);
        assert_eq!(dirty.state, EntropySourceInspectionState::Incomplete);
        assert!(!dirty.qualified_miss_eligible());
        assert!(
            dirty
                .reason_refs
                .contains(&"source.dirty_bytes.excluded".into())
        );

        let stale = complete
            .mark_stale(2, "2026-08-03T20:01:00Z".into())
            .expect("stale generation");
        assert_eq!(stale.state, EntropySourceInspectionState::Stale);
        assert!(!stale.qualified_miss_eligible());
    }

    #[test]
    fn missing_dependency_exclusion_and_missing_generated_input_forbid_completion() {
        let root = tempfile::tempdir().expect("temporary repository");
        fs::write(root.path().join("rng.c"), "int rng_get(void);\n").expect("C source");
        fs::write(root.path().join("README.md"), "excluded\n").expect("excluded source");
        let manifest = build_manifest(
            root.path(),
            vec![EntropyDependencyBinding {
                path: "external/libngu".into(),
                expected_revision: Some("537519a829259622ea6b0334fbafd6cae852852f".into()),
                observed_revision: None,
                availability: EntropyDependencyAvailability::Missing,
                materialization_error: Some("transport unavailable".into()),
            }],
        );
        let mut input = EntropySourceInspectionInput {
            inspection_ref: "inspection.entropy.coldcard.incomplete".into(),
            generation: 1,
            observed_revision: REVISION.into(),
            top_level_tree: "7abc9a4c680b5623fc8a64f70555dd2d3802e488".into(),
            focal_paths: vec!["rng.c".into()],
            reached_paths: Vec::new(),
            required_generated_input_paths: vec!["build/generated.c".into()],
            missing_generated_input_paths: vec!["build/generated.c".into()],
            required_excluded_paths: vec!["README.md".into()],
            dirty_excluded_paths: Vec::new(),
            observed_at: "2026-08-03T20:00:00Z".into(),
        };
        let inspection =
            EntropySourceInspection::from_manifest(&manifest, input.clone()).expect("inspection");
        assert_eq!(inspection.state, EntropySourceInspectionState::Incomplete);
        assert_eq!(inspection.dependency_paths, vec!["external/libngu"]);
        assert_eq!(inspection.excluded_paths, vec!["README.md"]);
        assert!(!inspection.qualified_miss_eligible());

        input.missing_generated_input_paths = vec!["undeclared.c".into()];
        assert!(EntropySourceInspection::from_manifest(&manifest, input).is_err());
    }

    #[test]
    fn coldcard_dependency_ab_and_wrong_revision_preserve_mechanical_truth() {
        let root = tempfile::tempdir().expect("temporary repository");
        fs::write(root.path().join("rng.c"), "int rng_get(void);\n").expect("C source");
        let dependency_revision = "537519a829259622ea6b0334fbafd6cae852852f";
        let dependency = |availability, observed_revision: Option<&str>| EntropyDependencyBinding {
            path: "external/libngu".into(),
            expected_revision: Some(dependency_revision.into()),
            observed_revision: observed_revision.map(str::to_string),
            availability,
            materialization_error: (availability != EntropyDependencyAvailability::Available)
                .then(|| "dependency delivery is incomplete".into()),
        };

        let missing = build_manifest(
            root.path(),
            vec![dependency(EntropyDependencyAvailability::Missing, None)],
        );
        let missing_inspection = source_inspection(&missing, Vec::new());
        assert_eq!(
            missing_inspection.state,
            EntropySourceInspectionState::Incomplete
        );
        assert!(!missing_inspection.qualified_miss_eligible());

        let available = build_manifest(
            root.path(),
            vec![dependency(
                EntropyDependencyAvailability::Available,
                Some(dependency_revision),
            )],
        );
        let available_inspection = source_inspection(&available, Vec::new());
        assert_eq!(
            available_inspection.state,
            EntropySourceInspectionState::Complete
        );
        assert!(available_inspection.qualified_miss_eligible());
        assert_eq!(
            available_inspection.dependency_facts[0]
                .observed_revision
                .as_deref(),
            Some(dependency_revision)
        );

        let wrong = build_manifest(
            root.path(),
            vec![dependency(
                EntropyDependencyAvailability::WrongRevision,
                Some("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"),
            )],
        );
        let wrong_inspection = source_inspection(&wrong, Vec::new());
        assert_eq!(
            wrong_inspection.state,
            EntropySourceInspectionState::Incomplete
        );
        assert!(
            wrong_inspection
                .reason_refs
                .contains(&"source.dependency.wrong_revision".into())
        );
    }

    #[test]
    fn oversized_source_and_wrong_top_level_revision_forbid_qualified_misses() {
        let root = tempfile::tempdir().expect("temporary repository");
        fs::write(root.path().join("rng.c"), vec![b'x'; 2_048]).expect("oversized source");
        let manifest = build_manifest(root.path(), Vec::new());
        let oversized = source_inspection(&manifest, Vec::new());
        assert_eq!(oversized.oversized_paths, vec!["rng.c"]);
        assert_eq!(oversized.state, EntropySourceInspectionState::Incomplete);
        assert!(!oversized.qualified_miss_eligible());

        let small_root = tempfile::tempdir().expect("temporary repository");
        fs::write(small_root.path().join("rng.c"), "int rng_get(void);\n").expect("source");
        let manifest = build_manifest(small_root.path(), Vec::new());
        let mut input = EntropySourceInspectionInput {
            inspection_ref: "inspection.entropy.coldcard.wrong-head".into(),
            generation: 1,
            observed_revision: "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".into(),
            top_level_tree: "7abc9a4c680b5623fc8a64f70555dd2d3802e488".into(),
            focal_paths: vec!["rng.c".into()],
            reached_paths: Vec::new(),
            required_generated_input_paths: Vec::new(),
            missing_generated_input_paths: Vec::new(),
            required_excluded_paths: Vec::new(),
            dirty_excluded_paths: Vec::new(),
            observed_at: "2026-08-03T20:00:00Z".into(),
        };
        let wrong_head =
            EntropySourceInspection::from_manifest(&manifest, input.clone()).expect("inspection");
        assert_eq!(wrong_head.state, EntropySourceInspectionState::Incomplete);
        assert!(
            wrong_head
                .reason_refs
                .contains(&"source.top_level.wrong_revision".into())
        );
        input.observed_revision = REVISION.into();
        assert!(
            EntropySourceInspection::from_manifest(&manifest, input)
                .expect("correct head")
                .qualified_miss_eligible()
        );
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
                materialization_error: Some("network transport failed".into()),
            }],
        );
        let mut run = EntropyRunProjection::new(run_binding(&manifest), manifest)
            .expect("run should preserve incomplete input");
        run.observe_all_tools_available().expect("tool inventory");
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
        run.observe_cleanup(
            "receipt.entropy.failed.cleanup".into(),
            "2026-08-02T18:20:03Z".into(),
        )
        .expect("cleanup truth");

        assert_eq!(run.phase, EntropyRunPhase::Failed);
        assert_eq!(run.summary.outcome, EntropyAccountingOutcome::Failed);
        assert_eq!(run.counts().failed, 1);
        assert!(
            run.limitations
                .iter()
                .any(|limitation| { limitation.message.contains("network transport failed") })
        );
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
    fn ordinary_clean_completion_requires_the_matching_complete_inspection() {
        let root = tempfile::tempdir().expect("temporary repository");
        fs::write(root.path().join("rng.c"), "int rng_get(void);\n").expect("C source");
        let manifest = build_manifest(root.path(), Vec::new());
        let complete_inspection = source_inspection(&manifest, Vec::new());
        let dirty_inspection = source_inspection(&manifest, vec!["rng.c".into()]);
        let mut run = EntropyRunProjection::new(run_binding(&manifest), manifest)
            .expect("complete source run");
        run.observe_all_tools_available().expect("tool inventory");
        let task = run
            .start_next_file("2026-08-02T18:20:01Z".into())
            .expect("start file")
            .expect("file task");
        run.apply_output(
            EntropyFileAnalysisOutput {
                schema: ENTROPY_FILE_OUTPUT_SCHEMA_V1.into(),
                run_ref: task.run_ref,
                file_path: task.file_path,
                observations: Vec::new(),
                hypotheses: Vec::new(),
                limitations: Vec::new(),
            },
            "2026-08-02T18:20:02Z".into(),
        )
        .expect("clean file result");
        assert_eq!(run.phase, EntropyRunPhase::AwaitingCleanup);
        assert!(!run.qualified_miss_eligible(&complete_inspection));
        run.observe_cleanup(
            "receipt.entropy.clean.cleanup".into(),
            "2026-08-02T18:20:03Z".into(),
        )
        .expect("cleanup truth");

        assert_eq!(run.phase, EntropyRunPhase::Completed);
        assert!(run.qualified_miss_eligible(&complete_inspection));
        assert!(!run.qualified_miss_eligible(&dirty_inspection));
    }

    #[test]
    fn total_session_failure_cannot_become_success_and_replay_does_not_double_count() {
        let root = tempfile::tempdir().expect("temporary repository");
        fs::write(root.path().join("a.c"), "int a;\n").expect("first source");
        fs::write(root.path().join("b.c"), "int b;\n").expect("second source");
        let manifest = build_manifest(root.path(), Vec::new());
        let mut run = EntropyRunProjection::new(run_binding(&manifest), manifest).expect("run");
        run.observe_all_tools_available().expect("tool inventory");

        for sequence in 1..=2 {
            let task = run
                .start_next_file(format!("2026-08-02T18:21:0{sequence}Z"))
                .expect("start")
                .expect("file task");
            let limitation = EntropyLimitation {
                class: EntropyLimitationClass::ModelFailure,
                reason_ref: format!("limitation.entropy.session.{sequence}.failed"),
                message: "model session failed".into(),
                file_path: Some(task.file_path),
            };
            run.fail_reading_file(limitation.clone(), format!("2026-08-02T18:21:1{sequence}Z"))
                .expect("failure");
            let event_count = run.events.len();
            run.fail_reading_file(limitation, format!("2026-08-02T18:21:2{sequence}Z"))
                .expect("exact replay is idempotent");
            assert_eq!(run.events.len(), event_count);
        }
        assert_eq!(run.phase, EntropyRunPhase::AwaitingCleanup);
        assert_eq!(
            run.summary.outcome,
            EntropyAccountingOutcome::RecoveryRequired
        );
        run.observe_cleanup(
            "receipt.entropy.total-failure.cleanup".into(),
            "2026-08-02T18:21:30Z".into(),
        )
        .expect("cleanup");
        assert_eq!(run.phase, EntropyRunPhase::Failed);
        assert_eq!(run.summary.sessions.attempted, 2);
        assert_eq!(run.summary.sessions.settled, 2);
        assert_eq!(run.summary.sessions.failed, 2);
        assert_eq!(run.summary.outputs.duplicates, 2);
        assert_eq!(run.summary.outputs.findings, 0);
        assert!(!run.summary.qualified_miss_eligible());
        run.validate().expect("canonical terminal accounting");
    }

    #[test]
    fn partial_session_failure_retains_output_and_exact_denominators() {
        let root = tempfile::tempdir().expect("temporary repository");
        fs::write(root.path().join("a.c"), "int a;\n").expect("first source");
        fs::write(root.path().join("b.c"), "int b;\n").expect("second source");
        let manifest = build_manifest(root.path(), Vec::new());
        let mut run = EntropyRunProjection::new(run_binding(&manifest), manifest).expect("run");
        run.observe_all_tools_available().expect("tool inventory");
        let first = run
            .start_next_file("2026-08-02T18:22:01Z".into())
            .expect("start")
            .expect("first task");
        let output = EntropyFileAnalysisOutput {
            schema: ENTROPY_FILE_OUTPUT_SCHEMA_V1.into(),
            run_ref: first.run_ref,
            file_path: first.file_path.clone(),
            observations: vec![EntropyObservation {
                observation_ref: "observation.entropy.partial.retained".into(),
                title: "Retained source observation".into(),
                analyzed_file: first.file_path.clone(),
                symbols: vec!["a".into()],
                suspected_mechanism: "A source fact remains inspectable".into(),
                secret_consumers: vec!["fixture".into()],
                source_refs: vec![ForensicSourceCitation {
                    source_ref: "source.entropy.partial.retained".into(),
                    commit: REVISION.into(),
                    path: first.file_path,
                    start_line: 1,
                    end_line: 1,
                    symbol: Some("a".into()),
                }],
                confidence_boundary: "Source observation only".into(),
            }],
            hypotheses: Vec::new(),
            limitations: Vec::new(),
        };
        run.apply_output(output.clone(), "2026-08-02T18:22:02Z".into())
            .expect("successful output");
        let events_after_output = run.events.len();
        run.apply_output(output, "2026-08-02T18:22:03Z".into())
            .expect("output replay");
        assert_eq!(run.events.len(), events_after_output);
        assert_eq!(run.summary.outputs.duplicates, 1);
        let before_reordered = run.summary.clone();
        assert!(
            run.apply_output(
                EntropyFileAnalysisOutput {
                    schema: ENTROPY_FILE_OUTPUT_SCHEMA_V1.into(),
                    run_ref: run.binding.run_ref.clone(),
                    file_path: "b.c".into(),
                    observations: Vec::new(),
                    hypotheses: Vec::new(),
                    limitations: Vec::new(),
                },
                "2026-08-02T18:22:03Z".into(),
            )
            .is_err()
        );
        assert_eq!(run.summary, before_reordered);
        let second = run
            .start_next_file("2026-08-02T18:22:04Z".into())
            .expect("start")
            .expect("second task");
        run.fail_reading_file(
            EntropyLimitation {
                class: EntropyLimitationClass::SessionTimedOut,
                reason_ref: "limitation.entropy.session.timeout".into(),
                message: "second session timed out".into(),
                file_path: Some(second.file_path),
            },
            "2026-08-02T18:22:05Z".into(),
        )
        .expect("timeout");
        run.observe_cleanup(
            "receipt.entropy.partial.cleanup".into(),
            "2026-08-02T18:22:06Z".into(),
        )
        .expect("cleanup");
        assert_eq!(run.phase, EntropyRunPhase::FailedWithPartialOutput);
        assert_eq!(run.summary.sessions.attempted, 2);
        assert_eq!(run.summary.sessions.settled, 2);
        assert_eq!(run.summary.sessions.timed_out, 1);
        assert_eq!(run.summary.outputs.findings, 1);
        assert_eq!(
            run.summary.output_refs,
            vec!["observation.entropy.partial.retained"]
        );
        run.validate().expect("partial output remains canonical");
    }

    #[test]
    fn tool_outcomes_remain_distinct_and_restore_before_cleanup_is_recoverable() {
        let root = tempfile::tempdir().expect("temporary repository");
        fs::write(root.path().join("rng.c"), "int rng(void);\n").expect("source");
        let manifest = build_manifest(root.path(), Vec::new());
        let mut binding = run_binding(&manifest);
        binding.tool_surface_refs = vec![
            "tool.entropy.unavailable".into(),
            "tool.entropy.denied".into(),
            "tool.entropy.timeout".into(),
        ];
        let mut run = EntropyRunProjection::new(binding, manifest).expect("run");
        run.observe_tool(
            "tool.entropy.unavailable",
            EntropyToolState::Unavailable,
            Some("tool.entropy.reason.unavailable".into()),
        )
        .expect("unavailable");
        run.observe_tool(
            "tool.entropy.denied",
            EntropyToolState::Denied,
            Some("tool.entropy.reason.denied".into()),
        )
        .expect("denied");
        run.observe_tool(
            "tool.entropy.timeout",
            EntropyToolState::TimedOut,
            Some("tool.entropy.reason.timeout".into()),
        )
        .expect("timed out");
        let task = run
            .start_next_file("2026-08-02T18:23:01Z".into())
            .expect("start")
            .expect("task");
        run.apply_output(
            EntropyFileAnalysisOutput {
                schema: ENTROPY_FILE_OUTPUT_SCHEMA_V1.into(),
                run_ref: task.run_ref,
                file_path: task.file_path,
                observations: Vec::new(),
                hypotheses: Vec::new(),
                limitations: Vec::new(),
            },
            "2026-08-02T18:23:02Z".into(),
        )
        .expect("file output");
        run.record_usage(
            EntropyUsageValue {
                value: Some(1_234),
                exactness: EntropyRunUsageExactness::Exact,
            },
            EntropyUsageValue::unavailable(),
            EntropyUsageValue::unavailable(),
            EntropyUsageValue::unavailable(),
        )
        .expect("mixed exact and unavailable usage truth");
        let encoded = serde_json::to_string(&run).expect("persist recoverable run");
        let mut restored: EntropyRunProjection =
            serde_json::from_str(&encoded).expect("restore recoverable run");
        restored.validate().expect("recoverable projection");
        assert_eq!(restored.phase, EntropyRunPhase::AwaitingCleanup);
        assert_eq!(
            restored.summary.outcome,
            EntropyAccountingOutcome::RecoveryRequired
        );
        assert_eq!(restored.summary.tools.unavailable, 1);
        assert_eq!(restored.summary.tools.denied, 1);
        assert_eq!(restored.summary.tools.timed_out, 1);
        assert_eq!(restored.summary.total_tokens.value, None);
        assert_eq!(restored.summary.elapsed_milliseconds.value, Some(1_234));
        restored
            .fail_cleanup(
                "cleanup.entropy.fixture.failed".into(),
                "2026-08-02T18:23:03Z".into(),
            )
            .expect("cleanup failure remains recoverable");
        assert_eq!(restored.cleanup.state, EntropyCleanupState::Failed);
        assert!(
            restored
                .summary
                .failure_refs
                .contains(&"cleanup.entropy.fixture.failed".into())
        );
        restored
            .observe_cleanup(
                "receipt.entropy.tool-matrix.cleanup".into(),
                "2026-08-02T18:23:04Z".into(),
            )
            .expect("cleanup");
        assert_eq!(
            restored.summary.outcome,
            EntropyAccountingOutcome::CompletedIncomplete
        );
        assert!(!restored.summary.qualified_miss_eligible());
    }

    #[test]
    fn legacy_terminal_restore_migrates_to_recovery_instead_of_false_success() {
        let root = tempfile::tempdir().expect("temporary repository");
        fs::write(root.path().join("rng.c"), "int rng(void);\n").expect("source");
        let manifest = build_manifest(root.path(), Vec::new());
        let mut run = EntropyRunProjection::new(run_binding(&manifest), manifest).expect("run");
        let task = run
            .start_next_file("2026-08-02T18:24:01Z".into())
            .expect("start")
            .expect("task");
        run.apply_output(
            EntropyFileAnalysisOutput {
                schema: ENTROPY_FILE_OUTPUT_SCHEMA_V1.into(),
                run_ref: task.run_ref,
                file_path: task.file_path,
                observations: Vec::new(),
                hypotheses: Vec::new(),
                limitations: Vec::new(),
            },
            "2026-08-02T18:24:02Z".into(),
        )
        .expect("output");
        let mut legacy = serde_json::to_value(&run).expect("encode");
        let object = legacy.as_object_mut().expect("run object");
        object.insert(
            "phase".into(),
            serde_json::Value::String("completed".into()),
        );
        object.remove("toolFacts");
        object.remove("cleanup");
        object.remove("summary");
        object.remove("duplicateTerminalEvents");
        let mut restored: EntropyRunProjection =
            serde_json::from_value(legacy).expect("decode legacy run");
        assert!(restored.migrate_legacy_accounting().expect("migration"));
        assert_eq!(restored.phase, EntropyRunPhase::AwaitingCleanup);
        assert_eq!(
            restored.summary.outcome,
            EntropyAccountingOutcome::RecoveryRequired
        );
        assert_eq!(restored.tool_facts[0].state, EntropyToolState::Requested);
        restored.validate().expect("honest migrated restore");
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
