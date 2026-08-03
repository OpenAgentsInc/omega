//! Omega Rust supervisor for packaged `@openagentsinc/omega-effectd`.
//!
//! Authority: OpenAgentsInc/omega#21 (`OMEGA-FA-02`).
//! Durable run truth stays in omega-effectd on disk. GPUI is not run authority.

mod all_work;
mod forensics_cloud;
mod issue31_nostr;
mod issue31_provider_handoff;
mod nostr_websocket_relay;
mod openagents_binding;
mod openagents_nostr_auth;
mod openagents_sarah_voice;
mod openagents_session;
mod protocol;
mod sarah_conversation;
mod supervisor;

use std::{
    collections::BTreeMap,
    rc::Rc,
    sync::{Arc, Mutex, RwLock},
};

use anyhow::{Result, anyhow};
use gpui::{App, Global};
use sha2::{Digest, Sha256};
use smol::lock::Mutex as AsyncMutex;

pub use openagents_binding::{
    BINDING_RECORD_SCHEMA, BindingEvent, BindingProjection, BindingState,
    OPENAGENTS_BINDING_CREDENTIAL_KEY, OPENAGENTS_OMEGA_CLIENT_ID, OWNER_SCOPE_REFUSED_MESSAGE,
    OpenAgentsBinding, apply_binding_transition, binding_record_path, default_binding_data_root,
    init_openagents_binding, openagents_binding, try_openagents_binding,
};

pub use all_work::generated as all_work_contract;
pub use forensics_cloud::*;
pub use issue31_nostr::*;
pub use issue31_provider_handoff::*;
pub use nostr_websocket_relay::{
    WebSocketRelayAdapter, publish_community_event, query_community_events,
    relay_authentication_projection,
};
pub use omega_device_bridge::PROTOCOL as DEVICE_BRIDGE_PROTOCOL;
pub use omega_device_bridge::{
    BindRefusal as DeviceBridgeBindRefusal, BridgeBindHost, BridgeError as DeviceBridgeError,
    ByeReason as DeviceBridgeByeReason, Cursor as DeviceBridgeCursor, ExecutorDisclosure,
    GrantAdmission as DeviceBridgeGrantAdmission, GrantRefusalReason, MessageRole, MirrorChange,
    MirrorHealth, MirrorMessage, MirrorRun, MirrorSnapshot, MirrorThread, PAIRING_BOOTSTRAP_SCHEMA,
    PairingBootstrap, PairingQr, ProjectionJournal, RunState,
    ServerConfig as DeviceBridgeServerConfig, ServerFrame as DeviceBridgeServerFrame,
    ServerHandle as DeviceBridgeServerHandle, ThreadState,
};
pub use openagents_nostr_auth::{
    HostedSessionBlocker, Nip98ProofReplayGuard, VerifiedNip98Proof, sign_nip98_request,
    verify_nip98_authorization,
};
pub use openagents_sarah_voice::{
    ManagedSarahVoiceSession, PreparedSarahVoiceAdmission, SARAH_VOICE_ADMISSION_SCHEMA,
    SARAH_VOICE_ADMISSION_URL, SARAH_VOICE_CHALLENGE_PROTOCOL, SARAH_VOICE_DEVICE_HEADER,
    SARAH_VOICE_NOSTR_CHALLENGE_URL, SARAH_VOICE_SESSION_HEADER, SARAH_VOICE_SETTLEMENT_SCHEMA,
    SARAH_VOICE_SETTLEMENT_URL, SARAH_VOICE_TICKET_HEADER, SarahLiveKitRoomTransport,
    SarahVoiceAdmissionProjection, SarahVoiceCapabilityId, SarahVoiceCreditMode,
    SarahVoiceExcludedAuthorities, SarahVoiceProjectionGap, SarahVoiceSettlementProjection,
    SarahVoiceSettlementState, SarahVoiceTransport,
};
pub use openagents_session::{
    HostedRetrySchedule, HostedSessionProjection, HostedSessionState, OpenAgentsSession,
    OpenAgentsSessionPhase, VerifiedOpenAgentsSession, init_openagents_session, openagents_session,
    openagents_session_if_initialized,
};

pub use protocol::{
    HealthResult, HostMethod, HostRequestFrame, HostResponseError, HostResponseErrorCode,
    HostResponseFrame, InitializeResult, PROTOCOL_SCHEMA, PROTOCOL_VERSION, ProtocolError,
    ProtocolErrorCode, RunSnapshot, SERVICE_VERSION,
};
pub use sarah_conversation::{
    BootstrapResult, ConversationIdentity, GapState, InterruptTurnResult,
    Issue31AgentThreadAdmissionState, Issue31HostProjectionDocuments, Issue31HostProjectionRequest,
    Issue31HostProjectionSource, Issue31PendingAgentThreadCommand, Issue31ProviderRosterSource,
    MockRelayAdapter, RelayTransport, RoomSnapshotResult, RoomStateEvent, SARAH_EVENT_ROOM_EVENT,
    SARAH_EVENT_ROOM_STATE, SARAH_FRAMED_METHODS, SARAH_METHOD_BOOTSTRAP,
    SARAH_METHOD_DEVICE_GRANTS, SARAH_METHOD_INTERRUPT_TURN, SARAH_METHOD_READMIT_DEVICE,
    SARAH_METHOD_RENEW_DEVICE_GRANT, SARAH_METHOD_REVOKE_DEVICE_GRANT, SARAH_METHOD_ROOM_SNAPSHOT,
    SARAH_METHOD_SEND_MESSAGE, SARAH_METHOD_SESSION_STATUS, SarahConversationClient,
    SarahConversationConfig, SarahConversationError, SendMessageResult, SessionStatusResult,
    SigningIdentity, asserts_no_khala_sync_client,
};
pub use supervisor::{
    AttentionDecision, MAX_FRAME_BYTES, OmegaEffectdCommand, OmegaEffectdHostFuture,
    OmegaEffectdHostHandler, OmegaEffectdSupervisor, OmegaEffectdSupervisorOptions,
    SupervisorError, default_options, fixture_command, resolve_effectd_command,
};
pub type SharedOmegaEffectdSupervisor = Rc<AsyncMutex<OmegaEffectdSupervisor>>;
pub type SharedIssue31HostController = Arc<RwLock<Issue31HostController>>;

struct Issue31DeviceBridgeAuthority {
    controller: SharedIssue31HostController,
    pairing_offers: Arc<Mutex<BTreeMap<String, DevicePairingOffer>>>,
    /// omega#164. Called once per freshly minted pairing grant so the quiet
    /// identity backup nudge learns the key now has something to lose. `None`
    /// outside the production servers, so state-machine tests never touch a
    /// real profile.
    grant_recorder: Option<Arc<dyn Fn() + Send + Sync>>,
}

impl Issue31DeviceBridgeAuthority {
    fn with_grant_recorder(mut self, recorder: Arc<dyn Fn() + Send + Sync>) -> Self {
        self.grant_recorder = Some(recorder);
        self
    }
}

#[derive(Clone)]
struct DevicePairingOffer {
    host_public_key_hex: String,
    scopes: Vec<Issue31PairingScope>,
    expires_at: u64,
    identity_action: Option<omega_identity::IdentityActionAuthorization>,
}

#[derive(Clone)]
pub struct DevicePairingEngine {
    controller: SharedIssue31HostController,
    pairing_offers: Arc<Mutex<BTreeMap<String, DevicePairingOffer>>>,
}

#[derive(Clone)]
struct DevicePairingRuntime {
    engine: DevicePairingEngine,
    endpoint: Issue31DirectEndpoint,
    host_public_key_hex: String,
    generation: u64,
    scopes: Vec<Issue31PairingScope>,
}

impl Global for DevicePairingRuntime {}

impl DevicePairingEngine {
    pub fn new(controller: SharedIssue31HostController) -> Self {
        Self {
            controller,
            pairing_offers: Arc::new(Mutex::new(BTreeMap::new())),
        }
    }

    pub fn issue(
        &self,
        endpoint: Issue31DirectEndpoint,
        host_public_key_hex: String,
        generation: u64,
        scopes: Vec<Issue31PairingScope>,
        now_millis: u64,
    ) -> Result<PairingBootstrap> {
        self.issue_with_identity_action(
            endpoint,
            host_public_key_hex,
            generation,
            scopes,
            now_millis,
            None,
        )
    }

    fn issue_with_identity_action(
        &self,
        endpoint: Issue31DirectEndpoint,
        host_public_key_hex: String,
        generation: u64,
        scopes: Vec<Issue31PairingScope>,
        now_millis: u64,
        identity_action: Option<omega_identity::IdentityActionAuthorization>,
    ) -> Result<PairingBootstrap> {
        let expires_at = now_millis.saturating_add(5 * 60 * 1_000);
        let secret_seed = rand::random::<[u8; 32]>();
        let pairing_secret = format!("{:x}", Sha256::digest(secret_seed));
        let secret_digest = format!("{:x}", Sha256::digest(pairing_secret.as_bytes()));
        let bootstrap = PairingBootstrap {
            schema: PAIRING_BOOTSTRAP_SCHEMA.into(),
            magic_dns_name: endpoint.magic_dns_name,
            port: endpoint.port,
            protocol: endpoint.protocol,
            host_public_key_hex: host_public_key_hex.clone(),
            pairing_secret,
            generation,
            issued_at: now_millis,
            expires_at,
        };
        bootstrap.validate(now_millis)?;
        let mut offers = self
            .pairing_offers
            .lock()
            .map_err(|_| anyhow!("device pairing registry is poisoned"))?;
        offers.retain(|_, offer| offer.expires_at > now_millis);
        if offers.len() >= 32 {
            return Err(anyhow!("device pairing registry is full"));
        }
        offers.insert(
            secret_digest,
            DevicePairingOffer {
                host_public_key_hex,
                scopes,
                expires_at,
                identity_action,
            },
        );
        Ok(bootstrap)
    }

    fn authority(&self) -> Issue31DeviceBridgeAuthority {
        Issue31DeviceBridgeAuthority {
            controller: self.controller.clone(),
            pairing_offers: self.pairing_offers.clone(),
            grant_recorder: None,
        }
    }
}

pub fn configure_device_pairing(
    engine: DevicePairingEngine,
    endpoint: Issue31DirectEndpoint,
    host_public_key_hex: String,
    generation: u64,
    scopes: Vec<Issue31PairingScope>,
    cx: &mut App,
) {
    cx.set_global(DevicePairingRuntime {
        engine,
        endpoint,
        host_public_key_hex,
        generation,
        scopes,
    });
}

/// Whether this process can already issue a direct device pairing bootstrap.
///
/// omega#124. The pairing control asks this before it asks for a bootstrap, so
/// a mode that has not yet started the transport can start it instead of
/// refusing the press.
pub fn has_device_pairing(cx: &App) -> bool {
    cx.has_global::<DevicePairingRuntime>()
}

pub fn issue_device_pairing_bootstrap(
    authorization: omega_identity::IdentityActionAuthorization,
    cx: &App,
) -> Result<PairingBootstrap> {
    if authorization.intent().kind != omega_identity::DurableIdentityActionKind::DeviceGrant {
        return Err(anyhow!(
            "the identity action does not authorize a device grant"
        ));
    }
    omega_identity::IdentityService::system(*app_identity::CHANNEL)
        .validate_identity_action_authorization(&authorization)?;
    let runtime = cx
        .try_global::<DevicePairingRuntime>()
        .ok_or_else(|| anyhow!("Direct phone pairing is not available on this host."))?;
    runtime.engine.issue_with_identity_action(
        runtime.endpoint.clone(),
        runtime.host_public_key_hex.clone(),
        runtime.generation,
        runtime.scopes.clone(),
        current_unix_millis(),
        Some(authorization),
    )
}

fn current_unix_millis() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |duration| {
            u64::try_from(duration.as_millis()).unwrap_or(u64::MAX)
        })
}

impl omega_device_bridge::GrantAuthority for Issue31DeviceBridgeAuthority {
    fn authorize(
        &self,
        device_public_key_hex: &str,
        host_public_key_hex: &str,
        grant_ref: Option<&str>,
        pairing_secret: Option<&str>,
        now_millis: u64,
    ) -> std::result::Result<DeviceBridgeGrantAdmission, GrantRefusalReason> {
        if let Some(pairing_secret) = pairing_secret {
            if grant_ref.is_some() {
                return Err(GrantRefusalReason::PairingRefused);
            }
            let secret_digest = format!("{:x}", Sha256::digest(pairing_secret.as_bytes()));
            let offer = self
                .pairing_offers
                .lock()
                .map_err(|_| GrantRefusalReason::PairingRefused)?
                .remove(&secret_digest)
                .ok_or(GrantRefusalReason::PairingRefused)?;
            if offer.host_public_key_hex != host_public_key_hex || now_millis >= offer.expires_at {
                return Err(GrantRefusalReason::PairingExpired);
            }
            if let Some(authorization) = &offer.identity_action
                && omega_identity::IdentityService::system(*app_identity::CHANNEL)
                    .validate_identity_action_authorization(authorization)
                    .is_err()
            {
                return Err(GrantRefusalReason::PairingRefused);
            }
            let mut controller = self
                .controller
                .write()
                .map_err(|_| GrantRefusalReason::PairingRefused)?;
            let grant = controller
                .issue_direct_pairing_grant(
                    device_public_key_hex.to_string(),
                    offer.scopes,
                    now_millis / 1_000,
                )
                .map_err(|_| GrantRefusalReason::PairingRefused)?;
            let Issue31PairingRecord::ScopedGrant {
                grant_ref,
                host_public_key_hex,
                device_public_key_hex,
                expires_at,
                generation,
                ..
            } = grant
            else {
                return Err(GrantRefusalReason::PairingRefused);
            };
            if let Some(recorder) = &self.grant_recorder {
                recorder();
            }
            return Ok(DeviceBridgeGrantAdmission {
                grant_ref,
                host_public_key_hex,
                device_public_key_hex,
                expires_at: expires_at.saturating_mul(1_000),
                generation,
            });
        }
        let grant_ref = grant_ref.ok_or(GrantRefusalReason::GrantMissing)?;
        let controller = self
            .controller
            .read()
            .map_err(|_| GrantRefusalReason::GrantMissing)?;
        let grant = controller
            .device_bridge_grant_state(grant_ref)
            .map_err(|_| GrantRefusalReason::GrantMissing)?
            .ok_or(GrantRefusalReason::GrantMissing)?;
        if grant.host_public_key_hex != host_public_key_hex
            || grant.device_public_key_hex != device_public_key_hex
        {
            return Err(GrantRefusalReason::GrantMissing);
        }
        if grant.status == Issue31GrantStatus::Revoked
            || controller.device_admission_is_revoked(device_public_key_hex)
        {
            return Err(GrantRefusalReason::GrantRevoked);
        }
        let now_seconds = now_millis / 1_000;
        if grant
            .expires_at
            .is_some_and(|expires_at| now_seconds >= expires_at)
        {
            return Err(GrantRefusalReason::GrantExpired);
        }
        let expires_at = grant
            .expires_at
            .and_then(|expires_at| expires_at.checked_mul(1_000))
            .unwrap_or(253_402_300_799_000);
        Ok(DeviceBridgeGrantAdmission {
            grant_ref: grant.grant_ref,
            host_public_key_hex: grant.host_public_key_hex,
            device_public_key_hex: grant.device_public_key_hex,
            expires_at,
            generation: grant.generation,
        })
    }
}

pub fn start_device_bridge_server(
    config: DeviceBridgeServerConfig,
    controller: SharedIssue31HostController,
    journal: ProjectionJournal,
) -> Result<DeviceBridgeServerHandle, DeviceBridgeError> {
    let engine = DevicePairingEngine::new(controller);
    let authority = engine
        .authority()
        .with_grant_recorder(device_grant_backup_value_recorder());
    DeviceBridgeServerHandle::spawn(config, Arc::new(authority), journal)
}

pub fn start_pairable_device_bridge_server(
    config: DeviceBridgeServerConfig,
    engine: DevicePairingEngine,
    journal: ProjectionJournal,
) -> Result<DeviceBridgeServerHandle, DeviceBridgeError> {
    let authority = engine
        .authority()
        .with_grant_recorder(device_grant_backup_value_recorder());
    DeviceBridgeServerHandle::spawn(config, Arc::new(authority), journal)
}

/// omega#164. A freshly minted device grant gives the background-created
/// identity something to lose, so it arms the quiet backup nudge. The record
/// is durable, idempotent, and fail-soft — never a pairing blocker.
fn device_grant_backup_value_recorder() -> Arc<dyn Fn() + Send + Sync> {
    Arc::new(|| {
        if let Err(error) = omega_identity::IdentityService::system(*app_identity::CHANNEL)
            .record_backup_value_accrued(omega_identity::BackupValueKind::DeviceGrant)
        {
            log::warn!("could not record identity backup value accrual: {error}");
        }
    })
}

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
    fn device_bridge_uses_the_durable_issue31_revocation_state() {
        let (configuration, mut controller, device_public_key_hex, grant_ref) =
            crate::issue31_nostr::paired_fixture(vec![Issue31PairingScope::ObserveIssue31]);
        let authority = Issue31DeviceBridgeAuthority {
            controller: Arc::new(RwLock::new(controller.clone())),
            pairing_offers: Arc::new(Mutex::new(BTreeMap::new())),
            grant_recorder: None,
        };
        let admission = omega_device_bridge::GrantAuthority::authorize(
            &authority,
            &device_public_key_hex,
            &configuration.host_public_key_hex,
            Some(&grant_ref),
            None,
            200_000,
        )
        .expect("active grant is admitted");
        assert_eq!(admission.grant_ref, grant_ref);

        let revocation = controller
            .revoke_grant(
                &admission.grant_ref,
                201,
                Some("reason.omega.owner_revoked".into()),
            )
            .expect("revoke");
        controller
            .record_emitted_pairing("f".repeat(64), revocation)
            .expect("record revocation");
        let authority = Issue31DeviceBridgeAuthority {
            controller: Arc::new(RwLock::new(controller)),
            pairing_offers: Arc::new(Mutex::new(BTreeMap::new())),
            grant_recorder: None,
        };
        assert_eq!(
            omega_device_bridge::GrantAuthority::authorize(
                &authority,
                &device_public_key_hex,
                &configuration.host_public_key_hex,
                Some(&admission.grant_ref),
                None,
                202_000,
            ),
            Err(GrantRefusalReason::GrantRevoked)
        );
    }

    #[test]
    fn direct_pairing_secret_is_one_use_and_mints_a_scoped_grant() {
        let (configuration, controller, _, _) =
            crate::issue31_nostr::paired_fixture(vec![Issue31PairingScope::ObserveIssue31]);
        let controller = Arc::new(RwLock::new(controller));
        let engine = DevicePairingEngine::new(controller);
        let bootstrap = engine
            .issue(
                Issue31DirectEndpoint {
                    magic_dns_name: "omega-primary.tail1234.ts.net".into(),
                    port: 4317,
                    protocol: DEVICE_BRIDGE_PROTOCOL.into(),
                },
                configuration.host_public_key_hex.clone(),
                configuration.generation,
                vec![
                    Issue31PairingScope::ObserveIssue31,
                    Issue31PairingScope::SendMessage,
                ],
                1_000_000,
            )
            .expect("pairing bootstrap");
        let authority = engine.authority();
        let device_public_key_hex = "9".repeat(64);
        let admission = omega_device_bridge::GrantAuthority::authorize(
            &authority,
            &device_public_key_hex,
            &configuration.host_public_key_hex,
            None,
            Some(&bootstrap.pairing_secret),
            1_001_000,
        )
        .expect("direct pairing admission");
        assert_eq!(admission.device_public_key_hex, device_public_key_hex);
        assert_eq!(admission.generation, 1);
        assert_eq!(
            omega_device_bridge::GrantAuthority::authorize(
                &authority,
                &device_public_key_hex,
                &configuration.host_public_key_hex,
                None,
                Some(&bootstrap.pairing_secret),
                1_002_000,
            ),
            Err(GrantRefusalReason::PairingRefused)
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
    fn typed_all_work_reads_cross_the_supervised_process_boundary() {
        smol::block_on(async {
            let root = tempdir().expect("tempdir");
            let mut supervisor = OmegaEffectdSupervisor::new(OmegaEffectdSupervisorOptions {
                data_root: root.path().to_path_buf(),
                command: fixture_command(&fixture_path()),
                initial_generation: 1,
                request_timeout: Duration::from_secs(5),
            });

            let initialized = supervisor.start().await.expect("start");
            assert_eq!(
                initialized
                    .all_work
                    .as_ref()
                    .map(|result| &result.selected_version),
                Some(&all_work_contract::ProtocolVersion::OmegaEffectdV2)
            );
            let index = supervisor
                .read_work_index(all_work_contract::WorkIndexReadRequest {
                    cursor: None,
                    limit: None,
                    filter: None,
                })
                .await
                .expect("typed Work Index read");
            assert_eq!(index.items.len(), 1);
            assert_eq!(
                index.items.first().expect("one Work item").work_ref.0,
                "work:fixture:1"
            );

            let snapshot = supervisor
                .read_work_snapshot(all_work_contract::WorkSnapshotReadRequest {
                    work_ref: all_work_contract::WorkRef::try_from("work:fixture:1".to_string())
                        .expect("valid Work ref"),
                })
                .await
                .expect("typed Work snapshot read");
            assert_eq!(
                snapshot.snapshot.run_refs.first().expect("one Run ref").0,
                "run:fixture:1"
            );
            let assigned = supervisor
                .execute_work_command(all_work_contract::WorkCommandExecuteRequest {
                    intent_ref: all_work_contract::IntentRef::try_from(
                        "intent:fixture:assign".to_string(),
                    )
                    .expect("intent ref"),
                    idempotency_key: all_work_contract::IdempotencyKey::try_from(
                        "fixture-work-command-assign".to_string(),
                    )
                    .expect("idempotency key"),
                    expected_revision: all_work_contract::SafeInteger(1),
                    effective_principal_ref: all_work_contract::PrincipalRef::try_from(
                        "principal:omega:owner".to_string(),
                    )
                    .expect("principal ref"),
                    organization_ref: all_work_contract::OrganizationRef::try_from(
                        "organization:openagents".to_string(),
                    )
                    .expect("organization ref"),
                    capability_ref: all_work_contract::CapabilityRef::try_from(
                        "capability:work-command:execute".to_string(),
                    )
                    .expect("capability ref"),
                    work_ref: all_work_contract::WorkRef::try_from("work:fixture:1".to_string())
                        .expect("Work ref"),
                    occurred_at: all_work_contract::IsoTimestamp::try_from(
                        "2026-08-03T11:00:00.000Z".to_string(),
                    )
                    .expect("timestamp"),
                    command: all_work_contract::WorkCommand::Assign {
                        assignee: all_work_contract::HumanAssignee {
                            kind: all_work_contract::AssigneeKind::Human,
                            principal_ref: all_work_contract::PrincipalRef::try_from(
                                "principal:omega:owner".to_string(),
                            )
                            .expect("assignee ref"),
                        },
                    },
                })
                .await
                .expect("typed Work command");
            assert_eq!(
                assigned
                    .snapshot
                    .summary
                    .assignee
                    .0
                    .as_ref()
                    .expect("assigned human")
                    .principal_ref
                    .0,
                "principal:omega:owner"
            );
            assert_eq!(assigned.receipt.github_write_count.0, 0);
            let planning = supervisor
                .read_planning_graph(all_work_contract::PlanningGraphReadRequest {
                    after_revision: None,
                })
                .await
                .expect("typed planning graph read");
            assert_eq!(planning.graph.revision.0, 1);
            assert_eq!(planning.graph.graph_ref.0, "planning-graph:fixture");
            let cutover = supervisor
                .read_work_cutover(all_work_contract::WorkCutoverReadRequest {})
                .await
                .expect("typed Work cutover read");
            assert_eq!(
                cutover.state.writer,
                all_work_contract::WorkWriter::LegacyGithub
            );
            let activated = supervisor
                .execute_work_cutover(all_work_contract::WorkCutoverExecuteRequest {
                    intent_ref: all_work_contract::IntentRef::try_from(
                        "intent:fixture:cutover".to_string(),
                    )
                    .expect("intent ref"),
                    idempotency_key: all_work_contract::IdempotencyKey::try_from(
                        "fixture-work-cutover".to_string(),
                    )
                    .expect("idempotency key"),
                    expected_revision: cutover.state.revision,
                    expected_generation: cutover.state.generation,
                    effective_principal_ref: all_work_contract::PrincipalRef::try_from(
                        "principal:omega:local-owner".to_string(),
                    )
                    .expect("principal ref"),
                    organization_ref: all_work_contract::OrganizationRef::try_from(
                        "organization:openagents".to_string(),
                    )
                    .expect("organization ref"),
                    capability_ref: all_work_contract::CapabilityRef::try_from(
                        "capability:work-cutover:write".to_string(),
                    )
                    .expect("capability ref"),
                    occurred_at: all_work_contract::IsoTimestamp::try_from(
                        "2026-08-03T12:00:00.000Z".to_string(),
                    )
                    .expect("timestamp"),
                    github_write_count: all_work_contract::ZeroInteger(0),
                    command: all_work_contract::WorkCutoverCommand::ActivateNative {
                        source_digest: cutover.state.source_digest,
                        reconciled_cursor: cutover.state.source_cursor,
                        receipt_ref: all_work_contract::ReceiptRef::try_from(
                            "receipt:fixture:cutover".to_string(),
                        )
                        .expect("receipt ref"),
                    },
                })
                .await
                .expect("typed Work cutover command");
            assert_eq!(
                activated.state.writer,
                all_work_contract::WorkWriter::NativeOmega
            );
            assert_eq!(activated.receipt.github_write_count.0, 0);
            let candidate = supervisor
                .execute_strict_bug_candidate(
                    serde_json::from_value(serde_json::json!({
                        "intentRef": "intent:fixture:strict-bug:ingest",
                        "idempotencyKey": "github-delivery:source:github:webhook:delivery:10001",
                        "expectedRevision": 0,
                        "effectivePrincipalRef": "principal:github:webhook",
                        "capabilityRef": "capability:strict-bug-candidate:ingest",
                        "occurredAt": "2026-08-03T12:01:00Z",
                        "githubWriteCount": 0,
                        "command": {
                            "command": "ingest",
                            "candidateRef": "strict-bug-candidate:omega:10001",
                            "sourceRef": "source:github:omega:issue:10001",
                            "deliveryRef": "source:github:webhook:delivery:10001",
                            "repositoryRef": "repository:omega",
                            "issueNumber": 10001,
                            "sourceUrl": "https://github.com/OpenAgentsInc/omega/issues/10001",
                            "title": "Strict fixture failure",
                            "affectedSurface": "Work candidate inbox",
                            "actualBehavior": "The report is absent.",
                            "expectedBehavior": "The report enters pending triage.",
                            "reproductionSteps": "1. Submit the strict form. 2. Open Work.",
                            "publicSafeEvidence": "The public issue exists.",
                            "severity": "s2",
                            "environment": "Fixture process at 2026-08-03T12:01:00Z.",
                            "safetyRedaction": "Sensitive values were removed.",
                            "requiredConfirmations": [
                                "specific_reproducible_bug",
                                "searched_existing_reports",
                                "sensitive_material_removed",
                                "exact_reproduction_and_evidence",
                                "malformed_report_policy_understood"
                            ],
                            "reporterRef": "source:github:user:fixture",
                            "attachmentRefs": [],
                            "signatureVerificationRef": "evidence:github-webhook-signature:delivery:10001"
                        }
                    }))
                    .expect("typed strict bug ingest request"),
                )
                .await
                .expect("typed strict bug candidate ingest");
            assert_eq!(candidate.receipt.github_write_count.0, 0);
            let candidate = candidate
                .ledger
                .candidates
                .first()
                .expect("one strict bug candidate");
            assert!(candidate.untrusted);
            assert_eq!(
                candidate.disposition,
                all_work_contract::StrictBugDisposition::Pending
            );
            let candidates = supervisor
                .read_strict_bug_candidates(all_work_contract::StrictBugCandidateReadRequest {
                    candidate_ref: None,
                })
                .await
                .expect("typed strict bug candidate read");
            assert_eq!(candidates.ledger.candidates.len(), 1);
            let claims = supervisor
                .read_repository_claims(all_work_contract::RepositoryClaimReadRequest {
                    after_revision: None,
                    repository_ref: None,
                    work_ref: None,
                })
                .await
                .expect("typed repository claim read");
            assert!(claims.ledger.packets.is_empty());
            let created = supervisor
                .execute_repository_claim(all_work_contract::RepositoryClaimExecuteRequest {
                    request_ref: all_work_contract::ClaimRequestRef::try_from(
                        "claim-request:fixture:create".to_string(),
                    )
                    .expect("request ref"),
                    idempotency_key: all_work_contract::IdempotencyKey::try_from(
                        "fixture-create-packet".to_string(),
                    )
                    .expect("idempotency key"),
                    expected_revision: all_work_contract::SafeInteger(0),
                    effective_principal_ref: all_work_contract::PrincipalRef::try_from(
                        "principal:fixture:owner".to_string(),
                    )
                    .expect("principal ref"),
                    capability_ref: all_work_contract::CapabilityRef::try_from(
                        "capability:repository-claim:write".to_string(),
                    )
                    .expect("capability ref"),
                    occurred_at: all_work_contract::IsoTimestamp::try_from(
                        "2026-08-03T08:30:00.000Z".to_string(),
                    )
                    .expect("timestamp"),
                    command: all_work_contract::RepositoryClaimCommand::CreatePacket {
                        packet_ref: all_work_contract::WorkPacketRef::try_from(
                            "work-packet:fixture:224".to_string(),
                        )
                        .expect("packet ref"),
                        work_ref: all_work_contract::WorkRef::try_from(
                            "work:github:openagentsinc/omega:224".to_string(),
                        )
                        .expect("work ref"),
                        repository_ref: all_work_contract::RepositoryRef::try_from(
                            "repository:omega".to_string(),
                        )
                        .expect("repository ref"),
                        title: all_work_contract::ShortText::try_from(
                            "Move repository claims into Omega".to_string(),
                        )
                        .expect("title"),
                        scope: all_work_contract::LongText::try_from(
                            "Exercise the generated Omega claim client.".to_string(),
                        )
                        .expect("scope"),
                        owned_paths: vec![],
                        hot_files: vec![],
                        hot_contracts: vec![],
                        verification: all_work_contract::LongText::try_from(
                            "Run the final deferred repository claim suite.".to_string(),
                        )
                        .expect("verification"),
                    },
                })
                .await
                .expect("typed Work Packet create");
            assert_eq!(created.ledger.revision.0, 1);
            assert_eq!(created.receipt.github_write_count.0, 0);
            let claimed = supervisor
                .execute_repository_claim(all_work_contract::RepositoryClaimExecuteRequest {
                    request_ref: all_work_contract::ClaimRequestRef::try_from(
                        "claim-request:fixture:claim".to_string(),
                    )
                    .expect("request ref"),
                    idempotency_key: all_work_contract::IdempotencyKey::try_from(
                        "fixture-claim-packet".to_string(),
                    )
                    .expect("idempotency key"),
                    expected_revision: all_work_contract::SafeInteger(1),
                    effective_principal_ref: all_work_contract::PrincipalRef::try_from(
                        "principal:fixture:owner".to_string(),
                    )
                    .expect("principal ref"),
                    capability_ref: all_work_contract::CapabilityRef::try_from(
                        "capability:repository-claim:write".to_string(),
                    )
                    .expect("capability ref"),
                    occurred_at: all_work_contract::IsoTimestamp::try_from(
                        "2026-08-03T08:31:00.000Z".to_string(),
                    )
                    .expect("timestamp"),
                    command: all_work_contract::RepositoryClaimCommand::ClaimPacket {
                        packet_ref: all_work_contract::WorkPacketRef::try_from(
                            "work-packet:fixture:224".to_string(),
                        )
                        .expect("packet ref"),
                        claim_ref: all_work_contract::RepositoryWorkClaimRef::try_from(
                            "repository-claim:fixture:224".to_string(),
                        )
                        .expect("claim ref"),
                    },
                })
                .await
                .expect("typed repository claim");
            assert_eq!(claimed.ledger.revision.0, 2);
            assert_eq!(claimed.ledger.claims.len(), 1);
            assert_eq!(claimed.ledger.claims[0].generation.0, 1);
            let workroom = supervisor
                .read_signed_workroom(all_work_contract::SignedWorkroomReadRequest {
                    after_revision: None,
                    workroom_ref: None,
                    work_ref: Some(Some(
                        all_work_contract::WorkRef::try_from(
                            "work:github:openagentsinc/omega:216".to_string(),
                        )
                        .expect("Work ref"),
                    )),
                })
                .await
                .expect("typed signed Workroom read");
            assert!(workroom.ledger.activities.is_empty());
            assert!(workroom.ledger.outbox.is_empty());
            supervisor.stop().await.expect("stop");
        });
    }

    #[test]
    #[ignore = "requires the pinned OpenAgents source checkout"]
    fn typed_all_work_index_crosses_the_openagents_process_boundary() {
        smol::block_on(async {
            let source_root = std::env::var_os("OPENAGENTS_ALL_WORK_SOURCE_ROOT")
                .map(PathBuf::from)
                .expect("OPENAGENTS_ALL_WORK_SOURCE_ROOT must name the pinned checkout");
            let entry = source_root.join("packages/omega-effectd/src/bin/omega-effectd.ts");
            assert!(entry.is_file(), "OpenAgents omega-effectd entry must exist");
            let pnpm = std::env::var_os("PNPM")
                .map(PathBuf::from)
                .unwrap_or_else(|| PathBuf::from("pnpm"));
            let root = tempdir().expect("tempdir");
            let mut supervisor = OmegaEffectdSupervisor::new(OmegaEffectdSupervisorOptions {
                data_root: root.path().to_path_buf(),
                command: OmegaEffectdCommand {
                    program: pnpm,
                    args: vec![
                        "--dir".to_string(),
                        source_root.display().to_string(),
                        "exec".to_string(),
                        "tsx".to_string(),
                        entry.display().to_string(),
                    ],
                },
                initial_generation: 1,
                request_timeout: Duration::from_secs(15),
            });

            let initialized = supervisor.start().await.expect("start OpenAgents process");
            assert_eq!(
                initialized
                    .all_work
                    .as_ref()
                    .map(|result| &result.selected_version),
                Some(&all_work_contract::ProtocolVersion::OmegaEffectdV2)
            );
            let index = supervisor
                .read_work_index(all_work_contract::WorkIndexReadRequest {
                    cursor: None,
                    limit: None,
                    filter: None,
                })
                .await
                .expect("typed Work Index read from OpenAgents process");
            assert!(index.items.is_empty());
            let planning = supervisor
                .read_planning_graph(all_work_contract::PlanningGraphReadRequest {
                    after_revision: None,
                })
                .await
                .expect("typed planning graph read from OpenAgents process");
            assert_eq!(planning.graph.work.len(), 34);
            assert_eq!(planning.graph.source_coordinates.len(), 34);
            assert_eq!(
                planning
                    .graph
                    .work
                    .iter()
                    .filter(|work| {
                        work.summary.state == all_work_contract::WorkState::Completed
                    })
                    .count(),
                6
            );
            supervisor.stop().await.expect("stop OpenAgents process");
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
