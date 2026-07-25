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
    Issue31FullAutoAdjunct, Issue31FullAutoAdjunctError, build_issue31_full_auto_adjunct,
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
