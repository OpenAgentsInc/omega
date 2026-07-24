//! Omega Rust supervisor for packaged `@openagentsinc/omega-effectd`.
//!
//! Authority: OpenAgentsInc/omega#21 (`OMEGA-FA-02`).
//! Durable run truth stays in omega-effectd on disk. GPUI is not run authority.

mod protocol;
mod supervisor;

pub use protocol::{
    HealthResult, InitializeResult, ProtocolError, ProtocolErrorCode, RunSnapshot, PROTOCOL_SCHEMA,
    PROTOCOL_VERSION, SERVICE_VERSION,
};
pub use supervisor::{
    default_options, fixture_command, OmegaEffectdCommand, OmegaEffectdSupervisor,
    OmegaEffectdSupervisorOptions, SupervisorError,
};

#[cfg(test)]
mod tests {
    use std::path::PathBuf;
    use std::time::Duration;

    use serde_json::json;
    use tempfile::tempdir;

    use super::*;

    fn fixture_path() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("fixtures/fake_effectd.mjs")
    }

    #[test]
    fn start_health_restart_stop_and_generation_fence() {
        smol::block_on(async {
            let root = tempdir().unwrap();
            let mut supervisor = OmegaEffectdSupervisor::new(OmegaEffectdSupervisorOptions {
                data_root: root.path().to_path_buf(),
                command: fixture_command(&fixture_path()),
                initial_generation: 1,
                request_timeout: Duration::from_secs(5),
            });

            let init = supervisor.start().await.expect("start");
            assert_eq!(init.generation, 1);
            assert_eq!(init.schema, PROTOCOL_SCHEMA);

            let health = supervisor.health().await.expect("health");
            assert_eq!(health.status, "running");
            assert_eq!(health.generation, 1);

            // Persist a run through the fixture file API, then restart.
            let runs_path = root.path().join("full-auto").join("runs.json");
            std::fs::create_dir_all(runs_path.parent().unwrap()).unwrap();
            std::fs::write(
                &runs_path,
                serde_json::to_string_pretty(&json!({
                    "schema": "openagents.desktop.full_auto_run_registry.v1",
                    "runs": [{
                        "runRef": "run.full-auto.recovery",
                        "threadRef": null,
                        "title": "Recovery proof",
                        "state": "paused",
                        "updatedAt": "2026-07-24T00:00:00.000Z"
                    }]
                }))
                .unwrap(),
            )
            .unwrap();

            let restarted = supervisor.restart().await.expect("restart");
            assert_eq!(restarted.generation, 2);

            let runs = supervisor.list_runs().await.expect("list after restart");
            assert!(
                runs.iter().any(|run| {
                    run.run_ref == "run.full-auto.recovery" && run.title == "Recovery proof"
                }),
                "durable disk truth must survive restart: {runs:?}"
            );

            // Stale generation must be refused by the child; supervisor tracks current gen.
            assert_eq!(supervisor.generation(), 2);
            supervisor.stop().await.expect("stop");
        });
    }
}
