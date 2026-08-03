use super::{
    COLDCARD_CURRENT_COMMIT, COLDCARD_FIXED_COMMIT, COLDCARD_VULNERABLE_COMMIT, EntropyLimitation,
    EntropyLimitationClass, EntropyModelParameters, EntropyPromptSnapshot, EntropyRunPhase,
    EntropyRunProjection, ForensicsError,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use url::Url;

pub const ENTROPY_PROJECT_CATALOG_SCHEMA_V1: &str = "openagents.omega.entropy-project-catalog.v1";
pub const ENTROPY_CAMPAIGN_SCHEMA_V1: &str = "openagents.omega.entropy-campaign.v1";
pub const ENTROPY_CAMPAIGN_COMPARISON_SCHEMA_V1: &str =
    "openagents.omega.entropy-campaign-comparison.v1";
pub const WALLET_ENTROPY_CATALOG_REF_V2: &str = "catalog.omega.wallet-entropy.2026-08-03.v2";
pub const ENTROPY_FILE_SELECTION_POLICY_REF_V1: &str =
    "policy.omega.entropy.supported-source-files.v1";
pub const COLDCARD_CURRENT_PRODUCT_REF: &str = "product.coldcard.mk4-q1";
pub const COLDCARD_VULNERABLE_PRODUCT_REF: &str = "product.coldcard.mk4.historical-vulnerable";
pub const COLDCARD_FIXED_PRODUCT_REF: &str = "product.coldcard.mk4.immediate-fixed-control";
pub const ENTROPY_GENERIC_ANALYSIS_PROFILE_REF: &str =
    "scan-profile-ref://entropy/repository-source-v1";

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EntropyProjectSourceAvailability {
    Available,
    SourceUnavailable,
    InputIncomplete,
}

impl EntropyProjectSourceAvailability {
    pub const fn label(self) -> &'static str {
        match self {
            Self::Available => "Source available",
            Self::SourceUnavailable => "Source unavailable",
            Self::InputIncomplete => "Input incomplete",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct EntropyProjectRecord {
    pub product_ref: String,
    pub product_name: String,
    pub repository_ref: Option<String>,
    pub repository_url: Option<String>,
    pub pinned_revision: Option<String>,
    pub license_or_access_status: String,
    pub dependency_policy_ref: String,
    pub source_availability: EntropyProjectSourceAvailability,
    pub limitation_refs: Vec<String>,
}

impl EntropyProjectRecord {
    pub fn analysis_profile_ref(&self) -> &'static str {
        match self.product_ref.as_str() {
            COLDCARD_VULNERABLE_PRODUCT_REF => "scan-profile-ref://coldcard/complete-vulnerable-v1",
            COLDCARD_FIXED_PRODUCT_REF => "scan-profile-ref://coldcard/fixed-v1",
            COLDCARD_CURRENT_PRODUCT_REF => "scan-profile-ref://coldcard/current-post-fix-v1",
            _ => ENTROPY_GENERIC_ANALYSIS_PROFILE_REF,
        }
    }

    pub fn validate(&self) -> Result<(), ForensicsError> {
        validate_ref("product", &self.product_ref)?;
        validate_ref("dependency policy", &self.dependency_policy_ref)?;
        if self.product_name.trim().is_empty() || self.product_name.len() > 128 {
            return invalid("product names must contain 1 to 128 bytes");
        }
        if self.license_or_access_status.trim().is_empty()
            || self.license_or_access_status.len() > 256
        {
            return invalid("license or access status must be present and bounded");
        }
        let mut limitations = BTreeSet::new();
        for limitation_ref in &self.limitation_refs {
            validate_ref("project limitation", limitation_ref)?;
            if !limitations.insert(limitation_ref) {
                return invalid("project limitation refs must be unique");
            }
        }
        match self.source_availability {
            EntropyProjectSourceAvailability::Available => {
                validate_source_identity(self)?;
            }
            EntropyProjectSourceAvailability::SourceUnavailable => {
                if self.repository_ref.is_some()
                    || self.repository_url.is_some()
                    || self.pinned_revision.is_some()
                    || self.limitation_refs.is_empty()
                {
                    return invalid(
                        "source-unavailable products must omit source identity and name a limitation",
                    );
                }
            }
            EntropyProjectSourceAvailability::InputIncomplete => {
                if self.limitation_refs.is_empty() {
                    return invalid("input-incomplete products must name a limitation");
                }
                if self.repository_ref.is_some()
                    || self.repository_url.is_some()
                    || self.pinned_revision.is_some()
                {
                    validate_source_identity(self)?;
                }
            }
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct EntropyProjectCatalog {
    pub schema: String,
    pub catalog_ref: String,
    pub observed_at: String,
    pub source_observation_ref: String,
    pub projects: Vec<EntropyProjectRecord>,
    pub canonical_digest: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct EntropyProjectCatalogDigestInput<'a> {
    schema: &'a str,
    catalog_ref: &'a str,
    observed_at: &'a str,
    source_observation_ref: &'a str,
    projects: &'a [EntropyProjectRecord],
}

impl EntropyProjectCatalog {
    pub fn new(
        catalog_ref: String,
        observed_at: String,
        source_observation_ref: String,
        projects: Vec<EntropyProjectRecord>,
    ) -> Result<Self, ForensicsError> {
        let mut catalog = Self {
            schema: ENTROPY_PROJECT_CATALOG_SCHEMA_V1.into(),
            catalog_ref,
            observed_at,
            source_observation_ref,
            projects,
            canonical_digest: String::new(),
        };
        catalog.canonical_digest = catalog.digest()?;
        catalog.validate()?;
        Ok(catalog)
    }

    pub fn wallet_entropy_v2() -> Result<Self, ForensicsError> {
        let available = |product_ref: &str,
                         product_name: &str,
                         repository_ref: &str,
                         repository_url: &str,
                         revision: &str,
                         license: &str| EntropyProjectRecord {
            product_ref: product_ref.into(),
            product_name: product_name.into(),
            repository_ref: Some(repository_ref.into()),
            repository_url: Some(repository_url.into()),
            pinned_revision: Some(revision.into()),
            license_or_access_status: license.into(),
            dependency_policy_ref: "policy.omega.dependencies.pinned-recursive.v1".into(),
            source_availability: EntropyProjectSourceAvailability::Available,
            limitation_refs: Vec::new(),
        };
        Self::new(
            WALLET_ENTROPY_CATALOG_REF_V2.into(),
            "2026-08-03T12:00:00Z".into(),
            "observation.openagents.wallet-comparison-plus-coldcard-controls.v2".into(),
            vec![
                available(
                    COLDCARD_CURRENT_PRODUCT_REF,
                    "Coldcard MK4 / Q1 · current post-fix",
                    "repository.github.coldcard.firmware",
                    "https://github.com/Coldcard/firmware",
                    COLDCARD_CURRENT_COMMIT,
                    "Public source; repository license is not machine-readable",
                ),
                available(
                    COLDCARD_VULNERABLE_PRODUCT_REF,
                    "Coldcard MK4 · historical vulnerable",
                    "repository.github.coldcard.firmware",
                    "https://github.com/Coldcard/firmware",
                    COLDCARD_VULNERABLE_COMMIT,
                    "Public historical source; repository license is not machine-readable",
                ),
                available(
                    COLDCARD_FIXED_PRODUCT_REF,
                    "Coldcard MK4 · immediate fixed control",
                    "repository.github.coldcard.firmware",
                    "https://github.com/Coldcard/firmware",
                    COLDCARD_FIXED_COMMIT,
                    "Public source; repository license is not machine-readable",
                ),
                available(
                    "product.trezor.model-one-model-t",
                    "Trezor Model One / Model T",
                    "repository.github.trezor.trezor-firmware",
                    "https://github.com/trezor/trezor-firmware",
                    "ded1c141b643b57ef3a4f9a71d7a2fb2ced083a6",
                    "Public source; repository license is not machine-readable",
                ),
                available(
                    "product.seedsigner",
                    "SeedSigner",
                    "repository.github.seedsigner.seedsigner",
                    "https://github.com/SeedSigner/seedsigner",
                    "1fb2956322ea978428a6a96b955baa93e965c877",
                    "MIT",
                ),
                available(
                    "product.sparrow",
                    "Sparrow",
                    "repository.github.sparrowwallet.sparrow",
                    "https://github.com/sparrowwallet/sparrow",
                    "2b9c3eb7370180207095774bbe14138a716b6b7f",
                    "Apache-2.0",
                ),
                available(
                    "product.trezor.safe-3-5-7",
                    "Trezor Safe 3 / 5 / 7",
                    "repository.github.trezor.trezor-firmware",
                    "https://github.com/trezor/trezor-firmware",
                    "ded1c141b643b57ef3a4f9a71d7a2fb2ced083a6",
                    "Public source; repository license is not machine-readable",
                ),
                available(
                    "product.bitbox02",
                    "BitBox02",
                    "repository.github.bitboxswiss.bitbox02-firmware",
                    "https://github.com/BitBoxSwiss/bitbox02-firmware",
                    "c838d7fd80190a02531ba30e4a904240e4485e1f",
                    "Apache-2.0",
                ),
                available(
                    "product.opendime",
                    "Opendime",
                    "repository.github.coinkite.opendime",
                    "https://github.com/coinkite/opendime",
                    "93a31397591c009bb069818017a4761a7092b398",
                    "Public source; no repository license metadata",
                ),
                available(
                    "product.bitkey",
                    "Bitkey",
                    "repository.github.proto-at-block.bitkey",
                    "https://github.com/proto-at-block/bitkey",
                    "cf16705543d0c66ff982635733d380944cc2677d",
                    "Public source; repository license is not machine-readable",
                ),
                available(
                    "product.bluewallet",
                    "BlueWallet",
                    "repository.github.bluewallet.bluewallet",
                    "https://github.com/BlueWallet/BlueWallet",
                    "635cfc2c1517c61695b1ceea07eb147455a9494f",
                    "MIT",
                ),
                available(
                    "product.phoenix",
                    "Phoenix",
                    "repository.github.acinq.phoenix",
                    "https://github.com/ACINQ/phoenix",
                    "3749b0817d60b0a17a97d21338e2d9038efb2d9e",
                    "Apache-2.0",
                ),
                available(
                    "product.blockstream.jade",
                    "Blockstream Jade",
                    "repository.github.blockstream.jade",
                    "https://github.com/Blockstream/Jade",
                    "cfce08ced9f58ed0244e516f1d7f0c61a82889ee",
                    "MIT",
                ),
                EntropyProjectRecord {
                    product_ref: "product.ledger".into(),
                    product_name: "Ledger".into(),
                    repository_ref: Some("repository.github.ledgerhq.ledger-secure-sdk".into()),
                    repository_url: Some("https://github.com/LedgerHQ/ledger-secure-sdk".into()),
                    pinned_revision: Some("63e94edeb12edd36e66ff5cf5efa998fafb2d7ee".into()),
                    license_or_access_status:
                        "Apache-2.0 public SDK; complete device source not supplied".into(),
                    dependency_policy_ref: "policy.omega.dependencies.pinned-recursive.v1".into(),
                    source_availability: EntropyProjectSourceAvailability::InputIncomplete,
                    limitation_refs: vec![
                        "limitation.entropy.complete_product_source_unavailable".into(),
                    ],
                },
                available(
                    "product.specter-diy",
                    "SpecterDIY",
                    "repository.github.cryptoadvance.specter-diy",
                    "https://github.com/cryptoadvance/specter-diy",
                    "eb8397d2b53bfe43cec0571f8efa235aa352d8ec",
                    "MIT",
                ),
                available(
                    "product.electrum.android",
                    "Electrum for Android",
                    "repository.github.spesmilo.electrum",
                    "https://github.com/spesmilo/electrum",
                    "ebe20010111f00dad2d8bd52ac2a6030d3b26d8b",
                    "MIT",
                ),
                available(
                    "product.samourai-wallet",
                    "Samourai Wallet (discontinued)",
                    "repository.github.samourai-wallet.samourai-wallet-android",
                    "https://github.com/Samourai-Wallet/samourai-wallet-android",
                    "c71f21a631bbc69db2bd411f2905c7f141d1523f",
                    "Unlicense; discontinued product source snapshot",
                ),
            ],
        )
    }

    pub fn validate(&self) -> Result<(), ForensicsError> {
        if self.schema != ENTROPY_PROJECT_CATALOG_SCHEMA_V1 {
            return Err(ForensicsError::InvalidSchema);
        }
        validate_ref("catalog", &self.catalog_ref)?;
        validate_ref("source observation", &self.source_observation_ref)?;
        if self.observed_at.trim().is_empty() || self.observed_at.len() > 64 {
            return invalid("catalog observation time must be present and bounded");
        }
        if self.projects.is_empty() || self.projects.len() > 64 {
            return invalid("project catalogs must contain 1 to 64 products");
        }
        let mut product_refs = BTreeSet::new();
        for project in &self.projects {
            project.validate()?;
            if !product_refs.insert(&project.product_ref) {
                return invalid("catalog product refs must be unique");
            }
        }
        if self.canonical_digest != self.digest()? {
            return invalid("catalog digest does not match its immutable records");
        }
        Ok(())
    }

    fn digest(&self) -> Result<String, ForensicsError> {
        let input = EntropyProjectCatalogDigestInput {
            schema: &self.schema,
            catalog_ref: &self.catalog_ref,
            observed_at: &self.observed_at,
            source_observation_ref: &self.source_observation_ref,
            projects: &self.projects,
        };
        let encoded = serde_json::to_vec(&input).map_err(|error| {
            ForensicsError::InvalidEntropyRun(format!("cannot digest project catalog: {error}"))
        })?;
        Ok(format!("sha256:{:x}", Sha256::digest(encoded)))
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct EntropyCampaignBinding {
    pub campaign_ref: String,
    pub catalog_ref: String,
    pub catalog_digest: String,
    pub prompt_snapshot: EntropyPromptSnapshot,
    pub prompt_digest: String,
    pub model_route_ref: String,
    pub model_parameters: EntropyModelParameters,
    pub tool_surface_refs: Vec<String>,
    pub file_selection_policy_ref: String,
    pub started_at: String,
}

impl EntropyCampaignBinding {
    pub fn validate(&self) -> Result<(), ForensicsError> {
        validate_ref("campaign", &self.campaign_ref)?;
        validate_ref("catalog", &self.catalog_ref)?;
        validate_digest("catalog", &self.catalog_digest)?;
        self.prompt_snapshot.validate()?;
        validate_digest("prompt", &self.prompt_digest)?;
        if self.prompt_digest != self.prompt_snapshot.canonical_digest {
            return invalid("campaign prompt digest must match its frozen snapshot");
        }
        validate_ref("model route", &self.model_route_ref)?;
        self.model_parameters.validate()?;
        validate_ref("file-selection policy", &self.file_selection_policy_ref)?;
        if self.started_at.trim().is_empty() || self.started_at.len() > 64 {
            return invalid("campaign start time must be present and bounded");
        }
        if self.tool_surface_refs.is_empty() || self.tool_surface_refs.len() > 32 {
            return invalid("campaign tool surface must contain 1 to 32 refs");
        }
        let mut tools = BTreeSet::new();
        for tool_ref in &self.tool_surface_refs {
            validate_ref("tool", tool_ref)?;
            if !tools.insert(tool_ref) {
                return invalid("campaign tool refs must be unique");
            }
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EntropyCampaignPhase {
    Ready,
    Running,
    Paused,
    Completed,
    CompletedWithLimitations,
    Cancelled,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EntropyCampaignProjectPhase {
    Queued,
    Running,
    Completed,
    CompletedWithLimitations,
    ProviderFailed,
    SourceFailed,
    SourceUnavailable,
    InputIncomplete,
    Cancelled,
}

impl EntropyCampaignProjectPhase {
    pub const fn label(self) -> &'static str {
        match self {
            Self::Queued => "Queued",
            Self::Running => "Running",
            Self::Completed => "Completed",
            Self::CompletedWithLimitations => "Completed with limits",
            Self::ProviderFailed => "Provider failed",
            Self::SourceFailed => "Source failed",
            Self::SourceUnavailable => "Source unavailable",
            Self::InputIncomplete => "Input incomplete",
            Self::Cancelled => "Cancelled",
        }
    }

    pub const fn is_terminal(self) -> bool {
        !matches!(self, Self::Queued | Self::Running)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EntropyUsageExactness {
    Exact,
    Unavailable,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct EntropyCampaignUsage {
    pub total_tokens: Option<u64>,
    pub exactness: EntropyUsageExactness,
}

impl EntropyCampaignUsage {
    pub const fn unavailable() -> Self {
        Self {
            total_tokens: None,
            exactness: EntropyUsageExactness::Unavailable,
        }
    }

    fn validate(&self) -> Result<(), ForensicsError> {
        if (self.total_tokens.is_some()) != (self.exactness == EntropyUsageExactness::Exact) {
            return invalid("campaign usage value and exactness must agree");
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct EntropyCampaignProject {
    pub product: EntropyProjectRecord,
    pub phase: EntropyCampaignProjectPhase,
    pub run: Option<EntropyRunProjection>,
    pub limitation_refs: Vec<String>,
    pub elapsed_milliseconds: Option<u64>,
    pub usage: EntropyCampaignUsage,
}

impl EntropyCampaignProject {
    pub fn files_analyzed(&self) -> u32 {
        self.run.as_ref().map_or(0, |run| {
            let counts = run.counts();
            counts.analyzed + counts.candidate
        })
    }

    pub fn candidate_count(&self) -> usize {
        self.run.as_ref().map_or(0, |run| {
            run.files
                .iter()
                .map(|file| file.observations.len() + file.hypotheses.len())
                .sum()
        })
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct EntropyCampaignProjection {
    pub schema: String,
    pub binding: EntropyCampaignBinding,
    pub catalog: EntropyProjectCatalog,
    pub phase: EntropyCampaignPhase,
    pub projects: Vec<EntropyCampaignProject>,
}

impl EntropyCampaignProjection {
    pub fn new(
        binding: EntropyCampaignBinding,
        catalog: EntropyProjectCatalog,
    ) -> Result<Self, ForensicsError> {
        binding.validate()?;
        catalog.validate()?;
        if binding.catalog_ref != catalog.catalog_ref
            || binding.catalog_digest != catalog.canonical_digest
        {
            return invalid("campaign binding does not match the immutable catalog");
        }
        let projects = catalog
            .projects
            .iter()
            .cloned()
            .map(|product| {
                let phase = match product.source_availability {
                    EntropyProjectSourceAvailability::Available => {
                        EntropyCampaignProjectPhase::Queued
                    }
                    EntropyProjectSourceAvailability::SourceUnavailable => {
                        EntropyCampaignProjectPhase::SourceUnavailable
                    }
                    EntropyProjectSourceAvailability::InputIncomplete => {
                        EntropyCampaignProjectPhase::InputIncomplete
                    }
                };
                EntropyCampaignProject {
                    limitation_refs: product.limitation_refs.clone(),
                    product,
                    phase,
                    run: None,
                    elapsed_milliseconds: None,
                    usage: EntropyCampaignUsage::unavailable(),
                }
            })
            .collect();
        let campaign = Self {
            schema: ENTROPY_CAMPAIGN_SCHEMA_V1.into(),
            binding,
            catalog,
            phase: EntropyCampaignPhase::Ready,
            projects,
        };
        campaign.validate()?;
        Ok(campaign)
    }

    pub fn validate(&self) -> Result<(), ForensicsError> {
        if self.schema != ENTROPY_CAMPAIGN_SCHEMA_V1 {
            return Err(ForensicsError::InvalidSchema);
        }
        self.binding.validate()?;
        self.catalog.validate()?;
        if self.binding.catalog_ref != self.catalog.catalog_ref
            || self.binding.catalog_digest != self.catalog.canonical_digest
            || self.projects.len() != self.catalog.projects.len()
        {
            return invalid("campaign projection drifted from its catalog binding");
        }
        let mut running = 0usize;
        for (project, catalog_product) in self.projects.iter().zip(&self.catalog.projects) {
            if &project.product != catalog_product {
                return invalid("campaign project order drifted from the catalog");
            }
            project.usage.validate()?;
            if project.phase == EntropyCampaignProjectPhase::Running {
                running += 1;
            }
            match project.product.source_availability {
                EntropyProjectSourceAvailability::SourceUnavailable
                    if project.phase != EntropyCampaignProjectPhase::SourceUnavailable
                        || project.run.is_some() =>
                {
                    return invalid("source-unavailable products cannot become analyzed results");
                }
                EntropyProjectSourceAvailability::InputIncomplete
                    if project.phase != EntropyCampaignProjectPhase::InputIncomplete
                        || project.run.is_some() =>
                {
                    return invalid("input-incomplete products cannot become clean results");
                }
                _ => {}
            }
            if let Some(run) = &project.run {
                run.validate()?;
                validate_run_binding(&self.binding, &project.product, run)?;
            }
            if matches!(
                project.phase,
                EntropyCampaignProjectPhase::Completed
                    | EntropyCampaignProjectPhase::CompletedWithLimitations
            ) && project.run.is_none()
            {
                return invalid("completed project rows require their isolated run");
            }
        }
        if running > 1 {
            return invalid("the campaign runner is sequential and may run one project at a time");
        }
        Ok(())
    }

    pub fn start(&mut self) -> Result<(), ForensicsError> {
        if self.phase != EntropyCampaignPhase::Ready {
            return invalid("only a ready campaign can start");
        }
        self.phase = EntropyCampaignPhase::Running;
        Ok(())
    }

    pub fn pause(&mut self) -> Result<(), ForensicsError> {
        if self.phase != EntropyCampaignPhase::Running {
            return invalid("only a running campaign can pause");
        }
        self.phase = EntropyCampaignPhase::Paused;
        Ok(())
    }

    pub fn resume(&mut self) -> Result<(), ForensicsError> {
        if self.phase != EntropyCampaignPhase::Paused {
            return invalid("only a paused campaign can resume");
        }
        self.phase = EntropyCampaignPhase::Running;
        Ok(())
    }

    pub fn cancel(&mut self, observed_at: String) -> Result<(), ForensicsError> {
        if matches!(
            self.phase,
            EntropyCampaignPhase::Completed
                | EntropyCampaignPhase::CompletedWithLimitations
                | EntropyCampaignPhase::Cancelled
        ) {
            return invalid("a terminal campaign cannot be cancelled again");
        }
        for project in &mut self.projects {
            if project.phase == EntropyCampaignProjectPhase::Queued {
                project.phase = EntropyCampaignProjectPhase::Cancelled;
                project
                    .limitation_refs
                    .push("limitation.entropy.campaign_cancelled".into());
            } else if project.phase == EntropyCampaignProjectPhase::Running {
                if let Some(run) = project.run.as_mut() {
                    run.cancel(observed_at.clone())?;
                }
                project.phase = EntropyCampaignProjectPhase::Cancelled;
                project
                    .limitation_refs
                    .push("limitation.entropy.campaign_cancelled".into());
            }
        }
        self.phase = EntropyCampaignPhase::Cancelled;
        Ok(())
    }

    pub fn start_next_project(&mut self) -> Result<Option<EntropyProjectRecord>, ForensicsError> {
        if self.phase != EntropyCampaignPhase::Running {
            return Ok(None);
        }
        if self
            .projects
            .iter()
            .any(|project| project.phase == EntropyCampaignProjectPhase::Running)
        {
            return invalid("finish the active project before starting another");
        }
        let Some(project) = self
            .projects
            .iter_mut()
            .find(|project| project.phase == EntropyCampaignProjectPhase::Queued)
        else {
            self.finish_if_idle();
            return Ok(None);
        };
        project.phase = EntropyCampaignProjectPhase::Running;
        Ok(Some(project.product.clone()))
    }

    pub fn update_project_run(
        &mut self,
        product_ref: &str,
        run: EntropyRunProjection,
        elapsed_milliseconds: Option<u64>,
        usage: EntropyCampaignUsage,
    ) -> Result<(), ForensicsError> {
        usage.validate()?;
        let binding = self.binding.clone();
        let project = self.project_mut(product_ref)?;
        if project.phase != EntropyCampaignProjectPhase::Running {
            return invalid("only the running project can receive a run projection");
        }
        validate_run_binding(&binding, &project.product, &run)?;
        project.phase = match run.phase {
            EntropyRunPhase::Completed if run.limitations.is_empty() => {
                EntropyCampaignProjectPhase::Completed
            }
            EntropyRunPhase::Completed | EntropyRunPhase::CompletedWithLimitations => {
                EntropyCampaignProjectPhase::CompletedWithLimitations
            }
            EntropyRunPhase::Cancelled => EntropyCampaignProjectPhase::Cancelled,
            EntropyRunPhase::Ready
            | EntropyRunPhase::Running
            | EntropyRunPhase::CancelRequested => EntropyCampaignProjectPhase::Running,
        };
        project.limitation_refs.extend(
            run.limitations
                .iter()
                .map(|limitation| limitation.reason_ref.clone()),
        );
        project.limitation_refs.sort();
        project.limitation_refs.dedup();
        project.elapsed_milliseconds = elapsed_milliseconds;
        project.usage = usage;
        project.run = Some(run);
        self.finish_if_idle();
        Ok(())
    }

    pub fn record_provider_failure(
        &mut self,
        product_ref: &str,
        message: String,
        elapsed_milliseconds: Option<u64>,
    ) -> Result<(), ForensicsError> {
        let project = self.project_mut(product_ref)?;
        if project.phase != EntropyCampaignProjectPhase::Running {
            return invalid("only the running project can record provider failure");
        }
        EntropyLimitation {
            class: EntropyLimitationClass::ModelFailure,
            reason_ref: "limitation.entropy.provider_failure".into(),
            message,
            file_path: None,
        }
        .validate()?;
        project.phase = EntropyCampaignProjectPhase::ProviderFailed;
        project
            .limitation_refs
            .push("limitation.entropy.provider_failure".into());
        project.elapsed_milliseconds = elapsed_milliseconds;
        self.finish_if_idle();
        Ok(())
    }

    pub fn record_source_failure(
        &mut self,
        product_ref: &str,
        message: String,
        elapsed_milliseconds: Option<u64>,
    ) -> Result<(), ForensicsError> {
        let project = self.project_mut(product_ref)?;
        if project.phase != EntropyCampaignProjectPhase::Running {
            return invalid("only the running project can record source failure");
        }
        EntropyLimitation {
            class: EntropyLimitationClass::SourceUnavailable,
            reason_ref: "limitation.entropy.source_materialization_failed".into(),
            message,
            file_path: None,
        }
        .validate()?;
        project.phase = EntropyCampaignProjectPhase::SourceFailed;
        project
            .limitation_refs
            .push("limitation.entropy.source_materialization_failed".into());
        project.elapsed_milliseconds = elapsed_milliseconds;
        self.finish_if_idle();
        Ok(())
    }

    pub fn project(&self, product_ref: &str) -> Option<&EntropyCampaignProject> {
        self.projects
            .iter()
            .find(|project| project.product.product_ref == product_ref)
    }

    fn project_mut(
        &mut self,
        product_ref: &str,
    ) -> Result<&mut EntropyCampaignProject, ForensicsError> {
        self.projects
            .iter_mut()
            .find(|project| project.product.product_ref == product_ref)
            .ok_or_else(|| {
                ForensicsError::InvalidEntropyRun(
                    "campaign product is not in the immutable catalog".into(),
                )
            })
    }

    fn finish_if_idle(&mut self) {
        if self.phase != EntropyCampaignPhase::Running
            || self.projects.iter().any(|project| {
                matches!(
                    project.phase,
                    EntropyCampaignProjectPhase::Queued | EntropyCampaignProjectPhase::Running
                )
            })
        {
            return;
        }
        self.phase = if self.projects.iter().any(|project| {
            project.phase != EntropyCampaignProjectPhase::Completed
                || !project.limitation_refs.is_empty()
        }) {
            EntropyCampaignPhase::CompletedWithLimitations
        } else {
            EntropyCampaignPhase::Completed
        };
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct EntropyCampaignProjectComparison {
    pub product_ref: String,
    pub source_availability: EntropyProjectSourceAvailability,
    pub source_revision: Option<String>,
    pub run_a_ref: Option<String>,
    pub run_b_ref: Option<String>,
    pub gained: u32,
    pub lost: u32,
    pub changed: u32,
    pub unchanged: u32,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct EntropyCampaignComparison {
    pub schema: String,
    pub campaign_a_ref: String,
    pub campaign_b_ref: String,
    pub catalog_digest: String,
    pub prompt_a_digest: String,
    pub prompt_b_digest: String,
    pub projects: Vec<EntropyCampaignProjectComparison>,
}

impl EntropyCampaignComparison {
    pub fn between(
        campaign_a: &EntropyCampaignProjection,
        campaign_b: &EntropyCampaignProjection,
    ) -> Result<Self, ForensicsError> {
        campaign_a.validate()?;
        campaign_b.validate()?;
        if campaign_a.binding.catalog_digest != campaign_b.binding.catalog_digest
            || campaign_a.catalog.projects != campaign_b.catalog.projects
        {
            return invalid("campaign comparison requires the exact same project catalog");
        }
        let projects = campaign_a
            .projects
            .iter()
            .zip(&campaign_b.projects)
            .map(|(project_a, project_b)| {
                let candidates_a = candidate_digests(project_a.run.as_ref())?;
                let candidates_b = candidate_digests(project_b.run.as_ref())?;
                let mut gained = 0u32;
                let mut lost = 0u32;
                let mut changed = 0u32;
                let mut unchanged = 0u32;
                for (candidate_ref, digest_a) in &candidates_a {
                    match candidates_b.get(candidate_ref) {
                        Some(digest_b) if digest_a == digest_b => unchanged += 1,
                        Some(_) => changed += 1,
                        None => lost += 1,
                    }
                }
                for candidate_ref in candidates_b.keys() {
                    if !candidates_a.contains_key(candidate_ref) {
                        gained += 1;
                    }
                }
                Ok(EntropyCampaignProjectComparison {
                    product_ref: project_a.product.product_ref.clone(),
                    source_availability: project_a.product.source_availability,
                    source_revision: project_a.product.pinned_revision.clone(),
                    run_a_ref: project_a
                        .run
                        .as_ref()
                        .map(|run| run.binding.run_ref.clone()),
                    run_b_ref: project_b
                        .run
                        .as_ref()
                        .map(|run| run.binding.run_ref.clone()),
                    gained,
                    lost,
                    changed,
                    unchanged,
                })
            })
            .collect::<Result<Vec<_>, ForensicsError>>()?;
        Ok(Self {
            schema: ENTROPY_CAMPAIGN_COMPARISON_SCHEMA_V1.into(),
            campaign_a_ref: campaign_a.binding.campaign_ref.clone(),
            campaign_b_ref: campaign_b.binding.campaign_ref.clone(),
            catalog_digest: campaign_a.binding.catalog_digest.clone(),
            prompt_a_digest: campaign_a.binding.prompt_digest.clone(),
            prompt_b_digest: campaign_b.binding.prompt_digest.clone(),
            projects,
        })
    }
}

fn validate_run_binding(
    campaign: &EntropyCampaignBinding,
    product: &EntropyProjectRecord,
    run: &EntropyRunProjection,
) -> Result<(), ForensicsError> {
    let expected_repository_ref = product.repository_ref.as_deref().ok_or_else(|| {
        ForensicsError::InvalidEntropyRun("campaign product has no repository ref".into())
    })?;
    let expected_revision = product.pinned_revision.as_deref().ok_or_else(|| {
        ForensicsError::InvalidEntropyRun("campaign product has no pinned revision".into())
    })?;
    if run.binding.repository.repository_ref != expected_repository_ref
        || run.binding.repository.revision != expected_revision
        || run.binding.prompt_snapshot != campaign.prompt_snapshot
        || run.binding.prompt_digest != campaign.prompt_digest
        || run.binding.model_route_ref != campaign.model_route_ref
        || run.binding.model_parameters != campaign.model_parameters
        || run.binding.tool_surface_refs != campaign.tool_surface_refs
    {
        return invalid(
            "project run must preserve the campaign source, prompt, model, and tool binding",
        );
    }
    Ok(())
}

fn candidate_digests(
    run: Option<&EntropyRunProjection>,
) -> Result<BTreeMap<String, String>, ForensicsError> {
    let mut candidates = BTreeMap::new();
    let Some(run) = run else {
        return Ok(candidates);
    };
    for file in &run.files {
        for observation in &file.observations {
            let encoded = serde_json::to_vec(observation).map_err(|error| {
                ForensicsError::InvalidEntropyRun(format!(
                    "cannot compare entropy observation: {error}"
                ))
            })?;
            let candidate_ref = observation.observation_ref.clone();
            if candidates
                .insert(
                    candidate_ref.clone(),
                    format!("sha256:{:x}", Sha256::digest(encoded)),
                )
                .is_some()
            {
                return invalid(format!(
                    "candidate reference {candidate_ref} is not unique within its project run"
                ));
            }
        }
        for hypothesis in &file.hypotheses {
            let encoded = serde_json::to_vec(hypothesis).map_err(|error| {
                ForensicsError::InvalidEntropyRun(format!(
                    "cannot compare entropy hypothesis: {error}"
                ))
            })?;
            let candidate_ref = hypothesis.hypothesis_ref.clone();
            if candidates
                .insert(
                    candidate_ref.clone(),
                    format!("sha256:{:x}", Sha256::digest(encoded)),
                )
                .is_some()
            {
                return invalid(format!(
                    "candidate reference {candidate_ref} is not unique within its project run"
                ));
            }
        }
    }
    Ok(candidates)
}

fn validate_source_identity(project: &EntropyProjectRecord) -> Result<(), ForensicsError> {
    let repository_ref = project.repository_ref.as_deref().ok_or_else(|| {
        ForensicsError::InvalidEntropyRun("eligible products require a repository ref".into())
    })?;
    let repository_url = project.repository_url.as_deref().ok_or_else(|| {
        ForensicsError::InvalidEntropyRun("eligible products require a repository URL".into())
    })?;
    let revision = project.pinned_revision.as_deref().ok_or_else(|| {
        ForensicsError::InvalidEntropyRun("eligible products require a pinned revision".into())
    })?;
    validate_ref("repository", repository_ref)?;
    validate_revision(revision)?;
    let parsed = Url::parse(repository_url).map_err(|error| {
        ForensicsError::InvalidEntropyRun(format!("invalid repository URL: {error}"))
    })?;
    if parsed.scheme() != "https" || parsed.host_str() != Some("github.com") {
        return invalid("campaign repository URLs must be public GitHub HTTPS URLs");
    }
    Ok(())
}

fn validate_ref(label: &str, value: &str) -> Result<(), ForensicsError> {
    if value.trim().is_empty()
        || value.len() > 512
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"._:/-".contains(&byte))
    {
        return invalid(format!("{label} ref is invalid"));
    }
    Ok(())
}

fn validate_revision(value: &str) -> Result<(), ForensicsError> {
    if value.len() != 40 || !value.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return invalid("project revision must be a full 40-character commit");
    }
    Ok(())
}

fn validate_digest(label: &str, value: &str) -> Result<(), ForensicsError> {
    let Some(hex) = value.strip_prefix("sha256:") else {
        return invalid(format!("{label} digest must be sha256"));
    };
    if hex.len() != 64 || !hex.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return invalid(format!("{label} digest must contain 64 hexadecimal bytes"));
    }
    Ok(())
}

fn invalid<T>(message: impl Into<String>) -> Result<T, ForensicsError> {
    Err(ForensicsError::InvalidEntropyRun(message.into()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        ENTROPY_FILE_OUTPUT_SCHEMA_V1, EntropyFileAnalysisOutput, EntropyManifest,
        EntropyObservation, EntropyRepositoryBinding, EntropyRunBinding,
    };
    use std::fs;

    fn snapshot(prompt_ref: &str, text: &str) -> EntropyPromptSnapshot {
        EntropyPromptSnapshot::new(
            prompt_ref.into(),
            None,
            None,
            text.into(),
            "2026-08-02T08:15:00Z".into(),
        )
        .expect("prompt fixture")
    }

    fn binding(
        catalog: &EntropyProjectCatalog,
        prompt: EntropyPromptSnapshot,
    ) -> EntropyCampaignBinding {
        EntropyCampaignBinding {
            campaign_ref: format!("campaign.{}", prompt.prompt_ref),
            catalog_ref: catalog.catalog_ref.clone(),
            catalog_digest: catalog.canonical_digest.clone(),
            prompt_digest: prompt.canonical_digest.clone(),
            prompt_snapshot: prompt,
            model_route_ref: "model-route.fixture.kimi".into(),
            model_parameters: EntropyModelParameters {
                temperature_millis: 0,
                thinking_allowed: true,
                reasoning_effort_ref: None,
            },
            tool_surface_refs: vec!["tool.omega.project.read".into()],
            file_selection_policy_ref: ENTROPY_FILE_SELECTION_POLICY_REF_V1.into(),
            started_at: "2026-08-02T08:15:00Z".into(),
        }
    }

    fn small_catalog() -> EntropyProjectCatalog {
        let all = EntropyProjectCatalog::wallet_entropy_v2().expect("wallet catalog");
        EntropyProjectCatalog::new(
            "catalog.fixture.entropy.v1".into(),
            all.observed_at,
            all.source_observation_ref,
            vec![
                all.projects[0].clone(),
                EntropyProjectRecord {
                    product_ref: "product.fixture.closed".into(),
                    product_name: "Closed fixture".into(),
                    repository_ref: None,
                    repository_url: None,
                    pinned_revision: None,
                    license_or_access_status: "No public source supplied".into(),
                    dependency_policy_ref: "policy.omega.dependencies.pinned-recursive.v1".into(),
                    source_availability: EntropyProjectSourceAvailability::SourceUnavailable,
                    limitation_refs: vec!["limitation.entropy.source_unavailable".into()],
                },
                all.projects
                    .iter()
                    .find(|project| project.product_ref == "product.ledger")
                    .expect("Ledger fixture")
                    .clone(),
            ],
        )
        .expect("small catalog")
    }

    fn run_for(
        product: &EntropyProjectRecord,
        campaign: &EntropyCampaignBinding,
        candidate: bool,
    ) -> EntropyRunProjection {
        let directory = tempfile::tempdir().expect("temporary source");
        fs::write(
            directory.path().join("rng.c"),
            "int rng(void) { return 4; }\n",
        )
        .expect("source fixture");
        let repository = EntropyRepositoryBinding {
            repository_ref: product.repository_ref.clone().expect("repository ref"),
            display_name: product.product_name.clone(),
            revision: product.pinned_revision.clone().expect("revision"),
        };
        let manifest = EntropyManifest::build(
            directory.path(),
            format!("manifest.{}", product.product_ref),
            repository.clone(),
            Vec::new(),
            512 * 1_024,
        )
        .expect("manifest");
        let mut run = EntropyRunProjection::new(
            EntropyRunBinding {
                run_ref: format!("run.{}", product.product_ref),
                repository,
                manifest_ref: manifest.manifest_ref.clone(),
                manifest_digest: manifest.canonical_digest.clone(),
                prompt_snapshot: campaign.prompt_snapshot.clone(),
                prompt_digest: campaign.prompt_digest.clone(),
                model_route_ref: campaign.model_route_ref.clone(),
                model_parameters: campaign.model_parameters.clone(),
                tool_surface_refs: campaign.tool_surface_refs.clone(),
                started_at: "2026-08-02T08:15:01Z".into(),
            },
            manifest,
        )
        .expect("run");
        let task = run
            .start_next_file("2026-08-02T08:15:02Z".into())
            .expect("start file")
            .expect("file task");
        let observations = if candidate {
            vec![EntropyObservation {
                observation_ref: "observation.fixture.rng".into(),
                title: "Deterministic entropy source".into(),
                analyzed_file: task.file_path.clone(),
                symbols: vec!["rng".into()],
                suspected_mechanism: "A constant reaches the entropy API".into(),
                secret_consumers: vec!["wallet seed".into()],
                source_refs: vec![crate::ForensicSourceCitation {
                    source_ref: "source.fixture.rng".into(),
                    commit: product.pinned_revision.clone().expect("revision"),
                    path: task.file_path.clone(),
                    start_line: 1,
                    end_line: 1,
                    symbol: Some("rng".into()),
                }],
                confidence_boundary: "Source observation only".into(),
            }]
        } else {
            Vec::new()
        };
        run.apply_output(
            EntropyFileAnalysisOutput {
                schema: ENTROPY_FILE_OUTPUT_SCHEMA_V1.into(),
                run_ref: run.binding.run_ref.clone(),
                file_path: task.file_path,
                observations,
                hypotheses: Vec::new(),
                limitations: Vec::new(),
            },
            "2026-08-02T08:15:03Z".into(),
        )
        .expect("output");
        assert!(
            run.start_next_file("2026-08-02T08:15:04Z".into())
                .expect("finish")
                .is_none()
        );
        run
    }

    #[test]
    fn wallet_catalog_preserves_intake_rows_and_coldcard_differential_controls_without_grades() {
        let catalog = EntropyProjectCatalog::wallet_entropy_v2().expect("wallet catalog");
        assert_eq!(catalog.projects.len(), 17);
        assert_eq!(
            catalog.projects[0].product_name,
            "Coldcard MK4 / Q1 · current post-fix"
        );
        assert_eq!(
            catalog
                .projects
                .iter()
                .find(|project| project.product_ref == "product.ledger")
                .expect("Ledger row")
                .source_availability,
            EntropyProjectSourceAvailability::InputIncomplete
        );
        assert_eq!(
            catalog.projects[1].pinned_revision.as_deref(),
            Some(COLDCARD_VULNERABLE_COMMIT)
        );
        assert_eq!(
            catalog.projects[2].pinned_revision.as_deref(),
            Some(COLDCARD_FIXED_COMMIT)
        );
        assert_ne!(COLDCARD_VULNERABLE_COMMIT, crate::COLDCARD_VULNERABLE_TREE);
        assert!(catalog.projects.iter().all(|project| {
            !project.license_or_access_status.contains("green")
                && !project.license_or_access_status.contains("yellow")
                && !project.license_or_access_status.contains("red")
        }));
    }

    #[test]
    fn unavailable_and_incomplete_sources_never_become_clean_runs() {
        let catalog = small_catalog();
        let prompt = snapshot("prompt.fixture.sources", "Inspect entropy");
        let campaign =
            EntropyCampaignProjection::new(binding(&catalog, prompt), catalog).expect("campaign");
        assert_eq!(
            campaign.projects[1].phase,
            EntropyCampaignProjectPhase::SourceUnavailable
        );
        assert_eq!(
            campaign.projects[2].phase,
            EntropyCampaignProjectPhase::InputIncomplete
        );
        campaign.validate().expect("valid limitations");
    }

    #[test]
    fn campaign_retains_clean_candidate_provider_failure_and_mixed_terminal_states() {
        let all = EntropyProjectCatalog::wallet_entropy_v2().expect("wallet catalog");
        let catalog = EntropyProjectCatalog::new(
            "catalog.fixture.entropy.mixed.v1".into(),
            all.observed_at,
            all.source_observation_ref,
            vec![
                all.projects
                    .iter()
                    .find(|project| project.product_ref == COLDCARD_CURRENT_PRODUCT_REF)
                    .expect("Coldcard fixture")
                    .clone(),
                all.projects
                    .iter()
                    .find(|project| project.product_ref == "product.trezor.model-one-model-t")
                    .expect("Trezor fixture")
                    .clone(),
                all.projects
                    .iter()
                    .find(|project| project.product_ref == "product.seedsigner")
                    .expect("SeedSigner fixture")
                    .clone(),
                all.projects
                    .iter()
                    .find(|project| project.product_ref == "product.ledger")
                    .expect("Ledger fixture")
                    .clone(),
            ],
        )
        .expect("mixed catalog");
        let prompt = snapshot("prompt.fixture.mixed", "Inspect entropy");
        let binding = binding(&catalog, prompt);
        let mut campaign =
            EntropyCampaignProjection::new(binding.clone(), catalog).expect("campaign");
        campaign.start().expect("start");
        let coldcard = campaign
            .start_next_project()
            .expect("next")
            .expect("coldcard");
        let candidate_run = run_for(&coldcard, &binding, true);
        campaign
            .update_project_run(
                &coldcard.product_ref,
                candidate_run,
                Some(10),
                EntropyCampaignUsage::unavailable(),
            )
            .expect("candidate result");
        let trezor = campaign
            .start_next_project()
            .expect("next")
            .expect("trezor");
        let clean_run = run_for(&trezor, &binding, false);
        campaign
            .update_project_run(
                &trezor.product_ref,
                clean_run,
                Some(7),
                EntropyCampaignUsage::unavailable(),
            )
            .expect("clean result");
        let seedsigner = campaign
            .start_next_project()
            .expect("next")
            .expect("seedsigner");
        campaign
            .record_provider_failure(
                &seedsigner.product_ref,
                "provider unavailable".into(),
                Some(4),
            )
            .expect("provider failure");
        assert_eq!(campaign.start_next_project().expect("finish"), None);
        assert_eq!(
            campaign.phase,
            EntropyCampaignPhase::CompletedWithLimitations
        );
        assert_eq!(campaign.projects[0].candidate_count(), 1);
        assert_eq!(
            campaign.projects[1].phase,
            EntropyCampaignProjectPhase::Completed
        );
        assert_eq!(campaign.projects[1].candidate_count(), 0);
        assert_eq!(
            campaign.projects[2].phase,
            EntropyCampaignProjectPhase::ProviderFailed
        );
        campaign.validate().expect("mixed campaign");
    }

    #[test]
    fn campaign_pauses_resumes_and_cancels_without_losing_partial_rows() {
        let catalog = small_catalog();
        let prompt = snapshot("prompt.fixture.control", "Inspect entropy");
        let mut campaign =
            EntropyCampaignProjection::new(binding(&catalog, prompt), catalog).expect("campaign");
        campaign.start().expect("start");
        assert!(campaign.start_next_project().expect("next").is_some());
        campaign.pause().expect("pause");
        assert!(
            campaign
                .start_next_project()
                .expect("paused next")
                .is_none()
        );
        campaign.resume().expect("resume");
        campaign
            .cancel("2026-08-02T08:16:00Z".into())
            .expect("cancel");
        assert_eq!(campaign.phase, EntropyCampaignPhase::Cancelled);
        assert_eq!(
            campaign.projects[0].phase,
            EntropyCampaignProjectPhase::Cancelled
        );
    }

    #[test]
    fn prompt_comparison_preserves_campaign_run_and_source_identities() {
        let all = EntropyProjectCatalog::wallet_entropy_v2().expect("wallet catalog");
        let catalog = EntropyProjectCatalog::new(
            "catalog.fixture.compare.v1".into(),
            all.observed_at,
            all.source_observation_ref,
            vec![all.projects[0].clone()],
        )
        .expect("catalog");
        let binding_a = binding(&catalog, snapshot("prompt.fixture.a", "Inspect entropy A"));
        let binding_b = binding(&catalog, snapshot("prompt.fixture.b", "Inspect entropy B"));
        let mut campaign_a =
            EntropyCampaignProjection::new(binding_a.clone(), catalog.clone()).expect("campaign A");
        let mut campaign_b =
            EntropyCampaignProjection::new(binding_b.clone(), catalog).expect("campaign B");
        campaign_a.start().expect("start A");
        let product_a = campaign_a
            .start_next_project()
            .expect("next A")
            .expect("product A");
        campaign_a
            .update_project_run(
                &product_a.product_ref,
                run_for(&product_a, &binding_a, true),
                Some(3),
                EntropyCampaignUsage::unavailable(),
            )
            .expect("run A");
        campaign_b.start().expect("start B");
        let product_b = campaign_b
            .start_next_project()
            .expect("next B")
            .expect("product B");
        campaign_b
            .update_project_run(
                &product_b.product_ref,
                run_for(&product_b, &binding_b, false),
                Some(3),
                EntropyCampaignUsage::unavailable(),
            )
            .expect("run B");
        let comparison =
            EntropyCampaignComparison::between(&campaign_a, &campaign_b).expect("comparison");
        assert_eq!(comparison.campaign_a_ref, binding_a.campaign_ref);
        assert_eq!(comparison.campaign_b_ref, binding_b.campaign_ref);
        assert_eq!(comparison.projects[0].lost, 1);
        assert_eq!(
            comparison.projects[0].source_revision,
            product_a.pinned_revision
        );
        assert!(comparison.projects[0].run_a_ref.is_some());
        assert!(comparison.projects[0].run_b_ref.is_some());
    }
}
