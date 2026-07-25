use std::cell::RefCell;
use std::collections::HashMap as StdHashMap;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::{Arc, LazyLock, Mutex};
use std::time::Duration;

use acp_thread::{AgentThreadEntry, ThreadStatus};
use chrono::Utc;
use gpui::{AnyWindowHandle, App, AppContext, AsyncApp, Entity, WeakEntity};
use omega_effectd::{
    ConversationIdentity, HostMethod, HostRequestFrame, HostResponseError, HostResponseErrorCode,
    OmegaEffectdHostHandler, OpenAgentsSession, SARAH_METHOD_BOOTSTRAP, SARAH_METHOD_DEVICE_GRANTS,
    SARAH_METHOD_INTERRUPT_TURN, SARAH_METHOD_RENEW_DEVICE_GRANT, SARAH_METHOD_REVOKE_DEVICE_GRANT,
    SARAH_METHOD_ROOM_SNAPSHOT, SARAH_METHOD_SEND_MESSAGE, SARAH_METHOD_SESSION_STATUS,
    SarahConversationClient, SarahConversationConfig, VerifiedOpenAgentsSession,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use workspace::{AppState, Workspace};

use crate::agent_panel::CreateThreadOptions;
use crate::{
    Agent, AgentPanel, AgentThreadSource, ConversationView, ThreadId,
    agent_connection_store::AgentConnectionStatus,
};

const SUPERVISED_WORKSPACE_REF: &str = "workspace.omega.supervised";
const CODEX_LOCAL_LANE: &str = "codex-local";
const CLAUDE_LOCAL_LANE: &str = "claude-local";
const CODEX_AGENT_ID: &str = "codex-acp";
const CLAUDE_AGENT_ID: &str = "claude-acp";
const THREAD_CONNECT_ATTEMPTS: usize = 100;
const THREAD_CONNECT_INTERVAL: Duration = Duration::from_millis(100);
const CLAUDE_TURN_TIMEOUT: Duration = Duration::from_secs(10 * 60);
const ACP_CANCEL_GRACE_PERIOD: Duration = Duration::from_secs(10);
const MAX_ASSISTANT_TEXT_BYTES: usize = 24 * 1024;
const MAX_EVIDENCE_TURNS: usize = 48;
const MAX_TOTAL_ASSISTANT_TEXT_BYTES: usize = 6 * 1024;
const CORRELATION_SCHEMA: &str = "openagents.omega.full_auto_host_correlation.v1";
const CORRELATION_FILE: &str = "full-auto-host-correlation.json";

/// OMEGA-DELTA-0021. Which `omega-effectd` lane run each host-bridge thread
/// belongs to, so the thread surface can disclose it.
///
/// A read-mostly index, not a store. It is derived twice from the correlation
/// journal — once when the journal is loaded at startup, and again on every
/// write — so it cannot outlive or disagree with the durable record. omega#77's
/// falsifier names a *new durable* store for disclosure; this is neither new
/// nor durable, and deleting the file it mirrors empties it.
///
/// It exists because `HostBridgeState` lives inside the host handler's closure
/// and is not reachable from a render, while the disclosure has to be readable
/// wherever a thread is drawn.
static ENGINE_LANE_RUNS: LazyLock<Mutex<StdHashMap<ThreadId, String>>> =
    LazyLock::new(|| Mutex::new(StdHashMap::new()));

/// The lane index a set of host threads implies.
fn engine_lane_runs_from(threads: &[HostThread]) -> StdHashMap<ThreadId, String> {
    threads
        .iter()
        .map(|thread| (thread.thread_id, thread.operation_ref.clone()))
        .collect()
}

/// Replace the lane index with what the correlation journal now says.
///
/// Wholesale replacement rather than insertion: a thread dropped from the
/// journal must stop being disclosed as a lane run, and an incremental index
/// would keep disclosing it.
fn republish_engine_lane_runs(threads: &[HostThread]) {
    let runs = engine_lane_runs_from(threads);
    match ENGINE_LANE_RUNS.lock() {
        Ok(mut index) => *index = runs,
        Err(poisoned) => *poisoned.into_inner() = runs,
    }
}

/// Publish one lane correlation, for a harness that has no engine to run.
///
/// `OMEGA-DELTA-0021`'s engine-lane disclosure is normally reached only by a
/// real `omega-effectd` run, which a rendering harness cannot start. This is
/// the seam that lets the *rendered* engine-lane line be captured without
/// weakening the production path: it is behind `test-support`, so no shipped
/// build can reach it, and it writes the same index the correlation journal
/// writes.
#[cfg(any(test, feature = "test-support"))]
pub fn publish_engine_lane_run_for_tests(thread_id: ThreadId, operation_ref: String) {
    match ENGINE_LANE_RUNS.lock() {
        Ok(mut index) => {
            index.insert(thread_id, operation_ref);
        }
        Err(poisoned) => {
            poisoned.into_inner().insert(thread_id, operation_ref);
        }
    }
}

/// The `omega-effectd` lane run this thread belongs to, if it is one.
///
/// Returns `None` for every thread the user started themselves, which is what
/// keeps a hand-driven `codex-acp` thread disclosed as routed rather than as a
/// delegated lane run.
pub fn engine_lane_run(thread_id: ThreadId) -> Option<String> {
    match ENGINE_LANE_RUNS.lock() {
        Ok(index) => index.get(&thread_id).cloned(),
        Err(poisoned) => poisoned.into_inner().get(&thread_id).cloned(),
    }
}

#[derive(Clone)]
struct WorkspaceBinding {
    workspace_ref: String,
    workspace: WeakEntity<Workspace>,
    window: AnyWindowHandle,
}

#[derive(Clone)]
struct HostThread {
    workspace_ref: String,
    lane: String,
    operation_ref: String,
    thread_id: ThreadId,
    conversation: Option<WeakEntity<ConversationView>>,
    turns: Vec<HostTurn>,
    revision: u64,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct HostTurn {
    turn_ref: String,
    lane: String,
    account_ref: Option<String>,
    model: Option<String>,
    provider_session_ref: String,
    start_entry_index: usize,
    end_entry_index: Option<usize>,
    phase: String,
    disposition: Option<String>,
    created_at: String,
    updated_at: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum HostTurnOutcome {
    Completed,
    Failed,
    TimedOut,
}

struct HostBridgeState {
    workspace: Option<WorkspaceBinding>,
    threads: Vec<HostThread>,
    correlation_path: PathBuf,
    load_error: Option<String>,
    sarah_conversation: Option<Arc<Mutex<SarahConversationClient>>>,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct CorrelationJournal {
    schema: String,
    threads: Vec<PersistedHostThread>,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct PersistedHostThread {
    workspace_ref: String,
    lane: String,
    operation_ref: String,
    thread_ref: String,
    turns: Vec<HostTurn>,
    revision: u64,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ResolveWorkspaceParams {
    expected_workspace_ref: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ResolveSyncSessionParams {}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct CreateThreadParams {
    title: String,
    lane: String,
    workspace_ref: String,
    operation_ref: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct LaneReadinessParams {
    lane: String,
    excluding_thread_ref: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct DispatchTurnParams {
    run_ref: String,
    workspace_ref: String,
    thread_ref: String,
    turn_ref: String,
    message: String,
    profile: Option<DispatchProfile>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct DispatchProfile {
    lane: Option<String>,
    account_ref: Option<String>,
    model: Option<String>,
    reasoning_effort: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RefreshEvidenceParams {
    run_ref: String,
    thread_ref: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct InterruptTurnParams {
    thread_ref: String,
    turn_ref: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct AppendSystemNoteParams {
    thread_ref: String,
    note_ref: String,
    text: String,
}

pub fn omega_effectd_host_handler(cx: &App) -> OmegaEffectdHostHandler {
    let async_cx = cx.to_async();
    let openagents_session = omega_effectd::openagents_session(cx);
    let correlation_path = paths::data_dir().join("openagents").join(CORRELATION_FILE);
    let (threads, load_error) = match load_correlation_journal(&correlation_path) {
        Ok(threads) => (threads, None),
        Err(error) => (Vec::new(), Some(error.to_string())),
    };
    // OMEGA-DELTA-0021. This is the restart edge: the lane index is empty in a
    // freshly started process, and the journal on disk is what refills it, so a
    // thread resumed after a restart still discloses the lane that owns it.
    republish_engine_lane_runs(&threads);
    let state = Rc::new(RefCell::new(HostBridgeState {
        workspace: None,
        threads,
        correlation_path,
        load_error,
        sarah_conversation: None,
    }));
    Rc::new(move |request| {
        let async_cx = async_cx.clone();
        let state = state.clone();
        let openagents_session = openagents_session.clone();
        Box::pin(async move { handle_request(request, state, openagents_session, async_cx).await })
    })
}

fn production_sarah_conversation() -> Result<SarahConversationClient, HostResponseError> {
    let relay_urls = std::env::var("OPENAGENTS_OMEGA_NOSTR_RELAYS")
        .map_err(|_| unavailable("OPENAGENTS_OMEGA_NOSTR_RELAYS is not configured."))?
        .split(',')
        .map(str::trim)
        .filter(|relay_url| !relay_url.is_empty())
        .map(str::to_owned)
        .collect::<Vec<_>>();
    let sarah_public_key_hex = std::env::var("OPENAGENTS_OMEGA_SARAH_PUBLIC_KEY_HEX")
        .map_err(|_| unavailable("OPENAGENTS_OMEGA_SARAH_PUBLIC_KEY_HEX is not configured."))?;
    let admitted_device_public_key_hexes =
        std::env::var("OPENAGENTS_OMEGA_NOSTR_DEVICE_PUBLIC_KEYS")
            .map_err(|_| {
                unavailable("OPENAGENTS_OMEGA_NOSTR_DEVICE_PUBLIC_KEYS is not configured.")
            })?
            .split(',')
            .map(str::trim)
            .filter(|public_key| !public_key.is_empty())
            .map(str::to_owned)
            .collect::<Vec<_>>();
    let approved_device_scopes = std::env::var("OPENAGENTS_OMEGA_NOSTR_DEVICE_SCOPES")
        .map_err(|_| unavailable("OPENAGENTS_OMEGA_NOSTR_DEVICE_SCOPES is not configured."))?
        .split(',')
        .map(str::trim)
        .filter(|scope| !scope.is_empty())
        .map(omega_effectd::Issue31PairingScope::parse)
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| {
            unavailable(format!("Issue 31 device scope policy is invalid: {error}"))
        })?;
    let community_group_ids = std::env::var("OPENAGENTS_OMEGA_NOSTR_COMMUNITY_GROUP_IDS")
        .unwrap_or_default()
        .split(',')
        .map(str::trim)
        .filter(|group_id| !group_id.is_empty())
        .map(str::to_owned)
        .collect::<Vec<_>>();
    let community_public_key_hexes = std::env::var("OPENAGENTS_OMEGA_NOSTR_COMMUNITY_PUBLIC_KEYS")
        .unwrap_or_default()
        .split(',')
        .map(str::trim)
        .filter(|public_key| !public_key.is_empty())
        .map(str::to_owned)
        .collect::<Vec<_>>();
    let identity_service = Arc::new(omega_identity::IdentityService::system(
        *app_identity::CHANNEL,
    ));
    let custody = identity_service
        .inspect()
        .map_err(|error| unavailable(format!("Omega identity custody is unavailable: {error}")))?;
    let owner_public_key_hex = custody
        .identity
        .map(|identity| identity.public_key_hex().as_str().to_string())
        .ok_or_else(|| unavailable("Omega identity custody is not ready."))?;
    let conversation_digest = std::env::var("OPENAGENTS_OMEGA_SARAH_CONVERSATION_DIGEST")
        .unwrap_or_else(|_| owner_public_key_hex.chars().take(24).collect());
    let config = SarahConversationConfig {
        generation: 1,
        conversation_digest,
        identity: ConversationIdentity {
            owner_public_key_hex,
            sarah_public_key_hex,
            account_label: None,
            binding_state: omega_effectd::BindingState::Unbound,
        },
        relay_url: relay_urls.first().cloned(),
        admitted_device_public_key_hexes,
        approved_device_scopes,
        community_group_ids,
        community_public_key_hexes,
    };
    let mut conversation = SarahConversationClient::new_production(
        config,
        relay_urls,
        identity_service,
    )
    .map_err(|error| unavailable(format!("Sarah Nostr transport is unavailable: {error}")))?;
    // omega#49: the host pump publishes the omega#47 snapshot and its Full Auto
    // detail to every admitted device. Without this the two documents are built
    // by the desktop panel and never leave the machine, and a paired phone
    // reports `no_host_projection` for a host that is running work.
    conversation.set_issue31_host_projection_source(
        full_auto_ui::issue31_host_projection_source(),
    );
    Ok(conversation)
}

async fn sarah_request(
    request: HostRequestFrame,
    state: &Rc<RefCell<HostBridgeState>>,
    cx: &mut AsyncApp,
) -> Result<Value, HostResponseError> {
    let method = match request.method {
        HostMethod::SarahSessionStatus => SARAH_METHOD_SESSION_STATUS,
        HostMethod::SarahBootstrap => SARAH_METHOD_BOOTSTRAP,
        HostMethod::SarahRoomSnapshot => SARAH_METHOD_ROOM_SNAPSHOT,
        HostMethod::SarahSendMessage => SARAH_METHOD_SEND_MESSAGE,
        HostMethod::SarahInterruptTurn => SARAH_METHOD_INTERRUPT_TURN,
        HostMethod::SarahDeviceGrants => SARAH_METHOD_DEVICE_GRANTS,
        HostMethod::SarahRenewDeviceGrant => SARAH_METHOD_RENEW_DEVICE_GRANT,
        HostMethod::SarahRevokeDeviceGrant => SARAH_METHOD_REVOKE_DEVICE_GRANT,
        _ => return Err(unsupported("Unknown Sarah host method.")),
    };
    let conversation = state.borrow().sarah_conversation.clone();
    let conversation = match conversation {
        Some(conversation) => conversation,
        None => match production_sarah_conversation() {
            Ok(conversation) => {
                let conversation = Arc::new(Mutex::new(conversation));
                let mut state = state.borrow_mut();
                state.sarah_conversation = Some(conversation.clone());
                conversation
            }
            Err(error) => return Err(error),
        },
    };
    let params = request.params;
    let generation = request.generation;
    cx.background_spawn(async move {
        let mut conversation = conversation
            .lock()
            .map_err(|_| unavailable("Sarah Nostr transport state is unavailable."))?;
        conversation
            .synchronize_process_generation(generation)
            .map_err(sarah_host_error)?;
        conversation
            .handle_request(method, generation, Some(&params))
            .map_err(sarah_host_error)
    })
    .await
}

fn sarah_host_error(error: omega_effectd::SarahConversationError) -> HostResponseError {
    let code = match error.protocol_code() {
        omega_effectd::ProtocolErrorCode::StaleGeneration => HostResponseErrorCode::StaleGeneration,
        omega_effectd::ProtocolErrorCode::InvalidRequest => HostResponseErrorCode::InvalidRequest,
        omega_effectd::ProtocolErrorCode::UnknownMethod => HostResponseErrorCode::Unsupported,
        omega_effectd::ProtocolErrorCode::HostUnavailable => HostResponseErrorCode::Unavailable,
        omega_effectd::ProtocolErrorCode::HostTimeout => HostResponseErrorCode::Unavailable,
        omega_effectd::ProtocolErrorCode::NotRunning
        | omega_effectd::ProtocolErrorCode::RunNotFound => HostResponseErrorCode::Unavailable,
        omega_effectd::ProtocolErrorCode::FrameTooLarge => HostResponseErrorCode::InvalidRequest,
        omega_effectd::ProtocolErrorCode::Internal => HostResponseErrorCode::Internal,
    };
    HostResponseError {
        code,
        message: error.to_string(),
    }
}

async fn handle_request(
    request: HostRequestFrame,
    state: Rc<RefCell<HostBridgeState>>,
    openagents_session: OpenAgentsSession,
    mut cx: AsyncApp,
) -> Result<Value, HostResponseError> {
    match request.method.clone() {
        HostMethod::ResolveWorkspace => resolve_workspace(request.params, &state, &mut cx),
        HostMethod::ResolveSyncSession => {
            resolve_sync_session(request.params, &openagents_session, &mut cx).await
        }
        HostMethod::CreateThread => create_thread(request.params, &state, &mut cx),
        HostMethod::LaneReadiness => lane_readiness(request.params, &state, &cx),
        HostMethod::DispatchTurn => dispatch_turn(request.params, &state, &mut cx).await,
        HostMethod::RefreshEvidence => {
            refresh_evidence(request.params, request.generation, &state, &cx)
        }
        HostMethod::InterruptTurn => interrupt_turn(request.params, &state, &mut cx).await,
        HostMethod::AppendSystemNote => append_system_note(request.params),
        HostMethod::SarahSessionStatus
        | HostMethod::SarahBootstrap
        | HostMethod::SarahRoomSnapshot
        | HostMethod::SarahSendMessage
        | HostMethod::SarahInterruptTurn
        | HostMethod::SarahDeviceGrants => sarah_request(request, &state, &mut cx).await,
        HostMethod::SarahRenewDeviceGrant | HostMethod::SarahRevokeDeviceGrant => {
            sarah_request(request, &state, &mut cx).await
        }
        HostMethod::Unsupported => Err(unsupported("Unknown Omega host method.")),
    }
}

async fn resolve_sync_session(
    params: Value,
    session: &OpenAgentsSession,
    cx: &mut AsyncApp,
) -> Result<Value, HostResponseError> {
    let _: ResolveSyncSessionParams = decode_params(params)?;
    Ok(sync_session_result(session.resolve_verified(cx).await))
}

fn sync_session_result(session: Option<VerifiedOpenAgentsSession>) -> Value {
    match session {
        Some(session) => json!({
            "available": true,
            "baseUrl": session.base_url,
            "accessToken": session.access_token,
        }),
        None => json!({ "available": false }),
    }
}

fn resolve_workspace(
    params: Value,
    state: &Rc<RefCell<HostBridgeState>>,
    cx: &mut AsyncApp,
) -> Result<Value, HostResponseError> {
    require_loaded_journal(state)?;
    let params: ResolveWorkspaceParams = decode_params(params)?;
    if let Some(expected) = params.expected_workspace_ref.as_deref() {
        validate_ref(expected, "expectedWorkspaceRef")?;
    }
    if let Some(binding) = state.borrow().workspace.clone()
        && binding.workspace.upgrade().is_some()
    {
        if params
            .expected_workspace_ref
            .as_ref()
            .is_some_and(|expected| expected != &binding.workspace_ref)
        {
            return Err(unavailable("The requested workspace is not active."));
        }
        return Ok(json!({ "workspaceRef": binding.workspace_ref }));
    }

    let candidates = cx.update(|cx| {
        let app_state = AppState::global(cx);
        app_state
            .workspace_store
            .read(cx)
            .workspaces_with_windows()
            .filter_map(|(window, workspace)| {
                let workspace = workspace.upgrade()?;
                let has_worktree = workspace
                    .read(cx)
                    .project()
                    .read(cx)
                    .worktrees(cx)
                    .next()
                    .is_some();
                has_worktree.then(|| (window, workspace))
            })
            .collect::<Vec<_>>()
    });
    let [(window, workspace)] = candidates.as_slice() else {
        return Err(unavailable(
            "Omega requires exactly one open workspace with a worktree.",
        ));
    };
    let workspace_ref = params
        .expected_workspace_ref
        .unwrap_or_else(|| SUPERVISED_WORKSPACE_REF.to_string());
    state.borrow_mut().workspace = Some(WorkspaceBinding {
        workspace_ref: workspace_ref.clone(),
        workspace: workspace.downgrade(),
        window: *window,
    });
    rebind_persisted_threads(&workspace_ref, state, cx)?;
    Ok(json!({ "workspaceRef": workspace_ref }))
}

fn create_thread(
    params: Value,
    state: &Rc<RefCell<HostBridgeState>>,
    cx: &mut AsyncApp,
) -> Result<Value, HostResponseError> {
    let params: CreateThreadParams = decode_params(params)?;
    validate_ref(&params.operation_ref, "operationRef")?;
    validate_ref(&params.workspace_ref, "workspaceRef")?;
    if params.title.trim().is_empty() {
        return Err(invalid("title must not be empty."));
    }
    let agent = agent_for_lane(&params.lane)?;
    if let Some(thread_ref) = state.borrow().threads.iter().find_map(|thread| {
        (thread.workspace_ref == params.workspace_ref
            && thread.lane == params.lane
            && thread.operation_ref == params.operation_ref)
            .then(|| thread.thread_id.to_key_string())
    }) {
        return Ok(json!({ "threadRef": thread_ref }));
    }
    let binding = require_workspace(&params.workspace_ref, state)?;
    let workspace = binding
        .workspace
        .upgrade()
        .ok_or_else(|| unavailable("The bound workspace was closed."))?;
    let (thread_id, conversation) = binding
        .window
        .update(cx, |_root, window, cx| {
            let panel = workspace
                .read(cx)
                .panel::<AgentPanel>(cx)
                .ok_or_else(|| unavailable("The workspace Agent panel is unavailable."))?;
            let thread_id = panel.update(cx, |panel, cx| {
                panel.create_thread_with_options(
                    CreateThreadOptions {
                        title: Some(params.title.clone().into()),
                        agent: Some(agent),
                        ..Default::default()
                    },
                    AgentThreadSource::AgentPanel,
                    window,
                    cx,
                )
            });
            let conversation = panel
                .read(cx)
                .conversation_view_for_id(&thread_id, cx)
                .cloned()
                .ok_or_else(|| internal("The created thread was not retained."))?;
            Ok((thread_id, conversation))
        })
        .map_err(|error| unavailable(format!("The workspace window is unavailable: {error}")))??;
    let thread_ref = thread_id.to_key_string();
    state.borrow_mut().threads.push(HostThread {
        workspace_ref: params.workspace_ref,
        lane: params.lane,
        operation_ref: params.operation_ref,
        thread_id,
        conversation: Some(conversation.downgrade()),
        turns: Vec::new(),
        revision: 1,
    });
    persist_state(state)?;
    Ok(json!({ "threadRef": thread_ref }))
}

fn lane_readiness(
    params: Value,
    state: &Rc<RefCell<HostBridgeState>>,
    cx: &AsyncApp,
) -> Result<Value, HostResponseError> {
    let params: LaneReadinessParams = decode_params(params)?;
    validate_lane(&params.lane)?;
    if let Some(thread_ref) = params.excluding_thread_ref.as_deref() {
        validate_ref(thread_ref, "excludingThreadRef")?;
    }
    if !is_supported_lane(&params.lane) {
        return Ok(json!({
            "known": false,
            "admitted": false,
            "fullAuto": false,
            "state": "unavailable",
        }));
    }
    let workspace_ready = state
        .borrow()
        .workspace
        .as_ref()
        .is_some_and(|binding| binding.workspace.upgrade().is_some());
    let dispatch_thread_exists =
        params
            .excluding_thread_ref
            .as_ref()
            .is_some_and(|excluding_thread_ref| {
                state.borrow().threads.iter().any(|thread| {
                    thread.lane == params.lane
                        && thread.thread_id.to_key_string() == *excluding_thread_ref
                })
            });
    let (authority_ready, authority_state) = match params.lane.as_str() {
        CODEX_LOCAL_LANE => {
            external_agent_lane_readiness(state, cx, CODEX_AGENT_ID, dispatch_thread_exists)?
        }
        CLAUDE_LOCAL_LANE => {
            external_agent_lane_readiness(state, cx, CLAUDE_AGENT_ID, dispatch_thread_exists)?
        }
        _ => return Err(unsupported("The provider lane is not supported.")),
    };
    let busy = state.borrow().threads.iter().any(|host_thread| {
        if host_thread.lane != params.lane {
            return false;
        }
        if params
            .excluding_thread_ref
            .as_ref()
            .is_some_and(|thread_ref| thread_ref == &host_thread.thread_id.to_key_string())
        {
            return false;
        }
        let Some(conversation) = host_thread
            .conversation
            .as_ref()
            .and_then(WeakEntity::upgrade)
        else {
            return false;
        };
        cx.update(|cx| {
            conversation
                .read(cx)
                .root_thread(cx)
                .is_some_and(|thread| thread.read(cx).status() == ThreadStatus::Generating)
        })
    });
    let admitted = workspace_ready && authority_ready;
    Ok(json!({
        "known": true,
        "admitted": admitted,
        "fullAuto": admitted,
        "state": if admitted && !busy { "available" } else if busy { "busy" } else { authority_state },
    }))
}

async fn dispatch_turn(
    params: Value,
    state: &Rc<RefCell<HostBridgeState>>,
    cx: &mut AsyncApp,
) -> Result<Value, HostResponseError> {
    let params: DispatchTurnParams = decode_params(params)?;
    validate_ref(&params.run_ref, "runRef")?;
    validate_ref(&params.workspace_ref, "workspaceRef")?;
    validate_ref(&params.thread_ref, "threadRef")?;
    validate_ref(&params.turn_ref, "turnRef")?;
    if params.message.is_empty() {
        return Err(invalid("message must not be empty."));
    }
    require_workspace(&params.workspace_ref, state)?;
    let lane = params
        .profile
        .as_ref()
        .and_then(|profile| profile.lane.clone())
        .unwrap_or_else(|| CODEX_LOCAL_LANE.to_string());
    if !is_supported_lane(&lane) {
        return Ok(json!({
            "accepted": false,
            "reason": "unsupported_lane",
            "failureCause": "lane_unavailable",
        }));
    }
    validate_lane(&lane)?;
    if let Some(profile) = params.profile.as_ref() {
        if let Some(account_ref) = profile.account_ref.as_deref() {
            validate_ref(account_ref, "profile.accountRef")?;
        }
        if let Some(model) = profile.model.as_deref() {
            validate_ref(model, "profile.model")?;
        }
        if let Some(reasoning_effort) = profile.reasoning_effort.as_deref() {
            validate_ref(reasoning_effort, "profile.reasoningEffort")?;
        }
    }
    if params
        .profile
        .as_ref()
        .is_some_and(|profile| profile.model.is_some() || profile.reasoning_effort.is_some())
    {
        return Ok(json!({
            "accepted": false,
            "reason": "profile_override_unavailable",
            "failureCause": "lane_unavailable",
        }));
    }
    let conversation = {
        let bridge = state.borrow();
        let host_thread = bridge
            .threads
            .iter()
            .find(|thread| thread.thread_id.to_key_string() == params.thread_ref)
            .ok_or_else(|| unavailable("The requested Agent thread is not bound."))?;
        if host_thread.workspace_ref != params.workspace_ref {
            return Err(unavailable("The thread belongs to a different workspace."));
        }
        if host_thread.lane != lane {
            return Err(invalid(
                "The dispatch lane does not match the Agent thread lane.",
            ));
        }
        host_thread
            .conversation
            .as_ref()
            .and_then(WeakEntity::upgrade)
            .ok_or_else(|| unavailable("The requested Agent thread was closed."))?
    };
    let thread = wait_for_root_thread(&conversation, cx).await?;
    let (start_entry_index, provider_session_ref) = cx.update(|cx| {
        let thread = thread.read(cx);
        if thread.status() == ThreadStatus::Generating {
            return Err(unavailable("The Agent thread already has a running turn."));
        }
        Ok((thread.entries().len(), thread.session_id().0.to_string()))
    })?;
    validate_ref(&provider_session_ref, "providerSessionRef")?;
    let turn_timeout = turn_timeout_for_lane(&lane);
    let now = Utc::now().to_rfc3339();
    {
        let mut bridge = state.borrow_mut();
        let host_thread = bridge
            .threads
            .iter_mut()
            .find(|thread| thread.thread_id.to_key_string() == params.thread_ref)
            .ok_or_else(|| unavailable("The requested Agent thread is not bound."))?;
        if host_thread
            .turns
            .iter()
            .any(|turn| turn.turn_ref == params.turn_ref)
        {
            return Ok(json!({ "accepted": false, "reason": "duplicate_turn_ref" }));
        }
        host_thread.turns.push(HostTurn {
            turn_ref: params.turn_ref.clone(),
            lane,
            account_ref: params
                .profile
                .as_ref()
                .and_then(|profile| profile.account_ref.clone()),
            model: params
                .profile
                .as_ref()
                .and_then(|profile| profile.model.clone()),
            provider_session_ref,
            start_entry_index,
            end_entry_index: None,
            phase: "streaming".to_string(),
            disposition: None,
            created_at: now.clone(),
            updated_at: now,
        });
        host_thread.revision += 1;
    }
    persist_state(state)?;
    let send = thread.update(cx, |thread, cx| {
        thread.send(vec![params.message.into()], cx)
    });
    let turn_ref = params.turn_ref;
    let state_for_completion = state.clone();
    let thread_for_completion = thread.clone();
    cx.spawn(async move |cx| {
        let outcome = if let Some(timeout) = turn_timeout {
            let timer = cx.background_executor().timer(timeout);
            match futures::future::select(send, timer).await {
                futures::future::Either::Left((result, _)) => {
                    if result.is_ok() {
                        HostTurnOutcome::Completed
                    } else {
                        HostTurnOutcome::Failed
                    }
                }
                futures::future::Either::Right(((), _)) => {
                    log::warn!("Omega Full Auto Claude turn exceeded its host deadline");
                    let cancel = cx.update(|cx| {
                        thread_for_completion.update(cx, |thread, cx| {
                            (thread.status() == ThreadStatus::Generating).then(|| thread.cancel(cx))
                        })
                    });
                    if let Some(cancel) = cancel {
                        let cancel_deadline =
                            cx.background_executor().timer(ACP_CANCEL_GRACE_PERIOD);
                        if matches!(
                            futures::future::select(cancel, cancel_deadline).await,
                            futures::future::Either::Right(_)
                        ) {
                            log::warn!(
                                "Omega Full Auto Claude cancellation exceeded its grace period"
                            );
                        }
                    }
                    HostTurnOutcome::TimedOut
                }
            }
        } else if send.await.is_ok() {
            HostTurnOutcome::Completed
        } else {
            HostTurnOutcome::Failed
        };
        let (end_entry_index, had_error) = cx.update(|cx| {
            let thread = thread_for_completion.read(cx);
            (thread.entries().len(), thread.had_error())
        });
        let outcome = if outcome == HostTurnOutcome::Completed && had_error {
            HostTurnOutcome::Failed
        } else {
            outcome
        };
        if let Err(error) =
            record_turn_completion(&state_for_completion, &turn_ref, end_entry_index, outcome)
        {
            log::error!("failed to persist Omega Full Auto host correlation: {error:#}");
        }
    })
    .detach();
    Ok(json!({ "accepted": true }))
}

fn refresh_evidence(
    params: Value,
    generation: u64,
    state: &Rc<RefCell<HostBridgeState>>,
    cx: &AsyncApp,
) -> Result<Value, HostResponseError> {
    let params: RefreshEvidenceParams = decode_params(params)?;
    validate_ref(&params.run_ref, "runRef")?;
    validate_ref(&params.thread_ref, "threadRef")?;
    let (conversation, mut turns, mut revision) = {
        let bridge = state.borrow();
        let Some(thread) = bridge
            .threads
            .iter()
            .find(|thread| thread.thread_id.to_key_string() == params.thread_ref)
        else {
            return Ok(json!({ "present": false, "revision": 0, "live": null, "turns": [] }));
        };
        (
            thread.conversation.as_ref().and_then(WeakEntity::upgrade),
            thread.turns.clone(),
            thread.revision,
        )
    };
    if turns.len() > MAX_EVIDENCE_TURNS {
        turns.drain(..turns.len() - MAX_EVIDENCE_TURNS);
    }
    let Some(conversation) = conversation else {
        return Ok(json!({ "present": false, "revision": revision, "live": null, "turns": [] }));
    };
    let Some(thread) = cx.update(|cx| conversation.read(cx).root_thread(cx)) else {
        return Ok(json!({ "present": true, "revision": revision, "live": null, "turns": [] }));
    };
    let (status, entry_count, assistant_texts) = cx.update(|cx| {
        let thread = thread.read(cx);
        let mut texts = turns
            .iter()
            .map(|turn| {
                let end_entry_index = turn.end_entry_index.unwrap_or(thread.entries().len());
                let text = thread
                    .entries()
                    .iter()
                    .skip(turn.start_entry_index)
                    .take(end_entry_index.saturating_sub(turn.start_entry_index))
                    .filter_map(|entry| match entry {
                        AgentThreadEntry::AssistantMessage(message) => {
                            Some(message.to_markdown(cx))
                        }
                        _ => None,
                    })
                    .collect::<Vec<_>>()
                    .join("\n");
                truncate_utf8(&text, MAX_ASSISTANT_TEXT_BYTES)
            })
            .collect::<Vec<_>>();
        let mut remaining_bytes = MAX_TOTAL_ASSISTANT_TEXT_BYTES;
        for text in texts.iter_mut().rev() {
            *text = truncate_utf8(text, text.len().min(remaining_bytes));
            remaining_bytes = remaining_bytes.saturating_sub(text.len());
        }
        (thread.status(), thread.entries().len(), texts)
    });
    if status == ThreadStatus::Idle
        && let Some(turn) = turns
            .iter_mut()
            .rev()
            .find(|turn| turn.disposition.is_none())
    {
        turn.phase = "completed".to_string();
        turn.disposition = Some("completed".to_string());
        turn.end_entry_index = Some(entry_count);
        turn.updated_at = Utc::now().to_rfc3339();
        {
            let mut bridge = state.borrow_mut();
            if let Some(host_thread) = bridge
                .threads
                .iter_mut()
                .find(|thread| thread.thread_id.to_key_string() == params.thread_ref)
                && let Some(stored_turn) = host_thread
                    .turns
                    .iter_mut()
                    .find(|stored_turn| stored_turn.turn_ref == turn.turn_ref)
                && stored_turn.disposition.is_none()
            {
                *stored_turn = turn.clone();
                host_thread.revision += 1;
                revision = host_thread.revision;
            }
        }
        persist_state(state)?;
    }
    let active_turn = turns.iter().rev().find(|turn| turn.disposition.is_none());
    let last_turn = turns.last();
    let live = if let Some(turn) = active_turn {
        json!({ "state": "turn_running", "turnRef": turn.turn_ref })
    } else if let Some(turn) = last_turn {
        match turn.disposition.as_deref() {
            Some("completed") => json!({ "state": "turn_completed", "turnRef": turn.turn_ref }),
            Some("failed") => json!({
                "state": "blocked",
                "turnRef": turn.turn_ref,
                "reason": "agent_turn_failed",
            }),
            Some("timed_out") => json!({
                "state": "blocked",
                "turnRef": turn.turn_ref,
                "reason": "provider_turn_timed_out",
            }),
            Some("owner_interrupted") => json!({
                "state": "blocked",
                "turnRef": turn.turn_ref,
                "reason": "owner_interrupted",
            }),
            _ => Value::Null,
        }
    } else {
        Value::Null
    };
    let turn_records = turns
        .iter()
        .zip(assistant_texts)
        .map(|(turn, assistant_text)| {
            let user_message_key = message_key(&turn.turn_ref, "user");
            let assistant_message_key = message_key(&turn.turn_ref, "assistant");
            let assistant_segments = if assistant_text.is_empty() {
                Vec::new()
            } else {
                vec![json!({
                    "key": assistant_message_key,
                    "text": assistant_text,
                })]
            };
            json!({
                "schema": "openagents.desktop.local_turn_record.v1",
                "threadRef": params.thread_ref,
                "turnRef": turn.turn_ref,
                "lane": turn.lane,
                "userMessageKey": user_message_key,
                "assistantMessageKey": assistant_message_key,
                "accountRef": turn.account_ref,
                "providerSessionRef": turn.provider_session_ref,
                "model": turn.model,
                "phase": turn.phase,
                "persistedCursor": assistant_text.len(),
                "assistantText": assistant_text,
                "assistantSegments": assistant_segments,
                "recoveryGeneration": generation,
                "disposition": turn.disposition,
                "createdAt": turn.created_at,
                "updatedAt": turn.updated_at,
            })
        })
        .collect::<Vec<_>>();
    Ok(json!({
        "present": true,
        "revision": revision,
        "live": live,
        "turns": turn_records,
    }))
}

async fn interrupt_turn(
    params: Value,
    state: &Rc<RefCell<HostBridgeState>>,
    cx: &mut AsyncApp,
) -> Result<Value, HostResponseError> {
    let params: InterruptTurnParams = decode_params(params)?;
    validate_ref(&params.thread_ref, "threadRef")?;
    if let Some(turn_ref) = params.turn_ref.as_deref() {
        validate_ref(turn_ref, "turnRef")?;
    }
    let (conversation, turn_ref) = {
        let bridge = state.borrow();
        let host_thread = bridge
            .threads
            .iter()
            .find(|thread| thread.thread_id.to_key_string() == params.thread_ref)
            .ok_or_else(|| unavailable("The requested Agent thread is not bound."))?;
        let turn_ref = params.turn_ref.clone().or_else(|| {
            host_thread
                .turns
                .iter()
                .rev()
                .find(|turn| turn.disposition.is_none())
                .map(|turn| turn.turn_ref.clone())
        });
        (
            host_thread
                .conversation
                .as_ref()
                .and_then(WeakEntity::upgrade)
                .ok_or_else(|| unavailable("The requested Agent thread was closed."))?,
            turn_ref,
        )
    };
    let Some(turn_ref) = turn_ref else {
        return Ok(json!({ "interrupted": false }));
    };
    let thread = wait_for_root_thread(&conversation, cx).await?;
    let cancel = thread.update(cx, |thread, cx| {
        if thread.status() != ThreadStatus::Generating {
            None
        } else {
            Some(thread.cancel(cx))
        }
    });
    let Some(cancel) = cancel else {
        return Ok(json!({ "interrupted": false }));
    };
    cancel.await;
    let end_entry_index = cx.update(|cx| thread.read(cx).entries().len());
    {
        let mut bridge = state.borrow_mut();
        if let Some(host_thread) = bridge
            .threads
            .iter_mut()
            .find(|thread| thread.thread_id.to_key_string() == params.thread_ref)
            && let Some(turn) = host_thread
                .turns
                .iter_mut()
                .find(|turn| turn.turn_ref == turn_ref)
        {
            turn.phase = "interrupted".to_string();
            turn.disposition = Some("owner_interrupted".to_string());
            turn.end_entry_index = Some(end_entry_index);
            turn.updated_at = Utc::now().to_rfc3339();
            host_thread.revision += 1;
        }
    }
    persist_state(state)?;
    Ok(json!({ "interrupted": true }))
}

fn append_system_note(params: Value) -> Result<Value, HostResponseError> {
    let params: AppendSystemNoteParams = decode_params(params)?;
    validate_ref(&params.thread_ref, "threadRef")?;
    validate_ref(&params.note_ref, "noteRef")?;
    if params.text.is_empty() {
        return Err(invalid("text must not be empty."));
    }
    Err(unavailable(
        "Agent threads do not expose an owner-visible system-note authority.",
    ))
}

async fn wait_for_root_thread(
    conversation: &Entity<ConversationView>,
    cx: &mut AsyncApp,
) -> Result<Entity<acp_thread::AcpThread>, HostResponseError> {
    for _ in 0..THREAD_CONNECT_ATTEMPTS {
        if let Some(thread) = cx.update(|cx| conversation.read(cx).root_thread(cx)) {
            return Ok(thread);
        }
        cx.background_executor()
            .timer(THREAD_CONNECT_INTERVAL)
            .await;
    }
    Err(unavailable(
        "The Agent thread did not connect before the host deadline.",
    ))
}

fn require_workspace(
    workspace_ref: &str,
    state: &Rc<RefCell<HostBridgeState>>,
) -> Result<WorkspaceBinding, HostResponseError> {
    let binding = state
        .borrow()
        .workspace
        .clone()
        .ok_or_else(|| unavailable("No Omega workspace is bound."))?;
    if binding.workspace_ref != workspace_ref {
        return Err(unavailable("The requested workspace is not bound."));
    }
    if binding.workspace.upgrade().is_none() {
        return Err(unavailable("The bound workspace was closed."));
    }
    Ok(binding)
}

fn require_loaded_journal(state: &Rc<RefCell<HostBridgeState>>) -> Result<(), HostResponseError> {
    if let Some(error) = state.borrow().load_error.as_deref() {
        Err(internal(format!(
            "Omega Full Auto host correlation could not be loaded: {error}"
        )))
    } else {
        Ok(())
    }
}

fn rebind_persisted_threads(
    workspace_ref: &str,
    state: &Rc<RefCell<HostBridgeState>>,
    cx: &mut AsyncApp,
) -> Result<(), HostResponseError> {
    let thread_ids = state
        .borrow()
        .threads
        .iter()
        .filter(|thread| {
            thread.workspace_ref == workspace_ref
                && thread
                    .conversation
                    .as_ref()
                    .and_then(WeakEntity::upgrade)
                    .is_none()
        })
        .map(|thread| (thread.thread_id, thread.lane.clone()))
        .collect::<Vec<_>>();
    if thread_ids.is_empty() {
        return Ok(());
    }

    let binding = require_workspace(workspace_ref, state)?;
    let workspace = binding
        .workspace
        .upgrade()
        .ok_or_else(|| unavailable("The bound workspace was closed."))?;
    let rebound = binding
        .window
        .update(cx, |_root, window, cx| {
            let panel = workspace
                .read(cx)
                .panel::<AgentPanel>(cx)
                .ok_or_else(|| unavailable("The workspace Agent panel is unavailable."))?;
            let mut rebound = Vec::with_capacity(thread_ids.len());
            for (thread_id, lane) in thread_ids {
                let agent = agent_for_lane(&lane)?;
                let conversation = panel.update(cx, |panel, cx| {
                    if panel.conversation_view_for_id(&thread_id, cx).is_none() {
                        panel.load_agent_thread(
                            agent,
                            thread_id,
                            None,
                            None,
                            false,
                            AgentThreadSource::AgentPanel,
                            window,
                            cx,
                        );
                    }
                    panel.conversation_view_for_id(&thread_id, cx).cloned()
                });
                let conversation = conversation.ok_or_else(|| {
                    unavailable("The persisted Full Auto Agent thread could not be restored.")
                })?;
                rebound.push((thread_id, conversation.downgrade()));
            }
            Ok(rebound)
        })
        .map_err(|error| unavailable(format!("The workspace window is unavailable: {error}")))??;

    let mut bridge = state.borrow_mut();
    for (thread_id, conversation) in rebound {
        if let Some(thread) = bridge
            .threads
            .iter_mut()
            .find(|thread| thread.thread_id == thread_id)
        {
            thread.conversation = Some(conversation);
        }
    }
    Ok(())
}

fn load_correlation_journal(path: &Path) -> anyhow::Result<Vec<HostThread>> {
    let bytes = match std::fs::read(path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(error.into()),
    };
    let journal: CorrelationJournal = serde_json::from_slice(&bytes)?;
    anyhow::ensure!(
        journal.schema == CORRELATION_SCHEMA,
        "unsupported correlation schema {}",
        journal.schema
    );
    journal
        .threads
        .into_iter()
        .map(|thread| {
            Ok(HostThread {
                workspace_ref: thread.workspace_ref,
                lane: thread.lane,
                operation_ref: thread.operation_ref,
                thread_id: ThreadId::from_key_string(&thread.thread_ref)?,
                conversation: None,
                turns: thread.turns,
                revision: thread.revision,
            })
        })
        .collect()
}

fn persist_state(state: &Rc<RefCell<HostBridgeState>>) -> Result<(), HostResponseError> {
    persist_correlation_journal(&state.borrow()).map_err(|error| {
        internal(format!(
            "Omega Full Auto host correlation could not be persisted: {error:#}"
        ))
    })
}

fn record_turn_completion(
    state: &Rc<RefCell<HostBridgeState>>,
    turn_ref: &str,
    end_entry_index: usize,
    outcome: HostTurnOutcome,
) -> anyhow::Result<()> {
    {
        let mut bridge = state.borrow_mut();
        if let Some(host_thread) = bridge
            .threads
            .iter_mut()
            .find(|thread| thread.turns.iter().any(|turn| turn.turn_ref == turn_ref))
        {
            let changed = if let Some(turn) = host_thread
                .turns
                .iter_mut()
                .find(|turn| turn.turn_ref == turn_ref)
            {
                if turn.disposition.is_none() {
                    match outcome {
                        HostTurnOutcome::Completed => {
                            turn.phase = "completed".to_string();
                            turn.disposition = Some("completed".to_string());
                        }
                        HostTurnOutcome::Failed => {
                            turn.phase = "failed".to_string();
                            turn.disposition = Some("failed".to_string());
                        }
                        HostTurnOutcome::TimedOut => {
                            turn.phase = "failed".to_string();
                            turn.disposition = Some("timed_out".to_string());
                        }
                    }
                    turn.end_entry_index = Some(end_entry_index);
                    turn.updated_at = Utc::now().to_rfc3339();
                    true
                } else {
                    false
                }
            } else {
                false
            };
            if changed {
                host_thread.revision += 1;
            }
        }
    }
    persist_correlation_journal(&state.borrow())
}

fn turn_timeout_for_lane(lane: &str) -> Option<Duration> {
    (lane == CLAUDE_LOCAL_LANE).then_some(CLAUDE_TURN_TIMEOUT)
}

fn persist_correlation_journal(state: &HostBridgeState) -> anyhow::Result<()> {
    // OMEGA-DELTA-0021. Every mutation of `state.threads` reaches disk through
    // here, so republishing here is what keeps the lane index and the durable
    // journal from drifting apart within a session.
    republish_engine_lane_runs(&state.threads);
    let journal = CorrelationJournal {
        schema: CORRELATION_SCHEMA.to_string(),
        threads: state
            .threads
            .iter()
            .map(|thread| PersistedHostThread {
                workspace_ref: thread.workspace_ref.clone(),
                lane: thread.lane.clone(),
                operation_ref: thread.operation_ref.clone(),
                thread_ref: thread.thread_id.to_key_string(),
                turns: thread.turns.clone(),
                revision: thread.revision,
            })
            .collect(),
    };
    let parent = state
        .correlation_path
        .parent()
        .ok_or_else(|| anyhow::anyhow!("correlation path has no parent"))?;
    std::fs::create_dir_all(parent)?;
    let temporary_path = state.correlation_path.with_extension("json.tmp");
    let bytes = serde_json::to_vec_pretty(&journal)?;
    let mut temporary_file = std::fs::OpenOptions::new()
        .create(true)
        .truncate(true)
        .write(true)
        .open(&temporary_path)?;
    temporary_file.write_all(&bytes)?;
    temporary_file.sync_all()?;
    std::fs::rename(&temporary_path, &state.correlation_path)?;
    std::fs::File::open(parent)?.sync_all()?;
    Ok(())
}

fn agent_for_lane(lane: &str) -> Result<Agent, HostResponseError> {
    match lane {
        CODEX_LOCAL_LANE => Ok(Agent::Custom {
            id: CODEX_AGENT_ID.into(),
        }),
        CLAUDE_LOCAL_LANE => Ok(Agent::Custom {
            id: CLAUDE_AGENT_ID.into(),
        }),
        _ => Err(unsupported(format!("The {lane} lane is not supported."))),
    }
}

fn is_supported_lane(lane: &str) -> bool {
    matches!(lane, CODEX_LOCAL_LANE | CLAUDE_LOCAL_LANE)
}

fn external_agent_lane_readiness(
    state: &Rc<RefCell<HostBridgeState>>,
    cx: &AsyncApp,
    agent_id: &'static str,
    dispatch_thread_exists: bool,
) -> Result<(bool, &'static str), HostResponseError> {
    let binding = state
        .borrow()
        .workspace
        .clone()
        .ok_or_else(|| unavailable("No Omega workspace is bound."))?;
    let workspace = binding
        .workspace
        .upgrade()
        .ok_or_else(|| unavailable("The bound workspace was closed."))?;
    let agent = Agent::Custom {
        id: agent_id.into(),
    };
    let (registered, connection_status) = cx.update(|cx| {
        let project = workspace.read(cx).project().clone();
        let registered = project
            .read(cx)
            .agent_server_store()
            .read(cx)
            .external_agents()
            .any(|registered_agent_id| registered_agent_id.as_ref() == agent_id);
        let connection_status = workspace
            .read(cx)
            .panel::<AgentPanel>(cx)
            .map(|panel| {
                panel
                    .read(cx)
                    .connection_store()
                    .read(cx)
                    .connection_status(&agent, cx)
            })
            .unwrap_or(AgentConnectionStatus::Disconnected);
        (registered, connection_status)
    });
    Ok(external_agent_authority_state(
        registered,
        connection_status,
        dispatch_thread_exists,
    ))
}

fn external_agent_authority_state(
    registered: bool,
    connection_status: AgentConnectionStatus,
    dispatch_thread_exists: bool,
) -> (bool, &'static str) {
    if !registered {
        return (false, "unavailable");
    }
    match connection_status {
        AgentConnectionStatus::Connected => (true, "available"),
        // Dispatch waits for this exact retained thread's root ACP session, so
        // treating its bootstrap as unavailable would strand the run at zero turns.
        AgentConnectionStatus::Connecting if dispatch_thread_exists => (true, "available"),
        AgentConnectionStatus::Connecting => (false, "connecting"),
        AgentConnectionStatus::Disconnected => (true, "available"),
    }
}

fn decode_params<T: for<'de> Deserialize<'de>>(params: Value) -> Result<T, HostResponseError> {
    serde_json::from_value(params)
        .map_err(|error| invalid(format!("Invalid host request parameters: {error}")))
}

fn validate_ref(value: &str, name: &str) -> Result<(), HostResponseError> {
    if value.is_empty() || value.len() > 180 {
        Err(invalid(format!("{name} must contain 1 to 180 bytes.")))
    } else {
        Ok(())
    }
}

fn validate_lane(value: &str) -> Result<(), HostResponseError> {
    if value.is_empty() || value.len() > 64 {
        Err(invalid("lane must contain 1 to 64 bytes."))
    } else {
        Ok(())
    }
}

fn truncate_utf8(value: &str, max_bytes: usize) -> String {
    if value.len() <= max_bytes {
        return value.to_string();
    }
    let mut boundary = max_bytes;
    while !value.is_char_boundary(boundary) {
        boundary -= 1;
    }
    value[..boundary].to_string()
}

fn message_key(turn_ref: &str, role: &str) -> String {
    let name = format!("{turn_ref}.{role}");
    format!(
        "omega.{role}.{}",
        uuid::Uuid::new_v5(&uuid::Uuid::NAMESPACE_OID, name.as_bytes())
    )
}

fn invalid(message: impl Into<String>) -> HostResponseError {
    HostResponseError {
        code: HostResponseErrorCode::InvalidRequest,
        message: message.into(),
    }
}

fn unsupported(message: impl Into<String>) -> HostResponseError {
    HostResponseError {
        code: HostResponseErrorCode::Unsupported,
        message: message.into(),
    }
}

fn unavailable(message: impl Into<String>) -> HostResponseError {
    HostResponseError::unavailable(message)
}

fn internal(message: impl Into<String>) -> HostResponseError {
    HostResponseError {
        code: HostResponseErrorCode::Internal,
        message: message.into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use omega_front_door::{ExecutorClass, ExecutorDisclosure};
    use tempfile::tempdir;

    /// `ENGINE_LANE_RUNS` is process-wide and every publication replaces it
    /// wholesale, so two tests publishing at once would clobber each other's
    /// assertions. Every test that publishes takes this first.
    static LANE_INDEX_GUARD: Mutex<()> = Mutex::new(());

    fn lane_index_guard() -> std::sync::MutexGuard<'static, ()> {
        LANE_INDEX_GUARD.lock().unwrap_or_else(|poisoned| {
            LANE_INDEX_GUARD.clear_poison();
            poisoned.into_inner()
        })
    }

    #[test]
    fn validates_bounded_refs_and_params() {
        assert!(validate_ref("turn.full-auto.1", "turnRef").is_ok());
        assert!(validate_ref("", "turnRef").is_err());
        assert!(validate_ref(&"x".repeat(181), "turnRef").is_err());
        assert!(decode_params::<ResolveWorkspaceParams>(json!({})).is_ok());
        assert!(decode_params::<ResolveWorkspaceParams>(json!({ "extra": true })).is_err());
        assert!(decode_params::<ResolveSyncSessionParams>(json!({})).is_ok());
        assert!(decode_params::<ResolveSyncSessionParams>(json!({ "token": "no" })).is_err());
    }

    #[test]
    fn sync_session_host_result_is_unavailable_or_runtime_only_verified_material() {
        assert_eq!(sync_session_result(None), json!({ "available": false }));
        assert_eq!(
            sync_session_result(Some(VerifiedOpenAgentsSession {
                base_url: "https://openagents.com".to_string(),
                access_token: "runtime-only-fixture".to_string(),
            })),
            json!({
                "available": true,
                "baseUrl": "https://openagents.com",
                "accessToken": "runtime-only-fixture",
            })
        );
    }

    #[test]
    fn assistant_evidence_truncation_preserves_utf8_boundaries() {
        let text = format!("{}é", "x".repeat(MAX_ASSISTANT_TEXT_BYTES));
        let truncated = truncate_utf8(&text, MAX_ASSISTANT_TEXT_BYTES + 1);
        assert!(truncated.is_char_boundary(truncated.len()));
        assert!(truncated.len() <= MAX_ASSISTANT_TEXT_BYTES + 1);
    }

    #[test]
    fn evidence_keys_remain_bounded_for_maximum_turn_refs() {
        let turn_ref = format!("{}é", "x".repeat(178));
        assert_eq!(turn_ref.len(), 180);

        let user_key = message_key(&turn_ref, "user");
        let assistant_key = message_key(&turn_ref, "assistant");

        assert!(user_key.len() <= 180);
        assert!(assistant_key.len() <= 180);
        assert_ne!(user_key, assistant_key);
        assert_eq!(user_key, message_key(&turn_ref, "user"));
    }

    #[test]
    fn correlation_journal_round_trips_without_conversation_entities() {
        let _lane_index = lane_index_guard();
        let directory = tempdir().expect("tempdir");
        let path = directory.path().join(CORRELATION_FILE);
        let thread_id = ThreadId::new();
        let state = HostBridgeState {
            workspace: None,
            threads: vec![HostThread {
                workspace_ref: "workspace.omega.supervised".to_string(),
                lane: CLAUDE_LOCAL_LANE.to_string(),
                operation_ref: "operation.full-auto.1".to_string(),
                thread_id,
                conversation: None,
                turns: vec![HostTurn {
                    turn_ref: "turn.full-auto.1".to_string(),
                    lane: CODEX_LOCAL_LANE.to_string(),
                    account_ref: Some("account.owner".to_string()),
                    model: None,
                    provider_session_ref: "session.native.1".to_string(),
                    start_entry_index: 4,
                    end_entry_index: Some(8),
                    phase: "completed".to_string(),
                    disposition: Some("completed".to_string()),
                    created_at: "2026-07-24T12:00:00Z".to_string(),
                    updated_at: "2026-07-24T12:01:00Z".to_string(),
                }],
                revision: 3,
            }],
            correlation_path: path.clone(),
            load_error: None,
            sarah_conversation: None,
        };

        persist_correlation_journal(&state).expect("persist correlation journal");
        let restored = load_correlation_journal(&path).expect("load correlation journal");

        assert_eq!(restored.len(), 1);
        assert_eq!(restored[0].thread_id, thread_id);
        assert_eq!(restored[0].lane, CLAUDE_LOCAL_LANE);
        assert_eq!(restored[0].operation_ref, "operation.full-auto.1");
        assert!(restored[0].conversation.is_none());
        assert_eq!(restored[0].revision, 3);
        assert_eq!(restored[0].turns[0].turn_ref, "turn.full-auto.1");
        assert_eq!(
            restored[0].turns[0].disposition.as_deref(),
            Some("completed")
        );
    }

    /// OMEGA-DELTA-0021, omega#77's exit: a thread still names its executor
    /// after a restart.
    ///
    /// The restart is real rather than mimed. The lane index is process state,
    /// so it is emptied here exactly as a process exit empties it, and the only
    /// thing that survives into the assertion is the file on disk. A disclosure
    /// that lived only in memory fails this test at the `engine_lane_run` call.
    #[test]
    fn a_restarted_process_still_discloses_the_lane_that_owns_a_thread() {
        let _lane_index = lane_index_guard();
        let directory = tempdir().expect("tempdir");
        let path = directory.path().join(CORRELATION_FILE);
        let thread_id = ThreadId::new();
        let state = HostBridgeState {
            workspace: None,
            threads: vec![HostThread {
                workspace_ref: SUPERVISED_WORKSPACE_REF.to_string(),
                lane: CODEX_LOCAL_LANE.to_string(),
                operation_ref: "operation.full-auto.77".to_string(),
                thread_id,
                conversation: None,
                turns: Vec::new(),
                revision: 1,
            }],
            correlation_path: path.clone(),
            load_error: None,
            sarah_conversation: None,
        };
        persist_correlation_journal(&state).expect("persist correlation journal");

        // The process ends here. Everything below is a cold start.
        republish_engine_lane_runs(&[]);
        assert_eq!(
            engine_lane_run(thread_id),
            None,
            "a cold process must know nothing until it reads the journal"
        );

        let restored = load_correlation_journal(&path).expect("load correlation journal");
        republish_engine_lane_runs(&restored);

        let run = engine_lane_run(thread_id).expect("the reloaded journal names this thread's run");
        assert_eq!(run, "operation.full-auto.77");

        let disclosure = crate::omega_executor_disclosure::delegated_to_run(
            ExecutorDisclosure {
                class: ExecutorClass::ExternalAcp,
                agent_id: CODEX_AGENT_ID.to_string(),
                provider: None,
                model: None,
                run_ref: None,
                route: Some(omega_front_door::RouteReason::PinHonored),
            },
            run,
        );
        assert_eq!(disclosure.class, ExecutorClass::EngineLane);
        assert!(disclosure.is_coherent());

        let line = disclosure.label();
        assert!(line.contains(CODEX_AGENT_ID), "{line:?}");
        assert!(line.contains("engine_lane"), "{line:?}");
        assert!(line.contains("operation.full-auto.77"), "{line:?}");
    }

    /// A thread the user started themselves is not a lane run, and must not be
    /// disclosed as one. Without this, the index's default answer would
    /// silently attribute every hand-driven thread to Full Auto.
    #[test]
    fn a_thread_that_is_not_a_lane_run_is_not_disclosed_as_one() {
        let _lane_index = lane_index_guard();
        let hand_started = ThreadId::new();
        republish_engine_lane_runs(&[HostThread {
            workspace_ref: SUPERVISED_WORKSPACE_REF.to_string(),
            lane: CLAUDE_LOCAL_LANE.to_string(),
            operation_ref: "operation.full-auto.1".to_string(),
            thread_id: ThreadId::new(),
            conversation: None,
            turns: Vec::new(),
            revision: 1,
        }]);
        assert_eq!(engine_lane_run(hand_started), None);
    }

    #[test]
    fn completed_turn_is_persisted_after_releasing_mutable_state_borrow() {
        let _lane_index = lane_index_guard();
        let directory = tempdir().expect("tempdir");
        let path = directory.path().join(CORRELATION_FILE);
        let thread_id = ThreadId::new();
        let state = Rc::new(RefCell::new(HostBridgeState {
            workspace: None,
            threads: vec![HostThread {
                workspace_ref: "workspace.omega.supervised".to_string(),
                lane: CODEX_LOCAL_LANE.to_string(),
                operation_ref: "operation.full-auto.1".to_string(),
                thread_id,
                conversation: None,
                turns: vec![HostTurn {
                    turn_ref: "turn.full-auto.1".to_string(),
                    lane: CODEX_LOCAL_LANE.to_string(),
                    account_ref: None,
                    model: None,
                    provider_session_ref: "session.native.1".to_string(),
                    start_entry_index: 2,
                    end_entry_index: None,
                    phase: "streaming".to_string(),
                    disposition: None,
                    created_at: "2026-07-24T12:00:00Z".to_string(),
                    updated_at: "2026-07-24T12:00:00Z".to_string(),
                }],
                revision: 1,
            }],
            correlation_path: path.clone(),
            load_error: None,
            sarah_conversation: None,
        }));

        record_turn_completion(&state, "turn.full-auto.1", 7, HostTurnOutcome::Completed)
            .expect("record completed turn");

        let restored = load_correlation_journal(&path).expect("load correlation journal");
        let turn = &restored[0].turns[0];
        assert_eq!(restored[0].revision, 2);
        assert_eq!(turn.phase, "completed");
        assert_eq!(turn.disposition.as_deref(), Some("completed"));
        assert_eq!(turn.end_entry_index, Some(7));
    }

    #[test]
    fn claude_turns_have_a_bounded_host_deadline() {
        assert_eq!(
            turn_timeout_for_lane(CLAUDE_LOCAL_LANE),
            Some(CLAUDE_TURN_TIMEOUT)
        );
        assert_eq!(turn_timeout_for_lane(CODEX_LOCAL_LANE), None);
    }

    #[test]
    fn timed_out_turn_is_persisted_as_a_terminal_failure() {
        let _lane_index = lane_index_guard();
        let directory = tempdir().expect("tempdir");
        let path = directory.path().join(CORRELATION_FILE);
        let thread_id = ThreadId::new();
        let state = Rc::new(RefCell::new(HostBridgeState {
            workspace: None,
            threads: vec![HostThread {
                workspace_ref: "workspace.omega.supervised".to_string(),
                lane: CLAUDE_LOCAL_LANE.to_string(),
                operation_ref: "operation.full-auto.timeout".to_string(),
                thread_id,
                conversation: None,
                turns: vec![HostTurn {
                    turn_ref: "turn.full-auto.timeout".to_string(),
                    lane: CLAUDE_LOCAL_LANE.to_string(),
                    account_ref: None,
                    model: None,
                    provider_session_ref: "session.native.timeout".to_string(),
                    start_entry_index: 2,
                    end_entry_index: None,
                    phase: "streaming".to_string(),
                    disposition: None,
                    created_at: "2026-07-24T12:00:00Z".to_string(),
                    updated_at: "2026-07-24T12:00:00Z".to_string(),
                }],
                revision: 1,
            }],
            correlation_path: path.clone(),
            load_error: None,
            sarah_conversation: None,
        }));

        record_turn_completion(
            &state,
            "turn.full-auto.timeout",
            5,
            HostTurnOutcome::TimedOut,
        )
        .expect("record timed-out turn");

        let restored = load_correlation_journal(&path).expect("load correlation journal");
        let turn = &restored[0].turns[0];
        assert_eq!(turn.phase, "failed");
        assert_eq!(turn.disposition.as_deref(), Some("timed_out"));
        assert_eq!(turn.end_entry_index, Some(5));
    }

    #[test]
    fn correlation_journal_rejects_unknown_schema_and_invalid_thread_refs() {
        let directory = tempdir().expect("tempdir");
        let path = directory.path().join(CORRELATION_FILE);
        std::fs::write(
            &path,
            serde_json::to_vec(&json!({
                "schema": "openagents.omega.full_auto_host_correlation.v0",
                "threads": [],
            }))
            .expect("serialize fixture"),
        )
        .expect("write fixture");
        assert!(load_correlation_journal(&path).is_err());

        std::fs::write(
            &path,
            serde_json::to_vec(&json!({
                "schema": CORRELATION_SCHEMA,
                "threads": [{
                    "workspaceRef": "workspace.omega.supervised",
                    "lane": "codex-local",
                    "operationRef": "operation.full-auto.1",
                    "threadRef": "not-a-uuid",
                    "turns": [],
                    "revision": 1,
                }],
            }))
            .expect("serialize fixture"),
        )
        .expect("write fixture");
        assert!(load_correlation_journal(&path).is_err());
    }

    #[test]
    fn local_lanes_resolve_to_their_real_agent_authorities() {
        assert_eq!(
            agent_for_lane(CODEX_LOCAL_LANE).expect("codex lane"),
            Agent::Custom {
                id: CODEX_AGENT_ID.into(),
            }
        );
        assert_eq!(
            agent_for_lane(CLAUDE_LOCAL_LANE).expect("claude lane"),
            Agent::Custom {
                id: CLAUDE_AGENT_ID.into(),
            }
        );
        assert!(agent_for_lane("claude-cloud").is_err());
    }

    #[test]
    fn external_agent_readiness_allows_dispatch_to_bootstrap_its_connection() {
        assert_eq!(
            external_agent_authority_state(false, AgentConnectionStatus::Disconnected, false,),
            (false, "unavailable")
        );
        assert_eq!(
            external_agent_authority_state(true, AgentConnectionStatus::Disconnected, false,),
            (true, "available")
        );
        assert_eq!(
            external_agent_authority_state(true, AgentConnectionStatus::Connecting, true),
            (true, "available")
        );
        assert_eq!(
            external_agent_authority_state(true, AgentConnectionStatus::Connecting, false),
            (false, "connecting")
        );
        assert_eq!(
            external_agent_authority_state(true, AgentConnectionStatus::Connected, true),
            (true, "available")
        );
        assert_eq!(
            external_agent_authority_state(true, AgentConnectionStatus::Disconnected, true),
            (true, "available")
        );
    }
}
