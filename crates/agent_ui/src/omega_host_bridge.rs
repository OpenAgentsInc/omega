use std::cell::RefCell;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::Duration;

use acp_thread::{AgentThreadEntry, ThreadStatus};
use chrono::Utc;
use gpui::{AnyWindowHandle, App, AsyncApp, Entity, WeakEntity};
use omega_effectd::{
    HostMethod, HostRequestFrame, HostResponseError, HostResponseErrorCode,
    OmegaEffectdHostHandler, OpenAgentsSession, VerifiedOpenAgentsSession,
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
const MAX_ASSISTANT_TEXT_BYTES: usize = 24 * 1024;
const MAX_EVIDENCE_TURNS: usize = 48;
const MAX_TOTAL_ASSISTANT_TEXT_BYTES: usize = 6 * 1024;
const CORRELATION_SCHEMA: &str = "openagents.omega.full_auto_host_correlation.v1";
const CORRELATION_FILE: &str = "full-auto-host-correlation.json";

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

struct HostBridgeState {
    workspace: Option<WorkspaceBinding>,
    threads: Vec<HostThread>,
    correlation_path: PathBuf,
    load_error: Option<String>,
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
    let state = Rc::new(RefCell::new(HostBridgeState {
        workspace: None,
        threads,
        correlation_path,
        load_error,
    }));
    Rc::new(move |request| {
        let async_cx = async_cx.clone();
        let state = state.clone();
        let openagents_session = openagents_session.clone();
        Box::pin(async move { handle_request(request, state, openagents_session, async_cx).await })
    })
}

async fn handle_request(
    request: HostRequestFrame,
    state: Rc<RefCell<HostBridgeState>>,
    openagents_session: OpenAgentsSession,
    mut cx: AsyncApp,
) -> Result<Value, HostResponseError> {
    match request.method {
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
        let result = send.await;
        let (end_entry_index, had_error) = cx.update(|cx| {
            let thread = thread_for_completion.read(cx);
            (thread.entries().len(), thread.had_error())
        });
        if let Err(error) = record_turn_completion(
            &state_for_completion,
            &turn_ref,
            end_entry_index,
            result.is_ok() && !had_error,
        ) {
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
    succeeded: bool,
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
                    if succeeded {
                        turn.phase = "completed".to_string();
                        turn.disposition = Some("completed".to_string());
                    } else {
                        turn.phase = "failed".to_string();
                        turn.disposition = Some("failed".to_string());
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

fn persist_correlation_journal(state: &HostBridgeState) -> anyhow::Result<()> {
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
    use tempfile::tempdir;

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

    #[test]
    fn completed_turn_is_persisted_after_releasing_mutable_state_borrow() {
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
        }));

        record_turn_completion(&state, "turn.full-auto.1", 7, true).expect("record completed turn");

        let restored = load_correlation_journal(&path).expect("load correlation journal");
        let turn = &restored[0].turns[0];
        assert_eq!(restored[0].revision, 2);
        assert_eq!(turn.phase, "completed");
        assert_eq!(turn.disposition.as_deref(), Some("completed"));
        assert_eq!(turn.end_entry_index, Some(7));
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
