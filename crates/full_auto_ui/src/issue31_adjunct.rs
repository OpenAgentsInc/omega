//! Project the live Full Auto panel sources into the OMEGA-MOB-31-03 headless
//! contract (omega#47).
//!
//! `workroom_receipts::build_issue31_full_auto_adjunct` owns the boundaries.
//! This module owns the adaptation: it takes the exact `omega_effectd`
//! responses the panel already holds — the run details behind omega#41, the
//! capacity record behind omega#42, and the report/receipt pairs behind
//! omega#43 — and puts them into the emitter's input shape.
//!
//! The three panels and the phone therefore read one set of host records. A
//! disagreement between what Omega shows on the desktop and what the owner sees
//! on the phone would have to be a bug in this one adapter rather than a second
//! opinion about the host's state.
//!
//! Nothing here decides what is safe to project. Every value goes through the
//! emitter, which routes it through the decoder, so this file cannot widen the
//! boundary even by mistake.

use serde_json::{Value, json};
use workroom_receipts::{
    Issue31CommandStateInput, Issue31FullAutoAdjunct, Issue31FullAutoAdjunctError,
    Issue31HostAdjunct, Issue31HostAdjunctError, Issue31HostProjectionInput, Issue31HostSources,
    Issue31ObservedGap, Issue31RoleInput, MAX_ISSUE31_PROJECTION_REFS, ProjectionFreshness,
    build_issue31_full_auto_adjunct, build_issue31_host_adjunct,
};

use crate::provider_roster::parse_provider_accounts;

/// Why a live host state could not be projected at all.
///
/// There is deliberately no partial success. A projection that dropped the one
/// run the owner is watching, while showing the others, would be the most
/// misleading result available.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Issue31FullAutoProjectionError {
    /// The host reported a run with no objective, lane, or reference.
    IncompleteRunRecord,
    /// The host never recorded when this run began, so its exact unattended
    /// duration is unknown. Showing zero would be a claim nothing supports.
    UnattendedDurationUnknown,
    /// The assembled projection was refused by the contract.
    Contract(Issue31FullAutoAdjunctError),
}

impl std::fmt::Display for Issue31FullAutoProjectionError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::IncompleteRunRecord => {
                formatter.write_str("full auto run record is missing a required public field")
            }
            Self::UnattendedDurationUnknown => {
                formatter.write_str("full auto run has no host-recorded start time")
            }
            Self::Contract(error) => write!(formatter, "{error}"),
        }
    }
}

impl std::error::Error for Issue31FullAutoProjectionError {}

/// The controls this host accepts for a run in a given lifecycle state.
///
/// The host reported the state; these are the mutations `FullAutoPanel` itself
/// sends for it. A run only ever carries controls the same host would honour,
/// so the phone cannot present a button whose completion can never arrive. A
/// host that declares its own `permittedControls` overrides this entirely.
fn controls_for_state(state: &str) -> &'static [&'static str] {
    match state {
        "running" | "retrying" | "stalled" => &["pause", "stop"],
        "paused" => &["resume", "stop"],
        "queued" => &["stop"],
        // `pausing` and `stopping` already have a mutation in flight, and every
        // remaining state is terminal.
        _ => &[],
    }
}

/// The host's own start time for a run, in milliseconds.
///
/// Only a numeric field the host recorded is accepted. Re-deriving a start from
/// a formatted display string would make the duration a parse of Omega's UI
/// rather than a measurement of the run.
fn started_at_ms(run: &Value) -> Option<u64> {
    for field in ["startedAtMs", "startedAtEpochMs", "createdAtMs"] {
        if let Some(value) = run.get(field).and_then(Value::as_u64) {
            return Some(value);
        }
    }
    None
}

fn project_run(run: &Value, host_generation: u64) -> Result<Value, Issue31FullAutoProjectionError> {
    let run_ref = run
        .get("runRef")
        .and_then(Value::as_str)
        .ok_or(Issue31FullAutoProjectionError::IncompleteRunRecord)?;
    let objective = run
        .get("objective")
        .and_then(Value::as_str)
        .filter(|objective| !objective.trim().is_empty())
        .ok_or(Issue31FullAutoProjectionError::IncompleteRunRecord)?;
    // A run with no lane cannot be related to any provider account, and
    // omega#47 requires the account-to-lane relation to be explicit.
    let lane_ref = run
        .get("laneRef")
        .or_else(|| run.get("lane"))
        .and_then(Value::as_str)
        .ok_or(Issue31FullAutoProjectionError::IncompleteRunRecord)?;
    let state = run
        .get("state")
        .and_then(Value::as_str)
        .ok_or(Issue31FullAutoProjectionError::IncompleteRunRecord)?;
    let started_at_ms =
        started_at_ms(run).ok_or(Issue31FullAutoProjectionError::UnattendedDurationUnknown)?;

    let permitted: Vec<&str> = match run.get("permittedControls").and_then(Value::as_array) {
        Some(declared) => declared.iter().filter_map(Value::as_str).collect(),
        None => controls_for_state(state).to_vec(),
    };

    let mut projected = json!({
        "runRef": run_ref,
        "objective": objective,
        "laneRef": lane_ref,
        "state": state,
        "generation": host_generation,
        "startedAtMs": started_at_ms,
        "permittedControls": permitted,
    });
    let object = projected
        .as_object_mut()
        .ok_or(Issue31FullAutoProjectionError::IncompleteRunRecord)?;
    for field in ["liveWorkRef", "terminalReasonRef"] {
        if let Some(value) = run.get(field)
            && !value.is_null()
        {
            object.insert(field.into(), value.clone());
        }
    }
    Ok(projected)
}

/// The live host state the three Full Auto panels read.
pub struct Issue31FullAutoLiveSources<'a> {
    /// The `host.v1` snapshot this detail projection is bound to. A phone that
    /// holds a different snapshot renders `snapshot_mismatch` rather than runs.
    pub host_ref: &'a str,
    pub snapshot_ref: &'a str,
    pub generated_at_ms: u64,
    /// The supervised `omega_effectd` generation. Every control is bound to it,
    /// so a control minted before a host restart is refused afterwards rather
    /// than replayed against a run the owner can no longer see.
    pub host_generation: u64,
    /// One `get_run` record per projected run.
    pub run_details: &'a [Value],
    /// The `get_capacity` record the provider roster (omega#42) parses.
    pub capacity: &'a Value,
    /// Host-owned provider connection handoff records.
    pub handoffs: &'a [Value],
    /// One `(get_report, get_receipt)` pair per run with evidence.
    pub evidence: &'a [(Value, Value)],
}

/// Build the headless contract from live host state.
pub fn project_issue31_full_auto_adjunct(
    sources: &Issue31FullAutoLiveSources<'_>,
) -> Result<Issue31FullAutoAdjunct, Issue31FullAutoProjectionError> {
    let runs = sources
        .run_details
        .iter()
        .map(|run| project_run(run, sources.host_generation))
        .collect::<Result<Vec<_>, _>>()?;

    // Routed through the same parser the desktop roster renders, so the panel
    // and the phone cannot disagree about which accounts exist or which lane
    // each one serves.
    let accounts: Vec<Value> = parse_provider_accounts(sources.capacity)
        .into_iter()
        .map(|account| {
            json!({
                "accountRef": account.account_ref,
                "provider": account.provider,
                "label": account.label,
                "state": account.readiness,
                "quotaState": account.quota,
                "lane": account.lane,
            })
        })
        .collect();

    build_issue31_full_auto_adjunct(
        sources.host_ref,
        sources.snapshot_ref,
        sources.generated_at_ms,
        &json!({ "runs": runs }),
        &json!({ "accounts": accounts }),
        &json!({ "handoffs": sources.handoffs }),
        sources.evidence,
    )
    .map_err(Issue31FullAutoProjectionError::Contract)
}

// ---------------------------------------------------------------------------
// The `host.v1` snapshot the detail projection is published beside
// ---------------------------------------------------------------------------

/// The one capability the three Full Auto panels do not hold.
///
/// Connection identity is the host announcement and the owner device grant.
/// Its records live with the pairing/grant surface, so the caller supplies
/// them; everything else in the snapshot is derived from the exact same live
/// values the detail projection uses.
pub struct Issue31HostIdentitySource<'a> {
    pub source_ref: &'a str,
    pub observed_at_ms: u64,
    /// The grant that makes the reader's owner role active. Absent means this
    /// host cannot presently state the reader's role, which is projected as an
    /// unknown role with no permitted actions rather than as a working one.
    pub owner_grant_ref: Option<&'a str>,
    pub record_refs: &'a [&'a str],
    pub permitted_action_refs: &'a [&'a str],
}

/// Both omega#47 documents, produced from one reading of the host.
///
/// The contract says the detail projection is published "beside the `host.v1`
/// snapshot". Returning them together, built from a single
/// `Issue31FullAutoLiveSources`, is what makes "beside" a fact rather than a
/// convention: they carry the same `hostRef`, the same `snapshotRef`, and the
/// same `generatedAtMs` because there is no code path that could give them
/// different ones.
#[derive(Debug)]
pub struct Issue31HostPublication {
    pub host: Issue31HostAdjunct,
    pub detail: Issue31FullAutoAdjunct,
}

/// Why a live host state could not be published as a `host.v1` snapshot.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Issue31HostProjectionError {
    /// The snapshot itself was refused by the contract.
    Host(Issue31HostAdjunctError),
    /// The detail projection beside it was refused. Neither is published: a
    /// snapshot advertising Full Auto records the owner cannot then open would
    /// claim more than the host can show.
    Detail(Issue31FullAutoProjectionError),
}

impl std::fmt::Display for Issue31HostProjectionError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Host(error) => write!(formatter, "{error}"),
            Self::Detail(error) => write!(formatter, "{error}"),
        }
    }
}

impl std::error::Error for Issue31HostProjectionError {}

/// Truncate a record-reference list to the contract bound, reporting whether
/// anything was dropped.
///
/// A host with more records than one snapshot can cite is a real situation, and
/// the honest answer is `partial` — "this is not all of them" — rather than a
/// refusal that would hide every record, or a silent truncation that would
/// claim completeness the snapshot does not have.
fn bounded_refs(refs: Vec<String>) -> (Vec<String>, Issue31ObservedGap) {
    if refs.len() > MAX_ISSUE31_PROJECTION_REFS {
        (
            refs.into_iter().take(MAX_ISSUE31_PROJECTION_REFS).collect(),
            Issue31ObservedGap::Partial,
        )
    } else {
        (refs, Issue31ObservedGap::Complete)
    }
}

/// Publish the `host.v1` snapshot and the detail projection that sits beside
/// it, from one reading of live host state.
pub fn publish_issue31_host_snapshot(
    sources: &Issue31FullAutoLiveSources<'_>,
    identity: &Issue31HostIdentitySource<'_>,
) -> Result<Issue31HostPublication, Issue31HostProjectionError> {
    // Built first: a snapshot must never advertise records whose detail the
    // host would refuse to project.
    let detail =
        project_issue31_full_auto_adjunct(sources).map_err(Issue31HostProjectionError::Detail)?;

    let run_refs: Vec<String> = detail
        .runs
        .iter()
        .map(|run| format!("record.full-auto.run.{}", run.run_ref.as_str()))
        .collect();
    let account_refs: Vec<String> = detail
        .accounts
        .iter()
        .map(|account| format!("record.provider.account.{}", account.account_ref.as_str()))
        .collect();
    let evidence_refs: Vec<String> = detail
        .runs
        .iter()
        .take(detail.evidence.len())
        .map(|run| format!("record.evidence.chain.{}", run.run_ref.as_str()))
        .collect();

    let (run_refs, run_gap) = bounded_refs(run_refs);
    let (account_refs, account_gap) = bounded_refs(account_refs);
    let (evidence_refs, evidence_gap) = bounded_refs(evidence_refs);
    let run_refs: Vec<&str> = run_refs.iter().map(String::as_str).collect();
    let account_refs: Vec<&str> = account_refs.iter().map(String::as_str).collect();
    let evidence_refs: Vec<&str> = evidence_refs.iter().map(String::as_str).collect();

    // A run this host is willing to control is one whose controls it already
    // minted. Advertising an action for a snapshot with no controllable run
    // would offer the owner a button the host would not honour.
    let run_actions: Vec<&str> = if detail.runs.iter().any(|run| !run.controls.is_empty()) {
        vec![
            "action.full-auto.pause",
            "action.full-auto.resume",
            "action.full-auto.stop",
        ]
    } else {
        Vec::new()
    };

    let owner = match identity.owner_grant_ref {
        Some(grant_ref) => Issue31RoleInput::Active {
            kind: workroom_receipts::Issue31RoleKind::Owner,
            grant_ref,
        },
        None => Issue31RoleInput::Unknown {
            kind: workroom_receipts::Issue31RoleKind::Owner,
        },
    };
    // An unknown role carries no permitted actions at all — that is enforced at
    // decode, and honouring it here keeps the emitter from building the
    // violation and then discovering it.
    let granted = identity.owner_grant_ref.is_some();
    let empty: &[&str] = &[];

    let host_snapshot = build_issue31_host_adjunct(
        sources.host_ref,
        sources.snapshot_ref,
        sources.generated_at_ms,
        &Issue31HostSources {
            connection_identity: Issue31HostProjectionInput::Observed {
                source_ref: identity.source_ref,
                observed_at_ms: identity.observed_at_ms,
                freshness: ProjectionFreshness::Current,
                gap: if identity.record_refs.is_empty() {
                    Issue31ObservedGap::Partial
                } else {
                    Issue31ObservedGap::Complete
                },
                role: owner,
                record_refs: identity.record_refs,
                permitted_action_refs: if granted {
                    identity.permitted_action_refs
                } else {
                    empty
                },
                command_state: Issue31CommandStateInput::Idle,
            },
            full_auto_runs: Issue31HostProjectionInput::Observed {
                source_ref: "source.omega.full-auto-registry",
                observed_at_ms: sources.generated_at_ms,
                freshness: ProjectionFreshness::Current,
                gap: run_gap,
                role: owner,
                record_refs: &run_refs,
                permitted_action_refs: if granted { &run_actions } else { empty },
                command_state: Issue31CommandStateInput::Idle,
            },
            provider_accounts: Issue31HostProjectionInput::Observed {
                source_ref: "source.omega.provider-roster",
                observed_at_ms: sources.generated_at_ms,
                freshness: ProjectionFreshness::Current,
                gap: account_gap,
                role: owner,
                record_refs: &account_refs,
                // Provider login is host-owned. The phone may only ASK for a
                // handoff; it can never carry the login itself.
                permitted_action_refs: if granted {
                    &["action.provider.request-connect-handoff"]
                } else {
                    empty
                },
                command_state: Issue31CommandStateInput::Idle,
            },
            evidence_chain: Issue31HostProjectionInput::Observed {
                source_ref: "source.omega.evidence-inspector",
                observed_at_ms: sources.generated_at_ms,
                freshness: ProjectionFreshness::Current,
                gap: evidence_gap,
                role: owner,
                record_refs: &evidence_refs,
                // Evidence is read, never commanded from the phone.
                permitted_action_refs: empty,
                command_state: Issue31CommandStateInput::Idle,
            },
        },
    )
    .map_err(Issue31HostProjectionError::Host)?;

    Ok(Issue31HostPublication {
        host: host_snapshot,
        detail,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use workroom_receipts::{
        Issue31EvidenceChain, Issue31EvidenceUnavailableReason, Issue31FullAutoLifecycle,
    };

    const NOW: u64 = 1_784_894_400_000;
    const HOST_GENERATION: u64 = 19;

    fn run_detail() -> Value {
        json!({
            "runRef": "run.full-auto.run-01",
            "title": "Mobile workroom",
            "objective": "Finish the issue 31 mobile workroom.",
            "doneCondition": "Every exit holds with evidence.",
            "lane": "codex-local",
            "state": "running",
            "startedAtMs": NOW - 5_400_000,
            "liveWorkRef": "work.run-01.unit-14"
        })
    }

    fn capacity() -> Value {
        json!({
            "lanes": [{"lane": "codex-local", "state": "available", "activeRuns": 1}],
            "accounts": [
                {"accountRef":"account.codex.1","provider":"openai","label":"ChatGPT Personal","state":"busy","quotaState":"available","lane":"codex-local"},
                {"accountRef":"account.claude.1","provider":"anthropic","label":"Claude","state":"ready","quotaState":"available","lane":"claude-local"}
            ]
        })
    }

    fn project(
        run_details: &[Value],
        capacity: &Value,
        handoffs: &[Value],
        evidence: &[(Value, Value)],
    ) -> Result<Issue31FullAutoAdjunct, Issue31FullAutoProjectionError> {
        project_issue31_full_auto_adjunct(&Issue31FullAutoLiveSources {
            host_ref: "host.omega.device-alpha",
            snapshot_ref: "snapshot.omega.issue31.000042",
            generated_at_ms: NOW,
            host_generation: HOST_GENERATION,
            run_details,
            capacity,
            handoffs,
            evidence,
        })
    }

    #[test]
    fn projects_a_live_run_with_its_exact_host_measured_duration() {
        let adjunct = project(&[run_detail()], &capacity(), &[], &[]).expect("projects");
        assert_eq!(adjunct.runs.len(), 1);
        let run = &adjunct.runs[0];
        assert_eq!(run.lifecycle, Issue31FullAutoLifecycle::Running);
        assert_eq!(run.unattended_ms, 5_400_000);
        assert_eq!(run.lane_ref.as_str(), "codex-local");
    }

    #[test]
    fn binds_every_control_to_the_live_host_generation() {
        let adjunct = project(&[run_detail()], &capacity(), &[], &[]).expect("projects");
        let run = &adjunct.runs[0];
        assert_eq!(run.controls.len(), 2);
        for control in &run.controls {
            assert_eq!(control.run_generation, HOST_GENERATION);
            assert_eq!(control.run_generation, run.generation);
            assert!(control.idempotency_ref.as_str().contains("run.full-auto"));
        }
    }

    #[test]
    fn a_paused_run_offers_resume_and_a_finished_run_offers_nothing() {
        let mut paused = run_detail();
        paused["state"] = json!("paused");
        let adjunct = project(&[paused], &capacity(), &[], &[]).expect("projects");
        let kinds: Vec<_> = adjunct.runs[0]
            .controls
            .iter()
            .map(|control| control.kind)
            .collect();
        assert_eq!(kinds.len(), 2);

        let mut finished = run_detail();
        finished["state"] = json!("succeeded");
        finished
            .as_object_mut()
            .expect("object")
            .remove("liveWorkRef");
        let adjunct = project(&[finished], &capacity(), &[], &[]).expect("projects");
        assert!(adjunct.runs[0].controls.is_empty());
        assert!(adjunct.runs[0].terminal_reason_ref.is_some());
    }

    #[test]
    fn the_phone_and_the_roster_read_the_same_accounts() {
        let adjunct = project(&[run_detail()], &capacity(), &[], &[]).expect("projects");
        let projected: Vec<(&str, &str)> = adjunct
            .accounts
            .iter()
            .map(|account| (account.account_ref.as_str(), account.lane_ref.as_str()))
            .collect();
        let roster: Vec<(String, String)> = parse_provider_accounts(&capacity())
            .into_iter()
            .map(|row| (row.account_ref, row.lane))
            .collect();
        assert_eq!(projected.len(), roster.len());
        for (index, (account_ref, lane_ref)) in projected.iter().enumerate() {
            assert_eq!(*account_ref, roster[index].0);
            assert_eq!(*lane_ref, roster[index].1);
        }
    }

    #[test]
    fn a_run_with_no_host_recorded_start_is_refused_rather_than_shown_as_new() {
        let mut run = run_detail();
        run.as_object_mut().expect("object").remove("startedAtMs");
        assert_eq!(
            project(&[run], &capacity(), &[], &[]).expect_err("must refuse"),
            Issue31FullAutoProjectionError::UnattendedDurationUnknown
        );
    }

    #[test]
    fn a_run_with_no_lane_cannot_be_projected_against_provider_accounts() {
        let mut run = run_detail();
        run.as_object_mut().expect("object").remove("lane");
        assert_eq!(
            project(&[run], &capacity(), &[], &[]).expect_err("must refuse"),
            Issue31FullAutoProjectionError::IncompleteRunRecord
        );
    }

    #[test]
    fn a_credential_shaped_objective_cannot_leave_the_host() {
        let mut run = run_detail();
        run["objective"] = json!("Rotate the key in ~/.codex/auth.json");
        let error = project(&[run], &capacity(), &[], &[]).expect_err("must refuse");
        assert_eq!(
            error,
            Issue31FullAutoProjectionError::Contract(Issue31FullAutoAdjunctError::UnsafeText)
        );
        let rendered = error.to_string();
        assert!(!rendered.contains("auth.json"));
    }

    #[test]
    fn a_phone_initiated_handoff_reports_only_its_host_owned_outcome() {
        let handoffs = [json!({
            "handoffRef": "handoff.codex.2",
            "provider": "openai",
            "state": "refused",
            "requestedAtMs": NOW - 120_000,
            "reasonClass": "reason.handoff.owner_declined",
            "outcomeRef": "outcome.handoff.declined",
            // Host-side truth that must never cross to the phone.
            "isolatedHome": "/Users/owner/.pylon/accounts/codex/codex-2",
            "authorizationResponse": "Bearer sk-live-0000"
        })];
        let adjunct = project(&[run_detail()], &capacity(), &handoffs, &[]).expect("projects");
        assert_eq!(adjunct.handoffs.len(), 1);
        let handoff = &adjunct.handoffs[0];
        assert_eq!(
            handoff.outcome_ref.as_ref().map(|value| value.as_str()),
            Some("outcome.handoff.declined")
        );
        // A refused handoff stays unattributed rather than appearing against a
        // working account of the same provider.
        assert!(handoff.account_ref.is_none());
    }

    // -----------------------------------------------------------------
    // The `host.v1` snapshot the detail projection is published beside.
    // -----------------------------------------------------------------

    fn identity() -> Issue31HostIdentitySource<'static> {
        Issue31HostIdentitySource {
            source_ref: "source.omega.identity-binding",
            observed_at_ms: NOW - 1_000,
            owner_grant_ref: Some("grant.omega.mobile.owner-01"),
            record_refs: &[
                "record.omega.host-announcement.01",
                "record.omega.owner-binding.01",
            ],
            permitted_action_refs: &[
                "action.omega.device.pair",
                "action.omega.device.renew",
                "action.omega.device.revoke",
            ],
        }
    }

    fn publish(
        run_details: &[Value],
        capacity: &Value,
        identity: &Issue31HostIdentitySource<'_>,
    ) -> Result<Issue31HostPublication, Issue31HostProjectionError> {
        publish_issue31_host_snapshot(
            &Issue31FullAutoLiveSources {
                host_ref: "host.omega.device-alpha",
                snapshot_ref: "snapshot.omega.issue31.000042",
                generated_at_ms: NOW,
                host_generation: HOST_GENERATION,
                run_details,
                capacity,
                handoffs: &[],
                evidence: &[],
            },
            identity,
        )
    }

    #[test]
    fn the_snapshot_and_the_detail_beside_it_cannot_describe_different_hosts() {
        let published = publish(&[run_detail()], &capacity(), &identity()).expect("publishes");
        assert_eq!(
            published.host.host_ref.as_str(),
            published.detail.host_ref.as_str()
        );
        assert_eq!(
            published.host.snapshot_ref.as_str(),
            published.detail.snapshot_ref.as_str()
        );
        assert_eq!(
            published.host.generated_at_ms,
            published.detail.generated_at_ms
        );
    }

    #[test]
    fn the_snapshot_cites_exactly_the_runs_and_accounts_the_detail_carries() {
        let published = publish(&[run_detail()], &capacity(), &identity()).expect("publishes");
        let runs = published
            .host
            .projections
            .iter()
            .find(|projection| {
                projection.capability == workroom_receipts::Issue31ProjectionCapability::FullAutoRuns
            })
            .expect("full auto runs projection");
        assert_eq!(runs.record_refs.len(), published.detail.runs.len());
        assert!(
            runs.record_refs[0]
                .as_str()
                .ends_with(published.detail.runs[0].run_ref.as_str())
        );

        let accounts = published
            .host
            .projections
            .iter()
            .find(|projection| {
                projection.capability
                    == workroom_receipts::Issue31ProjectionCapability::ProviderAccounts
            })
            .expect("provider accounts projection");
        assert_eq!(accounts.record_refs.len(), published.detail.accounts.len());
    }

    /// A snapshot must never advertise Full Auto records the host would then
    /// refuse to detail. Publishing neither is the honest outcome.
    #[test]
    fn a_run_the_detail_would_refuse_blocks_the_whole_publication() {
        let mut run = run_detail();
        run.as_object_mut().expect("object").remove("startedAtMs");
        let error = publish(&[run], &capacity(), &identity()).expect_err("must refuse");
        assert_eq!(
            error,
            Issue31HostProjectionError::Detail(
                Issue31FullAutoProjectionError::UnattendedDurationUnknown
            )
        );
    }

    /// An ungranted reader is not a reader with fewer buttons; it is a reader
    /// whose role this host cannot state, and it gets none.
    #[test]
    fn a_host_that_cannot_state_the_readers_role_offers_no_actions_at_all() {
        let mut ungranted = identity();
        ungranted.owner_grant_ref = None;
        let published = publish(&[run_detail()], &capacity(), &ungranted).expect("publishes");
        for projection in &published.host.projections {
            assert!(
                projection.permitted_action_refs.is_empty(),
                "{:?} offered an action without a grant",
                projection.capability
            );
            assert_eq!(
                projection.command_state,
                workroom_receipts::Issue31CommandState::Idle
            );
        }
    }

    /// A finished run has no controls, so the snapshot must not advertise a
    /// Full Auto action the host would refuse to honour.
    #[test]
    fn a_snapshot_of_only_finished_runs_advertises_no_run_controls() {
        let mut finished = run_detail();
        finished["state"] = json!("succeeded");
        finished
            .as_object_mut()
            .expect("object")
            .remove("liveWorkRef");
        let published = publish(&[finished], &capacity(), &identity()).expect("publishes");
        let runs = published
            .host
            .projections
            .iter()
            .find(|projection| {
                projection.capability == workroom_receipts::Issue31ProjectionCapability::FullAutoRuns
            })
            .expect("full auto runs projection");
        assert!(runs.permitted_action_refs.is_empty());
        assert!(published.detail.runs[0].controls.is_empty());
    }

    /// More records than one snapshot can cite is `partial` — "this is not all
    /// of them" — not a refusal that would hide every record.
    #[test]
    fn more_accounts_than_the_reference_bound_is_partial_rather_than_a_refusal() {
        let accounts: Vec<Value> = (0..MAX_ISSUE31_PROJECTION_REFS + 4)
            .map(|index| {
                json!({
                    "accountRef": format!("account.codex.{index}"),
                    "provider": "openai",
                    "label": format!("ChatGPT {index}"),
                    "state": "ready",
                    "quotaState": "available",
                    "lane": "codex-local"
                })
            })
            .collect();
        let capacity = json!({
            "lanes": [{"lane": "codex-local", "state": "available", "activeRuns": 1}],
            "accounts": accounts
        });
        let published = publish(&[run_detail()], &capacity, &identity()).expect("publishes");
        let projection = published
            .host
            .projections
            .iter()
            .find(|projection| {
                projection.capability
                    == workroom_receipts::Issue31ProjectionCapability::ProviderAccounts
            })
            .expect("provider accounts projection");
        assert_eq!(
            projection.record_refs.len(),
            MAX_ISSUE31_PROJECTION_REFS,
            "cites as many as the contract admits"
        );
        assert_eq!(projection.gap, workroom_receipts::Issue31Gap::Partial);
    }

    /// The provider boundary reaches the snapshot too.
    #[test]
    fn a_private_identity_record_cannot_be_published_and_is_not_echoed() {
        let mut leaking = identity();
        leaking.record_refs = &["/Users/owner/.codex/auth.json"];
        let error = publish(&[run_detail()], &capacity(), &leaking).expect_err("must refuse");
        assert_eq!(
            error,
            Issue31HostProjectionError::Host(Issue31HostAdjunctError::UnsafeReference)
        );
        assert!(!error.to_string().contains("/Users/"));
        assert!(!error.to_string().contains("auth.json"));
    }

    // -----------------------------------------------------------------
    // The live-engine walk.
    //
    // These four fixtures are not hand-written. They are the EXACT bytes a
    // running `omega-effectd` returned for `get_run`, `get_capacity`,
    // `get_report`, and `get_receipt` after a real `start`, captured on
    // 2026-07-25 against the engine at `omega-effectd-v0.1.0-rc.8` plus the
    // host-recorded numeric run start (openagents `startedAtMs`). Projecting
    // them here is what stops this adapter from agreeing only with fixtures
    // that were written to make it agree.
    // -----------------------------------------------------------------

    const LIVE_RUN: &str = include_str!("../fixtures/live-omega-effectd.get_run.json");
    const LIVE_CAPACITY: &str = include_str!("../fixtures/live-omega-effectd.get_capacity.json");
    const LIVE_REPORT: &str = include_str!("../fixtures/live-omega-effectd.get_report.json");
    const LIVE_RECEIPT: &str = include_str!("../fixtures/live-omega-effectd.get_receipt.json");
    /// The host's own start in the captured `get_run`.
    const LIVE_STARTED_AT_MS: u64 = 1_785_001_886_429;

    fn live(name: &str, raw: &str) -> Value {
        serde_json::from_str(raw).unwrap_or_else(|error| panic!("live {name} parses: {error}"))
    }

    /// The gap this issue stayed open on. A live host now records a numeric
    /// run start, so its exact unattended duration is a measurement rather
    /// than a refusal.
    #[test]
    fn a_live_host_run_projects_its_exact_unattended_duration() {
        let run = live("get_run", LIVE_RUN);
        assert_eq!(
            run.get("startedAtMs").and_then(Value::as_u64),
            Some(LIVE_STARTED_AT_MS),
            "the live engine records a numeric run start"
        );

        let generated_at_ms = LIVE_STARTED_AT_MS + 5_400_000;
        let adjunct = project_issue31_full_auto_adjunct(&Issue31FullAutoLiveSources {
            host_ref: "host.omega.device-alpha",
            snapshot_ref: "snapshot.omega.issue31.live-walk",
            generated_at_ms,
            host_generation: HOST_GENERATION,
            run_details: &[run],
            capacity: &live("get_capacity", LIVE_CAPACITY),
            handoffs: &[],
            evidence: &[],
        })
        .expect("a live host run projects");

        assert_eq!(adjunct.runs.len(), 1);
        assert_eq!(adjunct.runs[0].unattended_ms, 5_400_000);
        assert_eq!(adjunct.runs[0].lifecycle, Issue31FullAutoLifecycle::Running);
        assert_eq!(adjunct.runs[0].lane_ref.as_str(), "codex-local");
        // The panel's roster and the phone read the same live capacity record.
        assert_eq!(
            adjunct.accounts.len(),
            parse_provider_accounts(&live("get_capacity", LIVE_CAPACITY)).len()
        );
    }

    /// The formatted `updatedAt` a live run carries is still exactly that, and
    /// the duration above owes nothing to it. If a future change ever derived
    /// the unattended duration from this string, this test is where it shows.
    #[test]
    fn the_live_duration_owes_nothing_to_the_formatted_timestamp() {
        let mut run = live("get_run", LIVE_RUN);
        assert!(run.get("updatedAt").and_then(Value::as_str).is_some());
        run["updatedAt"] = json!("not a timestamp at all");
        let generated_at_ms = LIVE_STARTED_AT_MS + 90_000;
        let adjunct = project_issue31_full_auto_adjunct(&Issue31FullAutoLiveSources {
            host_ref: "host.omega.device-alpha",
            snapshot_ref: "snapshot.omega.issue31.live-walk",
            generated_at_ms,
            host_generation: HOST_GENERATION,
            run_details: &[run],
            capacity: &live("get_capacity", LIVE_CAPACITY),
            handoffs: &[],
            evidence: &[],
        })
        .expect("projects");
        assert_eq!(adjunct.runs[0].unattended_ms, 90_000);
    }

    /// The walk's honest stopping point, pinned rather than described.
    ///
    /// A live `omega-effectd` `get_report` / `get_receipt` pair does NOT carry
    /// the omega#43 chain: the report has no `evidence` block naming the
    /// objective, turn, change, project generation, verification, test outcome,
    /// and host execution, and the receipt has no `decisionRef` or
    /// `authorityReceiptRef`. It carries digests and lifecycle history instead.
    ///
    /// So a viewer cannot yet follow one finished unit from objective through
    /// authority receipt against a live host — not because the contract fails,
    /// but because the host does not emit those hops. The contract's answer is
    /// `unavailable`, which is the correct one: it claims no partial proof. If
    /// the engine ever starts emitting the chain, this test goes red and is the
    /// place to record that the walk now completes.
    #[test]
    fn a_live_report_and_receipt_do_not_yet_form_an_authority_chain() {
        let report = live("get_report", LIVE_REPORT);
        let receipt = live("get_receipt", LIVE_RECEIPT);
        assert!(
            report.get("evidence").is_none(),
            "a live report carrying an evidence block would complete this walk"
        );
        assert!(receipt.get("authorityReceiptRef").is_none());
        assert!(receipt.get("decisionRef").is_none());

        let adjunct = project_issue31_full_auto_adjunct(&Issue31FullAutoLiveSources {
            host_ref: "host.omega.device-alpha",
            snapshot_ref: "snapshot.omega.issue31.live-walk",
            generated_at_ms: LIVE_STARTED_AT_MS + 60_000,
            host_generation: HOST_GENERATION,
            run_details: &[live("get_run", LIVE_RUN)],
            capacity: &live("get_capacity", LIVE_CAPACITY),
            handoffs: &[],
            evidence: &[(report, receipt)],
        })
        .expect("the run still projects beside an unavailable chain");

        match &adjunct.evidence[0] {
            Issue31EvidenceChain::Unavailable { reason, .. } => {
                assert_eq!(*reason, Issue31EvidenceUnavailableReason::HopMissing);
            }
            Issue31EvidenceChain::Complete { .. } => {
                panic!("a live host does not yet produce the omega#43 chain")
            }
        }
        // The run beside it still renders. One unavailable chain never hides
        // the work the owner is entitled to see.
        assert_eq!(adjunct.runs.len(), 1);
    }

    #[test]
    fn a_self_reported_run_projects_as_unavailable_beside_a_live_run() {
        let report = json!({
            "runRef": "run.full-auto.run-01",
            "evidence": {
                "objectiveRef": "objective.run-01",
                "turnRef": "turn.run-01.11",
                "changeRef": "change.run-01.11",
                "projectGeneration": "generation.project.00219",
                "verificationRef": "verification.run-01.11",
                "testOutcome": "outcome.test.passed",
                "testCommand": "cargo test -p workroom_receipts",
                "diffSummary": "3 files changed",
                "hostExecuted": false
            }
        });
        let receipt = json!({
            "runRef": "run.full-auto.run-01",
            "objectiveRef": "objective.run-01",
            "turnRef": "turn.run-01.11",
            "changeRef": "change.run-01.11",
            "verificationRef": "verification.run-01.11",
            "decisionRef": "decision.run-01.11",
            "authorityReceiptRef": "receipt.run-01.11",
            "allowed": true
        });
        let adjunct =
            project(&[run_detail()], &capacity(), &[], &[(report, receipt)]).expect("projects");
        match &adjunct.evidence[0] {
            Issue31EvidenceChain::Unavailable { reason, .. } => {
                assert_eq!(*reason, Issue31EvidenceUnavailableReason::SelfReported);
            }
            Issue31EvidenceChain::Complete { .. } => {
                panic!("a run reporting its own success is not verified")
            }
        }
    }
}
