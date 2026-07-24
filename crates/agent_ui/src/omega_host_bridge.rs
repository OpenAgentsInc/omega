use std::cell::RefCell;
use std::rc::Rc;
use std::time::Duration;

use acp_thread::{AgentThreadEntry, ThreadStatus};
use chrono::Utc;
use gpui::{AnyWindowHandle, App, AsyncApp, Entity, WeakEntity};
use language_model::LanguageModelRegistry;
use omega_effectd::{
    HostMethod, HostRequestFrame, HostResponseError, HostResponseErrorCode, OmegaEffectdHostHandler,
};
use serde::Deserialize;
use serde_json::{Value, json};
use workspace::{AppState, Workspace};

use crate::agent_panel::CreateThreadOptions;
use crate::{Agent, AgentPanel, AgentThreadSource, ConversationView, ThreadId};

const SUPERVISED_WORKSPACE_REF: &str = "workspace.omega.supervised";
const CODEX_LOCAL_LANE: &str = "codex-local";
const THREAD_CONNECT_ATTEMPTS: usize = 100;
const THREAD_CONNECT_INTERVAL: Duration = Duration::from_millis(100);
const MAX_ASSISTANT_TEXT_BYTES: usize = 24 * 1024;
const MAX_EVIDENCE_TURNS: usize = 48;
const MAX_TOTAL_ASSISTANT_TEXT_BYTES: usize = 6 * 1024;

#[derive(Clone)]
struct WorkspaceBinding {
    workspace_ref: String,
    workspace: WeakEntity<Workspace>,
    window: AnyWindowHandle,
}

#[derive(Clone)]
struct HostThread {
    workspace_ref: String,
    thread_id: ThreadId,
    conversation: WeakEntity<ConversationView>,
    turns: Vec<HostTurn>,
    revision: u64,
}

#[derive(Clone)]
struct HostTurn {
    turn_ref: String,
    lane: String,
    account_ref: Option<String>,
    model: Option<String>,
    provider_session_ref: String,
    start_entry_index: usize,
    end_entry_index: Option<usize>,
    phase: &'static str,
    disposition: Option<&'static str>,
    created_at: String,
    updated_at: String,
}

#[derive(Default)]
struct HostBridgeState {
    workspace: Option<WorkspaceBinding>,
    threads: Vec<HostThread>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ResolveWorkspaceParams {
    expected_workspace_ref: Option<String>,
}

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
    let state = Rc::new(RefCell::new(HostBridgeState::default()));
    Rc::new(move |request| {
        let async_cx = async_cx.clone();
        let state = state.clone();
        Box::pin(async move { handle_request(request, state, async_cx).await })
    })
}

async fn handle_request(
    request: HostRequestFrame,
    state: Rc<RefCell<HostBridgeState>>,
    mut cx: AsyncApp,
) -> Result<Value, HostResponseError> {
    match request.method {
        HostMethod::ResolveWorkspace => resolve_workspace(request.params, &state, &cx),
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

fn resolve_workspace(
    params: Value,
    state: &Rc<RefCell<HostBridgeState>>,
    cx: &AsyncApp,
) -> Result<Value, HostResponseError> {
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
    if params.lane != CODEX_LOCAL_LANE {
        return Err(unsupported(
            "Only the codex-local lane is currently supported.",
        ));
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
                        agent: Some(Agent::NativeAgent),
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
        thread_id,
        conversation: conversation.downgrade(),
        turns: Vec::new(),
        revision: 1,
    });
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
    if params.lane != CODEX_LOCAL_LANE {
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
    let model_ready = cx.update(|cx| {
        LanguageModelRegistry::read_global(cx)
            .default_model()
            .is_some()
    });
    let busy = state.borrow().threads.iter().any(|host_thread| {
        if params
            .excluding_thread_ref
            .as_ref()
            .is_some_and(|thread_ref| thread_ref == &host_thread.thread_id.to_key_string())
        {
            return false;
        }
        let Some(conversation) = host_thread.conversation.upgrade() else {
            return false;
        };
        cx.update(|cx| {
            conversation
                .read(cx)
                .root_thread(cx)
                .is_some_and(|thread| thread.read(cx).status() == ThreadStatus::Generating)
        })
    });
    let admitted = workspace_ready && model_ready;
    Ok(json!({
        "known": true,
        "admitted": admitted,
        "fullAuto": admitted,
        "state": if admitted && !busy { "available" } else if busy { "busy" } else { "unavailable" },
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
    if lane != CODEX_LOCAL_LANE {
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
        host_thread
            .conversation
            .upgrade()
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
            phase: "streaming",
            disposition: None,
            created_at: now.clone(),
            updated_at: now,
        });
        host_thread.revision += 1;
    }
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
        let now = Utc::now().to_rfc3339();
        let mut bridge = state_for_completion.borrow_mut();
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
                    if result.is_ok() && !had_error {
                        turn.phase = "completed";
                        turn.disposition = Some("completed");
                    } else {
                        turn.phase = "failed";
                        turn.disposition = Some("failed");
                    }
                    turn.end_entry_index = Some(end_entry_index);
                    turn.updated_at = now;
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
            thread.conversation.upgrade(),
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
        turn.phase = "completed";
        turn.disposition = Some("completed");
        turn.end_entry_index = Some(entry_count);
        turn.updated_at = Utc::now().to_rfc3339();
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
    let active_turn = turns.iter().rev().find(|turn| turn.disposition.is_none());
    let last_turn = turns.last();
    let live = if let Some(turn) = active_turn {
        json!({ "state": "turn_running", "turnRef": turn.turn_ref })
    } else if let Some(turn) = last_turn {
        match turn.disposition {
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
                .upgrade()
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
        turn.phase = "interrupted";
        turn.disposition = Some("owner_interrupted");
        turn.end_entry_index = Some(end_entry_index);
        turn.updated_at = Utc::now().to_rfc3339();
        host_thread.revision += 1;
    }
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

    #[test]
    fn validates_bounded_refs_and_params() {
        assert!(validate_ref("turn.full-auto.1", "turnRef").is_ok());
        assert!(validate_ref("", "turnRef").is_err());
        assert!(validate_ref(&"x".repeat(181), "turnRef").is_err());
        assert!(decode_params::<ResolveWorkspaceParams>(json!({})).is_ok());
        assert!(decode_params::<ResolveWorkspaceParams>(json!({ "extra": true })).is_err());
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
}
