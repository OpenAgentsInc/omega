//! Read this host's own Full Auto state off a running `omega-effectd` (omega#97).
//!
//! omega#49 and omega#91 both shipped with the same named substitution:
//!
//! > the Full Auto reading. No `omega-effectd` daemon is attached to this
//! > process, so the host's roster reading is supplied here rather than polled.
//!
//! That substitution is the whole of omega#97. A phone paired to a host that is
//! genuinely running work still rendered `no host projection`, because nothing
//! outside the desktop Full Auto panel ever handed the pump a reading, and the
//! obvious way to fill the rows — replaying
//! `fixtures/live-omega-effectd.get_run.json` — would make a recorded fixture
//! the host authority for a device proof, which omega#49's exit forbids in as
//! many words.
//!
//! So the reading is taken here instead, from a daemon that answered.
//!
//! ## One reading path, not two
//!
//! The polling this module does was previously inlined in `panel.rs`, reachable
//! only from a GPUI view. That is the wrong owner twice over: a headless proof
//! cannot reach it, and a host publishes its Full Auto state because it is a
//! host, not because someone opened a panel. `panel.rs` now calls
//! `observe_issue31_full_auto` like everyone else, so the desktop and the phone
//! cannot end up looking at two different readings of one daemon.
//!
//! ## Refused rather than shortened
//!
//! If the daemon lists runs and then fails to describe one of them, this
//! returns an error and no reading at all. A reading that dropped the run would
//! be published as a shorter run list than the host actually has, and a run
//! that vanishes from a snapshot reads on the phone as a run that ended. The
//! host has no way to distinguish those after the fact, so the incomplete
//! reading is never built.
//!
//! Evidence is the one exception, and deliberately: a run with no report or
//! receipt yet is a run whose evidence chain has not been produced, which is an
//! observation rather than a failure. It contributes no pair and the reading
//! stands.
//!
//! ## What this module does not do
//!
//! It never starts, pauses, resumes, retries, or stops a run. Reading a run
//! registry is not the same act as directing one, and Full Auto authority does
//! not begin on a path a model can reach. Every method called here is one of
//! `list_runs`, `get_run`, `get_capacity`, `get_report`, `get_receipt`.

use omega_effectd::{SharedOmegaEffectdSupervisor, SupervisorError};
use serde_json::Value;

use crate::issue31_delivery::Issue31FullAutoReading;

/// The omega#47 contract carries at most this many runs in one projection.
pub const MAX_ISSUE31_PROJECTED_RUNS: usize = 16;

/// Why this host could not state a reading of its own Full Auto surface.
///
/// Every variant means "no reading", never "an empty reading". The distinction
/// omega#49 turns on is that a host which could not look is not a host that
/// looked and found nothing: the first publishes silence and the phone says
/// `no host projection`, the second publishes zero runs and the phone says the
/// host is running nothing.
#[derive(Debug)]
pub enum Issue31ObservationError {
    /// `list_runs` did not answer. The host does not know what it is running.
    RunsUnavailable(String),
    /// `get_capacity` did not answer. Without it there is no account-to-lane
    /// mapping, and a snapshot carrying runs but no roster would report this
    /// host as holding no provider accounts.
    CapacityUnavailable(String),
    /// A run the daemon listed could not be described. See the module note:
    /// this refuses the whole reading rather than publishing a shorter one.
    RunDetailUnavailable { run_ref: String, error: String },
}

impl std::fmt::Display for Issue31ObservationError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::RunsUnavailable(error) => {
                write!(formatter, "omega-effectd did not list its runs: {error}")
            }
            Self::CapacityUnavailable(error) => {
                write!(formatter, "omega-effectd did not report capacity: {error}")
            }
            Self::RunDetailUnavailable { run_ref, error } => write!(
                formatter,
                "omega-effectd listed run {run_ref} and then could not describe it: {error}",
            ),
        }
    }
}

impl std::error::Error for Issue31ObservationError {}

impl Issue31ObservationError {
    /// A stable token for logs and proof output, so a failed observation is
    /// reported as the specific thing that failed rather than as a sentence.
    pub fn token(&self) -> &'static str {
        match self {
            Self::RunsUnavailable(_) => "observation.omega.runs_unavailable",
            Self::CapacityUnavailable(_) => "observation.omega.capacity_unavailable",
            Self::RunDetailUnavailable { .. } => "observation.omega.run_detail_unavailable",
        }
    }
}

fn describe(error: SupervisorError) -> String {
    error.to_string()
}

/// Take one complete reading of this host's Full Auto surface.
///
/// The supervisor is the same one the desktop drives, so this is the daemon
/// the owner's Omega is actually running, answering the same five read
/// methods. Nothing here is recorded, replayed, or defaulted: if the daemon
/// does not answer, the host states no reading.
pub async fn observe_issue31_full_auto(
    supervisor: &SharedOmegaEffectdSupervisor,
) -> Result<Issue31FullAutoReading, Issue31ObservationError> {
    let listed = {
        let mut guard = supervisor.lock().await;
        guard
            .list_runs()
            .await
            .map_err(|error| Issue31ObservationError::RunsUnavailable(describe(error)))?
    };
    let capacity = {
        let mut guard = supervisor.lock().await;
        guard
            .get_capacity()
            .await
            .map_err(|error| Issue31ObservationError::CapacityUnavailable(describe(error)))?
    };
    let host_generation = {
        let guard = supervisor.lock().await;
        guard.generation()
    };

    let mut run_details: Vec<Value> = Vec::new();
    let mut evidence: Vec<(Value, Value)> = Vec::new();
    for run in listed.iter().take(MAX_ISSUE31_PROJECTED_RUNS) {
        let mut guard = supervisor.lock().await;
        let detail = guard.get_run(&run.run_ref).await.map_err(|error| {
            Issue31ObservationError::RunDetailUnavailable {
                run_ref: run.run_ref.clone(),
                error: describe(error),
            }
        })?;
        run_details.push(detail);
        // A run whose evidence chain has not been produced yet contributes no
        // pair. Both halves or neither: a report without its receipt is half a
        // chain, and omega#43's chain is refused hop by hop rather than shown
        // with a hop missing.
        if let (Ok(report), Ok(receipt)) = (
            guard.get_report(&run.run_ref).await,
            guard.get_receipt(&run.run_ref).await,
        ) {
            evidence.push((report, receipt));
        }
    }

    // Stamped inside the constructor, from one reading of this host's clock,
    // with no parameter for a caller to supply. See `Issue31FullAutoReading`.
    Ok(Issue31FullAutoReading::observed(
        host_generation,
        run_details,
        capacity,
        evidence,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    use serde_json::json;

    /// Stand up the observation stub against a data root of its own.
    ///
    /// The stub is a test double for this module's error handling, never host
    /// authority — see the header of `fixtures/observation_stub_effectd.mjs`.
    /// Nothing built here is published to a relay by any test.
    fn stub(scenario: serde_json::Value) -> (tempfile::TempDir, SharedOmegaEffectdSupervisor) {
        let temporary = tempfile::tempdir().expect("tempdir");
        let data_root = temporary.path().join("effectd");
        std::fs::create_dir_all(&data_root).expect("data root");
        std::fs::write(
            data_root.join("scenario.json"),
            serde_json::to_string(&scenario).expect("scenario"),
        )
        .expect("write scenario");
        let fixture = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("fixtures")
            .join("observation_stub_effectd.mjs");
        let supervisor = std::rc::Rc::new(smol::lock::Mutex::new(
            omega_effectd::OmegaEffectdSupervisor::new(omega_effectd::default_options(
                data_root,
                omega_effectd::fixture_command(&fixture),
            )),
        ));
        smol::block_on(async {
            let mut guard = supervisor.lock().await;
            guard.start().await
        })
        .expect("the observation stub must start");
        (temporary, supervisor)
    }

    fn one_run_scenario(refuse_get_run: bool) -> serde_json::Value {
        json!({
            "runs": [{
                "runRef": "run.omega.1",
                "threadRef": null,
                "state": "running",
                "title": "Observed run",
                "updatedAt": "2026-07-26T12:00:00Z",
                "startedAtMs": 1_785_000_000_000u64,
            }],
            "details": {
                "run.omega.1": {
                    "runRef": "run.omega.1",
                    "state": "running",
                    "title": "Observed run",
                    "objective": "Prove the host measured this.",
                    "doneCondition": "The phone reads it.",
                    "lane": "codex-local",
                    "turnCap": 40,
                    "successfulAttempts": 1,
                    "failedAttempts": 0,
                    "recoveryAction": "none",
                    "startedAtMs": 1_785_000_000_000u64,
                    "updatedAt": "2026-07-26T12:00:00Z",
                    "turns": [],
                },
            },
            "capacity": {
                "activeRunLimit": 8,
                "activeRunCount": 1,
                "lanes": [{ "lane": "codex-local", "state": "available", "activeRuns": 1 }],
            },
            "refuseGetRun": if refuse_get_run { vec!["run.omega.1"] } else { Vec::<&str>::new() },
        })
    }

    /// The happy path, against a daemon that answered.
    #[test]
    fn a_daemon_that_answers_yields_the_runs_it_reported() {
        let (_temporary, supervisor) = stub(one_run_scenario(false));
        let reading =
            smol::block_on(observe_issue31_full_auto(&supervisor)).expect("the daemon answered");
        assert_eq!(reading.run_details.len(), 1);
        assert_eq!(
            reading.run_details[0].get("runRef").and_then(|v| v.as_str()),
            Some("run.omega.1"),
        );
        assert_eq!(
            reading.capacity.get("activeRunCount").and_then(|v| v.as_u64()),
            Some(1),
            "the capacity record must be the daemon's own, not a constructed one",
        );
        // No report or receipt exists yet, so no evidence pair is invented.
        assert!(reading.evidence.is_empty());
    }

    /// A run the daemon listed and then would not describe refuses the whole
    /// reading.
    ///
    /// Publishing the shorter list instead would drop a run the host is
    /// actually holding, and a run that vanishes from a snapshot reads on the
    /// phone as a run that ended. The host cannot tell those apart afterwards,
    /// so the incomplete reading is never built.
    #[test]
    fn a_run_the_daemon_will_not_describe_refuses_the_whole_reading() {
        let (_temporary, supervisor) = stub(one_run_scenario(true));
        let error = smol::block_on(observe_issue31_full_auto(&supervisor))
            .expect_err("an undescribable run must refuse the reading");
        assert_eq!(error.token(), "observation.omega.run_detail_unavailable");
        assert!(
            matches!(
                &error,
                Issue31ObservationError::RunDetailUnavailable { run_ref, .. }
                    if run_ref == "run.omega.1"
            ),
            "the refusal must name the run it could not describe: {error}",
        );
    }

    /// A host that could not look is not a host that looked and found nothing.
    #[test]
    fn a_daemon_that_will_not_list_its_runs_yields_no_reading_rather_than_an_empty_one() {
        let (_temporary, supervisor) = stub(json!({ "refuseListRuns": true }));
        let error = smol::block_on(observe_issue31_full_auto(&supervisor))
            .expect_err("a host that could not look states no reading");
        assert_eq!(error.token(), "observation.omega.runs_unavailable");
    }

    /// Capacity is the account-to-lane mapping's only source. A reading that
    /// carried runs and no capacity would report this host as holding no
    /// provider accounts, which is a claim rather than an observation.
    #[test]
    fn a_daemon_that_will_not_report_capacity_yields_no_reading() {
        let (_temporary, supervisor) = stub(json!({ "runs": [], "refuseCapacity": true }));
        let error = smol::block_on(observe_issue31_full_auto(&supervisor))
            .expect_err("no capacity means no reading");
        assert_eq!(error.token(), "observation.omega.capacity_unavailable");
    }

    /// A daemon with nothing running is a real observation, and a different one
    /// from a daemon that was never asked.
    #[test]
    fn a_daemon_running_nothing_yields_an_empty_reading_rather_than_a_refusal() {
        let (_temporary, supervisor) = stub(json!({
            "runs": [],
            "capacity": { "activeRunLimit": 8, "activeRunCount": 0, "lanes": [] },
        }));
        let reading = smol::block_on(observe_issue31_full_auto(&supervisor))
            .expect("a host that looked and found nothing still has a reading");
        assert!(reading.run_details.is_empty());
    }

    /// The reading a host takes of a daemon it did not read is not an empty
    /// reading — there is no way to express one.
    ///
    /// This is a compile-shaped assertion written as a runtime one: the only
    /// constructor reachable from outside `issue31_delivery` is `observed`,
    /// which takes no stamp. If someone adds a public setter or restores
    /// `Default`, the omega#97 property is gone, and the tests that name it in
    /// `issue31_delivery` go red.
    #[test]
    fn an_observed_reading_is_stamped_by_the_host_that_took_it() {
        let before = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("clock")
            .as_millis() as u64;
        let reading = Issue31FullAutoReading::observed(
            7,
            Vec::new(),
            serde_json::json!({ "accounts": [] }),
            Vec::new(),
        );
        let after = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("clock")
            .as_millis() as u64;
        assert!(
            reading.generated_at_ms() >= before && reading.generated_at_ms() <= after,
            "the stamp must be this host's own clock reading, not a supplied value: \
             {before} <= {} <= {after}",
            reading.generated_at_ms(),
        );
        assert_eq!(reading.host_generation, 7);
    }

    #[test]
    fn every_observation_failure_is_a_distinct_token() {
        let tokens = [
            Issue31ObservationError::RunsUnavailable("x".into()).token(),
            Issue31ObservationError::CapacityUnavailable("x".into()).token(),
            Issue31ObservationError::RunDetailUnavailable {
                run_ref: "run.1".into(),
                error: "x".into(),
            }
            .token(),
        ];
        let unique: std::collections::BTreeSet<&str> = tokens.iter().copied().collect();
        assert_eq!(
            unique.len(),
            tokens.len(),
            "a host that could not look must say which look failed",
        );
    }
}
