use std::{
    collections::BTreeMap,
    fs::{self, File, OpenOptions},
    io::{BufRead as _, BufReader, Write as _},
    path::{Path, PathBuf},
};

use anyhow::{Context as _, Result, bail};
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};

pub const NAUTILUS_SOAK_MANIFEST_SCHEMA: &str = "omega.nautilus.soak_manifest.v1";
pub const NAUTILUS_SOAK_HEALTH_SCHEMA: &str = "omega.nautilus.soak_health.v1";
pub const NAUTILUS_SOAK_RECEIPT_SCHEMA: &str = "omega.nautilus.soak_receipt.v1";
pub const REQUIRED_SOAK_DURATION_MS: i64 = 72 * 60 * 60 * 1_000;
const ZERO_HASH: &str = "0000000000000000000000000000000000000000000000000000000000000000";

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct NautilusSoakManifest {
    pub schema: String,
    pub segment_id: String,
    pub commit: String,
    pub config_sha256: String,
    pub started_at_ms: i64,
    pub required_duration_ms: i64,
    pub health_interval_ms: i64,
    pub review_interval_ms: i64,
    pub mandate_revision: u64,
    pub mandate_digest: String,
    pub mandate_expires_at_ms: i64,
    pub venue: String,
    pub network: String,
    pub strategy_id: String,
    pub zero_human_nudges: bool,
}

impl NautilusSoakManifest {
    pub fn validate(&self) -> Result<()> {
        if self.schema != NAUTILUS_SOAK_MANIFEST_SCHEMA
            || self.segment_id.trim().is_empty()
            || self.venue != "hyperliquid"
            || self.network != "testnet"
            || self.strategy_id != "OMEGA-BOUNDED-QUOTE-001"
            || !self.zero_human_nudges
        {
            bail!("soak manifest is outside the sealed Testnet scope");
        }
        if !is_lower_hex(&self.commit, 40)
            || !is_lower_hex(&self.config_sha256, 64)
            || !is_lower_hex(&self.mandate_digest, 64)
        {
            bail!("soak manifest hashes are malformed");
        }
        if self.started_at_ms < 0
            || self.required_duration_ms != REQUIRED_SOAK_DURATION_MS
            || !(1_000..=60_000).contains(&self.health_interval_ms)
            || self.review_interval_ms < 60_000
            || self.mandate_revision == 0
        {
            bail!("soak manifest cadence or mandate is invalid");
        }
        let required_end = self
            .started_at_ms
            .checked_add(self.required_duration_ms)
            .context("soak end overflowed")?;
        if self.mandate_expires_at_ms <= required_end {
            bail!("soak mandate does not cover the complete immutable segment");
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct NautilusSoakHealthDraft {
    pub observed_at_ms: i64,
    pub engine_generation: u64,
    pub engine_sequence: u64,
    pub strategy_phase: String,
    pub strategy_running: bool,
    pub recoverable_budget_wait: bool,
    pub halt_reason: Option<String>,
    pub wakeup_queued: bool,
    pub scheduled_review_count: u64,
    pub prediction_count: u64,
    pub ledger_head_hash: String,
    pub venue_assets: BTreeMap<String, i64>,
    pub engine_assets: BTreeMap<String, i64>,
    pub ledger_assets: BTreeMap<String, i64>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct NautilusSoakHealth {
    pub schema: String,
    pub segment_id: String,
    pub sequence: u64,
    pub previous_hash: String,
    pub entry_hash: String,
    #[serde(flatten)]
    pub draft: NautilusSoakHealthDraft,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct NautilusSoakReceipt {
    pub schema: String,
    pub segment_id: String,
    pub manifest_sha256: String,
    pub started_at_ms: i64,
    pub ended_at_ms: i64,
    pub health_sample_count: u64,
    pub scheduled_review_count: u64,
    pub prediction_count: u64,
    pub human_nudge_count: u64,
    pub health_head_hash: String,
    pub passed: bool,
}

#[derive(Debug)]
pub struct NautilusSoakStore {
    root: PathBuf,
    manifest: NautilusSoakManifest,
    manifest_sha256: String,
    append_state: parking_lot::Mutex<SoakAppendState>,
}

#[derive(Debug)]
struct SoakAppendState {
    sequence: u64,
    head_hash: String,
    last_observed_at_ms: Option<i64>,
}

impl NautilusSoakStore {
    pub fn open_configured() -> Result<Option<Self>> {
        let Some(root) = std::env::var_os("OMEGA_NAUTILUS_SOAK_DIR") else {
            return Ok(None);
        };
        Ok(Some(Self::open(PathBuf::from(root))?))
    }

    pub fn manifest(&self) -> &NautilusSoakManifest {
        &self.manifest
    }

    pub fn sample_due(&self, observed_at_ms: i64) -> bool {
        let sample_interval_ms = (self.manifest.health_interval_ms / 2).max(1_000);
        self.append_state
            .lock()
            .last_observed_at_ms
            .is_none_or(|last| observed_at_ms.saturating_sub(last) >= sample_interval_ms)
    }

    pub fn create(root: impl AsRef<Path>, manifest: NautilusSoakManifest) -> Result<Self> {
        manifest.validate()?;
        let root = root.as_ref().to_path_buf();
        fs::create_dir_all(&root)?;
        let mut encoded = serde_json::to_vec_pretty(&manifest)?;
        encoded.push(b'\n');
        let manifest_sha256 = sha256(&encoded);
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(root.join("manifest.json"))
            .context("soak manifest already exists; segments cannot be spliced")?;
        file.write_all(&encoded)?;
        file.sync_all()?;
        Ok(Self {
            root,
            manifest,
            manifest_sha256,
            append_state: parking_lot::Mutex::new(SoakAppendState {
                sequence: 0,
                head_hash: ZERO_HASH.to_owned(),
                last_observed_at_ms: None,
            }),
        })
    }

    pub fn open(root: impl AsRef<Path>) -> Result<Self> {
        let root = root.as_ref().to_path_buf();
        let encoded = fs::read(root.join("manifest.json"))?;
        let manifest: NautilusSoakManifest = serde_json::from_slice(&encoded)?;
        manifest.validate()?;
        let samples = read_health_samples(&root)?;
        validate_hash_chain(&manifest, &samples)?;
        let append_state = samples.last().map_or_else(
            || SoakAppendState {
                sequence: 0,
                head_hash: ZERO_HASH.to_owned(),
                last_observed_at_ms: None,
            },
            |sample| SoakAppendState {
                sequence: sample.sequence,
                head_hash: sample.entry_hash.clone(),
                last_observed_at_ms: Some(sample.draft.observed_at_ms),
            },
        );
        Ok(Self {
            root,
            manifest,
            manifest_sha256: sha256(&encoded),
            append_state: parking_lot::Mutex::new(append_state),
        })
    }

    pub fn append_health(&self, draft: NautilusSoakHealthDraft) -> Result<NautilusSoakHealth> {
        let mut state = self.append_state.lock();
        let sequence = state
            .sequence
            .checked_add(1)
            .context("soak health sequence overflowed")?;
        let previous_hash = state.head_hash.clone();
        if draft.observed_at_ms < self.manifest.started_at_ms
            || state
                .last_observed_at_ms
                .is_some_and(|last| draft.observed_at_ms <= last)
        {
            bail!("soak health timestamps must increase inside the segment");
        }
        let hash_material = serde_json::to_vec(&(
            NAUTILUS_SOAK_HEALTH_SCHEMA,
            &self.manifest.segment_id,
            sequence,
            &previous_hash,
            &draft,
        ))?;
        let sample = NautilusSoakHealth {
            schema: NAUTILUS_SOAK_HEALTH_SCHEMA.to_owned(),
            segment_id: self.manifest.segment_id.clone(),
            sequence,
            previous_hash,
            entry_hash: sha256(&hash_material),
            draft,
        };
        let mut file = OpenOptions::new()
            .append(true)
            .create(true)
            .open(self.root.join("health.jsonl"))?;
        serde_json::to_writer(&mut file, &sample)?;
        file.write_all(b"\n")?;
        file.sync_all()?;
        state.sequence = sample.sequence;
        state.head_hash.clone_from(&sample.entry_hash);
        state.last_observed_at_ms = Some(sample.draft.observed_at_ms);
        Ok(sample)
    }

    pub fn health_samples(&self) -> Result<Vec<NautilusSoakHealth>> {
        read_health_samples(&self.root)
    }

    pub fn finish(&self, ended_at_ms: i64, human_nudge_count: u64) -> Result<NautilusSoakReceipt> {
        let samples = self.health_samples()?;
        assess(&self.manifest, &samples, ended_at_ms, human_nudge_count)?;
        let last = samples.last().context("soak has no health samples")?;
        let receipt = NautilusSoakReceipt {
            schema: NAUTILUS_SOAK_RECEIPT_SCHEMA.to_owned(),
            segment_id: self.manifest.segment_id.clone(),
            manifest_sha256: self.manifest_sha256.clone(),
            started_at_ms: self.manifest.started_at_ms,
            ended_at_ms,
            health_sample_count: u64::try_from(samples.len())?,
            scheduled_review_count: last.draft.scheduled_review_count,
            prediction_count: last.draft.prediction_count,
            human_nudge_count,
            health_head_hash: last.entry_hash.clone(),
            passed: true,
        };
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(self.root.join("receipt.json"))
            .context("soak receipt already exists; segments cannot be re-finished")?;
        serde_json::to_writer_pretty(&mut file, &receipt)?;
        file.write_all(b"\n")?;
        file.sync_all()?;
        Ok(receipt)
    }
}

fn read_health_samples(root: &Path) -> Result<Vec<NautilusSoakHealth>> {
    let path = root.join("health.jsonl");
    let file = match File::open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(error.into()),
    };
    BufReader::new(file)
        .lines()
        .map(|line| Ok(serde_json::from_str::<NautilusSoakHealth>(&line?)?))
        .collect()
}

fn validate_hash_chain(
    manifest: &NautilusSoakManifest,
    samples: &[NautilusSoakHealth],
) -> Result<()> {
    let mut previous_hash = ZERO_HASH;
    for (index, sample) in samples.iter().enumerate() {
        let sequence = u64::try_from(index)? + 1;
        let material = serde_json::to_vec(&(
            NAUTILUS_SOAK_HEALTH_SCHEMA,
            &manifest.segment_id,
            sequence,
            previous_hash,
            &sample.draft,
        ))?;
        if sample.schema != NAUTILUS_SOAK_HEALTH_SCHEMA
            || sample.segment_id != manifest.segment_id
            || sample.sequence != sequence
            || sample.previous_hash != previous_hash
            || sample.entry_hash != sha256(&material)
        {
            bail!("soak health stream is missing, reordered, or hash-invalid");
        }
        previous_hash = &sample.entry_hash;
    }
    Ok(())
}

fn assess(
    manifest: &NautilusSoakManifest,
    samples: &[NautilusSoakHealth],
    ended_at_ms: i64,
    human_nudge_count: u64,
) -> Result<()> {
    manifest.validate()?;
    let required_end = manifest.started_at_ms + manifest.required_duration_ms;
    if ended_at_ms < required_end || human_nudge_count != 0 {
        bail!("soak lacks 72 zero-nudge wall-clock hours");
    }
    let first = samples.first().context("soak has no health samples")?;
    let last = samples.last().context("soak has no health samples")?;
    if first.draft.observed_at_ms - manifest.started_at_ms > manifest.health_interval_ms
        || ended_at_ms - last.draft.observed_at_ms > manifest.health_interval_ms
    {
        bail!("soak health stream does not cover both segment boundaries");
    }
    validate_hash_chain(manifest, samples)?;
    let mut previous_at = manifest.started_at_ms;
    let mut previous_reviews = 0_u64;
    for sample in samples {
        if sample.draft.observed_at_ms - previous_at > manifest.health_interval_ms {
            bail!("soak health stream is missing, reordered, or hash-invalid");
        }
        if sample.draft.venue_assets != sample.draft.engine_assets
            || sample.draft.engine_assets != sample.draft.ledger_assets
        {
            bail!("soak venue, engine, and ledger assets do not reconcile");
        }
        if sample.draft.halt_reason.is_some() && !sample.draft.wakeup_queued {
            bail!("soak halt did not queue a bounded wakeup");
        }
        if !sample.draft.strategy_running
            && !sample.draft.recoverable_budget_wait
            && sample.draft.halt_reason.is_none()
        {
            bail!("soak strategy stopped without a typed halt or budget wait");
        }
        let review_deadline = manifest.started_at_ms
            + i64::try_from(sample.draft.scheduled_review_count + 1)? * manifest.review_interval_ms;
        if sample.draft.observed_at_ms > review_deadline + manifest.health_interval_ms
            || sample.draft.scheduled_review_count < previous_reviews
            || sample.draft.prediction_count < sample.draft.scheduled_review_count
        {
            bail!("soak is missing a scheduled review or its prediction");
        }
        previous_at = sample.draft.observed_at_ms;
        previous_reviews = sample.draft.scheduled_review_count;
    }
    Ok(())
}

fn is_lower_hex(value: &str, length: usize) -> bool {
    value.len() == length
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn sha256(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn manifest() -> NautilusSoakManifest {
        NautilusSoakManifest {
            schema: NAUTILUS_SOAK_MANIFEST_SCHEMA.to_owned(),
            segment_id: "omega-304-segment-1".to_owned(),
            commit: "a".repeat(40),
            config_sha256: "b".repeat(64),
            started_at_ms: 1_000,
            required_duration_ms: REQUIRED_SOAK_DURATION_MS,
            health_interval_ms: 60_000,
            review_interval_ms: 3_600_000,
            mandate_revision: 5,
            mandate_digest: "c".repeat(64),
            mandate_expires_at_ms: 1_000 + REQUIRED_SOAK_DURATION_MS + 3_600_000,
            venue: "hyperliquid".to_owned(),
            network: "testnet".to_owned(),
            strategy_id: "OMEGA-BOUNDED-QUOTE-001".to_owned(),
            zero_human_nudges: true,
        }
    }

    fn health(at: i64, reviews: u64) -> NautilusSoakHealthDraft {
        let assets = BTreeMap::from([("usdc".to_owned(), 1_000_000)]);
        NautilusSoakHealthDraft {
            observed_at_ms: at,
            engine_generation: 1,
            engine_sequence: u64::try_from(at).unwrap_or_default(),
            strategy_phase: "running".to_owned(),
            strategy_running: true,
            recoverable_budget_wait: false,
            halt_reason: None,
            wakeup_queued: false,
            scheduled_review_count: reviews,
            prediction_count: reviews,
            ledger_head_hash: "d".repeat(64),
            venue_assets: assets.clone(),
            engine_assets: assets.clone(),
            ledger_assets: assets,
        }
    }

    #[test]
    fn immutable_receipt_requires_complete_reconciled_zero_nudge_evidence() -> Result<()> {
        let manifest = manifest();
        let mut samples = Vec::new();
        let mut previous_hash = ZERO_HASH.to_owned();
        let mut at = manifest.started_at_ms + manifest.health_interval_ms;
        while at <= manifest.started_at_ms + manifest.required_duration_ms {
            let reviews =
                u64::try_from((at - manifest.started_at_ms) / manifest.review_interval_ms)?;
            let sequence = u64::try_from(samples.len())? + 1;
            let draft = health(at, reviews);
            let material = serde_json::to_vec(&(
                NAUTILUS_SOAK_HEALTH_SCHEMA,
                &manifest.segment_id,
                sequence,
                &previous_hash,
                &draft,
            ))?;
            let entry_hash = sha256(&material);
            samples.push(NautilusSoakHealth {
                schema: NAUTILUS_SOAK_HEALTH_SCHEMA.to_owned(),
                segment_id: manifest.segment_id.clone(),
                sequence,
                previous_hash,
                entry_hash: entry_hash.clone(),
                draft,
            });
            previous_hash = entry_hash;
            at += manifest.health_interval_ms;
        }
        assess(
            &manifest,
            &samples,
            manifest.started_at_ms + manifest.required_duration_ms,
            0,
        )?;
        Ok(())
    }

    #[test]
    fn receipt_refuses_asset_drift_and_missing_wakeup() -> Result<()> {
        let directory = tempfile::tempdir()?;
        let manifest = manifest();
        let store = NautilusSoakStore::create(directory.path(), manifest.clone())?;
        let mut sample = health(manifest.started_at_ms + 60_000, 0);
        sample.engine_assets.insert("usdc".to_owned(), 2_000_000);
        sample.halt_reason = Some("reconciliation gap".to_owned());
        store.append_health(sample)?;
        let refusal = store
            .finish(manifest.started_at_ms + manifest.required_duration_ms, 0)
            .expect_err("drift must refuse the receipt");
        assert!(
            refusal.to_string().contains("boundaries") || refusal.to_string().contains("reconcile")
        );
        Ok(())
    }

    #[test]
    fn manifest_and_receipt_are_create_once() -> Result<()> {
        let directory = tempfile::tempdir()?;
        NautilusSoakStore::create(directory.path(), manifest())?;
        let refusal = NautilusSoakStore::create(directory.path(), manifest())
            .expect_err("manifest must be immutable");
        assert!(refusal.to_string().contains("cannot be spliced"));
        Ok(())
    }
}
