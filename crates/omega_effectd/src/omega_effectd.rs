//! Omega Rust supervisor for packaged `@openagentsinc/omega-effectd`.
//!
//! Authority: OpenAgentsInc/omega#21 (`OMEGA-FA-02`).
//! Durable run truth stays in omega-effectd on disk. GPUI is not run authority.

mod issue31_nostr;
mod issue31_provider_handoff;
mod nostr_websocket_relay;
mod openagents_binding;
mod openagents_session;
mod protocol;
mod sarah_conversation;
mod supervisor;

use std::{rc::Rc, sync::Arc};

use anyhow::{Result, anyhow};
use gpui::{App, Global};
use smol::lock::Mutex as AsyncMutex;

pub use openagents_binding::{
    BINDING_RECORD_SCHEMA, BindingEvent, BindingProjection, BindingState,
    OPENAGENTS_BINDING_CREDENTIAL_KEY, OPENAGENTS_OMEGA_CLIENT_ID, OWNER_SCOPE_REFUSED_MESSAGE,
    OpenAgentsBinding, apply_binding_transition, binding_record_path, default_binding_data_root,
    init_openagents_binding, openagents_binding, try_openagents_binding,
};

pub use issue31_nostr::*;
pub use issue31_provider_handoff::*;
pub use nostr_websocket_relay::WebSocketRelayAdapter;
pub use openagents_session::{
    OpenAgentsSession, OpenAgentsSessionPhase, VerifiedOpenAgentsSession, init_openagents_session,
    openagents_session,
};

pub use protocol::{
    HealthResult, HostMethod, HostRequestFrame, HostResponseError, HostResponseErrorCode,
    HostResponseFrame, InitializeResult, PROTOCOL_SCHEMA, PROTOCOL_VERSION, ProtocolError,
    ProtocolErrorCode, RunSnapshot, SERVICE_VERSION,
};
pub use sarah_conversation::{
    BootstrapResult, ConversationIdentity, GapState, InterruptTurnResult, MockRelayAdapter,
    RelayTransport, RoomSnapshotResult, RoomStateEvent, SARAH_EVENT_ROOM_EVENT,
    SARAH_EVENT_ROOM_STATE, SARAH_FRAMED_METHODS, SARAH_METHOD_BOOTSTRAP,
    SARAH_METHOD_DEVICE_GRANTS, SARAH_METHOD_INTERRUPT_TURN, SARAH_METHOD_READMIT_DEVICE,
    SARAH_METHOD_RENEW_DEVICE_GRANT, SARAH_METHOD_REVOKE_DEVICE_GRANT,
    SARAH_METHOD_ROOM_SNAPSHOT, SARAH_METHOD_SEND_MESSAGE,
    SARAH_METHOD_SESSION_STATUS, Issue31HostProjectionDocuments, Issue31HostProjectionRequest,
    Issue31HostProjectionSource, Issue31ProviderRosterSource,
    SarahConversationClient, SarahConversationConfig,
    SarahConversationError, SendMessageResult, SessionStatusResult, SigningIdentity,
    asserts_no_khala_sync_client,
};
pub use supervisor::{
    AttentionDecision, MAX_FRAME_BYTES, OmegaEffectdCommand, OmegaEffectdHostFuture,
    OmegaEffectdHostHandler, OmegaEffectdSupervisor, OmegaEffectdSupervisorOptions,
    SupervisorError, default_options, fixture_command, resolve_effectd_command,
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
    // OMEGA-SW-01: binding is independent of effectd packaging availability.
    init_openagents_binding(cx);

    if cx.has_global::<OmegaEffectdRuntime>() {
        return;
    }

    start_served_acp_surface();

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

/// `OMEGA-DELTA-0041`, omega#82. Serve Omega Agent over ACP, if the flag says
/// so.
///
/// The supervisor layer owns this, not GPUI. The socket itself lives in
/// `crates/omega_acp_server`, which depends on no part of GPUI, and this is its
/// only production call site — `crates/omega_deltas` fails if a second one
/// appears in a UI crate. Omega's own windows never open a listener.
///
/// Off unless `OMEGA_ACP_SERVER` is exactly `1`, so the shipped default binds
/// nothing at all and this is a no-op in every normal launch.
fn start_served_acp_surface() {
    match omega_acp_server::start_if_enabled() {
        omega_acp_server::StartOutcome::NotStarted(reason) => {
            log::debug!(
                "OMEGA-DELTA-0041: Omega Agent is not served over ACP ({})",
                reason.token()
            );
        }
        omega_acp_server::StartOutcome::Listening(address) => {
            log::info!(
                "OMEGA-DELTA-0041: Omega Agent is served over ACP on {address} \
                 (loopback, unauthenticated, read-only)"
            );
        }
        omega_acp_server::StartOutcome::Failed(error) => {
            log::error!(
                "OMEGA-DELTA-0041: the served ACP surface was asked for and \
                 could not listen: {error}"
            );
        }
    }
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

    #[test]
    fn sarah_nr06_conversation_methods_via_fixture() {
        smol::block_on(async {
            let root = tempdir().expect("tempdir");
            let mut supervisor = OmegaEffectdSupervisor::new(OmegaEffectdSupervisorOptions {
                data_root: root.path().to_path_buf(),
                command: fixture_command(&fixture_path()),
                initial_generation: 1,
                request_timeout: Duration::from_secs(5),
            });
            supervisor.start().await.expect("start");

            let status = supervisor
                .sarah_session_status()
                .await
                .expect("session status");
            assert_eq!(status.get("signedIn").and_then(|v| v.as_bool()), Some(true));
            let encoded = status.to_string();
            assert!(!encoded.contains("token"));
            assert!(!encoded.contains("bearer"));

            let boot = supervisor.sarah_bootstrap().await.expect("bootstrap");
            assert_eq!(
                boot.get("principalRef").and_then(|v| v.as_str()),
                Some("principal.sarah")
            );
            let conversation_ref = boot
                .get("conversationRef")
                .and_then(|v| v.as_str())
                .expect("conversationRef")
                .to_string();
            assert!(conversation_ref.starts_with("sarah."));

            let sent = supervisor
                .sarah_send_message("hello from fixture", "idempotency.fixture.send.1")
                .await
                .expect("send");
            assert_eq!(sent.get("accepted").and_then(|v| v.as_bool()), Some(true));
            let turn_ref = sent
                .get("turnRef")
                .and_then(|v| v.as_str())
                .expect("turnRef")
                .to_string();

            let snap = supervisor
                .sarah_room_snapshot(Some(json!({ "limit": 10 })))
                .await
                .expect("snapshot");
            assert_eq!(
                snap.get("conversationRef").and_then(|v| v.as_str()),
                Some(conversation_ref.as_str())
            );
            assert!(
                snap.pointer("/transcript/gapState")
                    .and_then(|v| v.as_str())
                    .is_some()
            );
            assert!(
                snap.pointer("/transcript/cursor")
                    .and_then(|v| v.as_str())
                    .is_some()
            );

            let interrupt = supervisor
                .sarah_interrupt_turn(&turn_ref, "idempotency.fixture.interrupt.1")
                .await
                .expect("interrupt");
            assert_eq!(
                interrupt.get("pending").and_then(|v| v.as_bool()),
                Some(true)
            );
            assert_eq!(
                interrupt.get("status").and_then(|v| v.as_str()),
                Some("pending")
            );

            // OMEGA-SW-02 cut: no Khala Sync client on this lane.
            assert!(asserts_no_khala_sync_client());
            supervisor.stop().await.expect("stop");
        });
    }

    #[test]
    fn attention_decisions_are_typed_and_deduplicated() {
        smol::block_on(async {
            let root = tempdir().expect("tempdir");
            let runs_path = root.path().join("full-auto").join("runs.json");
            std::fs::create_dir_all(
                runs_path
                    .parent()
                    .expect("runs path should have a parent directory"),
            )
            .expect("create runs directory");
            std::fs::write(
                &runs_path,
                serde_json::to_string_pretty(&json!({
                    "schema": "openagents.desktop.full_auto_run_registry.v1",
                    "runs": [{
                        "runRef": "run.full-auto.attention",
                        "threadRef": null,
                        "title": "SECRET_OBJECTIVE /Users/owner/private",
                        "state": "stalled",
                        "stallCause": "dispatch_overdue",
                        "updatedAt": "2026-07-24T00:00:00.000Z"
                    }]
                }))
                .expect("encode fixture runs"),
            )
            .expect("write fixture runs");

            let mut supervisor = OmegaEffectdSupervisor::new(OmegaEffectdSupervisorOptions {
                data_root: root.path().to_path_buf(),
                command: fixture_command(&fixture_path()),
                initial_generation: 1,
                request_timeout: Duration::from_secs(5),
            });
            supervisor.start().await.expect("start");

            let decision = supervisor
                .decide_attention("run.full-auto.attention", true, None)
                .await
                .expect("attention decision")
                .expect("stalled run should produce a decision");
            assert!(decision.notify);
            assert_eq!(decision.title, "Full Auto stalled");
            assert!(decision.body.contains("SECRET_OBJECTIVE"));

            let duplicate = supervisor
                .decide_attention("run.full-auto.attention", true, Some(&decision.dedup_key))
                .await
                .expect("deduplicated attention decision");
            assert!(duplicate.is_none());
            supervisor.stop().await.expect("stop");
        });
    }
}
