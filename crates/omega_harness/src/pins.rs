//! The component ledger: which harness versions the owner froze, and at which
//! bytes.
//!
//! A pin that lives in memory is not a pin. Omega restarts, the registry
//! document refreshes, and the next launch resolves whatever version the
//! registry now advertises. So the ledger is a file, it is re-read on every
//! maintenance decision rather than cached, and it is the *only* thing the
//! decision consults. Nothing in the enforcement path can hold a stale copy,
//! because nothing in the enforcement path holds a copy at all.
//!
//! A pin names two things:
//!
//! * a **version**, which is what the owner typed, and
//! * a **digest**, which is what the bytes were when the pin was taken.
//!
//! Both are checked. A version alone would be satisfied by a re-tagged release
//! — the exact substitution the falsifier on omega#81 describes — and a digest
//! alone would produce a refusal message nobody could act on.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

/// The schema every pin ledger on disk carries.
pub const HARNESS_PIN_LEDGER_SCHEMA: &str = "openagents.omega.harness.pins.v1";

/// The file name of the ledger, relative to the external agents directory.
pub const HARNESS_PIN_LEDGER_FILE_NAME: &str = "omega-harness-pins.json";

/// The most pins one ledger may hold. A bound, so a corrupt or hostile file
/// cannot make the launch path do unbounded work.
pub const MAX_HARNESS_PINS: usize = 128;

/// One frozen harness.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct HarnessPin {
    /// The registry id of the harness, for example `codex-acp`.
    pub harness_id: String,
    /// The version the owner froze.
    pub version: String,
    /// The digest of the installed tree when the pin was taken.
    ///
    /// A `String` rather than a
    /// [`MeasuredDigest`](crate::MeasuredDigest): a pin read back from disk is
    /// a recorded claim about a past measurement, not a measurement this
    /// process made, and the two must not share a type. It is only ever
    /// compared against a live measurement, never written into a receipt as
    /// one.
    pub digest: String,
}

/// The owner's frozen set.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HarnessPinLedger {
    pins: BTreeMap<String, HarnessPin>,
}

/// Every way a ledger file can fail to be a ledger.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HarnessPinLedgerError {
    InvalidJson,
    InvalidSchema,
    PinBoundExceeded,
    DuplicatePin,
    InvalidPin,
}

impl std::fmt::Display for HarnessPinLedgerError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self {
            Self::InvalidJson => "harness pin ledger is not valid contract JSON",
            Self::InvalidSchema => "harness pin ledger schema is not supported",
            Self::PinBoundExceeded => "harness pin ledger holds more pins than are admitted",
            Self::DuplicatePin => "harness pin ledger pins one harness twice",
            Self::InvalidPin => "harness pin ledger holds a pin that names no version or no bytes",
        })
    }
}

impl std::error::Error for HarnessPinLedgerError {}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RawHarnessPinLedger {
    schema: String,
    pins: Vec<HarnessPin>,
}

impl HarnessPinLedger {
    /// A ledger with nothing frozen. What a machine with no pin file has.
    #[must_use]
    pub fn empty() -> Self {
        Self {
            pins: BTreeMap::new(),
        }
    }

    /// The pin covering one harness, if the owner froze it.
    #[must_use]
    pub fn pin(&self, harness_id: &str) -> Option<&HarnessPin> {
        self.pins.get(harness_id)
    }

    /// Every pin, in a stable order.
    pub fn pins(&self) -> impl Iterator<Item = &HarnessPin> {
        self.pins.values()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.pins.is_empty()
    }

    /// Freeze one harness at the bytes the host just measured.
    ///
    /// Takes a [`MeasuredDigest`](crate::MeasuredDigest) rather than a string,
    /// so a pin cannot be created against bytes nobody read. That is what makes
    /// the pin worth checking later: it was a measurement once.
    pub fn set_pin(&mut self, harness_id: &str, version: &str, digest: &crate::MeasuredDigest) {
        self.pins.insert(
            harness_id.to_string(),
            HarnessPin {
                harness_id: harness_id.to_string(),
                version: version.to_string(),
                digest: digest.as_str().to_string(),
            },
        );
    }

    /// Unfreeze one harness. Returns whether anything was frozen.
    pub fn remove_pin(&mut self, harness_id: &str) -> bool {
        self.pins.remove(harness_id).is_some()
    }
}

/// Read a ledger file.
///
/// Fails closed on every shape it does not recognise. A ledger that cannot be
/// read is not an empty ledger — see
/// [`decide_maintenance`](crate::decide_maintenance) for what the caller does
/// with the error, which is refuse rather than proceed unpinned.
pub fn decode_harness_pin_ledger(input: &str) -> Result<HarnessPinLedger, HarnessPinLedgerError> {
    let raw: RawHarnessPinLedger =
        serde_json::from_str(input).map_err(|_| HarnessPinLedgerError::InvalidJson)?;
    if raw.schema != HARNESS_PIN_LEDGER_SCHEMA {
        return Err(HarnessPinLedgerError::InvalidSchema);
    }
    if raw.pins.len() > MAX_HARNESS_PINS {
        return Err(HarnessPinLedgerError::PinBoundExceeded);
    }

    let mut pins = BTreeMap::new();
    for pin in raw.pins {
        if pin.harness_id.trim().is_empty()
            || pin.version.trim().is_empty()
            || pin.digest.trim().is_empty()
        {
            return Err(HarnessPinLedgerError::InvalidPin);
        }
        if pins.insert(pin.harness_id.clone(), pin).is_some() {
            return Err(HarnessPinLedgerError::DuplicatePin);
        }
    }
    Ok(HarnessPinLedger { pins })
}

/// Write a ledger file.
///
/// Routes back through [`decode_harness_pin_ledger`] before returning, for the
/// same reason the receipt emitter does: a writer that can produce a file its
/// own reader refuses turns the next restart into the moment the pins vanish.
pub fn encode_harness_pin_ledger(
    ledger: &HarnessPinLedger,
) -> Result<String, HarnessPinLedgerError> {
    let raw = RawHarnessPinLedger {
        schema: HARNESS_PIN_LEDGER_SCHEMA.to_string(),
        pins: ledger.pins.values().cloned().collect(),
    };
    let serialized =
        serde_json::to_string_pretty(&raw).map_err(|_| HarnessPinLedgerError::InvalidJson)?;
    decode_harness_pin_ledger(&serialized)?;
    Ok(serialized)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::MeasuredDigest;

    fn ledger_with_codex() -> HarnessPinLedger {
        let mut ledger = HarnessPinLedger::empty();
        ledger.set_pin(
            "codex-acp",
            "0.9.4",
            &MeasuredDigest::measure(b"codex-acp 0.9.4 tree"),
        );
        ledger
    }

    /// The restart law, expressed without a filesystem: whatever the process
    /// held becomes bytes, and those bytes become the same thing again. The
    /// on-disk half of this is
    /// `a_pin_taken_before_a_restart_is_read_back_after_one` in
    /// `crates/omega_harness/src/omega_harness.rs`.
    #[test]
    fn a_ledger_survives_a_round_trip_through_its_own_file_format() {
        let ledger = ledger_with_codex();
        let encoded = encode_harness_pin_ledger(&ledger).expect("a ledger encodes");
        let decoded = decode_harness_pin_ledger(&encoded).expect("its own bytes decode");
        assert_eq!(decoded, ledger);
        assert_eq!(decoded.pin("codex-acp").expect("pinned").version, "0.9.4");
    }

    #[test]
    fn an_unpinned_harness_has_no_pin() {
        assert!(ledger_with_codex().pin("gemini-cli").is_none());
    }

    #[test]
    fn a_pin_records_the_bytes_that_were_measured_not_a_supplied_string() {
        let digest = MeasuredDigest::measure(b"codex-acp 0.9.4 tree");
        let ledger = ledger_with_codex();
        assert_eq!(
            ledger.pin("codex-acp").expect("pinned").digest,
            digest.as_str()
        );
    }

    #[test]
    fn an_unknown_schema_is_refused_rather_than_read_as_an_empty_ledger() {
        let file = serde_json::json!({
            "schema": "openagents.omega.harness.pins.v0",
            "pins": [],
        })
        .to_string();
        assert_eq!(
            decode_harness_pin_ledger(&file),
            Err(HarnessPinLedgerError::InvalidSchema)
        );
    }

    /// A ledger that pins one harness twice has no single answer to
    /// "what is `codex-acp` frozen at", and silently keeping the last one would
    /// make the effective pin depend on file order.
    #[test]
    fn a_duplicated_harness_is_refused_rather_than_last_write_wins() {
        let file = serde_json::json!({
            "schema": HARNESS_PIN_LEDGER_SCHEMA,
            "pins": [
                { "harnessId": "codex-acp", "version": "0.9.4", "digest": "aa" },
                { "harnessId": "codex-acp", "version": "0.9.5", "digest": "bb" },
            ],
        })
        .to_string();
        assert_eq!(
            decode_harness_pin_ledger(&file),
            Err(HarnessPinLedgerError::DuplicatePin)
        );
    }

    #[test]
    fn a_pin_that_names_no_bytes_is_refused() {
        for (version, digest) in [("0.9.4", "   "), ("", "aa")] {
            let file = serde_json::json!({
                "schema": HARNESS_PIN_LEDGER_SCHEMA,
                "pins": [
                    { "harnessId": "codex-acp", "version": version, "digest": digest },
                ],
            })
            .to_string();
            assert_eq!(
                decode_harness_pin_ledger(&file),
                Err(HarnessPinLedgerError::InvalidPin)
            );
        }
    }

    #[test]
    fn an_unadmitted_field_is_refused_rather_than_ignored() {
        let file = serde_json::json!({
            "schema": HARNESS_PIN_LEDGER_SCHEMA,
            "pins": [
                {
                    "harnessId": "codex-acp",
                    "version": "0.9.4",
                    "digest": "aa",
                    "allowUnpinnedUpdate": true,
                },
            ],
        })
        .to_string();
        assert_eq!(
            decode_harness_pin_ledger(&file),
            Err(HarnessPinLedgerError::InvalidJson)
        );
    }

    #[test]
    fn the_pin_bound_holds() {
        let pins: Vec<serde_json::Value> = (0..=MAX_HARNESS_PINS)
            .map(|index| {
                serde_json::json!({
                    "harnessId": format!("harness-{index}"),
                    "version": "1.0.0",
                    "digest": "aa",
                })
            })
            .collect();
        let file =
            serde_json::json!({ "schema": HARNESS_PIN_LEDGER_SCHEMA, "pins": pins }).to_string();
        assert_eq!(
            decode_harness_pin_ledger(&file),
            Err(HarnessPinLedgerError::PinBoundExceeded)
        );
    }

    #[test]
    fn removing_a_pin_unfreezes_exactly_one_harness() {
        let mut ledger = ledger_with_codex();
        ledger.set_pin("gemini-cli", "1.2.3", &MeasuredDigest::measure(b"gemini"));
        assert!(ledger.remove_pin("codex-acp"));
        assert!(!ledger.remove_pin("codex-acp"));
        assert!(ledger.pin("codex-acp").is_none());
        assert!(ledger.pin("gemini-cli").is_some());
    }
}
