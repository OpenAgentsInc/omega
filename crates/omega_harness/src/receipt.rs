//! The maintenance decision, and the receipt that records it.
//!
//! Two rules shape this file.
//!
//! **One decision, one receipt.** [`decide_maintenance`] is the only thing that
//! says yes, and [`receipt_for_decision`] is the only thing that writes a
//! receipt, and it takes that decision as its input. A receipt therefore cannot
//! describe an outcome different from the one the enforcement path acted on,
//! because there is no second place where either is computed.
//! `every_decision_produces_a_receipt_that_agrees_with_it` holds that shut.
//!
//! **The producer routes through its own reader.** [`build_harness_maintenance_receipt`]
//! serialises a JSON document and hands it to [`decode_harness_maintenance_receipt`]
//! rather than constructing the typed value, exactly as
//! `build_issue31_host_adjunct` does. Every refusal a later reader could raise
//! against a stored receipt is raised here first, against the same bytes.
//!
//! On top of those, the input types make the incoherent states unwritable
//! rather than merely refused: an `Applied` outcome carries a
//! [`MeasuredDigest`](crate::MeasuredDigest) and not a string, so a receipt
//! saying an update was applied structurally cannot exist unless this host
//! hashed the bytes it applied.

use serde::{Deserialize, Serialize};

use crate::{HarnessPin, MeasuredDigest};

/// The schema every receipt this Omega writes carries.
pub const HARNESS_MAINTENANCE_RECEIPT_SCHEMA: &str = "openagents.omega.harness.maintenance.v1";

/// The file name of the append-only receipt log, relative to the external
/// agents directory. One JSON document per line.
pub const HARNESS_MAINTENANCE_LOG_FILE_NAME: &str = "omega-harness-receipts.jsonl";

/// The longest a reference may be. Bounded so a corrupt log line cannot make a
/// reader do unbounded work.
pub const MAX_HARNESS_REF_LEN: usize = 128;

/// The largest admitted millisecond timestamp, shared with the other Omega
/// receipt contracts: the JavaScript `Date` bound.
pub const MAX_HARNESS_TIMESTAMP_MS: u64 = 8_640_000_000_000_000;

// ---------------------------------------------------------------------------
// Actions
// ---------------------------------------------------------------------------

/// The maintenance actions Omega performs on a wrapped harness.
///
/// A closed set. A new kind of maintenance is a schema change, not a new
/// string, because a reader that cannot name an action cannot judge it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MaintenanceAction {
    /// First install on a machine that had none.
    Install,
    /// Replace the installed tree with a different version.
    Update,
    /// Re-measure an already-installed tree without changing it. This is what
    /// runs on every launch, and it is the reason a pin cannot be defeated by
    /// swapping the bytes after the install receipt was written.
    Verify,
    /// Turn what the registry channel currently advertises into a concrete
    /// candidate version.
    ///
    /// Separate from [`Self::Update`] because it happens when no bytes move and
    /// no launch is pending: the registry document refreshes in the background
    /// and the store decides whether to *offer* the version it now names. A
    /// harness frozen at another version must not be offered an update Omega
    /// would refuse to apply, and the resolution that decided so is the thing
    /// worth recording — the update that never started leaves no other trace.
    ResolveChannel,
    /// Re-establish what an installed tree is after it changed, rather than
    /// carrying the previous answer forward.
    ///
    /// Separate from [`Self::Verify`] because `Verify` is what the launch path
    /// does on the way to spawning a harness, while this is what the owner's
    /// front door does on demand with nothing about to run. Collapsing them
    /// would make the log unable to say whether a measurement was taken because
    /// something was about to execute or because a person asked.
    ReprobeCapability,
}

/// Every admitted action, in declaration order.
///
/// Named rather than derived, so adding a variant without a wire token that
/// decodes fails `every_admitted_action_round_trips_through_a_receipt`.
pub const ADMITTED_MAINTENANCE_ACTIONS: &[MaintenanceAction] = &[
    MaintenanceAction::Install,
    MaintenanceAction::Update,
    MaintenanceAction::Verify,
    MaintenanceAction::ResolveChannel,
    MaintenanceAction::ReprobeCapability,
];

// ---------------------------------------------------------------------------
// Decision
// ---------------------------------------------------------------------------

/// What the ledger says about one harness.
///
/// `Unreadable` is not `Unpinned`. A ledger that fails to parse is a machine
/// whose pins are unknown, and treating unknown as unpinned would turn
/// corrupting one file into a way to unfreeze every harness on the machine.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PinState<'a> {
    /// The ledger was read and this harness is frozen.
    Pinned(&'a HarnessPin),
    /// The ledger was read and this harness is not frozen.
    Unpinned,
    /// The ledger could not be read. Fails closed.
    Unreadable,
}

/// The artifact a maintenance action is about to make runnable.
///
/// `version` is what the registry document *claims*. `digest` is what this host
/// *measured*. Only the second one authorises anything: the version is compared
/// against the pin's version so a refusal can name something an owner
/// recognises, but a matching version with different bytes is still refused.
/// See `a_retagged_release_does_not_satisfy_a_pin`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CandidateArtifact<'a> {
    Measured {
        version: &'a str,
        digest: &'a MeasuredDigest,
    },
    /// The host could not read the installed tree. There is no digest field
    /// here, so nothing downstream can invent one.
    Unmeasured { version: &'a str },
}

/// Why a maintenance action was refused.
///
/// A closed set with stable wire tokens. Free text would make the reason
/// unreadable to anything but a human, and this is the value the front door
/// renders and the receipt records.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MaintenanceRefusal {
    /// The owner froze this harness at a different version.
    PinnedVersion {
        pinned_version: String,
        candidate_version: String,
    },
    /// The version matches the pin but the bytes do not. This is the
    /// re-tagged-release case, and the one a version-only pin would miss.
    PinnedDigest {
        pinned_version: String,
        pinned_digest: String,
        measured_digest: String,
    },
    /// The host could not measure the bytes that would run.
    ProvenanceUnavailable { version: String },
    /// The pin ledger exists but could not be read.
    PinLedgerUnreadable,
    /// The owner froze this harness, and this harness is distributed in a way
    /// that gives Omega nothing to freeze.
    ///
    /// A package manager resolves the bytes at exec time into a cache Omega
    /// owns no directory for, so there is no tree to hash and the version the
    /// registry names is a ceiling rather than an exact request. A pin on such
    /// a harness cannot be enforced. Launching anyway would make the pin a
    /// decoration — the owner would have said *not that one* and Omega would
    /// have run whatever `npm` chose — so the pin refuses the launch instead.
    UnpinnableDistribution {
        pinned_version: String,
        /// The resolver that owns the bytes, e.g. `npx`.
        resolver: String,
    },
}

impl MaintenanceRefusal {
    /// The stable wire token. Persisted and compared, never shown alone.
    #[must_use]
    pub const fn reason_class(&self) -> &'static str {
        match self {
            Self::PinnedVersion { .. } => "pinned_version",
            Self::PinnedDigest { .. } => "pinned_digest",
            Self::ProvenanceUnavailable { .. } => "provenance_unavailable",
            Self::PinLedgerUnreadable => "pin_ledger_unreadable",
            Self::UnpinnableDistribution { .. } => "unpinnable_distribution",
        }
    }

    /// The sentence the front door shows next to the disabled control.
    ///
    /// A refusal the owner cannot see is the same defect as a refusal that
    /// never happened, so every class has a sentence and the sentence names
    /// what to do about it.
    #[must_use]
    pub fn reason(&self) -> String {
        match self {
            Self::PinnedVersion {
                pinned_version,
                candidate_version,
            } => format!(
                "Pinned to {pinned_version}. The registry now offers {candidate_version}; \
                 remove the pin to take it."
            ),
            Self::PinnedDigest {
                pinned_version,
                pinned_digest,
                measured_digest,
            } => format!(
                "Pinned to {pinned_version} at {}, but the installed files hash to {}. \
                 The pinned release was replaced; re-pin only if you meant to.",
                short_digest(pinned_digest),
                short_digest(measured_digest),
            ),
            Self::ProvenanceUnavailable { version } => format!(
                "Omega could not read the installed files for {version}, so it cannot \
                 verify what would run. Reinstall this agent."
            ),
            Self::PinLedgerUnreadable => {
                "Omega could not read the pin ledger, so it cannot tell which versions \
                 you froze. Every maintenance action is held until it can."
                    .to_string()
            }
            Self::UnpinnableDistribution {
                pinned_version,
                resolver,
            } => format!(
                "Pinned to {pinned_version}, but this agent runs through {resolver}, which \
                 resolves its own bytes at launch. Omega cannot hold it at a version, so it \
                 will not run it while the pin stands. Remove the pin to run it unpinned."
            ),
        }
    }
}

fn short_digest(digest: &str) -> String {
    digest.chars().take(12).collect()
}

/// The outcome of a maintenance decision.
///
/// `Permitted` carries the measurement that permitted it, so the receipt writer
/// cannot be handed a permission without also being handed its evidence.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MaintenanceDecision<'a> {
    Permitted {
        version: &'a str,
        digest: &'a MeasuredDigest,
    },
    Refused(MaintenanceRefusal),
}

impl MaintenanceDecision<'_> {
    #[must_use]
    pub fn is_permitted(&self) -> bool {
        matches!(self, Self::Permitted { .. })
    }

    /// The refusal, if this decision is one.
    #[must_use]
    pub fn refusal(&self) -> Option<&MaintenanceRefusal> {
        match self {
            Self::Permitted { .. } => None,
            Self::Refused(refusal) => Some(refusal),
        }
    }
}

/// Decide whether a harness may be made runnable.
///
/// The single gate. Everything the enforcement path knows arrives here, and
/// nothing downstream re-derives an answer.
///
/// Both non-`Permitted` inputs fail closed:
///
/// * an unreadable ledger refuses everything, so corrupting one file unfreezes
///   nothing;
/// * an unmeasured artifact refuses **whether or not a pin exists**, because
///   the falsifier on omega#81 is precisely a binary that runs with full tool
///   permissions without a verifiable provenance record — a machine with no pin
///   is not a machine that consented to running unread bytes.
#[must_use]
pub fn decide_maintenance<'a>(
    pin_state: PinState<'_>,
    candidate: CandidateArtifact<'a>,
) -> MaintenanceDecision<'a> {
    if pin_state == PinState::Unreadable {
        return MaintenanceDecision::Refused(MaintenanceRefusal::PinLedgerUnreadable);
    }

    let (version, digest) = match candidate {
        CandidateArtifact::Unmeasured { version } => {
            return MaintenanceDecision::Refused(MaintenanceRefusal::ProvenanceUnavailable {
                version: version.to_string(),
            });
        }
        CandidateArtifact::Measured { version, digest } => (version, digest),
    };

    let PinState::Pinned(pin) = pin_state else {
        return MaintenanceDecision::Permitted { version, digest };
    };

    if pin.version != version {
        return MaintenanceDecision::Refused(MaintenanceRefusal::PinnedVersion {
            pinned_version: pin.version.clone(),
            candidate_version: version.to_string(),
        });
    }
    if !digest.matches_recorded(&pin.digest) {
        return MaintenanceDecision::Refused(MaintenanceRefusal::PinnedDigest {
            pinned_version: pin.version.clone(),
            pinned_digest: pin.digest.clone(),
            measured_digest: digest.as_str().to_string(),
        });
    }
    MaintenanceDecision::Permitted { version, digest }
}

/// Whether the host may *fetch* a version at all, before any bytes move.
///
/// [`decide_maintenance`] is the authority, and it needs a measurement, which
/// means it can only run after a download. That is too late to stop Omega
/// pulling a release the owner froze out — the refusal would still hold, but
/// the network transfer and the disk write would already have happened.
///
/// So this is a prefilter, and it is deliberately weaker: it may refuse only
/// what [`decide_maintenance`] would also refuse, and it may never permit
/// anything on its own. `the_prefilter_never_admits_what_the_gate_refuses`
/// holds both halves.
#[must_use]
pub fn admits_version(pin_state: PinState<'_>, candidate_version: &str) -> Option<MaintenanceRefusal> {
    match pin_state {
        PinState::Unreadable => Some(MaintenanceRefusal::PinLedgerUnreadable),
        PinState::Unpinned => None,
        PinState::Pinned(pin) if pin.version != candidate_version => {
            Some(MaintenanceRefusal::PinnedVersion {
                pinned_version: pin.version.clone(),
                candidate_version: candidate_version.to_string(),
            })
        }
        PinState::Pinned(_) => None,
    }
}

/// Whether a harness whose bytes no directory of Omega's holds may launch.
///
/// [`decide_maintenance`] cannot answer this: it needs a
/// [`CandidateArtifact`], and for a package-manager-resolved harness there is
/// no tree to build one from. Routing such a harness through the
/// `Unmeasured` arm would refuse **every** npx agent on every machine, which
/// is not what the pin ledger says and not what this issue asks for. So the
/// question is narrowed to the one the ledger can actually answer:
///
/// * an unreadable ledger refuses, exactly as everywhere else — a machine whose
///   pins are unknown does not launch a harness it cannot attest;
/// * a **pinned** harness refuses, because the pin cannot be enforced and a pin
///   that is silently ignored is worse than no pin at all;
/// * an unpinned harness launches, unattested, and the front door says so.
///
/// The last line is the honest limit of this gate and is stated rather than
/// hidden: it raises no bar on an unpinned npx harness. What it removes is the
/// state where an owner froze one and Omega ran whatever `npm` resolved.
#[must_use]
pub fn admits_package_manager_launch(
    pin_state: PinState<'_>,
    resolver: &str,
) -> Option<MaintenanceRefusal> {
    match pin_state {
        PinState::Unreadable => Some(MaintenanceRefusal::PinLedgerUnreadable),
        PinState::Unpinned => None,
        PinState::Pinned(pin) => Some(MaintenanceRefusal::UnpinnableDistribution {
            pinned_version: pin.version.clone(),
            resolver: resolver.to_string(),
        }),
    }
}

// ---------------------------------------------------------------------------
// Receipt
// ---------------------------------------------------------------------------

/// What a receipt says happened.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum MaintenanceOutcome {
    Applied {
        version: String,
        #[serde(rename = "artifactDigest")]
        artifact_digest: String,
    },
    Refused {
        #[serde(rename = "reasonClass")]
        reason_class: String,
        #[serde(rename = "reasonDetail")]
        reason_detail: serde_json::Value,
    },
}

/// One maintenance action, recorded.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HarnessMaintenanceReceipt {
    pub schema: &'static str,
    pub host_ref: String,
    pub harness_ref: String,
    pub action: MaintenanceAction,
    /// The host's clock, read once by the caller and stamped here.
    ///
    /// No input to [`build_harness_maintenance_receipt`] other than this
    /// parameter carries a time, so there is no path by which a registry
    /// document, a settings file, or a remote request can supply one. It is a
    /// measurement of when this host acted, not a claim about when something
    /// happened.
    pub observed_at_ms: u64,
    pub outcome: MaintenanceOutcome,
}

impl HarnessMaintenanceReceipt {
    /// The digest this receipt binds to, when it binds to one.
    ///
    /// A refused action applied no bytes, so it has none. There is no default.
    #[must_use]
    pub fn artifact_digest(&self) -> Option<&str> {
        match &self.outcome {
            MaintenanceOutcome::Applied {
                artifact_digest, ..
            } => Some(artifact_digest.as_str()),
            MaintenanceOutcome::Refused { .. } => None,
        }
    }

    #[must_use]
    pub fn reason_class(&self) -> Option<&str> {
        match &self.outcome {
            MaintenanceOutcome::Applied { .. } => None,
            MaintenanceOutcome::Refused { reason_class, .. } => Some(reason_class.as_str()),
        }
    }
}

/// Every way a receipt can fail to be a receipt.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HarnessMaintenanceReceiptError {
    InvalidJson,
    InvalidSchema,
    UnsafeReference,
    InvalidTimestamp,
    InvalidOutcome,
}

impl std::fmt::Display for HarnessMaintenanceReceiptError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self {
            Self::InvalidJson => "harness maintenance receipt is not valid contract JSON",
            Self::InvalidSchema => "harness maintenance receipt schema is not supported",
            Self::UnsafeReference => "harness maintenance receipt contains an unsafe reference",
            Self::InvalidTimestamp => "harness maintenance receipt timestamp is not admitted",
            Self::InvalidOutcome => "harness maintenance receipt outcome is not coherent",
        })
    }
}

impl std::error::Error for HarnessMaintenanceReceiptError {}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RawHarnessMaintenanceReceipt {
    schema: String,
    host_ref: String,
    harness_ref: String,
    action: MaintenanceAction,
    observed_at_ms: u64,
    outcome: RawMaintenanceOutcome,
}

#[derive(Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
enum RawMaintenanceOutcome {
    Applied {
        version: String,
        #[serde(rename = "artifactDigest")]
        artifact_digest: String,
    },
    Refused {
        #[serde(rename = "reasonClass")]
        reason_class: String,
        #[serde(rename = "reasonDetail")]
        reason_detail: serde_json::Value,
    },
}

/// The refusal classes a receipt may carry. Kept beside
/// [`MaintenanceRefusal::reason_class`] so a decoder cannot admit a class no
/// producer can mean.
const ADMITTED_REASON_CLASSES: &[&str] = &[
    "pinned_version",
    "pinned_digest",
    "provenance_unavailable",
    "pin_ledger_unreadable",
    "unpinnable_distribution",
];

/// Read a receipt.
pub fn decode_harness_maintenance_receipt(
    input: &str,
) -> Result<HarnessMaintenanceReceipt, HarnessMaintenanceReceiptError> {
    let raw: RawHarnessMaintenanceReceipt =
        serde_json::from_str(input).map_err(|_| HarnessMaintenanceReceiptError::InvalidJson)?;
    if raw.schema != HARNESS_MAINTENANCE_RECEIPT_SCHEMA {
        return Err(HarnessMaintenanceReceiptError::InvalidSchema);
    }
    if raw.observed_at_ms == 0 || raw.observed_at_ms > MAX_HARNESS_TIMESTAMP_MS {
        return Err(HarnessMaintenanceReceiptError::InvalidTimestamp);
    }
    let host_ref = safe_ref(&raw.host_ref)?;
    let harness_ref = safe_ref(&raw.harness_ref)?;

    let outcome = match raw.outcome {
        RawMaintenanceOutcome::Applied {
            version,
            artifact_digest,
        } => {
            if version.trim().is_empty() || !is_sha256_hex(&artifact_digest) {
                return Err(HarnessMaintenanceReceiptError::InvalidOutcome);
            }
            MaintenanceOutcome::Applied {
                version,
                artifact_digest: artifact_digest.to_ascii_lowercase(),
            }
        }
        RawMaintenanceOutcome::Refused {
            reason_class,
            reason_detail,
        } => {
            if !ADMITTED_REASON_CLASSES.contains(&reason_class.as_str())
                || !reason_detail.is_object()
            {
                return Err(HarnessMaintenanceReceiptError::InvalidOutcome);
            }
            for value in reason_detail.as_object().expect("checked above").values() {
                if let Some(text) = value.as_str() {
                    safe_detail(text)?;
                }
            }
            MaintenanceOutcome::Refused {
                reason_class,
                reason_detail,
            }
        }
    };

    Ok(HarnessMaintenanceReceipt {
        schema: HARNESS_MAINTENANCE_RECEIPT_SCHEMA,
        host_ref,
        harness_ref,
        action: raw.action,
        observed_at_ms: raw.observed_at_ms,
        outcome,
    })
}

fn is_sha256_hex(value: &str) -> bool {
    value.len() == 64 && value.chars().all(|c| c.is_ascii_hexdigit())
}

fn safe_ref(raw: &str) -> Result<String, HarnessMaintenanceReceiptError> {
    if raw != raw.trim() || raw.is_empty() || raw.len() > MAX_HARNESS_REF_LEN {
        return Err(HarnessMaintenanceReceiptError::UnsafeReference);
    }
    if !raw
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-' | ':'))
    {
        return Err(HarnessMaintenanceReceiptError::UnsafeReference);
    }
    Ok(raw.to_string())
}

/// A refusal detail is shown to a person, so it is prose rather than a
/// reference — but it must still not be able to carry a filesystem path out of
/// the machine, which is how a receipt turns into a disclosure.
fn safe_detail(raw: &str) -> Result<(), HarnessMaintenanceReceiptError> {
    if raw.len() > MAX_HARNESS_REF_LEN
        || raw.contains('/')
        || raw.contains('\\')
        || raw.contains('\0')
    {
        return Err(HarnessMaintenanceReceiptError::UnsafeReference);
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Emitter
// ---------------------------------------------------------------------------

/// What a caller may ask the receipt writer to record.
///
/// `Applied` takes a [`MeasuredDigest`](crate::MeasuredDigest), not a string.
/// That is the compile-time half of the provenance rule: a receipt claiming an
/// update was applied cannot be written by a caller that never hashed the
/// bytes, because there is no way to build the argument without hashing
/// something.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MaintenanceOutcomeInput<'a> {
    Applied {
        version: &'a str,
        digest: &'a MeasuredDigest,
    },
    Refused(&'a MaintenanceRefusal),
}

/// Write a receipt for a maintenance action.
///
/// `harness_id` is the registry id; the reference is built here rather than
/// accepted, so a caller cannot name the harness something other than what the
/// store calls it.
///
/// The document is serialised and handed to
/// [`decode_harness_maintenance_receipt`], so this cannot emit a record its own
/// reader would refuse.
pub fn build_harness_maintenance_receipt(
    host_ref: &str,
    harness_id: &str,
    observed_at_ms: u64,
    action: MaintenanceAction,
    outcome: &MaintenanceOutcomeInput<'_>,
) -> Result<HarnessMaintenanceReceipt, HarnessMaintenanceReceiptError> {
    let document = serde_json::json!({
        "schema": HARNESS_MAINTENANCE_RECEIPT_SCHEMA,
        "hostRef": host_ref,
        "harnessRef": format!("harness.{harness_id}"),
        "action": action,
        "observedAtMs": observed_at_ms,
        "outcome": match outcome {
            MaintenanceOutcomeInput::Applied { version, digest } => serde_json::json!({
                "kind": "applied",
                "version": version,
                "artifactDigest": digest.as_str(),
            }),
            MaintenanceOutcomeInput::Refused(refusal) => serde_json::json!({
                "kind": "refused",
                "reasonClass": refusal.reason_class(),
                "reasonDetail": refusal_detail(refusal),
            }),
        },
    });
    let serialized = serde_json::to_string(&document)
        .map_err(|_| HarnessMaintenanceReceiptError::InvalidJson)?;
    decode_harness_maintenance_receipt(&serialized)
}

fn refusal_detail(refusal: &MaintenanceRefusal) -> serde_json::Value {
    match refusal {
        MaintenanceRefusal::PinnedVersion {
            pinned_version,
            candidate_version,
        } => serde_json::json!({
            "pinnedVersion": pinned_version,
            "candidateVersion": candidate_version,
        }),
        MaintenanceRefusal::PinnedDigest {
            pinned_version,
            pinned_digest,
            measured_digest,
        } => serde_json::json!({
            "pinnedVersion": pinned_version,
            "pinnedDigest": pinned_digest,
            "measuredDigest": measured_digest,
        }),
        MaintenanceRefusal::ProvenanceUnavailable { version } => serde_json::json!({
            "version": version,
        }),
        MaintenanceRefusal::PinLedgerUnreadable => serde_json::json!({}),
        MaintenanceRefusal::UnpinnableDistribution {
            pinned_version,
            resolver,
        } => serde_json::json!({
            "pinnedVersion": pinned_version,
            "resolver": resolver,
        }),
    }
}

/// Write the receipt for a decision.
///
/// The only receipt writer the enforcement path uses. Because the decision is
/// the input, the receipt cannot say "applied" for an action that was refused,
/// or the reverse: there is no branch here that re-decides anything.
pub fn receipt_for_decision(
    host_ref: &str,
    harness_id: &str,
    observed_at_ms: u64,
    action: MaintenanceAction,
    decision: &MaintenanceDecision<'_>,
) -> Result<HarnessMaintenanceReceipt, HarnessMaintenanceReceiptError> {
    let outcome = match decision {
        MaintenanceDecision::Permitted { version, digest } => {
            MaintenanceOutcomeInput::Applied { version, digest }
        }
        MaintenanceDecision::Refused(refusal) => MaintenanceOutcomeInput::Refused(refusal),
    };
    build_harness_maintenance_receipt(host_ref, harness_id, observed_at_ms, action, &outcome)
}

// ---------------------------------------------------------------------------
// Reading a log written by another version of Omega
// ---------------------------------------------------------------------------

/// One line of the receipt log, as this Omega can read it.
///
/// The receipt log is append-only across Omega versions, so a reader meets
/// lines it did not write: records from before omega#81 added a digest, and
/// records from a future schema it does not know. Both decode. Neither is
/// backfilled.
///
/// `ProvenanceUnavailable` has no digest, no version, and no outcome — there is
/// nowhere to put an invented one. A record whose provenance a reader cannot
/// establish is reported as exactly that and refused, never rendered beside the
/// attested ones as though it carried the same evidence.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum HarnessMaintenanceRecord {
    Attested(HarnessMaintenanceReceipt),
    ProvenanceUnavailable { schema: String },
}

/// Read one line of the receipt log.
///
/// A line that is not JSON at all is an error, not a record: there is nothing
/// to report the provenance *of*.
pub fn decode_harness_maintenance_record(
    input: &str,
) -> Result<HarnessMaintenanceRecord, HarnessMaintenanceReceiptError> {
    // The schema is read before the strict decoder runs, not after it fails.
    // `deny_unknown_fields` rejects a future schema's extra fields as invalid
    // JSON, so a decoder that only reached this branch on `InvalidSchema` would
    // classify every genuinely-newer record as garbage — which is a silent
    // skip, and a silent skip is the backfill this contract refuses.
    let value: serde_json::Value =
        serde_json::from_str(input).map_err(|_| HarnessMaintenanceReceiptError::InvalidJson)?;
    let schema = value
        .get("schema")
        .and_then(serde_json::Value::as_str)
        .ok_or(HarnessMaintenanceReceiptError::InvalidJson)?;
    if schema != HARNESS_MAINTENANCE_RECEIPT_SCHEMA {
        safe_ref(schema)?;
        return Ok(HarnessMaintenanceRecord::ProvenanceUnavailable {
            schema: schema.to_string(),
        });
    }
    decode_harness_maintenance_receipt(input).map(HarnessMaintenanceRecord::Attested)
}

/// What a reader may conclude about an installed harness.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ProvenanceVerdict {
    /// A receipt this Omega can read binds the bytes now on disk.
    Verified { digest: String },
    Refused(ProvenanceGap),
}

/// Why provenance could not be established.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProvenanceGap {
    /// No receipt covers this installation. Every harness installed before
    /// omega#81 is in this state, and stays in it until a maintenance action
    /// measures it. It is not upgraded by assumption.
    Unattested,
    /// A receipt exists, but the bytes on disk are not the bytes it recorded.
    DigestMismatch,
    /// The most recent receipt records a refusal, so nothing was applied.
    LastActionRefused,
    /// The most recent record is one this Omega cannot read the provenance of.
    RecordUnreadable,
    /// The installed tree could not be read, so there is no measurement to
    /// judge a record against. Distinct from [`Self::Unattested`]: that one is
    /// "nobody recorded these bytes", this one is "this host cannot read them".
    TreeUnreadable,
    /// The harness has no tree on disk to attest, by construction — a package
    /// manager resolves its bytes at launch into a cache Omega owns no
    /// directory for. Not a defect in this installation; a property of how the
    /// harness is distributed.
    ResolvedAtLaunch,
}

impl ProvenanceGap {
    #[must_use]
    pub const fn reason(self) -> &'static str {
        match self {
            Self::Unattested => {
                "This agent was installed before Omega recorded provenance. Update it \
                 to bind it to a verified build."
            }
            Self::DigestMismatch => {
                "The installed files are not the files Omega recorded. Reinstall this \
                 agent before running it."
            }
            Self::LastActionRefused => {
                "The last maintenance action was refused, so nothing was installed."
            }
            Self::RecordUnreadable => {
                "Omega cannot read the provenance record for this agent. It was written \
                 by a different version."
            }
            Self::TreeUnreadable => {
                "Omega could not read this agent's installed files, so it cannot say what \
                 would run. Reinstall it."
            }
            Self::ResolvedAtLaunch => {
                "This agent's package manager resolves its own bytes when it starts, so \
                 there is nothing installed for Omega to attest."
            }
        }
    }
}

/// What the host knows about the bytes an installed harness will run.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum InstallationProvenance {
    /// The most recent record covering this harness.
    Recorded(HarnessMaintenanceRecord),
    /// No record covers this installation.
    Unattested,
}

/// Judge an installation against the bytes now on disk.
#[must_use]
pub fn verify_installation(
    provenance: &InstallationProvenance,
    measured: &MeasuredDigest,
) -> ProvenanceVerdict {
    match provenance {
        InstallationProvenance::Unattested => ProvenanceVerdict::Refused(ProvenanceGap::Unattested),
        InstallationProvenance::Recorded(HarnessMaintenanceRecord::ProvenanceUnavailable {
            ..
        }) => ProvenanceVerdict::Refused(ProvenanceGap::RecordUnreadable),
        InstallationProvenance::Recorded(HarnessMaintenanceRecord::Attested(receipt)) => {
            match receipt.artifact_digest() {
                None => ProvenanceVerdict::Refused(ProvenanceGap::LastActionRefused),
                Some(recorded) if measured.matches_recorded(recorded) => {
                    ProvenanceVerdict::Verified {
                        digest: recorded.to_string(),
                    }
                }
                Some(_) => ProvenanceVerdict::Refused(ProvenanceGap::DigestMismatch),
            }
        }
    }
}

// ---------------------------------------------------------------------------
// The front-door affordance
// ---------------------------------------------------------------------------

/// Whether the one-click control is live, and if not, why.
///
/// The `reason` is not optional decoration. omega 0.2.0-rc11 bound
/// `appendSystemNote` to `() => {}` on the framed provider path, so a handoff
/// was refused and the thread said nothing; a different model then spent the
/// owner's budget with no trace. A blocked update nobody can see is the same
/// defect. So a disabled affordance structurally carries its sentence: there is
/// no `Disabled` without a reason to put in it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MaintenanceAffordance {
    Enabled,
    Disabled { reason: String },
}

impl MaintenanceAffordance {
    #[must_use]
    pub fn is_enabled(&self) -> bool {
        matches!(self, Self::Enabled)
    }

    /// The sentence to render beside a disabled control.
    #[must_use]
    pub fn reason(&self) -> Option<&str> {
        match self {
            Self::Enabled => None,
            Self::Disabled { reason } => Some(reason.as_str()),
        }
    }
}

/// The state of the one-click update control for one harness.
#[must_use]
pub fn update_affordance(decision: &MaintenanceDecision<'_>) -> MaintenanceAffordance {
    match decision {
        MaintenanceDecision::Permitted { .. } => MaintenanceAffordance::Enabled,
        MaintenanceDecision::Refused(refusal) => MaintenanceAffordance::Disabled {
            reason: refusal.reason(),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::HarnessPinLedger;

    const HOST: &str = "host.omega.device-alpha";
    const NOW: u64 = 1_784_894_400_000;

    fn pinned_ledger() -> HarnessPinLedger {
        let mut ledger = HarnessPinLedger::empty();
        ledger.set_pin("codex-acp", "0.9.4", &MeasuredDigest::measure(RELEASE_0_9_4));
        ledger
    }

    const RELEASE_0_9_4: &[u8] = b"codex-acp 0.9.4 tree";
    const RELEASE_0_9_5: &[u8] = b"codex-acp 0.9.5 tree";

    // -------------------------------------------------------------------
    // The decision
    // -------------------------------------------------------------------

    #[test]
    fn an_unpinned_harness_may_be_updated() {
        let digest = MeasuredDigest::measure(RELEASE_0_9_5);
        let decision = decide_maintenance(
            PinState::Unpinned,
            CandidateArtifact::Measured {
                version: "0.9.5",
                digest: &digest,
            },
        );
        assert!(decision.is_permitted());
        assert!(update_affordance(&decision).is_enabled());
    }

    #[test]
    fn a_pinned_harness_is_not_updated_past_its_pin() {
        let ledger = pinned_ledger();
        let digest = MeasuredDigest::measure(RELEASE_0_9_5);
        let decision = decide_maintenance(
            PinState::Pinned(ledger.pin("codex-acp").expect("pinned")),
            CandidateArtifact::Measured {
                version: "0.9.5",
                digest: &digest,
            },
        );
        assert_eq!(
            decision.refusal().map(MaintenanceRefusal::reason_class),
            Some("pinned_version")
        );
    }

    #[test]
    fn a_pinned_harness_still_runs_at_the_version_it_is_pinned_to() {
        let ledger = pinned_ledger();
        let digest = MeasuredDigest::measure(RELEASE_0_9_4);
        let decision = decide_maintenance(
            PinState::Pinned(ledger.pin("codex-acp").expect("pinned")),
            CandidateArtifact::Measured {
                version: "0.9.4",
                digest: &digest,
            },
        );
        assert!(
            decision.is_permitted(),
            "a pin freezes a harness, it does not disable it"
        );
    }

    /// The reason the pin is not a version string. A release re-tagged in place
    /// carries the pinned version and different bytes; a version-only pin
    /// admits it, and this is exactly the substitution omega#81's falsifier
    /// describes.
    #[test]
    fn a_retagged_release_does_not_satisfy_a_pin() {
        let ledger = pinned_ledger();
        let substituted = MeasuredDigest::measure(b"codex-acp 0.9.4 tree, rebuilt");
        let decision = decide_maintenance(
            PinState::Pinned(ledger.pin("codex-acp").expect("pinned")),
            CandidateArtifact::Measured {
                version: "0.9.4",
                digest: &substituted,
            },
        );
        assert_eq!(
            decision.refusal().map(MaintenanceRefusal::reason_class),
            Some("pinned_digest")
        );
    }

    /// The falsifier, directly: bytes nobody hashed never become runnable, pin
    /// or no pin.
    #[test]
    fn bytes_the_host_could_not_read_are_refused_even_with_no_pin() {
        for pin_state in [PinState::Unpinned, PinState::Unreadable] {
            let decision =
                decide_maintenance(pin_state, CandidateArtifact::Unmeasured { version: "0.9.5" });
            assert!(!decision.is_permitted(), "{pin_state:?}");
        }
    }

    /// Corrupting the ledger must not be a way to unfreeze every harness.
    #[test]
    fn an_unreadable_ledger_refuses_everything_rather_than_reading_as_unpinned() {
        let digest = MeasuredDigest::measure(RELEASE_0_9_5);
        let decision = decide_maintenance(
            PinState::Unreadable,
            CandidateArtifact::Measured {
                version: "0.9.5",
                digest: &digest,
            },
        );
        assert_eq!(
            decision.refusal().map(MaintenanceRefusal::reason_class),
            Some("pin_ledger_unreadable")
        );
    }

    /// A package-manager harness nobody froze is not this gate's business. It
    /// launches unattested, and the front door says so — refusing every npx
    /// agent on every machine is a different change than omega#81 asks for, and
    /// pretending otherwise here would hide which bar this actually raises.
    #[test]
    fn an_unpinned_package_manager_harness_is_admitted() {
        assert_eq!(
            admits_package_manager_launch(PinState::Unpinned, "npx"),
            None
        );
    }

    /// The gap this closes: before it, pinning an npx harness did nothing at
    /// all. A pin Omega cannot enforce must refuse rather than be ignored,
    /// because an ignored pin tells the owner their "not that one" was heard.
    #[test]
    fn a_pin_on_a_package_manager_harness_refuses_rather_than_being_ignored() {
        let ledger = pinned_ledger();
        let pin = ledger.pin("codex-acp").expect("pinned");
        let refusal = admits_package_manager_launch(PinState::Pinned(pin), "npx")
            .expect("a pinned package-manager harness refuses");
        assert_eq!(refusal.reason_class(), "unpinnable_distribution");
        assert!(refusal.reason().contains("npx"));

        // And the refusal is recordable: a gate whose refusal could not be
        // written would enforce without leaving evidence it enforced.
        let receipt = build_harness_maintenance_receipt(
            HOST,
            "codex-acp",
            NOW,
            MaintenanceAction::Verify,
            &MaintenanceOutcomeInput::Refused(&refusal),
        )
        .expect("the refusal is a receipt");
        assert_eq!(receipt.reason_class(), Some("unpinnable_distribution"));
        assert_eq!(receipt.artifact_digest(), None);
    }

    /// An unreadable ledger refuses the package-manager path too. The gate that
    /// fails closed everywhere else must not be the one place a corrupt file
    /// buys a launch.
    #[test]
    fn an_unreadable_ledger_refuses_a_package_manager_launch_as_well() {
        assert_eq!(
            admits_package_manager_launch(PinState::Unreadable, "npx")
                .map(|refusal| refusal.reason_class()),
            Some("pin_ledger_unreadable")
        );
    }

    /// Every refusal a person can hit has a sentence, and the sentence says
    /// something. An empty or generic reason is the rc11 defect wearing a
    /// different shape.
    #[test]
    fn every_refusal_class_renders_a_reason_that_names_what_to_do() {
        let refusals = [
            MaintenanceRefusal::PinnedVersion {
                pinned_version: "0.9.4".into(),
                candidate_version: "0.9.5".into(),
            },
            MaintenanceRefusal::PinnedDigest {
                pinned_version: "0.9.4".into(),
                pinned_digest: "a".repeat(64),
                measured_digest: "b".repeat(64),
            },
            MaintenanceRefusal::ProvenanceUnavailable {
                version: "0.9.5".into(),
            },
            MaintenanceRefusal::PinLedgerUnreadable,
            MaintenanceRefusal::UnpinnableDistribution {
                pinned_version: "1.2.3".into(),
                resolver: "npx".into(),
            },
        ];
        let mut classes = std::collections::BTreeSet::new();
        for refusal in &refusals {
            let reason = refusal.reason();
            assert!(reason.len() > 40, "{reason:?} is not a sentence");
            assert!(!reason.contains('/'), "{reason:?} carries a path");
            classes.insert(refusal.reason_class());
        }
        assert_eq!(
            classes.len(),
            ADMITTED_REASON_CLASSES.len(),
            "a refusal class exists that the receipt contract does not admit"
        );
        for class in ADMITTED_REASON_CLASSES {
            assert!(classes.contains(class), "{class} has no refusal that means it");
        }
    }

    /// The prefilter runs before a download and the gate runs after one, so a
    /// disagreement between them is either a release Omega fetches and then
    /// refuses to run (waste) or, worse, one it declines to fetch that the gate
    /// would have permitted (a pin that blocks more than it says).
    ///
    /// Neither is allowed: over every combination of pin state and candidate,
    /// a prefilter refusal implies a gate refusal *of the same class*, and a
    /// prefilter pass never turns a gate refusal into a permit.
    #[test]
    fn the_prefilter_never_admits_what_the_gate_refuses() {
        let ledger = pinned_ledger();
        let pinned = ledger.pin("codex-acp").expect("pinned");
        let matching = MeasuredDigest::measure(RELEASE_0_9_4);
        let substituted = MeasuredDigest::measure(b"codex-acp 0.9.4 tree, rebuilt");
        let newer = MeasuredDigest::measure(RELEASE_0_9_5);

        let cases: Vec<(PinState<'_>, &str, &MeasuredDigest)> = vec![
            (PinState::Unpinned, "0.9.5", &newer),
            (PinState::Unreadable, "0.9.5", &newer),
            (PinState::Pinned(pinned), "0.9.4", &matching),
            (PinState::Pinned(pinned), "0.9.4", &substituted),
            (PinState::Pinned(pinned), "0.9.5", &newer),
        ];

        for (pin_state, version, digest) in cases {
            let prefiltered = admits_version(pin_state, version);
            let decided = decide_maintenance(
                pin_state,
                CandidateArtifact::Measured { version, digest },
            );
            match (&prefiltered, &decided) {
                (Some(refusal), MaintenanceDecision::Refused(gate)) => {
                    assert_eq!(
                        refusal.reason_class(),
                        gate.reason_class(),
                        "{pin_state:?} {version}: the prefilter and the gate refuse differently"
                    );
                }
                (Some(refusal), MaintenanceDecision::Permitted { .. }) => panic!(
                    "{pin_state:?} {version}: the prefilter refused {} where the gate permits",
                    refusal.reason_class()
                ),
                (None, _) => {}
            }
        }
    }

    #[test]
    fn a_refused_decision_disables_the_control_with_that_refusals_reason() {
        let ledger = pinned_ledger();
        let digest = MeasuredDigest::measure(RELEASE_0_9_5);
        let decision = decide_maintenance(
            PinState::Pinned(ledger.pin("codex-acp").expect("pinned")),
            CandidateArtifact::Measured {
                version: "0.9.5",
                digest: &digest,
            },
        );
        let affordance = update_affordance(&decision);
        assert!(!affordance.is_enabled());
        let reason = affordance.reason().expect("a disabled control carries a reason");
        assert!(reason.contains("0.9.4"), "{reason:?}");
        assert!(reason.contains("0.9.5"), "{reason:?}");
    }

    // -------------------------------------------------------------------
    // The receipt
    // -------------------------------------------------------------------

    #[test]
    fn a_permitted_update_produces_a_receipt_bound_to_the_measured_digest() {
        let digest = MeasuredDigest::measure(RELEASE_0_9_5);
        let decision = decide_maintenance(
            PinState::Unpinned,
            CandidateArtifact::Measured {
                version: "0.9.5",
                digest: &digest,
            },
        );
        let receipt = receipt_for_decision(HOST, "codex-acp", NOW, MaintenanceAction::Update, &decision)
            .expect("a permitted update writes a receipt");
        assert_eq!(receipt.harness_ref, "harness.codex-acp");
        assert_eq!(receipt.action, MaintenanceAction::Update);
        assert_eq!(receipt.observed_at_ms, NOW);
        assert_eq!(receipt.artifact_digest(), Some(digest.as_str()));
    }

    /// The binding between the gate and the record. A receipt cannot describe
    /// an outcome other than the one the gate reached, because the gate's
    /// output is the writer's input.
    #[test]
    fn every_decision_produces_a_receipt_that_agrees_with_it() {
        let ledger = pinned_ledger();
        let pinned = ledger.pin("codex-acp").expect("pinned");
        let matching = MeasuredDigest::measure(RELEASE_0_9_4);
        let newer = MeasuredDigest::measure(RELEASE_0_9_5);
        let substituted = MeasuredDigest::measure(b"codex-acp 0.9.4 tree, rebuilt");

        let cases: Vec<(PinState<'_>, CandidateArtifact<'_>)> = vec![
            (
                PinState::Unpinned,
                CandidateArtifact::Measured {
                    version: "0.9.5",
                    digest: &newer,
                },
            ),
            (
                PinState::Pinned(pinned),
                CandidateArtifact::Measured {
                    version: "0.9.4",
                    digest: &matching,
                },
            ),
            (
                PinState::Pinned(pinned),
                CandidateArtifact::Measured {
                    version: "0.9.5",
                    digest: &newer,
                },
            ),
            (
                PinState::Pinned(pinned),
                CandidateArtifact::Measured {
                    version: "0.9.4",
                    digest: &substituted,
                },
            ),
            (
                PinState::Pinned(pinned),
                CandidateArtifact::Unmeasured { version: "0.9.5" },
            ),
            (
                PinState::Unreadable,
                CandidateArtifact::Measured {
                    version: "0.9.5",
                    digest: &newer,
                },
            ),
        ];

        for (pin_state, candidate) in cases {
            let decision = decide_maintenance(pin_state, candidate);
            let receipt =
                receipt_for_decision(HOST, "codex-acp", NOW, MaintenanceAction::Update, &decision)
                    .expect("every decision is recordable");
            match &decision {
                MaintenanceDecision::Permitted { digest, .. } => {
                    assert_eq!(receipt.artifact_digest(), Some(digest.as_str()));
                    assert_eq!(receipt.reason_class(), None);
                }
                MaintenanceDecision::Refused(refusal) => {
                    assert_eq!(receipt.reason_class(), Some(refusal.reason_class()));
                    assert_eq!(
                        receipt.artifact_digest(),
                        None,
                        "a refused action applied no bytes and must bind to none"
                    );
                }
            }
        }
    }

    /// The producer routes through its own reader, so what it writes is what a
    /// later reader gets back.
    #[test]
    fn an_emitted_receipt_decodes_to_itself() {
        let digest = MeasuredDigest::measure(RELEASE_0_9_5);
        let receipt = build_harness_maintenance_receipt(
            HOST,
            "codex-acp",
            NOW,
            MaintenanceAction::Install,
            &MaintenanceOutcomeInput::Applied {
                version: "0.9.5",
                digest: &digest,
            },
        )
        .expect("emits");
        let encoded = serde_json::to_string(&receipt).expect("a receipt serialises");
        assert_eq!(
            decode_harness_maintenance_receipt(&encoded).expect("its own bytes decode"),
            receipt
        );
    }

    #[test]
    fn a_receipt_carries_no_filesystem_path() {
        let digest = MeasuredDigest::measure(RELEASE_0_9_5);
        let receipt = build_harness_maintenance_receipt(
            HOST,
            "codex-acp",
            NOW,
            MaintenanceAction::Install,
            &MaintenanceOutcomeInput::Applied {
                version: "0.9.5",
                digest: &digest,
            },
        )
        .expect("emits");
        let encoded = serde_json::to_string(&receipt).expect("serialises");
        assert!(!encoded.contains("/Users/"));
        assert!(!encoded.contains("credential"));
        assert!(!encoded.contains("Bearer "));
    }

    #[test]
    fn a_private_path_cannot_be_emitted_as_a_host_reference() {
        let digest = MeasuredDigest::measure(RELEASE_0_9_5);
        let error = build_harness_maintenance_receipt(
            "/Users/owner/.codex/auth.json",
            "codex-acp",
            NOW,
            MaintenanceAction::Install,
            &MaintenanceOutcomeInput::Applied {
                version: "0.9.5",
                digest: &digest,
            },
        )
        .expect_err("a private path must fail closed");
        assert_eq!(error, HarnessMaintenanceReceiptError::UnsafeReference);
        assert!(!error.to_string().contains("/Users/"));
    }

    /// The digest is the whole receipt. A stored record that lost it does not
    /// decode with a blank, an empty string, or the version standing in.
    #[test]
    fn an_applied_receipt_without_a_digest_is_refused_rather_than_defaulted() {
        let digest = MeasuredDigest::measure(RELEASE_0_9_5);
        let receipt = build_harness_maintenance_receipt(
            HOST,
            "codex-acp",
            NOW,
            MaintenanceAction::Update,
            &MaintenanceOutcomeInput::Applied {
                version: "0.9.5",
                digest: &digest,
            },
        )
        .expect("emits");
        let mut value = serde_json::to_value(&receipt).expect("value");
        value["outcome"]
            .as_object_mut()
            .expect("outcome object")
            .remove("artifactDigest");
        assert_eq!(
            decode_harness_maintenance_receipt(&value.to_string()),
            Err(HarnessMaintenanceReceiptError::InvalidJson)
        );

        let mut value = serde_json::to_value(&receipt).expect("value");
        value["outcome"]["artifactDigest"] = serde_json::Value::String(String::new());
        assert_eq!(
            decode_harness_maintenance_receipt(&value.to_string()),
            Err(HarnessMaintenanceReceiptError::InvalidOutcome)
        );

        let mut value = serde_json::to_value(&receipt).expect("value");
        value["outcome"]["artifactDigest"] = serde_json::Value::String("0.9.5".into());
        assert_eq!(
            decode_harness_maintenance_receipt(&value.to_string()),
            Err(HarnessMaintenanceReceiptError::InvalidOutcome)
        );
    }

    #[test]
    fn a_refusal_class_no_producer_can_mean_is_refused() {
        let receipt = receipt_for_decision(
            HOST,
            "codex-acp",
            NOW,
            MaintenanceAction::Update,
            &MaintenanceDecision::Refused(MaintenanceRefusal::PinLedgerUnreadable),
        )
        .expect("emits");
        let mut value = serde_json::to_value(&receipt).expect("value");
        value["outcome"]["reasonClass"] = serde_json::Value::String("owner_waived_the_pin".into());
        assert_eq!(
            decode_harness_maintenance_receipt(&value.to_string()),
            Err(HarnessMaintenanceReceiptError::InvalidOutcome)
        );
    }

    #[test]
    fn an_absent_or_out_of_range_timestamp_is_refused() {
        let digest = MeasuredDigest::measure(RELEASE_0_9_5);
        for stamp in [0, MAX_HARNESS_TIMESTAMP_MS + 1] {
            let error = build_harness_maintenance_receipt(
                HOST,
                "codex-acp",
                stamp,
                MaintenanceAction::Update,
                &MaintenanceOutcomeInput::Applied {
                    version: "0.9.5",
                    digest: &digest,
                },
            )
            .expect_err("an unstamped receipt is not a measurement");
            assert_eq!(error, HarnessMaintenanceReceiptError::InvalidTimestamp);
        }
    }

    #[test]
    fn an_unadmitted_receipt_field_is_refused_rather_than_ignored() {
        let receipt = receipt_for_decision(
            HOST,
            "codex-acp",
            NOW,
            MaintenanceAction::Verify,
            &MaintenanceDecision::Refused(MaintenanceRefusal::PinLedgerUnreadable),
        )
        .expect("emits");
        let mut value = serde_json::to_value(&receipt).expect("value");
        value
            .as_object_mut()
            .expect("object")
            .insert("verified".into(), serde_json::Value::Bool(true));
        assert_eq!(
            decode_harness_maintenance_receipt(&value.to_string()),
            Err(HarnessMaintenanceReceiptError::InvalidJson)
        );
    }

    // -------------------------------------------------------------------
    // Reading a log this Omega did not write
    // -------------------------------------------------------------------

    /// The no-backfill law. A record from a schema this reader does not know
    /// decodes, reports its provenance as unavailable, and is refused. It is
    /// never given a digest, because the value it decodes to has no field for
    /// one.
    #[test]
    fn a_record_from_another_schema_reports_unavailable_and_is_refused() {
        let line = serde_json::json!({
            "schema": "openagents.omega.harness.maintenance.v2",
            "hostRef": HOST,
            "harnessRef": "harness.codex-acp",
            "action": "update",
            "observedAtMs": NOW,
            "outcome": { "kind": "applied", "version": "1.0.0", "artifactDigest": "c".repeat(64) },
            "attestationChain": ["signature"],
        })
        .to_string();
        let record = decode_harness_maintenance_record(&line).expect("an unknown schema decodes");
        assert_eq!(
            record,
            HarnessMaintenanceRecord::ProvenanceUnavailable {
                schema: "openagents.omega.harness.maintenance.v2".into(),
            }
        );

        let verdict = verify_installation(
            &InstallationProvenance::Recorded(record),
            &MeasuredDigest::measure(RELEASE_0_9_5),
        );
        assert_eq!(
            verdict,
            ProvenanceVerdict::Refused(ProvenanceGap::RecordUnreadable)
        );
    }

    /// An installation that predates this packet has no receipt at all. It
    /// stays unattested until a maintenance action measures it; nothing
    /// promotes it by assumption.
    #[test]
    fn an_installation_from_before_this_packet_is_refused_not_assumed_good() {
        let verdict = verify_installation(
            &InstallationProvenance::Unattested,
            &MeasuredDigest::measure(RELEASE_0_9_4),
        );
        assert_eq!(verdict, ProvenanceVerdict::Refused(ProvenanceGap::Unattested));
        assert!(ProvenanceGap::Unattested.reason().contains("provenance"));
    }

    #[test]
    fn a_receipt_whose_bytes_were_swapped_afterwards_is_refused() {
        let installed = MeasuredDigest::measure(RELEASE_0_9_5);
        let receipt = build_harness_maintenance_receipt(
            HOST,
            "codex-acp",
            NOW,
            MaintenanceAction::Install,
            &MaintenanceOutcomeInput::Applied {
                version: "0.9.5",
                digest: &installed,
            },
        )
        .expect("emits");
        let provenance =
            InstallationProvenance::Recorded(HarnessMaintenanceRecord::Attested(receipt));

        assert_eq!(
            verify_installation(&provenance, &installed),
            ProvenanceVerdict::Verified {
                digest: installed.as_str().to_string()
            }
        );
        assert_eq!(
            verify_installation(&provenance, &MeasuredDigest::measure(b"swapped afterwards")),
            ProvenanceVerdict::Refused(ProvenanceGap::DigestMismatch)
        );
    }

    /// A refused action installed nothing, so the last record covering a
    /// harness being a refusal does not verify anything.
    #[test]
    fn a_refusal_receipt_never_verifies_an_installation() {
        let receipt = receipt_for_decision(
            HOST,
            "codex-acp",
            NOW,
            MaintenanceAction::Update,
            &MaintenanceDecision::Refused(MaintenanceRefusal::PinLedgerUnreadable),
        )
        .expect("emits");
        assert_eq!(
            verify_installation(
                &InstallationProvenance::Recorded(HarnessMaintenanceRecord::Attested(receipt)),
                &MeasuredDigest::measure(RELEASE_0_9_5),
            ),
            ProvenanceVerdict::Refused(ProvenanceGap::LastActionRefused)
        );
    }

    #[test]
    fn a_log_line_that_is_not_json_is_an_error_rather_than_a_record() {
        assert_eq!(
            decode_harness_maintenance_record("not json"),
            Err(HarnessMaintenanceReceiptError::InvalidJson)
        );
        assert_eq!(
            decode_harness_maintenance_record("{}"),
            Err(HarnessMaintenanceReceiptError::InvalidJson)
        );
    }
}
