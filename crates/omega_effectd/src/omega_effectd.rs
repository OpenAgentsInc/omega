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

    #[test]
    fn fa07_control_matrix_and_native_join_survive_restart() {
        smol::block_on(async {
            let root = tempdir().unwrap();
            let mut supervisor = OmegaEffectdSupervisor::new(OmegaEffectdSupervisorOptions {
                data_root: root.path().to_path_buf(),
                command: fixture_command(&fixture_path()),
                initial_generation: 1,
                request_timeout: Duration::from_secs(5),
            });

            supervisor.start().await.expect("start");
            let started = supervisor
                .start_run(json!({
                    "workspaceRef": "workspace.omega.supervised",
                    "title": "FA-07 control matrix",
                    "objective": "Prove pause resume stop and native join.",
                    "doneCondition": "Controls complete.",
                    "turnCap": 8,
                    "projectRef": "project.fa07",
                    "worktreeRef": "worktree.fa07",
                    "gitHead": "deadbeef"
                }))
                .await
                .expect("start_run");
            let run_ref = started
                .get("runRef")
                .and_then(|v| v.as_str())
                .expect("runRef")
                .to_string();
            assert_eq!(
                started
                    .pointer("/nativeEvidence/projectRef")
                    .and_then(|v| v.as_str()),
                Some("project.fa07")
            );

            let paused = supervisor.pause_run(&run_ref).await.expect("pause");
            assert_eq!(paused.get("state").and_then(|v| v.as_str()), Some("paused"));
            let resumed = supervisor.resume_run(&run_ref).await.expect("resume");
            assert_eq!(
                resumed.get("state").and_then(|v| v.as_str()),
                Some("running")
            );

            let binding = supervisor
                .get_native_binding(&run_ref)
                .await
                .expect("binding");
            assert_eq!(
                binding.get("projectRef").and_then(|v| v.as_str()),
                Some("project.fa07")
            );
            let assessment = supervisor
                .assess_native_boundary(&run_ref)
                .await
                .expect("assessment");
            assert_eq!(assessment.get("ok").and_then(|v| v.as_bool()), Some(true));

            let sync = supervisor.get_sync_status().await.expect("sync");
            assert_eq!(
                sync.get("publishBlocksDispatch").and_then(|v| v.as_bool()),
                Some(false)
            );

            supervisor.restart().await.expect("restart");
            let after = supervisor.get_run(&run_ref).await.expect("get after restart");
            assert_eq!(
                after.get("runRef").and_then(|v| v.as_str()),
                Some(run_ref.as_str())
            );
            assert_eq!(
                after
                    .pointer("/nativeEvidence/worktreeRef")
                    .and_then(|v| v.as_str()),
                Some("worktree.fa07")
            );

            let stopped = supervisor.stop_run(&run_ref).await.expect("stop");
            assert_eq!(
                stopped.get("state").and_then(|v| v.as_str()),
                Some("stopped")
            );
            supervisor.stop().await.expect("supervisor stop");
        });
    }

    #[test]
    fn ac01_agent_computer_session_survives_restart_without_bearer() {
        smol::block_on(async {
            let root = tempdir().unwrap();
            let mut supervisor = OmegaEffectdSupervisor::new(OmegaEffectdSupervisorOptions {
                data_root: root.path().to_path_buf(),
                command: fixture_command(&fixture_path()),
                initial_generation: 1,
                request_timeout: Duration::from_secs(5),
            });

            supervisor.start().await.expect("start");
            let session = supervisor
                .start_agent_computer_session(json!({
                    "bearerToken": "secret-fixture-token",
                    "controlPlaneBaseUrl": "https://openagents.com",
                    "repoRef": "OpenAgentsInc/openagents",
                    "objective": "Fixture Agent Computer launch",
                    "adapter": "codex",
                    "lane": "cloud-gcp",
                }))
                .await
                .expect("start agent computer");
            let session_ref = session
                .get("sessionRef")
                .and_then(|v| v.as_str())
                .expect("sessionRef")
                .to_string();
            assert_eq!(
                session.get("environment").and_then(|v| v.as_str()),
                Some("openagents_cloud")
            );

            let refreshed = supervisor
                .refresh_agent_computer_session("secret-fixture-token", &session_ref)
                .await
                .expect("refresh");
            assert_eq!(
                refreshed.get("state").and_then(|v| v.as_str()),
                Some("running")
            );

            let turn = supervisor
                .run_agent_computer_turn(json!({
                    "bearerToken": "secret-fixture-token",
                    "controlPlaneBaseUrl": "https://openagents.com",
                    "repoRef": "OpenAgentsInc/openagents",
                    "objective": "Fixture Agent Computer turn",
                }))
                .await
                .expect("run turn");
            assert_eq!(
                turn.get("finishReason").and_then(|v| v.as_str()),
                Some("stop")
            );
            assert_eq!(
                turn.pointer("/session/state").and_then(|v| v.as_str()),
                Some("completed")
            );

            supervisor.restart().await.expect("restart");
            let listed = supervisor
                .list_agent_computer_sessions()
                .await
                .expect("list after restart");
            let sessions = listed
                .get("sessions")
                .and_then(|v| v.as_array())
                .expect("sessions array");
            assert!(
                sessions.iter().any(|row| {
                    row.get("sessionRef").and_then(|v| v.as_str()) == Some(session_ref.as_str())
                }),
                "agent computer session must survive restart: {listed}"
            );

            let disk = std::fs::read_to_string(
                root.path().join("agent-computer").join("sessions.json"),
            )
            .expect("sessions disk");
            assert!(!disk.contains("secret-fixture-token"));
            assert!(!disk.contains("Fixture Agent Computer"));

            supervisor.stop().await.expect("supervisor stop");
        });
    }
}
