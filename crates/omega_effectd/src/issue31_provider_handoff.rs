//! Host-owned provider connection handoff records (omega#91).
//!
//! The phone can already ask: `issue31_nostr` maps `request_provider_handoff`
//! onto a pairing scope and `action.omega.provider_handoff` onto that scope, so
//! a paired device can hold the grant and send the intent. Until this module
//! existed nothing answered — the `fullauto.v1` delivery carried an empty
//! handoff vector forever, and the phone rendered a capability the host had
//! nothing to say about.
//!
//! This is the thing that has something to say. It is a ledger of host-owned
//! records with one lifecycle:
//!
//! ```text
//! requested ── bound to an account ──▶ active ──▶ completed
//!     │                                  │
//!     └──────────────────────────────────┴──▶ refused | failed | expired
//! ```
//!
//! Three properties are load-bearing and are why this is a ledger rather than a
//! field on a request:
//!
//! - **Every host-owned field is a measurement, not a claim.** `requestedAtMs`
//!   comes from one reading of `now` taken when the host admitted the request;
//!   the device supplies no timestamp and there is no input path for one. The
//!   account binding comes from the host's own roster; the device names a
//!   provider and nothing else. The terminal outcome and the reason for a
//!   non-successful end are decided here, from what the host observed.
//! - **A row is written only if it can be read.** Every projected row goes back
//!   through `workroom_receipts::decode_issue31_provider_handoff`, the exact
//!   function the whole-document decoder uses, so this cannot emit a handoff
//!   the phone would refuse.
//! - **Nothing is backfilled.** A persisted row whose `requestedAtMs` predates
//!   this field decodes, is reported unavailable, and is refused. It is never
//!   shown with a stamp taken at load time, which would give one field two
//!   provenances that nothing on the wire distinguishes.
//!
//! The record carries **the fact of a connection, never the connection
//! secret**. Nothing here reads, writes, moves, or names a provider credential,
//! an isolated provider home, or a filesystem path; the only strings that enter
//! are a provider token and references the host itself minted.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use workroom_receipts::{
    Issue31FullAutoAdjunctError, Issue31ProviderHandoffState, decode_issue31_provider_handoff,
    is_public_safe_ref,
};

/// The action a device sends to open one.
pub const ISSUE31_ACTION_REQUEST_PROVIDER_HANDOFF: &str = "action.omega.provider_handoff";

/// The only shape of `argumentsRef` this action accepts.
///
/// The device names a provider and nothing else. Everything after this prefix
/// is a bounded provider token, so there is no field on the wire through which
/// a device could state a time, an account, a lane, or an outcome — the four
/// things that have to be the host's own measurements.
pub const ISSUE31_PROVIDER_HANDOFF_ARGUMENTS_PREFIX: &str = "arguments.omega.provider_handoff.";

/// Matches `workroom_receipts::MAX_ISSUE31_FULL_AUTO_HANDOFFS`. Exceeding the
/// contract bound in the ledger would produce a projection the phone refuses
/// wholesale, so the ledger refuses to open the seventeenth instead.
pub const MAX_ISSUE31_PROVIDER_HANDOFFS: usize = 16;

/// How long the host will hold a handoff open before calling it expired.
///
/// The work behind a handoff is an owner completing a provider login at the
/// host, in the host's own isolated provider home. Fifteen minutes is long
/// enough for that and short enough that the phone is never left holding a
/// request that neither resolves nor fails — the state this issue exists to
/// eliminate.
pub const ISSUE31_PROVIDER_HANDOFF_DEADLINE_MS: u64 = 15 * 60 * 1_000;

pub const ISSUE31_HANDOFF_OUTCOME_CONNECTED: &str = "outcome.omega.handoff_connected";
pub const ISSUE31_HANDOFF_OUTCOME_REFUSED: &str = "outcome.omega.handoff_refused";
pub const ISSUE31_HANDOFF_OUTCOME_INTERRUPTED: &str = "outcome.omega.handoff_interrupted";
pub const ISSUE31_HANDOFF_OUTCOME_EXPIRED: &str = "outcome.omega.handoff_expired";
pub const ISSUE31_HANDOFF_OUTCOME_FAILED: &str = "outcome.omega.handoff_failed";

pub const ISSUE31_HANDOFF_REASON_ACCOUNT_REVOKED: &str = "reason.omega.handoff_account_revoked";
pub const ISSUE31_HANDOFF_REASON_ACCOUNT_WITHDRAWN: &str =
    "reason.omega.handoff_account_withdrawn";
pub const ISSUE31_HANDOFF_REASON_LANE_CONFLICT: &str = "reason.omega.handoff_account_lane_conflict";
pub const ISSUE31_HANDOFF_REASON_HOST_RESTARTED: &str = "reason.omega.handoff_host_restarted";
pub const ISSUE31_HANDOFF_REASON_DEADLINE_PASSED: &str = "reason.omega.handoff_deadline_passed";

/// Why the host would not open a handoff at all.
///
/// Each of these leaves **no record**, and that is the point: a request the
/// host never admitted is not a handoff that failed. The phone can tell them
/// apart because one produces a row it can watch and the other does not.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Issue31ProviderHandoffError {
    /// The `argumentsRef` was not `arguments.omega.provider_handoff.<provider>`.
    ArgumentsInvalid,
    /// The provider token was empty, oversized, or not public-safe.
    ProviderInvalid,
    /// The ledger already holds as many handoffs as the contract can carry.
    BoundExhausted,
    /// The host tried to write a row its own reader would refuse. This is a
    /// host defect, never a device one, and it is surfaced rather than
    /// swallowed so the defect cannot hide as an empty handoff list.
    Unprojectable(Issue31FullAutoAdjunctError),
}

impl std::fmt::Display for Issue31ProviderHandoffError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::ArgumentsInvalid => {
                formatter.write_str("provider handoff arguments do not name a provider")
            }
            Self::ProviderInvalid => {
                formatter.write_str("provider handoff names an unsafe provider token")
            }
            Self::BoundExhausted => formatter.write_str("provider handoff ledger bound is full"),
            Self::Unprojectable(error) => {
                write!(formatter, "provider handoff would not decode: {error}")
            }
        }
    }
}

impl std::error::Error for Issue31ProviderHandoffError {}

impl Issue31ProviderHandoffError {
    /// The public-safe reason reference a command result carries.
    #[must_use]
    pub const fn reason_ref(self) -> &'static str {
        match self {
            Self::ArgumentsInvalid => "reason.omega.handoff_arguments_invalid",
            Self::ProviderInvalid => "reason.omega.handoff_provider_invalid",
            Self::BoundExhausted => "reason.omega.handoff_bound_exhausted",
            Self::Unprojectable(_) => "reason.omega.handoff_unprojectable",
        }
    }
}

/// One provider account as the host's own roster reports it.
///
/// This is the host observing itself. The lane is carried because omega#42
/// asked for the account-to-lane relation and omega#91 needs it for a specific
/// reason: it is *why a handoff chose what it chose*. A handoff binds to an
/// account, and the account states the lane it serves, so a viewer can follow
/// the choice rather than infer it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Issue31ProviderRosterAccount {
    pub account_ref: String,
    pub provider: String,
    pub lane_ref: String,
    /// The roster's own readiness token: `ready`, `busy`, `revoked`, and so on.
    pub readiness: String,
}

/// One host-owned handoff.
///
/// Every optional field is optional for one reason only: the host has not
/// measured it yet. None of them has a device-supplied alternative.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Issue31ProviderHandoffRecord {
    pub handoff_ref: String,
    pub provider: String,
    pub state: Issue31ProviderHandoffState,
    /// One reading of `now`, taken when the host admitted the request, stamped
    /// once and never restamped.
    ///
    /// `None` means this row was persisted by a build that did not measure it.
    /// It is refused at projection rather than filled in at load: a value
    /// invented now and a value measured then are indistinguishable on the
    /// wire, and a viewer would have no way to know which it was reading.
    #[serde(default)]
    pub requested_at_ms: Option<u64>,
    /// The host's own bound on how long it will hold this open. Host-private:
    /// the contract carries no deadline, so this never reaches the phone.
    #[serde(default)]
    pub deadline_at_ms: Option<u64>,
    /// Set when the host bound this handoff to a concrete account of its own.
    #[serde(default)]
    pub account_ref: Option<String>,
    /// The lane that account served at the moment of binding. Host-private and
    /// checked on every later pass: the projected relation is
    /// `handoff.accountRef` → `account.laneRef`, and this is what makes a lane
    /// that moves underneath a bound handoff a detected conflict instead of a
    /// silent re-mapping.
    #[serde(default)]
    pub lane_ref: Option<String>,
    #[serde(default)]
    pub reason_class: Option<String>,
    #[serde(default)]
    pub outcome_ref: Option<String>,
}

impl Issue31ProviderHandoffRecord {
    #[must_use]
    pub fn is_terminal(&self) -> bool {
        self.state.is_terminal()
    }

    /// The contract row for this record, or `None` when the host cannot state
    /// one. Never a partially-invented row.
    fn contract_row(&self) -> Option<Value> {
        let requested_at_ms = self.requested_at_ms?;
        let mut row = json!({
            "handoffRef": self.handoff_ref,
            "provider": self.provider,
            "state": self.state,
            "requestedAtMs": requested_at_ms,
        });
        let object = row.as_object_mut()?;
        for (field, value) in [
            ("accountRef", self.account_ref.as_ref()),
            ("reasonClass", self.reason_class.as_ref()),
            ("outcomeRef", self.outcome_ref.as_ref()),
        ] {
            if let Some(value) = value {
                object.insert(field.into(), json!(value));
            }
        }
        Some(row)
    }
}

/// What the host can presently say about its handoffs.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Issue31ProviderHandoffProjection {
    /// Contract rows, each already accepted by the reader's own decoder.
    pub rows: Vec<Value>,
    /// Handoff references the host holds and cannot state.
    ///
    /// A non-empty list is a gap the owner must see. It is deliberately not
    /// merged into `rows` with substituted values, and deliberately not
    /// silently dropped either: the count is what turns "the host has nothing
    /// to say" into "the host has something it cannot say".
    pub unavailable: Vec<String>,
}

/// The durable ledger of host-owned provider connection handoffs.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Issue31ProviderHandoffLedger {
    #[serde(default)]
    entries: BTreeMap<String, Issue31ProviderHandoffRecord>,
}

impl Issue31ProviderHandoffLedger {
    #[must_use]
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    #[must_use]
    pub fn get(&self, handoff_ref: &str) -> Option<&Issue31ProviderHandoffRecord> {
        self.entries.get(handoff_ref)
    }

    #[must_use]
    pub fn records(&self) -> impl Iterator<Item = &Issue31ProviderHandoffRecord> {
        self.entries.values()
    }

    /// The reference the host mints for a command, without opening anything.
    ///
    /// Derived from the device's idempotency reference so that a command
    /// replayed after a failed durable commit reopens the same handoff rather
    /// than a second one. The device does not choose the reference: it chooses
    /// an idempotency label, and the host decides what record that maps to.
    #[must_use]
    pub fn handoff_ref_for(idempotency_ref: &str) -> String {
        let digest = format!("{:x}", Sha256::digest(idempotency_ref.as_bytes()));
        format!("handoff.omega.{}", &digest[..24])
    }

    /// The provider a `provider_handoff` command names, if it names one.
    ///
    /// This is the whole input surface of the action.
    pub fn provider_from_arguments_ref(
        arguments_ref: &str,
    ) -> Result<String, Issue31ProviderHandoffError> {
        let provider = arguments_ref
            .strip_prefix(ISSUE31_PROVIDER_HANDOFF_ARGUMENTS_PREFIX)
            .ok_or(Issue31ProviderHandoffError::ArgumentsInvalid)?;
        if provider.is_empty()
            || provider.len() > 32
            || !provider
                .bytes()
                .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
            || !is_public_safe_ref(provider)
        {
            return Err(Issue31ProviderHandoffError::ProviderInvalid);
        }
        Ok(provider.to_string())
    }

    /// Open a handoff for a command the host has already admitted.
    ///
    /// `now_ms` is the caller's single reading of the clock for this command;
    /// both the stamp and the deadline come from it, so a handoff cannot be
    /// requested at one instant and bounded from another.
    ///
    /// Opening is idempotent by handoff reference. A replay after a durable
    /// commit failed finds the record it already made instead of minting a
    /// second one with a later stamp.
    pub fn open(
        &mut self,
        arguments_ref: &str,
        idempotency_ref: &str,
        now_ms: u64,
    ) -> Result<Issue31ProviderHandoffRecord, Issue31ProviderHandoffError> {
        let provider = Self::provider_from_arguments_ref(arguments_ref)?;
        let handoff_ref = Self::handoff_ref_for(idempotency_ref);
        if let Some(existing) = self.entries.get(&handoff_ref) {
            return Ok(existing.clone());
        }
        if self.entries.len() >= MAX_ISSUE31_PROVIDER_HANDOFFS {
            return Err(Issue31ProviderHandoffError::BoundExhausted);
        }
        let record = Issue31ProviderHandoffRecord {
            handoff_ref: handoff_ref.clone(),
            provider,
            state: Issue31ProviderHandoffState::Requested,
            requested_at_ms: Some(now_ms),
            deadline_at_ms: Some(now_ms.saturating_add(ISSUE31_PROVIDER_HANDOFF_DEADLINE_MS)),
            account_ref: None,
            lane_ref: None,
            reason_class: None,
            outcome_ref: None,
        };
        // Unreadable states are unwritable. The row is checked against the
        // reader's own decoder before it enters the ledger, so a handoff the
        // phone would refuse never becomes durable host state in the first
        // place.
        let row = record
            .contract_row()
            .ok_or(Issue31ProviderHandoffError::Unprojectable(
                Issue31FullAutoAdjunctError::InvalidHandoffState,
            ))?;
        decode_issue31_provider_handoff(&row, now_ms)
            .map_err(Issue31ProviderHandoffError::Unprojectable)?;
        self.entries.insert(handoff_ref, record.clone());
        Ok(record)
    }

    /// Move every open handoff on by at most one step, from what the host sees.
    ///
    /// `roster` is the host's own reading of its provider accounts. `None`
    /// means the host has not looked — nothing advances, because a decision
    /// taken against an unread roster would be a guess wearing a measurement's
    /// clothes. `Some(&[])` means the host looked and holds no accounts, which
    /// is a real observation and does move the deadline clock.
    ///
    /// **At most one transition per handoff per pass**, on purpose. Binding and
    /// completing are two separate observations of the roster, and collapsing
    /// them would mean `active` is a state the host passes through without ever
    /// publishing — the phone would see a handoff appear and complete, and
    /// never see it bind.
    ///
    /// Returns how many handoffs moved.
    pub fn advance(
        &mut self,
        roster: Option<&[Issue31ProviderRosterAccount]>,
        now_ms: u64,
    ) -> usize {
        let mut moved = 0;
        for record in self.entries.values_mut() {
            if record.is_terminal() {
                continue;
            }
            // A row the host cannot state is not a row the host may decide
            // about. It is reported unavailable and left exactly as found.
            let Some(_) = record.requested_at_ms else {
                continue;
            };
            if record
                .deadline_at_ms
                .is_some_and(|deadline| now_ms >= deadline)
            {
                record.state = Issue31ProviderHandoffState::Expired;
                record.reason_class = Some(ISSUE31_HANDOFF_REASON_DEADLINE_PASSED.into());
                record.outcome_ref = Some(ISSUE31_HANDOFF_OUTCOME_EXPIRED.into());
                moved += 1;
                continue;
            }
            let Some(roster) = roster else {
                continue;
            };
            if advance_against_roster(record, roster) {
                moved += 1;
            }
        }
        moved
    }

    /// Settle every handoff that was in flight when the host process ended.
    ///
    /// The work behind an open handoff lived in the process that is gone: the
    /// isolated provider home it drove, and the login the owner was completing
    /// there. Nothing survives it, so the honest terminal answer is that the
    /// handoff was interrupted. This only ever under-claims — it cannot report
    /// a connection the host did not make — and it is what stops a restart
    /// leaving the phone with a request that neither resolves nor fails.
    ///
    /// Returns how many handoffs were settled.
    pub fn adopt_after_restart(&mut self) -> usize {
        let mut settled = 0;
        for record in self.entries.values_mut() {
            if record.is_terminal() || record.requested_at_ms.is_none() {
                continue;
            }
            record.state = Issue31ProviderHandoffState::Failed;
            record.reason_class = Some(ISSUE31_HANDOFF_REASON_HOST_RESTARTED.into());
            record.outcome_ref = Some(ISSUE31_HANDOFF_OUTCOME_INTERRUPTED.into());
            settled += 1;
        }
        settled
    }

    /// Handoffs this host holds and can never state.
    ///
    /// Only the permanently unstateable ones: a row whose request time was
    /// never measured. A row that is merely newer than the snapshot being
    /// built is not listed — it is not missing, it is not yet part of that
    /// reading, and it appears in the next one.
    #[must_use]
    pub fn unstateable_refs(&self) -> Vec<String> {
        self.entries
            .values()
            .filter(|record| record.requested_at_ms.is_none())
            .map(|record| record.handoff_ref.clone())
            .collect()
    }

    /// Fold another ledger's rows in. Test support for building a fixture that
    /// shows every lifecycle at once; the production path never merges.
    #[cfg(test)]
    pub(crate) fn merge_for_fixture(&mut self, other: Self) {
        for (handoff_ref, record) in other.entries {
            self.entries.insert(handoff_ref, record);
        }
    }

    /// The contract rows for this ledger, plus what it could not state.
    ///
    /// Every row is routed back through the reader's own decoder against the
    /// exact `generatedAtMs` the projection will carry, so a row stamped after
    /// the reading it would ride in is refused here rather than accepted and
    /// then rejected on the phone.
    #[must_use]
    pub fn projected(&self, generated_at_ms: u64) -> Issue31ProviderHandoffProjection {
        let mut projection = Issue31ProviderHandoffProjection::default();
        for record in self.entries.values() {
            if projection.rows.len() >= MAX_ISSUE31_PROVIDER_HANDOFFS {
                projection.unavailable.push(record.handoff_ref.clone());
                continue;
            }
            let stated = record
                .contract_row()
                .filter(|row| decode_issue31_provider_handoff(row, generated_at_ms).is_ok());
            match stated {
                Some(row) => projection.rows.push(row),
                None => projection.unavailable.push(record.handoff_ref.clone()),
            }
        }
        projection
    }
}

/// One roster-driven step for a single open handoff.
///
/// Returns true when the record moved.
fn advance_against_roster(
    record: &mut Issue31ProviderHandoffRecord,
    roster: &[Issue31ProviderRosterAccount],
) -> bool {
    match record.account_ref.clone() {
        Some(account_ref) => {
            let Some(account) = roster
                .iter()
                .find(|account| account.account_ref == account_ref)
            else {
                // The account this handoff was bound to is gone from the host's
                // own roster. Reporting the handoff as still active would point
                // the phone at a record the host no longer has.
                record.state = Issue31ProviderHandoffState::Failed;
                record.reason_class = Some(ISSUE31_HANDOFF_REASON_ACCOUNT_WITHDRAWN.into());
                record.outcome_ref = Some(ISSUE31_HANDOFF_OUTCOME_FAILED.into());
                return true;
            };
            if record.lane_ref.as_deref() != Some(account.lane_ref.as_str()) {
                // The account now serves a different lane than the one this
                // handoff chose. Quietly adopting the new lane would rewrite
                // the reason the handoff picked this account at all.
                record.state = Issue31ProviderHandoffState::Failed;
                record.reason_class = Some(ISSUE31_HANDOFF_REASON_LANE_CONFLICT.into());
                record.outcome_ref = Some(ISSUE31_HANDOFF_OUTCOME_FAILED.into());
                return true;
            }
            match account.readiness.as_str() {
                "revoked" => {
                    record.state = Issue31ProviderHandoffState::Refused;
                    record.reason_class = Some(ISSUE31_HANDOFF_REASON_ACCOUNT_REVOKED.into());
                    record.outcome_ref = Some(ISSUE31_HANDOFF_OUTCOME_REFUSED.into());
                    true
                }
                "ready" => {
                    record.state = Issue31ProviderHandoffState::Completed;
                    record.outcome_ref = Some(ISSUE31_HANDOFF_OUTCOME_CONNECTED.into());
                    true
                }
                // Connected but not presently usable. The handoff stays bound
                // and open; the deadline is what eventually ends it.
                _ => false,
            }
        }
        None => {
            let mut candidates: Vec<&Issue31ProviderRosterAccount> = roster
                .iter()
                .filter(|account| account.provider == record.provider)
                .collect();
            if candidates.is_empty() {
                // The host holds no account for this provider yet. That is not
                // a refusal: the owner may still be completing the login at the
                // host. The handoff stays `requested` until the host's own
                // deadline decides.
                return false;
            }
            // Deterministic: a ready account first, then by reference, so two
            // hosts reading the same roster bind the same way and a viewer can
            // reproduce the choice.
            candidates.sort_by(|left, right| {
                let left_ready = u8::from(left.readiness != "ready");
                let right_ready = u8::from(right.readiness != "ready");
                left_ready
                    .cmp(&right_ready)
                    .then_with(|| left.account_ref.cmp(&right.account_ref))
            });
            let Some(account) = candidates.first() else {
                return false;
            };
            record.state = Issue31ProviderHandoffState::Active;
            record.account_ref = Some(account.account_ref.clone());
            record.lane_ref = Some(account.lane_ref.clone());
            true
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOW_MS: u64 = 1_785_000_000_000;

    fn account(
        account_ref: &str,
        provider: &str,
        lane_ref: &str,
        readiness: &str,
    ) -> Issue31ProviderRosterAccount {
        Issue31ProviderRosterAccount {
            account_ref: account_ref.into(),
            provider: provider.into(),
            lane_ref: lane_ref.into(),
            readiness: readiness.into(),
        }
    }

    fn opened() -> (Issue31ProviderHandoffLedger, String) {
        let mut ledger = Issue31ProviderHandoffLedger::default();
        let record = ledger
            .open(
                "arguments.omega.provider_handoff.anthropic",
                "idempotency.issue31.handoff:one",
                NOW_MS,
            )
            .expect("the host opens a handoff for an admitted request");
        (ledger, record.handoff_ref)
    }

    #[test]
    fn a_handoff_appears_binds_and_completes_as_three_separate_observations() {
        let (mut ledger, handoff_ref) = opened();
        assert_eq!(
            ledger.get(&handoff_ref).expect("record").state,
            Issue31ProviderHandoffState::Requested,
        );
        assert!(ledger.get(&handoff_ref).expect("record").account_ref.is_none());

        let roster = [account(
            "account.claude.1",
            "anthropic",
            "lane.claude-local",
            "ready",
        )];
        assert_eq!(ledger.advance(Some(&roster), NOW_MS + 1_000), 1);
        let bound = ledger.get(&handoff_ref).expect("record").clone();
        assert_eq!(bound.state, Issue31ProviderHandoffState::Active);
        assert_eq!(bound.account_ref.as_deref(), Some("account.claude.1"));
        // The account-to-lane relation is why this handoff chose this account.
        assert_eq!(bound.lane_ref.as_deref(), Some("lane.claude-local"));
        assert!(bound.outcome_ref.is_none(), "an open handoff has no outcome");

        assert_eq!(ledger.advance(Some(&roster), NOW_MS + 2_000), 1);
        let done = ledger.get(&handoff_ref).expect("record").clone();
        assert_eq!(done.state, Issue31ProviderHandoffState::Completed);
        assert_eq!(
            done.outcome_ref.as_deref(),
            Some(ISSUE31_HANDOFF_OUTCOME_CONNECTED)
        );
        assert!(done.is_terminal());
        assert_eq!(ledger.advance(Some(&roster), NOW_MS + 3_000), 0);
    }

    #[test]
    fn a_failure_is_a_row_and_a_request_that_never_started_is_no_row_at_all() {
        // The distinction the exit turns on. A handoff the host admitted and
        // then could not complete leaves a record the phone can read and act
        // on. A request the host never admitted leaves nothing, and the phone
        // must not be able to confuse the two.
        let mut ledger = Issue31ProviderHandoffLedger::default();
        let refused = ledger.open(
            "arguments.omega.provider_handoff",
            "idempotency.issue31.handoff:never",
            NOW_MS,
        );
        assert_eq!(refused, Err(Issue31ProviderHandoffError::ArgumentsInvalid));
        assert!(ledger.is_empty(), "an unadmitted request leaves no record");
        assert!(ledger.projected(NOW_MS).rows.is_empty());

        let (mut ledger, handoff_ref) = opened();
        let roster = [account(
            "account.claude.1",
            "anthropic",
            "lane.claude-local",
            "revoked",
        )];
        ledger.advance(Some(&roster), NOW_MS + 1_000);
        ledger.advance(Some(&roster), NOW_MS + 2_000);
        let failed = ledger.get(&handoff_ref).expect("record").clone();
        assert_eq!(failed.state, Issue31ProviderHandoffState::Refused);
        assert_eq!(
            failed.reason_class.as_deref(),
            Some(ISSUE31_HANDOFF_REASON_ACCOUNT_REVOKED)
        );
        assert_eq!(
            failed.outcome_ref.as_deref(),
            Some(ISSUE31_HANDOFF_OUTCOME_REFUSED)
        );
        let projection = ledger.projected(NOW_MS + 3_000);
        assert_eq!(projection.rows.len(), 1, "a failure is visible on the wire");
        assert!(projection.unavailable.is_empty());
    }

    #[test]
    fn an_unread_roster_decides_nothing() {
        let (mut ledger, handoff_ref) = opened();
        assert_eq!(ledger.advance(None, NOW_MS + 1_000), 0);
        assert_eq!(
            ledger.get(&handoff_ref).expect("record").state,
            Issue31ProviderHandoffState::Requested,
        );
    }

    #[test]
    fn a_host_with_no_account_for_the_provider_waits_and_then_expires() {
        let (mut ledger, handoff_ref) = opened();
        let roster = [account("account.codex.1", "openai", "lane.codex-local", "ready")];
        assert_eq!(ledger.advance(Some(&roster), NOW_MS + 1_000), 0);
        assert_eq!(
            ledger.get(&handoff_ref).expect("record").state,
            Issue31ProviderHandoffState::Requested,
            "the owner may still be completing the login at the host",
        );
        assert_eq!(
            ledger.advance(
                Some(&roster),
                NOW_MS + ISSUE31_PROVIDER_HANDOFF_DEADLINE_MS
            ),
            1,
        );
        let expired = ledger.get(&handoff_ref).expect("record").clone();
        assert_eq!(expired.state, Issue31ProviderHandoffState::Expired);
        assert_eq!(
            expired.reason_class.as_deref(),
            Some(ISSUE31_HANDOFF_REASON_DEADLINE_PASSED)
        );
        assert_eq!(
            expired.outcome_ref.as_deref(),
            Some(ISSUE31_HANDOFF_OUTCOME_EXPIRED)
        );
    }

    #[test]
    fn a_restart_settles_what_was_in_flight_rather_than_losing_it() {
        let (mut ledger, handoff_ref) = opened();
        let roster = [account(
            "account.claude.1",
            "anthropic",
            "lane.claude-local",
            "busy",
        )];
        ledger.advance(Some(&roster), NOW_MS + 1_000);
        assert_eq!(
            ledger.get(&handoff_ref).expect("record").state,
            Issue31ProviderHandoffState::Active,
        );

        // The exact bytes that cross a restart.
        let serialized = serde_json::to_string(&ledger).expect("the ledger serializes");
        let mut reloaded: Issue31ProviderHandoffLedger =
            serde_json::from_str(&serialized).expect("the ledger survives a restart");
        assert_eq!(reloaded, ledger, "a restart loses nothing");
        assert_eq!(reloaded.adopt_after_restart(), 1);
        let settled = reloaded.get(&handoff_ref).expect("record").clone();
        assert_eq!(settled.state, Issue31ProviderHandoffState::Failed);
        assert_eq!(
            settled.reason_class.as_deref(),
            Some(ISSUE31_HANDOFF_REASON_HOST_RESTARTED)
        );
        assert_eq!(
            settled.outcome_ref.as_deref(),
            Some(ISSUE31_HANDOFF_OUTCOME_INTERRUPTED)
        );
        assert_eq!(
            reloaded.adopt_after_restart(),
            0,
            "a second restart does not re-settle a terminal handoff",
        );
    }

    #[test]
    fn a_restart_never_reports_a_connection_the_host_did_not_make() {
        let (mut ledger, handoff_ref) = opened();
        ledger.adopt_after_restart();
        let settled = ledger.get(&handoff_ref).expect("record").clone();
        assert_ne!(settled.state, Issue31ProviderHandoffState::Completed);
        assert!(settled.account_ref.is_none());
    }

    #[test]
    fn a_row_the_host_never_measured_is_reported_unavailable_rather_than_stamped() {
        // Exactly the shape a build older than `requestedAtMs` persisted.
        let ledger: Issue31ProviderHandoffLedger = serde_json::from_str(
            r#"{"entries":{"handoff.omega.legacy":{"handoffRef":"handoff.omega.legacy",
                "provider":"anthropic","state":"requested"}}}"#,
        )
        .expect("a pre-existing row decodes");
        let projection = ledger.projected(NOW_MS);
        assert!(
            projection.rows.is_empty(),
            "a row with no measured request time is never shown"
        );
        assert_eq!(projection.unavailable, vec!["handoff.omega.legacy"]);
    }

    #[test]
    fn a_row_the_host_never_measured_is_never_advanced_either() {
        let mut ledger: Issue31ProviderHandoffLedger = serde_json::from_str(
            r#"{"entries":{"handoff.omega.legacy":{"handoffRef":"handoff.omega.legacy",
                "provider":"anthropic","state":"requested"}}}"#,
        )
        .expect("a pre-existing row decodes");
        let roster = [account(
            "account.claude.1",
            "anthropic",
            "lane.claude-local",
            "ready",
        )];
        assert_eq!(ledger.advance(Some(&roster), NOW_MS), 0);
        assert_eq!(ledger.adopt_after_restart(), 0);
        assert_eq!(
            ledger.get("handoff.omega.legacy").expect("record").state,
            Issue31ProviderHandoffState::Requested,
        );
    }

    #[test]
    fn a_lane_that_moves_under_a_bound_handoff_is_a_conflict_not_a_re_mapping() {
        let (mut ledger, handoff_ref) = opened();
        let before = [account(
            "account.claude.1",
            "anthropic",
            "lane.claude-local",
            "busy",
        )];
        ledger.advance(Some(&before), NOW_MS + 1_000);
        let after = [account(
            "account.claude.1",
            "anthropic",
            "lane.claude-local-2",
            "ready",
        )];
        assert_eq!(ledger.advance(Some(&after), NOW_MS + 2_000), 1);
        let conflicted = ledger.get(&handoff_ref).expect("record").clone();
        assert_eq!(conflicted.state, Issue31ProviderHandoffState::Failed);
        assert_eq!(
            conflicted.reason_class.as_deref(),
            Some(ISSUE31_HANDOFF_REASON_LANE_CONFLICT)
        );
    }

    #[test]
    fn a_bound_account_that_leaves_the_roster_fails_rather_than_staying_active() {
        let (mut ledger, handoff_ref) = opened();
        let before = [account(
            "account.claude.1",
            "anthropic",
            "lane.claude-local",
            "busy",
        )];
        ledger.advance(Some(&before), NOW_MS + 1_000);
        assert_eq!(ledger.advance(Some(&[]), NOW_MS + 2_000), 1);
        assert_eq!(
            ledger
                .get(&handoff_ref)
                .expect("record")
                .reason_class
                .as_deref(),
            Some(ISSUE31_HANDOFF_REASON_ACCOUNT_WITHDRAWN),
        );
    }

    #[test]
    fn the_device_supplies_no_timestamp_account_lane_or_outcome() {
        // The action's entire input surface is a provider token, so there is no
        // field through which a device could state a host-owned fact.
        assert_eq!(
            Issue31ProviderHandoffLedger::provider_from_arguments_ref(
                "arguments.omega.provider_handoff.anthropic"
            ),
            Ok("anthropic".to_string()),
        );
        for rejected in [
            "arguments.omega.none",
            "arguments.omega.provider_handoff.",
            "arguments.omega.provider_handoff.Anthropic",
            "arguments.omega.provider_handoff.an thropic",
            "arguments.omega.provider_handoff.../../etc",
        ] {
            assert!(
                Issue31ProviderHandoffLedger::provider_from_arguments_ref(rejected).is_err(),
                "expected {rejected:?} to be refused"
            );
        }
    }

    #[test]
    fn a_replayed_command_reopens_the_same_handoff_with_its_first_stamp() {
        let mut ledger = Issue31ProviderHandoffLedger::default();
        let first = ledger
            .open(
                "arguments.omega.provider_handoff.anthropic",
                "idempotency.issue31.handoff:one",
                NOW_MS,
            )
            .expect("open");
        let replay = ledger
            .open(
                "arguments.omega.provider_handoff.anthropic",
                "idempotency.issue31.handoff:one",
                NOW_MS + 60_000,
            )
            .expect("replay");
        assert_eq!(first, replay, "a replay is not a second measurement");
        assert_eq!(ledger.len(), 1);
    }

    #[test]
    fn a_second_request_after_a_terminal_one_opens_a_new_handoff() {
        let (mut ledger, first_ref) = opened();
        ledger.adopt_after_restart();
        let second = ledger
            .open(
                "arguments.omega.provider_handoff.anthropic",
                "idempotency.issue31.handoff:two",
                NOW_MS + 1_000,
            )
            .expect("a fresh ask is a fresh handoff");
        assert_ne!(second.handoff_ref, first_ref);
        assert_eq!(second.state, Issue31ProviderHandoffState::Requested);
        assert_eq!(ledger.len(), 2);
    }

    #[test]
    fn the_ledger_refuses_the_row_after_the_contract_bound() {
        let mut ledger = Issue31ProviderHandoffLedger::default();
        for index in 0..MAX_ISSUE31_PROVIDER_HANDOFFS {
            ledger
                .open(
                    "arguments.omega.provider_handoff.anthropic",
                    &format!("idempotency.issue31.handoff:{index}"),
                    NOW_MS,
                )
                .expect("within the bound");
        }
        assert_eq!(
            ledger.open(
                "arguments.omega.provider_handoff.anthropic",
                "idempotency.issue31.handoff:overflow",
                NOW_MS,
            ),
            Err(Issue31ProviderHandoffError::BoundExhausted),
        );
        assert_eq!(ledger.projected(NOW_MS).rows.len(), MAX_ISSUE31_PROVIDER_HANDOFFS);
    }

    #[test]
    fn every_projected_row_is_one_the_reader_accepts() {
        let (mut ledger, _) = opened();
        let roster = [account(
            "account.claude.1",
            "anthropic",
            "lane.claude-local",
            "ready",
        )];
        ledger.advance(Some(&roster), NOW_MS + 1_000);
        ledger.advance(Some(&roster), NOW_MS + 2_000);
        let generated_at_ms = NOW_MS + 3_000;
        for row in ledger.projected(generated_at_ms).rows {
            decode_issue31_provider_handoff(&row, generated_at_ms)
                .expect("the emitter cannot write what the reader refuses");
        }
    }

    #[test]
    fn a_row_stamped_after_the_reading_it_would_ride_in_is_refused() {
        let (ledger, handoff_ref) = opened();
        let projection = ledger.projected(NOW_MS - 1);
        assert!(projection.rows.is_empty());
        assert_eq!(projection.unavailable, vec![handoff_ref]);
    }

    #[test]
    fn a_projected_row_carries_no_host_private_field() {
        let (mut ledger, _) = opened();
        let roster = [account(
            "account.claude.1",
            "anthropic",
            "lane.claude-local",
            "busy",
        )];
        ledger.advance(Some(&roster), NOW_MS + 1_000);
        let row = ledger.projected(NOW_MS + 2_000).rows.remove(0);
        let object = row.as_object().expect("row");
        // The deadline and the recorded lane are the host's own bookkeeping.
        // The contract has no field for either, and inventing one here would
        // widen the boundary the phone reads.
        assert!(object.get("deadlineAtMs").is_none());
        assert!(object.get("laneRef").is_none());
        assert!(object.get("lane").is_none());
        let mut keys: Vec<&str> = object.keys().map(String::as_str).collect();
        keys.sort_unstable();
        assert_eq!(
            keys,
            vec![
                "accountRef",
                "handoffRef",
                "provider",
                "requestedAtMs",
                "state"
            ]
        );
    }
}

/// The byte-shared lifecycle fixture (omega#91).
///
/// `crates/workroom_receipts/fixtures/openagents.omega.issue31.fullauto.v1.host-produced-handoffs.json`
/// and its byte-identical peer under `packages/sarah/fixtures/issue31-workroom/`
/// are not written by hand. They are what this ledger emits when it is driven
/// through every lifecycle, so a change to the lifecycle that the phone would
/// read differently fails here rather than being discovered on a device.
#[cfg(test)]
mod shared_fixture {
    use super::*;

    pub const FIXTURE_GENERATED_AT_MS: u64 = 1_785_000_600_000;
    pub const FIXTURE_OPENED_AT_MS: u64 = 1_785_000_000_000;

    pub fn fixture_roster() -> Vec<Issue31ProviderRosterAccount> {
        vec![
            Issue31ProviderRosterAccount {
                account_ref: "account.claude.1".into(),
                provider: "anthropic".into(),
                lane_ref: "lane.claude-local".into(),
                readiness: "busy".into(),
            },
            Issue31ProviderRosterAccount {
                account_ref: "account.codex.1".into(),
                provider: "openai".into(),
                lane_ref: "lane.codex-local".into(),
                readiness: "ready".into(),
            },
            Issue31ProviderRosterAccount {
                account_ref: "account.grok.1".into(),
                provider: "xai".into(),
                lane_ref: "lane.grok-local".into(),
                readiness: "revoked".into(),
            },
        ]
    }

    /// Six handoffs, every one produced by the real lifecycle.
    pub fn fixture_ledger() -> Issue31ProviderHandoffLedger {
        let roster = fixture_roster();
        let mut ledger = Issue31ProviderHandoffLedger::default();
        for (provider, idempotency) in [
            ("cohere", "idempotency.issue31.fixture-requested"),
            ("anthropic", "idempotency.issue31.fixture-active"),
            ("openai", "idempotency.issue31.fixture-completed"),
            ("xai", "idempotency.issue31.fixture-refused"),
            ("mistral", "idempotency.issue31.fixture-failed"),
            ("google", "idempotency.issue31.fixture-expired"),
        ] {
            ledger
                .open(
                    &format!("{ISSUE31_PROVIDER_HANDOFF_ARGUMENTS_PREFIX}{provider}"),
                    idempotency,
                    FIXTURE_OPENED_AT_MS,
                )
                .expect("open");
        }
        // One roster observation binds anthropic, openai and xai.
        ledger.advance(Some(&roster), FIXTURE_OPENED_AT_MS + 60_000);
        // A second settles openai (ready) and xai (revoked); anthropic is busy
        // and stays bound and open.
        ledger.advance(Some(&roster), FIXTURE_OPENED_AT_MS + 120_000);
        // The host process ends. `mistral` was still `requested`, so it is
        // settled as interrupted rather than lost -- but so would the others
        // be, which is why the restart sweep runs on a ledger holding only the
        // one still in flight in the real path. Here it is applied to a clone
        // and merged, so each row shows exactly one lifecycle.
        let mut interrupted = Issue31ProviderHandoffLedger::default();
        interrupted
            .open(
                &format!("{ISSUE31_PROVIDER_HANDOFF_ARGUMENTS_PREFIX}mistral"),
                "idempotency.issue31.fixture-failed",
                FIXTURE_OPENED_AT_MS,
            )
            .expect("open");
        interrupted.adopt_after_restart();
        let mut expired = Issue31ProviderHandoffLedger::default();
        expired
            .open(
                &format!("{ISSUE31_PROVIDER_HANDOFF_ARGUMENTS_PREFIX}google"),
                "idempotency.issue31.fixture-expired",
                FIXTURE_OPENED_AT_MS,
            )
            .expect("open");
        expired.advance(
            Some(&roster),
            FIXTURE_OPENED_AT_MS + ISSUE31_PROVIDER_HANDOFF_DEADLINE_MS,
        );
        ledger.merge_for_fixture(interrupted);
        ledger.merge_for_fixture(expired);
        ledger
    }

    pub fn build_fixture_document() -> serde_json::Value {
        let accounts: Vec<serde_json::Value> = fixture_roster()
            .into_iter()
            .map(|account| {
                json!({
                    "accountRef": account.account_ref,
                    "provider": account.provider,
                    "label": match account.provider.as_str() {
                        "anthropic" => "Claude",
                        "openai" => "ChatGPT Personal",
                        _ => "Grok",
                    },
                    "state": account.readiness,
                    "quotaState": if account.readiness == "ready" { "available" } else { "cooling" },
                    "lane": account.lane_ref,
                })
            })
            .collect();
        let handoffs = fixture_ledger().projected(FIXTURE_GENERATED_AT_MS).rows;
        let (_, document) = workroom_receipts::build_issue31_full_auto_adjunct_document(
            "omega.host.local",
            "snapshot.omega.issue31.handoff-lifecycle",
            FIXTURE_GENERATED_AT_MS,
            &json!({ "runs": [] }),
            &json!({ "accounts": accounts }),
            &json!({ "handoffs": handoffs }),
            &[],
        )
        .expect("the ledger's own rows build a readable adjunct");
        document
    }

    const HOST_PRODUCED: &str = include_str!(
        "../../workroom_receipts/fixtures/openagents.omega.issue31.fullauto.v1.host-produced-handoffs.json"
    );

    #[test]
    fn the_shared_fixture_is_exactly_what_the_ledger_emits() {
        let expected: serde_json::Value =
            serde_json::from_str(HOST_PRODUCED).expect("the shared fixture parses");
        assert_eq!(
            build_fixture_document(),
            expected,
            "the lifecycle changed. Re-share the fixture bytes with \
             packages/sarah rather than re-pinning one side.",
        );
    }

    #[test]
    fn the_shared_fixture_shows_every_lifecycle_the_host_can_reach() {
        let document = build_fixture_document();
        let states: Vec<&str> = document
            .get("handoffs")
            .and_then(serde_json::Value::as_array)
            .expect("handoffs")
            .iter()
            .filter_map(|handoff| handoff.get("state").and_then(serde_json::Value::as_str))
            .collect();
        for state in [
            "requested", "active", "completed", "refused", "failed", "expired",
        ] {
            assert!(states.contains(&state), "{state} is unrepresented");
        }
    }

    #[test]
    fn every_terminal_row_in_the_shared_fixture_states_a_host_owned_outcome() {
        let document = build_fixture_document();
        for handoff in document
            .get("handoffs")
            .and_then(serde_json::Value::as_array)
            .expect("handoffs")
        {
            let state = handoff
                .get("state")
                .and_then(serde_json::Value::as_str)
                .expect("state");
            let terminal = matches!(state, "completed" | "refused" | "failed" | "expired");
            assert_eq!(
                handoff.get("outcomeRef").is_some(),
                terminal,
                "an outcome is present exactly when the handoff is terminal: {handoff}",
            );
            if terminal && state != "completed" {
                assert!(
                    handoff.get("reasonClass").is_some(),
                    "a non-successful end must say why: {handoff}",
                );
            }
        }
    }

    #[test]
    fn the_shared_fixture_carries_no_credential_home_or_private_path() {
        // The record carries the fact of a connection, never the connection
        // secret. Asserted on the bytes that actually ship.
        let lowered = HOST_PRODUCED.to_ascii_lowercase();
        for forbidden in [
            "auth.json",
            "bearer ",
            "codex_home",
            "/users/",
            "/home/",
            "~/",
            "sk-",
            "access_token",
            "api_key",
        ] {
            assert!(
                !lowered.contains(forbidden),
                "the shared fixture leaked {forbidden}",
            );
        }
    }
}
