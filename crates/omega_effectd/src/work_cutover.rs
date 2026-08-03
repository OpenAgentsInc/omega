//! Reversible one-writer authority for the Omega-native Work cutover.
//!
//! Import, tests, or a rendered UI never activate this state. A caller must
//! supply the expected revision/generation and an explicit receipt reference.

use std::{fs, io::Write as _, path::Path};

use serde::{Deserialize, Serialize};
use thiserror::Error;

pub const WORK_CUTOVER_SCHEMA: &str = "openagents.omega.work-cutover.v1";

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkWriter {
    LegacyGithub,
    NativeOmega,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorkCutoverLedger {
    pub schema: String,
    pub revision: u64,
    pub generation: u64,
    pub writer: WorkWriter,
    pub source_digest: String,
    pub source_cursor: u64,
    pub native_high_watermark: u64,
    pub activation_receipt_ref: Option<String>,
    pub rollback_receipt_ref: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum WorkCutoverCommand {
    BindShadow {
        source_digest: String,
        source_cursor: u64,
    },
    ActivateNative {
        source_digest: String,
        reconciled_cursor: u64,
        receipt_ref: String,
    },
    RecordNativeWrite {
        event_cursor: u64,
    },
    RollbackLegacy {
        reconciled_native_cursor: u64,
        receipt_ref: String,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WorkCutoverRequest {
    pub expected_revision: u64,
    pub expected_generation: u64,
    pub github_write_count: u64,
    pub command: WorkCutoverCommand,
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum WorkCutoverError {
    #[error("stale cutover revision")]
    StaleRevision,
    #[error("stale cutover generation")]
    StaleGeneration,
    #[error("native cutover commands must create zero GitHub writes")]
    GithubWriteAttempt,
    #[error("cutover input is invalid")]
    InvalidInput,
    #[error("the command is invalid for the active writer")]
    WrongWriter,
    #[error("the legacy source changed after shadow reconciliation")]
    SourceChanged,
    #[error("rollback has not reconciled every post-cutover native event")]
    NativeHistoryGap,
    #[error("cutover storage is unavailable or invalid")]
    Storage,
}

impl WorkCutoverLedger {
    pub fn shadow(source_digest: String, source_cursor: u64) -> Result<Self, WorkCutoverError> {
        validate_digest(&source_digest)?;
        Ok(Self {
            schema: WORK_CUTOVER_SCHEMA.into(),
            revision: 1,
            generation: 1,
            writer: WorkWriter::LegacyGithub,
            source_digest,
            source_cursor,
            native_high_watermark: 0,
            activation_receipt_ref: None,
            rollback_receipt_ref: None,
        })
    }

    pub fn apply(&mut self, request: WorkCutoverRequest) -> Result<(), WorkCutoverError> {
        if request.expected_revision != self.revision {
            return Err(WorkCutoverError::StaleRevision);
        }
        if request.expected_generation != self.generation {
            return Err(WorkCutoverError::StaleGeneration);
        }
        if request.github_write_count != 0 {
            return Err(WorkCutoverError::GithubWriteAttempt);
        }
        match request.command {
            WorkCutoverCommand::BindShadow {
                source_digest,
                source_cursor,
            } => {
                require_writer(self.writer, WorkWriter::LegacyGithub)?;
                validate_digest(&source_digest)?;
                self.source_digest = source_digest;
                self.source_cursor = source_cursor;
            }
            WorkCutoverCommand::ActivateNative {
                source_digest,
                reconciled_cursor,
                receipt_ref,
            } => {
                require_writer(self.writer, WorkWriter::LegacyGithub)?;
                validate_ref(&receipt_ref)?;
                if source_digest != self.source_digest || reconciled_cursor != self.source_cursor {
                    return Err(WorkCutoverError::SourceChanged);
                }
                self.writer = WorkWriter::NativeOmega;
                self.generation = self.generation.saturating_add(1);
                self.native_high_watermark = reconciled_cursor;
                self.activation_receipt_ref = Some(receipt_ref);
                self.rollback_receipt_ref = None;
            }
            WorkCutoverCommand::RecordNativeWrite { event_cursor } => {
                require_writer(self.writer, WorkWriter::NativeOmega)?;
                if event_cursor <= self.native_high_watermark {
                    return Err(WorkCutoverError::InvalidInput);
                }
                self.native_high_watermark = event_cursor;
            }
            WorkCutoverCommand::RollbackLegacy {
                reconciled_native_cursor,
                receipt_ref,
            } => {
                require_writer(self.writer, WorkWriter::NativeOmega)?;
                validate_ref(&receipt_ref)?;
                if reconciled_native_cursor < self.native_high_watermark {
                    return Err(WorkCutoverError::NativeHistoryGap);
                }
                self.writer = WorkWriter::LegacyGithub;
                self.generation = self.generation.saturating_add(1);
                self.source_cursor = reconciled_native_cursor;
                self.rollback_receipt_ref = Some(receipt_ref);
            }
        }
        self.revision = self.revision.saturating_add(1);
        Ok(())
    }

    pub fn load(path: &Path) -> Result<Option<Self>, WorkCutoverError> {
        let bytes = match fs::read(path) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(_) => return Err(WorkCutoverError::Storage),
        };
        let ledger: Self = serde_json::from_slice(&bytes).map_err(|_| WorkCutoverError::Storage)?;
        ledger.validate()?;
        Ok(Some(ledger))
    }

    /// Atomically replace the public-safe cutover ledger. The receipt contains
    /// references and cursors only; credentials and private prompts never
    /// belong in this document.
    pub fn store(&self, path: &Path) -> Result<(), WorkCutoverError> {
        self.validate()?;
        let parent = path.parent().ok_or(WorkCutoverError::Storage)?;
        fs::create_dir_all(parent).map_err(|_| WorkCutoverError::Storage)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            fs::set_permissions(parent, fs::Permissions::from_mode(0o700))
                .map_err(|_| WorkCutoverError::Storage)?;
        }
        let temporary = path.with_extension(format!("tmp-{}", std::process::id()));
        let result = (|| {
            let bytes = serde_json::to_vec_pretty(self).map_err(|_| WorkCutoverError::Storage)?;
            let mut file = fs::OpenOptions::new()
                .create_new(true)
                .write(true)
                .open(&temporary)
                .map_err(|_| WorkCutoverError::Storage)?;
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt as _;
                file.set_permissions(fs::Permissions::from_mode(0o600))
                    .map_err(|_| WorkCutoverError::Storage)?;
            }
            file.write_all(&bytes)
                .and_then(|_| file.sync_all())
                .map_err(|_| WorkCutoverError::Storage)?;
            fs::rename(&temporary, path).map_err(|_| WorkCutoverError::Storage)
        })();
        if result.is_err() {
            let _ = fs::remove_file(&temporary);
        }
        result
    }

    fn validate(&self) -> Result<(), WorkCutoverError> {
        if self.schema != WORK_CUTOVER_SCHEMA || self.revision == 0 || self.generation == 0 {
            return Err(WorkCutoverError::Storage);
        }
        validate_digest(&self.source_digest).map_err(|_| WorkCutoverError::Storage)?;
        if self.writer == WorkWriter::NativeOmega && self.activation_receipt_ref.is_none() {
            return Err(WorkCutoverError::Storage);
        }
        for receipt in [
            self.activation_receipt_ref.as_deref(),
            self.rollback_receipt_ref.as_deref(),
        ]
        .into_iter()
        .flatten()
        {
            validate_ref(receipt).map_err(|_| WorkCutoverError::Storage)?;
        }
        Ok(())
    }
}

fn require_writer(actual: WorkWriter, expected: WorkWriter) -> Result<(), WorkCutoverError> {
    (actual == expected)
        .then_some(())
        .ok_or(WorkCutoverError::WrongWriter)
}

fn validate_digest(value: &str) -> Result<(), WorkCutoverError> {
    (value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit()))
        .then_some(())
        .ok_or(WorkCutoverError::InvalidInput)
}

fn validate_ref(value: &str) -> Result<(), WorkCutoverError> {
    (!value.is_empty()
        && value.len() <= 256
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b':' | b'-' | b'_' | b'.')))
    .then_some(())
    .ok_or(WorkCutoverError::InvalidInput)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request(ledger: &WorkCutoverLedger, command: WorkCutoverCommand) -> WorkCutoverRequest {
        WorkCutoverRequest {
            expected_revision: ledger.revision,
            expected_generation: ledger.generation,
            github_write_count: 0,
            command,
        }
    }

    #[test]
    fn activation_is_explicit_and_rollback_requires_complete_native_history() {
        let digest = "a".repeat(64);
        let mut ledger = WorkCutoverLedger::shadow(digest.clone(), 40).expect("shadow");
        let activate = request(
            &ledger,
            WorkCutoverCommand::ActivateNative {
                source_digest: digest,
                reconciled_cursor: 40,
                receipt_ref: "receipt:cutover:1".into(),
            },
        );
        ledger.apply(activate).expect("activate");
        let native_write = request(
            &ledger,
            WorkCutoverCommand::RecordNativeWrite { event_cursor: 45 },
        );
        ledger.apply(native_write).expect("native write");
        let gap = request(
            &ledger,
            WorkCutoverCommand::RollbackLegacy {
                reconciled_native_cursor: 44,
                receipt_ref: "receipt:rollback:1".into(),
            },
        );
        assert_eq!(ledger.apply(gap), Err(WorkCutoverError::NativeHistoryGap));
        let rollback = request(
            &ledger,
            WorkCutoverCommand::RollbackLegacy {
                reconciled_native_cursor: 45,
                receipt_ref: "receipt:rollback:2".into(),
            },
        );
        ledger.apply(rollback).expect("reconciled rollback");
        assert_eq!(ledger.writer, WorkWriter::LegacyGithub);
        assert_eq!(ledger.generation, 3);
    }

    #[test]
    fn stale_generation_and_github_writes_fail_before_transition() {
        let mut ledger = WorkCutoverLedger::shadow("b".repeat(64), 1).expect("shadow");
        let mut stale = request(
            &ledger,
            WorkCutoverCommand::BindShadow {
                source_digest: "c".repeat(64),
                source_cursor: 2,
            },
        );
        stale.expected_generation = 0;
        assert_eq!(ledger.apply(stale), Err(WorkCutoverError::StaleGeneration));
        let mut write = request(
            &ledger,
            WorkCutoverCommand::BindShadow {
                source_digest: "c".repeat(64),
                source_cursor: 2,
            },
        );
        write.github_write_count = 1;
        assert_eq!(
            ledger.apply(write),
            Err(WorkCutoverError::GithubWriteAttempt)
        );
    }

    #[test]
    fn ledger_storage_round_trips_without_a_default_activation() {
        let directory = tempfile::tempdir().expect("temporary directory");
        let path = directory.path().join("work-cutover.json");
        let ledger = WorkCutoverLedger::shadow("d".repeat(64), 9).expect("shadow");
        ledger.store(&path).expect("store");
        assert_eq!(WorkCutoverLedger::load(&path).expect("load"), Some(ledger));
    }
}
