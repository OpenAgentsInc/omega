//! Durable per-thread work-surface selection for cold restart (#131).
//!
//! Stores only the versioned logical `PersistedSelection` record — never
//! terminal processes, project trees, or GPUI entity handles.

use std::path::{Path, PathBuf};

use anyhow::{Context as _, Result, bail};
use omega_workbench_state::PersistedSelection;
use serde::{Deserialize, Serialize};

pub const WORKBENCH_SURFACE_STORE_SCHEMA: u32 = 1;
const STORE_DIR: &str = "workbench-surface-v1";

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkbenchSurfaceRecord {
    pub schema_version: u32,
    pub selection: PersistedSelection,
}

impl WorkbenchSurfaceRecord {
    pub fn new(selection: PersistedSelection) -> Self {
        Self {
            schema_version: WORKBENCH_SURFACE_STORE_SCHEMA,
            selection,
        }
    }

    pub fn validate(&self) -> Result<()> {
        if self.schema_version != WORKBENCH_SURFACE_STORE_SCHEMA {
            bail!(
                "unsupported workbench surface schema version {}",
                self.schema_version
            );
        }
        self.selection
            .validate()
            .map_err(|error| anyhow::anyhow!("{error}"))?;
        Ok(())
    }
}

pub fn store_dir(data_dir: &Path) -> PathBuf {
    data_dir.join(STORE_DIR)
}

pub fn record_path(data_dir: &Path, thread_id: &str) -> PathBuf {
    // Thread ids are UUID-like; reject path separators defensively.
    let safe = thread_id
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' {
                ch
            } else {
                '_'
            }
        })
        .collect::<String>();
    store_dir(data_dir).join(format!("{safe}.json"))
}

pub fn write_selection(data_dir: &Path, selection: &PersistedSelection) -> Result<()> {
    let record = WorkbenchSurfaceRecord::new(selection.clone());
    record.validate()?;
    let dir = store_dir(data_dir);
    std::fs::create_dir_all(&dir)
        .with_context(|| format!("creating workbench surface store {}", dir.display()))?;
    let path = record_path(data_dir, &selection.thread_id);
    let bytes = serde_json::to_vec_pretty(&record).context("encoding workbench surface record")?;
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, &bytes).with_context(|| {
        format!(
            "writing temporary workbench surface record {}",
            tmp.display()
        )
    })?;
    std::fs::rename(&tmp, &path)
        .with_context(|| format!("installing workbench surface record {}", path.display()))?;
    Ok(())
}

pub fn read_selection(data_dir: &Path, thread_id: &str) -> Result<Option<PersistedSelection>> {
    let path = record_path(data_dir, thread_id);
    match std::fs::read(&path) {
        Ok(bytes) => {
            let record: WorkbenchSurfaceRecord = serde_json::from_slice(&bytes)
                .with_context(|| format!("decoding workbench surface record {}", path.display()))?;
            record.validate()?;
            Ok(Some(record.selection))
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error)
            .with_context(|| format!("reading workbench surface record {}", path.display())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use omega_workbench_state::WorkSurface;
    use tempfile::tempdir;

    #[test]
    fn round_trips_a_valid_selection_and_rejects_unknown_schema() {
        let dir = tempdir().expect("temp dir");
        let selection = PersistedSelection {
            thread_id: "thread-1".into(),
            generation: 3,
            binding: None,
            requested_surface: Some(WorkSurface::Plan),
            dock_open: true,
            revision: 7,
        };
        write_selection(dir.path(), &selection).expect("write");
        let restored = read_selection(dir.path(), "thread-1")
            .expect("read")
            .expect("present");
        assert_eq!(restored, selection);

        let path = record_path(dir.path(), "thread-1");
        let mut bad: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&path).expect("bytes")).expect("json");
        bad["schema_version"] = serde_json::json!(99);
        std::fs::write(&path, serde_json::to_vec(&bad).expect("encode")).expect("write bad");
        assert!(read_selection(dir.path(), "thread-1").is_err());
    }

    #[test]
    fn missing_record_is_none_not_error() {
        let dir = tempdir().expect("temp dir");
        assert_eq!(read_selection(dir.path(), "missing").expect("read"), None);
    }
}
