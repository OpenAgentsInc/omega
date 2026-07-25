//! Harness maintenance with pinning and provenance. `OMEGA-DELTA-0025`,
//! omega#81.
//!
//! Omega wraps harnesses it did not build — `codex-acp` and every other ACP
//! registry agent — and those harnesses run with the tool permissions of the
//! thread that started them. Upstream resolves whichever version the registry
//! document currently advertises, downloads it, and runs it. There is no record
//! of which bytes ran, and no way for an owner to say *not that one*.
//!
//! This crate is the decision layer for both halves:
//!
//! * **Pins.** A ledger on disk freezes a harness at a version *and* at the
//!   digest of the tree that version installed. It is re-read on every
//!   decision, so it survives a restart by construction rather than by
//!   discipline, and an unreadable ledger refuses rather than reads as empty.
//! * **Provenance.** Every maintenance action produces a receipt bound to the
//!   SHA-256 of the installed tree, measured by this host. A record that
//!   predates the digest — an installation from an earlier Omega, a log line
//!   from a schema this build does not know — decodes, reports its provenance
//!   as unavailable, and is refused. Nothing is backfilled.
//!
//! It is a leaf: `serde`, `serde_json`, `sha2` and nothing else. The filesystem
//! and the GPUI entities live in `crates/project/src/agent_server_store.rs`,
//! which is the only caller that enforces any of this.

mod measured;
mod pins;
mod receipt;

pub use measured::MeasuredDigest;
pub use pins::{
    HARNESS_PIN_LEDGER_FILE_NAME, HARNESS_PIN_LEDGER_SCHEMA, HarnessPin, HarnessPinLedger,
    HarnessPinLedgerError, MAX_HARNESS_PINS, decode_harness_pin_ledger, encode_harness_pin_ledger,
};
pub use receipt::{
    ADMITTED_MAINTENANCE_ACTIONS, CandidateArtifact, HARNESS_MAINTENANCE_LOG_FILE_NAME,
    HARNESS_MAINTENANCE_RECEIPT_SCHEMA, HarnessMaintenanceReceipt,
    HarnessMaintenanceReceiptError, HarnessMaintenanceRecord, InstallationProvenance,
    MAX_HARNESS_REF_LEN, MAX_HARNESS_TIMESTAMP_MS, MaintenanceAction, MaintenanceAffordance,
    MaintenanceDecision, MaintenanceOutcome, MaintenanceOutcomeInput, MaintenanceRefusal,
    PinState, ProvenanceGap, ProvenanceVerdict, admits_version,
    build_harness_maintenance_receipt, decide_maintenance, decode_harness_maintenance_receipt,
    decode_harness_maintenance_record, receipt_for_decision, update_affordance,
    verify_installation,
};

/// The pin ledger as the enforcement path found it.
///
/// Three states, not two. "The file is not there" and "the file is there and I
/// cannot read it" are different facts about a machine, and collapsing them is
/// how corrupting one file would silently unfreeze every harness on it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LoadedPinLedger {
    /// No ledger file exists. Nothing is frozen.
    Absent,
    Loaded(HarnessPinLedger),
    /// A ledger file exists and does not parse.
    Unreadable(HarnessPinLedgerError),
}

impl LoadedPinLedger {
    /// Interpret the ledger file's contents.
    ///
    /// `None` is the file not existing. Anything else is bytes that have to
    /// parse.
    #[must_use]
    pub fn read(contents: Option<&str>) -> Self {
        match contents {
            None => Self::Absent,
            Some(text) => match decode_harness_pin_ledger(text) {
                Ok(ledger) => Self::Loaded(ledger),
                Err(error) => Self::Unreadable(error),
            },
        }
    }

    /// What this ledger says about one harness.
    #[must_use]
    pub fn pin_state(&self, harness_id: &str) -> PinState<'_> {
        match self {
            Self::Absent => PinState::Unpinned,
            Self::Unreadable(_) => PinState::Unreadable,
            Self::Loaded(ledger) => match ledger.pin(harness_id) {
                Some(pin) => PinState::Pinned(pin),
                None => PinState::Unpinned,
            },
        }
    }

    /// The ledger, when there is one to edit.
    #[must_use]
    pub fn ledger(&self) -> Option<&HarnessPinLedger> {
        match self {
            Self::Loaded(ledger) => Some(ledger),
            Self::Absent | Self::Unreadable(_) => None,
        }
    }
}

/// One receipt, as a line of the append-only log.
///
/// Serialising through the typed receipt rather than the caller's document
/// keeps the log's lines and the contract's shape the same thing.
pub fn receipt_log_line(receipt: &HarnessMaintenanceReceipt) -> String {
    let mut line = serde_json::to_string(receipt).unwrap_or_default();
    line.push('\n');
    line
}

/// The most recent record covering one harness, read out of the log.
///
/// Unreadable lines are not skipped past. If the newest line naming this
/// harness is one this build cannot read, that is the answer — reaching further
/// back for an older line this build *can* read would report provenance from
/// before the thing it cannot read happened.
#[must_use]
pub fn latest_record_for(log: &str, harness_id: &str) -> InstallationProvenance {
    let harness_ref = format!("harness.{harness_id}");
    for line in log.lines().rev() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        match decode_harness_maintenance_record(line) {
            Ok(HarnessMaintenanceRecord::Attested(receipt)) => {
                if receipt.harness_ref == harness_ref {
                    return InstallationProvenance::Recorded(HarnessMaintenanceRecord::Attested(
                        receipt,
                    ));
                }
            }
            Ok(HarnessMaintenanceRecord::ProvenanceUnavailable { schema }) => {
                // A record whose schema this build does not know may or may not
                // be about this harness — the field that would say is part of a
                // shape this build cannot read. Treating it as "not about us"
                // would let a future Omega's records be silently ignored, so it
                // is reported once the reader reaches it.
                if line.contains(&harness_ref) {
                    return InstallationProvenance::Recorded(
                        HarnessMaintenanceRecord::ProvenanceUnavailable { schema },
                    );
                }
            }
            Err(_) => continue,
        }
    }
    InstallationProvenance::Unattested
}

#[cfg(test)]
mod tests {
    use super::*;

    const HOST: &str = "host.omega.device-alpha";
    const NOW: u64 = 1_784_894_400_000;

    fn measured(bytes: &[u8]) -> MeasuredDigest {
        MeasuredDigest::measure(bytes)
    }

    fn scratch_dir(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("omega-harness-{name}-{}", std::process::id()));
        std::fs::remove_dir_all(&dir).ok();
        std::fs::create_dir_all(&dir).expect("scratch directory");
        dir
    }

    // -------------------------------------------------------------------
    // The pin survives a restart
    // -------------------------------------------------------------------

    /// The restart law, through a real file.
    ///
    /// The enforcement path holds no ledger between calls: it reads this file
    /// every time it decides. "Survives a restart" is therefore the same
    /// statement as "the bytes on disk still decide", which is what this
    /// asserts — a second, independent read of the same path refuses the update
    /// the first one refused.
    #[test]
    fn a_pin_taken_before_a_restart_is_read_back_after_one() {
        let dir = scratch_dir("restart");
        let path = dir.join(HARNESS_PIN_LEDGER_FILE_NAME);
        let installed = measured(b"codex-acp 0.9.4 tree");

        // Session one: the owner pins the installed version.
        let mut ledger = HarnessPinLedger::empty();
        ledger.set_pin("codex-acp", "0.9.4", &installed);
        std::fs::write(&path, encode_harness_pin_ledger(&ledger).expect("encodes"))
            .expect("the ledger is written");

        // Session two: a different process, holding nothing, reads the file.
        let reloaded = LoadedPinLedger::read(Some(
            &std::fs::read_to_string(&path).expect("the ledger is still there"),
        ));
        let candidate = measured(b"codex-acp 0.9.5 tree");
        let decision = decide_maintenance(
            reloaded.pin_state("codex-acp"),
            CandidateArtifact::Measured {
                version: "0.9.5",
                digest: &candidate,
            },
        );
        assert_eq!(
            decision.refusal().map(MaintenanceRefusal::reason_class),
            Some("pinned_version"),
            "a pin that does not outlive the process is not a pin"
        );

        // And the same file still permits the version it froze.
        assert!(
            decide_maintenance(
                reloaded.pin_state("codex-acp"),
                CandidateArtifact::Measured {
                    version: "0.9.4",
                    digest: &installed,
                },
            )
            .is_permitted()
        );

        std::fs::remove_dir_all(&dir).ok();
    }

    /// Removing the ledger file unfreezes; corrupting it does not.
    #[test]
    fn a_missing_ledger_is_unpinned_and_a_corrupt_one_is_not() {
        assert_eq!(
            LoadedPinLedger::read(None).pin_state("codex-acp"),
            PinState::Unpinned
        );
        assert_eq!(
            LoadedPinLedger::read(Some("{ truncated")).pin_state("codex-acp"),
            PinState::Unreadable
        );
        assert_eq!(
            LoadedPinLedger::read(Some(r#"{"schema":"something.else","pins":[]}"#))
                .pin_state("codex-acp"),
            PinState::Unreadable
        );
    }

    // -------------------------------------------------------------------
    // The log
    // -------------------------------------------------------------------

    fn applied_line(harness: &str, version: &str, digest: &MeasuredDigest, at: u64) -> String {
        receipt_log_line(
            &build_harness_maintenance_receipt(
                HOST,
                harness,
                at,
                MaintenanceAction::Update,
                &MaintenanceOutcomeInput::Applied { version, digest },
            )
            .expect("emits"),
        )
    }

    #[test]
    fn the_log_reports_the_newest_record_for_the_harness_that_was_asked_about() {
        let codex_old = measured(b"codex-acp 0.9.4 tree");
        let codex_new = measured(b"codex-acp 0.9.5 tree");
        let gemini = measured(b"gemini-cli 1.0.0 tree");
        let log = [
            applied_line("codex-acp", "0.9.4", &codex_old, NOW - 2000),
            applied_line("gemini-cli", "1.0.0", &gemini, NOW - 1000),
            applied_line("codex-acp", "0.9.5", &codex_new, NOW),
        ]
        .concat();

        assert_eq!(
            verify_installation(&latest_record_for(&log, "codex-acp"), &codex_new),
            ProvenanceVerdict::Verified {
                digest: codex_new.as_str().to_string()
            }
        );
        assert_eq!(
            verify_installation(&latest_record_for(&log, "gemini-cli"), &gemini),
            ProvenanceVerdict::Verified {
                digest: gemini.as_str().to_string()
            }
        );
        assert_eq!(
            latest_record_for(&log, "never-installed"),
            InstallationProvenance::Unattested
        );
    }

    /// The no-backfill law at the log level. A newer schema's record about this
    /// harness is reported as unreadable rather than skipped in favour of the
    /// older record underneath it, which would report provenance from before
    /// whatever that newer record described.
    #[test]
    fn a_newer_record_this_build_cannot_read_is_not_skipped_for_an_older_one_it_can() {
        let old = measured(b"codex-acp 0.9.4 tree");
        let mut log = applied_line("codex-acp", "0.9.4", &old, NOW - 1000);
        log.push_str(
            &serde_json::json!({
                "schema": "openagents.omega.harness.maintenance.v2",
                "harnessRef": "harness.codex-acp",
                "signedBy": "future-omega",
            })
            .to_string(),
        );
        log.push('\n');

        assert_eq!(
            verify_installation(&latest_record_for(&log, "codex-acp"), &old),
            ProvenanceVerdict::Refused(ProvenanceGap::RecordUnreadable),
            "an unreadable newer record must not be stepped over"
        );
    }

    #[test]
    fn a_corrupt_line_does_not_stop_the_reader_finding_the_record_underneath_it() {
        let digest = measured(b"codex-acp 0.9.4 tree");
        let mut log = applied_line("codex-acp", "0.9.4", &digest, NOW);
        log.push_str("<<< partially written line\n");
        assert_eq!(
            verify_installation(&latest_record_for(&log, "codex-acp"), &digest),
            ProvenanceVerdict::Verified {
                digest: digest.as_str().to_string()
            }
        );
    }

    #[test]
    fn an_empty_log_attests_nothing() {
        assert_eq!(
            latest_record_for("", "codex-acp"),
            InstallationProvenance::Unattested
        );
        assert_eq!(
            latest_record_for("\n\n  \n", "codex-acp"),
            InstallationProvenance::Unattested
        );
    }

    // -------------------------------------------------------------------
    // The whole flow
    // -------------------------------------------------------------------

    /// The acceptance sentence on omega#81, end to end and on disk: install is
    /// one action and produces a receipt; update is one action and produces a
    /// receipt; a pin blocks an unwanted update with a reason the front door
    /// can render.
    #[test]
    fn install_then_update_then_pin_then_a_blocked_update() {
        let dir = scratch_dir("flow");
        let ledger_path = dir.join(HARNESS_PIN_LEDGER_FILE_NAME);
        let log_path = dir.join(HARNESS_MAINTENANCE_LOG_FILE_NAME);

        let read_ledger = |path: &std::path::Path| {
            LoadedPinLedger::read(std::fs::read_to_string(path).ok().as_deref())
        };
        let read_log = |path: &std::path::Path| std::fs::read_to_string(path).unwrap_or_default();
        let append = |path: &std::path::Path, line: String| {
            let mut existing = std::fs::read_to_string(path).unwrap_or_default();
            existing.push_str(&line);
            std::fs::write(path, existing).expect("the log is appended");
        };

        // Install, on a clean machine: one action.
        let installed = measured(b"codex-acp 0.9.4 tree");
        let decision = decide_maintenance(
            read_ledger(&ledger_path).pin_state("codex-acp"),
            CandidateArtifact::Measured {
                version: "0.9.4",
                digest: &installed,
            },
        );
        assert!(decision.is_permitted());
        append(
            &log_path,
            receipt_log_line(
                &receipt_for_decision(
                    HOST,
                    "codex-acp",
                    NOW,
                    MaintenanceAction::Install,
                    &decision,
                )
                .expect("an install writes a receipt"),
            ),
        );
        assert_eq!(
            verify_installation(&latest_record_for(&read_log(&log_path), "codex-acp"), &installed),
            ProvenanceVerdict::Verified {
                digest: installed.as_str().to_string()
            }
        );

        // Update, unpinned: one action, and a receipt bound to the new bytes.
        let updated = measured(b"codex-acp 0.9.5 tree");
        let decision = decide_maintenance(
            read_ledger(&ledger_path).pin_state("codex-acp"),
            CandidateArtifact::Measured {
                version: "0.9.5",
                digest: &updated,
            },
        );
        assert!(decision.is_permitted());
        append(
            &log_path,
            receipt_log_line(
                &receipt_for_decision(HOST, "codex-acp", NOW + 1, MaintenanceAction::Update, &decision)
                    .expect("an update writes a receipt"),
            ),
        );
        assert_eq!(
            verify_installation(&latest_record_for(&read_log(&log_path), "codex-acp"), &updated),
            ProvenanceVerdict::Verified {
                digest: updated.as_str().to_string()
            }
        );

        // The owner pins what is installed.
        let mut ledger = HarnessPinLedger::empty();
        ledger.set_pin("codex-acp", "0.9.5", &updated);
        std::fs::write(&ledger_path, encode_harness_pin_ledger(&ledger).expect("encodes"))
            .expect("the ledger is written");

        // The registry now offers 0.9.6. The pin blocks it, visibly.
        let unwanted = measured(b"codex-acp 0.9.6 tree");
        let decision = decide_maintenance(
            read_ledger(&ledger_path).pin_state("codex-acp"),
            CandidateArtifact::Measured {
                version: "0.9.6",
                digest: &unwanted,
            },
        );
        assert!(!decision.is_permitted());
        let affordance = update_affordance(&decision);
        let reason = affordance.reason().expect("a blocked update says why");
        assert!(reason.contains("0.9.5") && reason.contains("0.9.6"), "{reason:?}");
        append(
            &log_path,
            receipt_log_line(
                &receipt_for_decision(HOST, "codex-acp", NOW + 2, MaintenanceAction::Update, &decision)
                    .expect("a refusal writes a receipt too"),
            ),
        );

        // And the refusal did not disturb what is installed: the last applied
        // record still describes 0.9.5, and 0.9.6's bytes verify against
        // nothing.
        let log = read_log(&log_path);
        assert_eq!(
            verify_installation(&latest_record_for(&log, "codex-acp"), &updated),
            ProvenanceVerdict::Refused(ProvenanceGap::LastActionRefused)
        );
        assert_eq!(log.lines().count(), 3, "every action left exactly one record");

        std::fs::remove_dir_all(&dir).ok();
    }

    /// Every admitted action is reachable and round-trips, so a new action
    /// cannot be added to the enum without a wire token that decodes.
    #[test]
    fn every_admitted_action_round_trips_through_a_receipt() {
        let digest = measured(b"tree");
        for action in ADMITTED_MAINTENANCE_ACTIONS {
            let receipt = build_harness_maintenance_receipt(
                HOST,
                "codex-acp",
                NOW,
                *action,
                &MaintenanceOutcomeInput::Applied {
                    version: "1.0.0",
                    digest: &digest,
                },
            )
            .expect("emits");
            assert_eq!(receipt.action, *action);
            let encoded = serde_json::to_string(&receipt).expect("serialises");
            assert_eq!(
                decode_harness_maintenance_receipt(&encoded)
                    .expect("decodes")
                    .action,
                *action
            );
        }
    }
}
