//! Omega Rust supervisor for packaged `@openagentsinc/omega-effectd`.
//!
//! Authority: OpenAgentsInc/omega#21 (`OMEGA-FA-02`).
//! Durable run truth stays in omega-effectd on disk. GPUI is not run authority.

mod protocol;
mod supervisor;

use std::{rc::Rc, sync::Arc};

use anyhow::{Result, anyhow};
use gpui::{App, Global};
use smol::lock::Mutex as AsyncMutex;

pub use protocol::{
    HealthResult, HostMethod, HostRequestFrame, HostResponseError, HostResponseErrorCode,
    HostResponseFrame, InitializeResult, PROTOCOL_SCHEMA, PROTOCOL_VERSION, ProtocolError,
    ProtocolErrorCode, RunSnapshot, SERVICE_VERSION,
};
pub use supervisor::{
    MAX_FRAME_BYTES, OmegaEffectdCommand, OmegaEffectdHostFuture, OmegaEffectdHostHandler,
    OmegaEffectdSupervisor, OmegaEffectdSupervisorOptions, SupervisorError, default_options,
    fixture_command, resolve_effectd_command,
};

pub type SharedOmegaEffectdSupervisor = Rc<AsyncMutex<OmegaEffectdSupervisor>>;

enum OmegaEffectdRuntime {
    Available(SharedOmegaEffectdSupervisor),
    Unavailable(Arc<str>),
}

impl Global for OmegaEffectdRuntime {}

pub fn init(cx: &mut App) {
    init_with_host_handler(None, cx);
}

pub fn init_with_host_handler(handler: Option<OmegaEffectdHostHandler>, cx: &mut App) {
    if cx.has_global::<OmegaEffectdRuntime>() {
        return;
    }

    let runtime = match std::env::current_exe()
        .map_err(anyhow::Error::from)
        .and_then(|executable| {
            resolve_effectd_command(
                std::env::var_os("OPENAGENTS_OMEGA_EFFECTD_BIN").as_deref(),
                &executable,
            )
        }) {
        Ok(command) => {
            let mut supervisor = OmegaEffectdSupervisor::new(default_options(
                paths::data_dir().join("openagents"),
                command,
            ));
            if let Some(handler) = handler {
                supervisor.set_host_handler(handler);
            }
            OmegaEffectdRuntime::Available(Rc::new(AsyncMutex::new(supervisor)))
        }
        Err(error) => OmegaEffectdRuntime::Unavailable(error.to_string().into()),
    };
    cx.set_global(runtime);
}

pub fn shared_supervisor(cx: &App) -> Result<SharedOmegaEffectdSupervisor> {
    match cx.try_global::<OmegaEffectdRuntime>() {
        Some(OmegaEffectdRuntime::Available(supervisor)) => Ok(supervisor.clone()),
        Some(OmegaEffectdRuntime::Unavailable(message)) => Err(anyhow!(message.to_string())),
        None => Err(anyhow!("omega-effectd runtime was not initialized")),
    }
}

#[cfg(test)]
mod tests {
    use std::ffi::OsStr;
    use std::path::PathBuf;
    use std::rc::Rc;
    use std::sync::{Arc, Mutex};
    use std::time::Duration;

    use serde_json::json;
    use tempfile::tempdir;

    use super::*;

    fn fixture_path() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("fixtures/fake_effectd.mjs")
    }

    #[test]
    fn missing_packaged_component_fails_closed_without_fixture_fallback() {
        let root = tempdir().expect("tempdir");
        let executable = root.path().join("Omega.app/Contents/MacOS/omega");
        std::fs::create_dir_all(executable.parent().expect("executable parent"))
            .expect("create app executable directory");

        let error = resolve_effectd_command(None, &executable).expect_err("component is absent");
        let message = error.to_string();
        assert!(message.contains("packaged omega-effectd component is unavailable"));
        assert!(!message.contains("fake_effectd"));
        assert!(!message.contains("openagents/packages"));
    }

    #[test]
    fn resolver_accepts_only_explicit_or_packaged_component_paths() {
        let root = tempdir().expect("tempdir");
        let executable = root.path().join("Omega.app/Contents/MacOS/omega");
        let component = root
            .path()
            .join("Omega.app/Contents/Resources/omega-effectd/bin/omega-effectd");
        std::fs::create_dir_all(component.parent().expect("component parent"))
            .expect("create component directory");
        std::fs::write(&component, "#!/bin/sh\n").expect("write component");

        let packaged = resolve_effectd_command(None, &executable).expect("packaged component");
        assert_eq!(packaged.program, component);
        assert!(packaged.args.is_empty());

        let explicit = root.path().join("explicit-effectd");
        std::fs::write(&explicit, "#!/bin/sh\n").expect("write explicit component");
        let overridden = resolve_effectd_command(Some(explicit.as_os_str()), &executable)
            .expect("explicit component");
        assert_eq!(overridden.program, explicit);

        let missing = root.path().join("missing-effectd");
        assert!(
            resolve_effectd_command(Some(OsStr::new(&missing)), &executable).is_err(),
            "an explicit missing component must not fall back"
        );
    }

    #[test]
    fn oversized_response_frame_fails_closed_and_stops_child() {
        smol::block_on(async {
            let root = tempdir().expect("tempdir");
            let mut command = fixture_command(&fixture_path());
            command.args.push("--oversized-health-response".into());
            let mut supervisor = OmegaEffectdSupervisor::new(OmegaEffectdSupervisorOptions {
                data_root: root.path().to_path_buf(),
                command,
                initial_generation: 1,
                request_timeout: Duration::from_secs(5),
            });

            supervisor.start().await.expect("start");
            let error = supervisor.health().await.expect_err("oversized frame");
            assert!(error.to_string().contains("response frame exceeds"));
            let stopped = supervisor.health().await.expect_err("child was torn down");
            assert!(stopped.to_string().contains("not started"));
        });
    }

    #[test]
    fn host_requests_are_multiplexed_with_matching_generation() {
        smol::block_on(async {
            let root = tempdir().expect("tempdir");
            let mut command = fixture_command(&fixture_path());
            command.args.push("--host-request-health".into());
            let mut supervisor = OmegaEffectdSupervisor::new(OmegaEffectdSupervisorOptions {
                data_root: root.path().to_path_buf(),
                command,
                initial_generation: 7,
                request_timeout: Duration::from_secs(5),
            });
            let observed = Arc::new(Mutex::new(None));
            supervisor.set_host_handler(Rc::new({
                let observed = observed.clone();
                move |request| {
                    let observed = observed.clone();
                    Box::pin(async move {
                        *observed.lock().expect("observed request lock") = Some(request.clone());
                        Ok(json!({ "workspaceRef": "workspace.omega.supervised" }))
                    })
                }
            }));

            supervisor.start().await.expect("start");
            supervisor.health().await.expect("health");
            let request = observed
                .lock()
                .expect("observed request lock")
                .clone()
                .expect("host request");
            assert_eq!(request.generation, 7);
            assert_eq!(request.method, HostMethod::ResolveWorkspace);
        });
    }

    #[test]
    fn missing_host_authority_returns_typed_unavailable_response() {
        smol::block_on(async {
            let root = tempdir().expect("tempdir");
            let mut command = fixture_command(&fixture_path());
            command
                .args
                .push("--unavailable-host-request-health".into());
            let mut supervisor = OmegaEffectdSupervisor::new(OmegaEffectdSupervisorOptions {
                data_root: root.path().to_path_buf(),
                command,
                initial_generation: 1,
                request_timeout: Duration::from_secs(5),
            });

            supervisor.start().await.expect("start");
            supervisor
                .health()
                .await
                .expect("typed unavailable response");
        });
    }

    #[test]
    fn host_authority_timeout_returns_unavailable_without_parking_request() {
        smol::block_on(async {
            let root = tempdir().expect("tempdir");
            let mut command = fixture_command(&fixture_path());
            command
                .args
                .push("--unavailable-host-request-health".into());
            let mut supervisor = OmegaEffectdSupervisor::new(OmegaEffectdSupervisorOptions {
                data_root: root.path().to_path_buf(),
                command,
                initial_generation: 1,
                request_timeout: Duration::from_secs(5),
            });
            supervisor.set_host_request_timeout(Duration::from_millis(10));
            supervisor.set_host_handler(Rc::new(|_| {
                Box::pin(futures::future::pending::<
                    std::result::Result<serde_json::Value, HostResponseError>,
                >())
            }));

            supervisor.start().await.expect("start");
            supervisor.health().await.expect("host timeout response");
        });
    }

    #[test]
    fn stale_host_request_gets_generation_matched_rejection() {
        smol::block_on(async {
            let root = tempdir().expect("tempdir");
            let mut command = fixture_command(&fixture_path());
            command.args.push("--stale-host-request-health".into());
            let mut supervisor = OmegaEffectdSupervisor::new(OmegaEffectdSupervisorOptions {
                data_root: root.path().to_path_buf(),
                command,
                initial_generation: 2,
                request_timeout: Duration::from_secs(5),
            });

            supervisor.start().await.expect("start");
            supervisor.health().await.expect("stale host rejection");
        });
    }

    #[test]
    fn stale_service_response_fails_closed_and_stops_child() {
        smol::block_on(async {
            let root = tempdir().expect("tempdir");
            let mut command = fixture_command(&fixture_path());
            command.args.push("--stale-health-response".into());
            let mut supervisor = OmegaEffectdSupervisor::new(OmegaEffectdSupervisorOptions {
                data_root: root.path().to_path_buf(),
                command,
                initial_generation: 2,
                request_timeout: Duration::from_secs(5),
            });

            supervisor.start().await.expect("start");
            let error = supervisor
                .health()
                .await
                .expect_err("stale service response");
            assert!(error.to_string().contains("stale generation"));
            assert!(supervisor.health().await.is_err(), "child must be stopped");
        });
    }

    #[test]
    fn oversized_host_response_fails_closed_and_stops_child() {
        smol::block_on(async {
            let root = tempdir().expect("tempdir");
            let mut command = fixture_command(&fixture_path());
            command.args.push("--host-request-health".into());
            let mut supervisor = OmegaEffectdSupervisor::new(OmegaEffectdSupervisorOptions {
                data_root: root.path().to_path_buf(),
                command,
                initial_generation: 1,
                request_timeout: Duration::from_secs(5),
            });
            supervisor.set_host_handler(Rc::new(|_| {
                Box::pin(async {
                    Ok(json!({
                        "workspaceRef": "x".repeat(MAX_FRAME_BYTES),
                    }))
                })
            }));

            supervisor.start().await.expect("start");
            let error = supervisor
                .health()
                .await
                .expect_err("oversized host response");
            assert!(error.to_string().contains("host response frame exceeds"));
            assert!(supervisor.health().await.is_err(), "child must be stopped");
        });
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
            let handed_off = supervisor
                .handoff_run(&run_ref, "claude-local")
                .await
                .expect("handoff");
            assert_eq!(
                handed_off.get("lane").and_then(|v| v.as_str()),
                Some("claude-local")
            );
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
            let after = supervisor
                .get_run(&run_ref)
                .await
                .expect("get after restart");
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

            let disk =
                std::fs::read_to_string(root.path().join("agent-computer").join("sessions.json"))
                    .expect("sessions disk");
            assert!(!disk.contains("secret-fixture-token"));
            assert!(!disk.contains("Fixture Agent Computer"));

            supervisor.stop().await.expect("supervisor stop");
        });
    }
}
