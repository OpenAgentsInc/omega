use acp_thread::{SUBAGENT_SESSION_INFO_META_KEY, SubagentSessionInfo};
use agent_client_protocol::schema::v1 as acp;
use anyhow::Result;
use gpui::{App, SharedString, Task};
use language_model::LanguageModelToolResultContent;
use schemars::JsonSchema;
use serde::{Deserialize, Deserializer, Serialize};
use std::borrow::Cow;
use std::rc::Rc;
use std::sync::Arc;

use omega_front_door::ExecutorDisclosure;

use crate::{
    AgentTool, ExecutorResolution, ThreadEnvironment, ToolCallEventStream, ToolInput,
    resolve_requested_executor,
};

/// What the delegating model is told this tool does.
///
/// OMEGA-DELTA-0209. A constant rather than a doc comment on the input type,
/// because a doc comment on that type reaches nobody. `AgentTool::description`
/// reads the input's JSON schema, `efdc784fa9` replaced the derived
/// `JsonSchema` for `SpawnAgentToolInput` with a hand-written one to pin the
/// delegate contract, and a hand-written `json_schema!` carries no doc comment
/// — so every word of this guidance had been absent from the tool the model
/// sees ever since. It was found while checking that a *new* line of guidance
/// arrived, which is the only way an empty string is ever noticed.
///
/// `SpawnAgentTool::description` is what serves this, with the structured
/// contracts appended from the catalog.
const DELEGATE_GUIDANCE: &str = r#"Spawn a sub-agent for a well-scoped task.

### Designing delegated subtasks
- An agent does not see your conversation history. Include all relevant context (file paths, requirements, constraints) in the message.
- Subtasks must be concrete, well-defined, and self-contained.
- Delegated subtasks must materially advance the main task.
- Do not duplicate work between your work and delegated subtasks.
- Do not use this tool for tasks you could accomplish directly with one or two tool calls. For example, don't ask the agent to read a single file and return the contents, you can do this yourself.
- When you delegate work, focus on coordinating and synthesizing results instead of duplicating the same work yourself.
- Avoid issuing multiple delegate calls for the same unresolved subproblem unless the new delegated task is genuinely different and necessary.
- Narrow the delegated ask to the concrete output you need next.
- For code-edit subtasks, decompose work so each delegated task has a disjoint write set.
- When sending a follow-up using an existing agent session_id, the agent already has the context from the previous turn. Send only a short, direct message. Do NOT repeat the original task or context.

### Parallel delegation patterns
- Run multiple independent information-seeking subtasks in parallel when you have distinct questions that can be answered independently.
- Split implementation into disjoint codebase slices and spawn multiple agents for them in parallel when the write scopes do not overlap.
- When a plan has multiple independent steps, prefer delegating those steps in parallel rather than serializing them unnecessarily.
- Reuse the returned session_id when you want to follow up on the same delegated subproblem instead of creating a duplicate session.

### Choosing what runs the agent
- Use `native` (or omit `executor` for stored-call compatibility) to run on Omega's own loop.
- Set `executor` to run the sub-agent as a *different* agent entirely — Codex, Claude Code, or Grok, each with its own login, its own tools and its own loop. Use this when the task suits another agent better, or when you want a second opinion from a genuinely independent one.
- Accepted values are installed agent ids such as `codex-acp`, `claude-acp`, and `grok`, plus `exo` and `engine:<lane>`. `auto` is not accepted.
- `executor` names an agent, not a language model. A model name such as `gpt-5` is not accepted and will fail rather than silently running on your own model.
- You may give different sub-agents different executors in the same turn, and they run concurrently.

### Agents that take a structured request
- Most executors have a model and read the `task` as prose. A few do not: they are deterministic tool servers, and their `task` must be exactly one JSON request with your values filled in — no prose around it, no explanation, nothing else.
- Which executors those are, and the exact request each one takes, is listed at the end of this description. That list is generated from the same catalog the delegation is offered from, so an executor named there does take that shape.
- Send anything else to one of them and the delegation is refused before the agent is started, naming the shape it wanted.

### Output
- You will receive only the agent's final message as output.
- The result also carries an `executor` record naming what actually produced it — its `class` (`native_loop` or `external_acp`) and its `agent_id` — so you can tell which agent gave you which answer. An external agent does not report its model, and `provider`/`model` are absent rather than guessed when it does not.
- Successful calls return a session_id that you can use for follow-up messages.
- Error results may also include a session_id if a session was already created."#;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct SpawnAgentToolInput {
    /// Short label displayed in the UI while the agent runs (e.g., "Researching alternatives")
    pub label: String,
    /// The task for the executor. For new sessions, include all context it needs. For follow-ups, rely on the existing session.
    #[serde(rename = "task", alias = "message")]
    pub task: String,
    /// Existing delegated session to continue. Omit to create a new session.
    #[serde(
        rename = "session",
        alias = "session_id",
        default,
        deserialize_with = "deserialize_session_id"
    )]
    pub session: Option<acp::SessionId>,
    /// Named target: `native`, an installed agent id, `exo`, or `engine:<lane>`.
    #[serde(default)]
    pub executor: Option<String>,
}

impl JsonSchema for SpawnAgentToolInput {
    fn schema_name() -> Cow<'static, str> {
        "DelegateToolInput".into()
    }

    fn json_schema(_: &mut schemars::SchemaGenerator) -> schemars::Schema {
        schemars::json_schema!({
            "type": "object",
            "properties": {
                "executor": {
                    "type": "string",
                    "description": "Named target: native, an installed ACP agent id, exo, or engine:<lane>."
                },
                "task": {
                    "type": "string",
                    "description": "The complete task for the delegated executor."
                },
                "label": {
                    "type": "string",
                    "description": "Short progress label shown while the delegate runs."
                },
                "session": {
                    "type": "string",
                    "description": "Existing delegated session for a follow-up turn."
                }
            },
            "required": ["executor", "task", "label"]
        })
    }
}

fn deserialize_session_id<'de, D>(deserializer: D) -> Result<Option<acp::SessionId>, D::Error>
where
    D: Deserializer<'de>,
{
    let Some(value) = Option::<serde_json::Value>::deserialize(deserializer)? else {
        return Ok(None);
    };

    if value
        .as_str()
        .is_some_and(|session_id| session_id.trim().is_empty())
    {
        return Ok(None);
    }

    serde_json::from_value(value)
        .map(Some)
        .map_err(serde::de::Error::custom)
}

/// What produced a subagent result, as parts rather than as a sentence.
///
/// A projection of [`ExecutorDisclosure`] onto the wire. It exists because the
/// record itself lives in `omega_front_door`, a leaf crate that deliberately
/// depends on nothing — including serde — and because the parent reading this
/// is a model rather than a person, so it gets the fields and not the line a
/// window renders.
///
/// **There is no `label` field, and that is the point.** `OMEGA-DELTA-0021`
/// fixed executor disclosure as a typed record that a label renders, never a
/// stored rendering, and the first cut of `OMEGA-DELTA-0061` disclosed
/// subagents with a hand-written sentence instead. A sentence cannot be
/// compared, cannot be re-rendered for a different reader, and cannot be
/// checked for coherence. Every field here comes from one
/// [`ExecutorDisclosure`], through [`From`], so there is exactly one source for
/// what ran.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct SubagentExecutorReport {
    /// `ExecutorClass::token()` — `native_loop` or `external_acp` for a
    /// subagent. A stable wire token, which is what a machine reader wants.
    pub class: String,
    /// The executor's own identifier: `codex-acp`, `claude-acp`, `grok`, or Omega's own
    /// for an inherited subagent.
    pub agent_id: String,
    /// `None` is **not disclosed**, and is different from an empty string.
    /// `AcpConnection` has no `model_selector`, so an external ACP agent does
    /// not tell Omega which model served the turn.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider: Option<String>,
    /// See [`provider`](Self::provider).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
}

fn executor_chain(report: &SubagentExecutorReport) -> Vec<SubagentExecutorReport> {
    if report.agent_id.starts_with("exo/") {
        let omega = ExecutorDisclosure {
            class: omega_front_door::ExecutorClass::NativeLoop,
            agent_id: "Omega Agent".to_owned(),
            provider: None,
            model: None,
            run_ref: None,
            route: None,
        };
        let exo = ExecutorDisclosure {
            class: omega_front_door::ExecutorClass::ExternalAcp,
            agent_id: "exo".to_owned(),
            provider: None,
            model: None,
            run_ref: None,
            route: None,
        };
        return vec![
            SubagentExecutorReport::from(&omega),
            SubagentExecutorReport::from(&exo),
            report.clone(),
        ];
    }
    vec![report.clone()]
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DelegateFailureClass {
    NoExecutor,
    AccountExhausted,
    AccountRateLimited,
    EngineUnavailable,
    /// OMEGA-DELTA-0209. The executor exists, starts, and cannot answer *this*
    /// task, because it takes a structured request and the task is not one.
    ///
    /// Its own class rather than an `ExecutionError`, because the two ask for
    /// different things: an execution error is retried or escalated, and this
    /// one is fixed by rewriting the task into the shape the refusal names.
    TaskNotInContract,
    #[default]
    ExecutionError,
}

/// OMEGA-DELTA-0209. The structured-contract catalog, as the model reads it.
///
/// Generated from `omega_agent_detect::CANDIDATES` rather than written into the
/// description by hand, so an agent added to the catalog is described by the
/// same edit that adds it. A model that is not told a target's contract writes
/// prose at it, which is what the owner saw.
fn structured_contract_section() -> String {
    let structured: Vec<String> = omega_agent_detect::CANDIDATES
        .iter()
        .filter_map(|candidate| match candidate.prompt {
            omega_agent_detect::AgentPromptContract::Structured(contract) => Some(format!(
                "- `{}` ({}) takes exactly: {}",
                candidate.id, candidate.name, contract.request
            )),
            omega_agent_detect::AgentPromptContract::Prose => None,
        })
        .collect();

    if structured.is_empty() {
        return String::new();
    }

    format!(
        "\n\n### Executors whose `task` is a JSON request\n{}\n",
        structured.join("\n")
    )
}

/// The task as the resolved executor will accept it, or the refusal to send.
///
/// OMEGA-DELTA-0209. A prose executor takes the task unchanged; a structured
/// one takes only what `shape_delegated_task` can shape. An executor Omega does
/// not have in its catalog — a settings-declared custom agent — takes the task
/// unchanged too, because Omega knows nothing about its contract and inventing
/// one would be worse than sending what the parent wrote.
fn task_for_executor(agent_id: &str, task: String) -> Result<String, String> {
    let Some(candidate) = omega_agent_detect::CANDIDATES
        .iter()
        .find(|candidate| candidate.id == agent_id)
    else {
        return Ok(task);
    };
    match candidate.prompt {
        omega_agent_detect::AgentPromptContract::Structured(contract) => {
            omega_agent_detect::shape_delegated_task(candidate.name, &contract, &task)
        }
        omega_agent_detect::AgentPromptContract::Prose => Ok(task),
    }
}

fn classify_execution_failure(error: &str) -> DelegateFailureClass {
    let error = error.to_ascii_lowercase();
    if error.contains("rate limit")
        || error.contains("rate_limit")
        || error.contains("too many requests")
    {
        DelegateFailureClass::AccountRateLimited
    } else if error.contains("at capacity")
        || error.contains("capacity exhausted")
        || error.contains("insufficient credits")
        || error.contains("credit balance")
        || error.contains("billing quota")
        || error.contains("billing limit")
        || error.contains("quota exceeded")
        || error.contains("quota exhausted")
    {
        DelegateFailureClass::AccountExhausted
    } else {
        DelegateFailureClass::ExecutionError
    }
}

fn disclosure_matches_requested_executor(requested: &str, disclosure: &ExecutorDisclosure) -> bool {
    match requested.trim() {
        "native" => disclosure.class == omega_front_door::ExecutorClass::NativeLoop,
        "exo" => disclosure.agent_id.starts_with("exo/"),
        requested => {
            disclosure.class == omega_front_door::ExecutorClass::ExternalAcp
                && disclosure.agent_id == requested
        }
    }
}

impl From<&ExecutorDisclosure> for SubagentExecutorReport {
    fn from(disclosure: &ExecutorDisclosure) -> Self {
        Self {
            class: disclosure.class.token().to_owned(),
            agent_id: disclosure.agent_id.clone(),
            provider: disclosure.provider.clone(),
            model: disclosure.model.clone(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
#[serde(rename_all = "snake_case")]
pub enum SpawnAgentToolOutput {
    Success {
        session_id: acp::SessionId,
        output: String,
        session_info: SubagentSessionInfo,
        /// What actually ran this subagent. Reported by the handle, so a mixed
        /// fan-out is attributable result by result.
        #[serde(default, deserialize_with = "deserialize_executor_report")]
        executor: Option<SubagentExecutorReport>,
    },
    Error {
        #[serde(skip_serializing_if = "Option::is_none")]
        #[serde(default)]
        session_id: Option<acp::SessionId>,
        error: String,
        #[serde(default)]
        class: DelegateFailureClass,
        session_info: Option<SubagentSessionInfo>,
        #[serde(default, deserialize_with = "deserialize_executor_report")]
        executor: Option<SubagentExecutorReport>,
    },
}

/// Read a stored `executor`, tolerating what an older build wrote there.
///
/// The first cut of `OMEGA-DELTA-0061` stored a rendered sentence in this
/// field. A sentence cannot be turned back into parts, so it reads as **not
/// disclosed** rather than being parsed into an invented class — the same rule
/// the record itself follows for a model nobody reported.
///
/// The tolerance is required, not merely kind. `SpawnAgentToolOutput` is an
/// untagged enum, so a field that fails to deserialize does not fail alone: the
/// whole variant is rejected, serde falls through to `Error`, that fails too,
/// and a replayed thread loses the tool call rather than losing one field of
/// it.
fn deserialize_executor_report<'de, D>(
    deserializer: D,
) -> Result<Option<SubagentExecutorReport>, D::Error>
where
    D: Deserializer<'de>,
{
    let Some(value) = Option::<serde_json::Value>::deserialize(deserializer)? else {
        return Ok(None);
    };
    Ok(serde_json::from_value(value).ok())
}

impl From<SpawnAgentToolOutput> for LanguageModelToolResultContent {
    fn from(output: SpawnAgentToolOutput) -> Self {
        match output {
            SpawnAgentToolOutput::Success {
                session_id,
                output,
                session_info: _, // Don't show this to the model
                executor,
            } => {
                let session_address = format!("session:{session_id}");
                let executor_chain = executor.as_ref().map(executor_chain);
                serde_json::to_string(
                    // The executor *is* shown to the model. Without it a mixed
                    // fan-out comes back as three anonymous answers.
                    &serde_json::json!({
                        "final_message": output,
                        "disclosure": executor,
                        "executor_chain": executor_chain,
                        "session_address": session_address
                    }),
                )
                .unwrap_or_else(|e| format!("Failed to serialize spawn_agent output: {e}"))
                .into()
            }
            SpawnAgentToolOutput::Error {
                session_id,
                error,
                class,
                session_info: _, // Don't show this to the model
                executor,
            } => serde_json::to_string(&serde_json::json!({
                "status": "unavailable",
                "class": class,
                "session_address": session_id.map(|id| format!("session:{id}")),
                "error": error,
                "disclosure": executor
            }))
            .unwrap_or_else(|e| format!("Failed to serialize spawn_agent output: {e}"))
            .into(),
        }
    }
}

/// Tool that spawns an agent thread to work on a task.
pub struct SpawnAgentTool {
    environment: Rc<dyn ThreadEnvironment>,
}

impl SpawnAgentTool {
    pub fn new(environment: Rc<dyn ThreadEnvironment>) -> Self {
        Self { environment }
    }
}

impl AgentTool for SpawnAgentTool {
    type Input = SpawnAgentToolInput;
    type Output = SpawnAgentToolOutput;

    const NAME: &'static str = "spawn_agent";

    /// OMEGA-DELTA-0209. The written guidance, plus the contract catalog.
    ///
    /// Two reasons this is not the default implementation. The default reads
    /// the input's JSON schema, and this tool's schema is hand-written and
    /// carries no description — so the default served an empty string, and had
    /// since `efdc784fa9`. And the structured contracts cannot be written into
    /// prose that a person maintains: a hand-kept list of which executors take
    /// JSON is a copy of the catalog, and a copy goes stale without saying so.
    fn description() -> SharedString {
        SharedString::new(format!(
            "{DELEGATE_GUIDANCE}{}",
            structured_contract_section()
        ))
    }

    fn kind() -> acp::ToolKind {
        acp::ToolKind::Other
    }

    fn initial_title(
        &self,
        input: Result<Self::Input, serde_json::Value>,
        _cx: &mut App,
    ) -> SharedString {
        match input {
            Ok(i) => i.label.into(),
            Err(value) => value
                .get("label")
                .and_then(|v| v.as_str())
                .map(|s| SharedString::from(s.to_owned()))
                .unwrap_or_else(|| "Spawning agent".into()),
        }
    }

    fn run(
        self: Arc<Self>,
        input: ToolInput<Self::Input>,
        event_stream: ToolCallEventStream,
        cx: &mut App,
    ) -> Task<Result<Self::Output, Self::Output>> {
        cx.spawn(async move |cx| {
            let input = input
                .recv()
                .await
                .map_err(|e| SpawnAgentToolOutput::Error {
                    session_id: None,
                    error: e.to_string(),
                    class: DelegateFailureClass::ExecutionError,
                    session_info: None,
                    executor: None,
                })?;

            // OMEGA-DELTA-0209. What will actually be sent, decided before
            // anything is started.
            //
            // A structured executor has no model, so a task it cannot parse is
            // not a task it can attempt: launching a process and opening a
            // session for it only moves the failure later and leaves the reader
            // with the agent's parse error instead of the shape it wanted.
            let mut task = input.task;

            let subagent = if let Some(session_id) = input.session {
                let subagent = cx
                    .update(|cx| self.environment.resume_subagent(session_id.clone(), cx))
                    .map_err(|err| SpawnAgentToolOutput::Error {
                        session_id: Some(session_id.clone()),
                        error: err.to_string(),
                        class: classify_execution_failure(&err.to_string()),
                        session_info: None,
                        executor: None,
                    })?;
                if let Some(requested) = input.executor.as_deref() {
                    let disclosure = cx.update(|cx| subagent.executor_disclosure(cx));
                    if !disclosure_matches_requested_executor(requested, &disclosure) {
                        return Err(SpawnAgentToolOutput::Error {
                            session_id: Some(session_id),
                            error: format!(
                                "Session belongs to `{}`, not the requested executor \
                                 `{requested}`. Omega did not substitute executors.",
                                disclosure.agent_id
                            ),
                            class: DelegateFailureClass::NoExecutor,
                            session_info: None,
                            executor: Some(SubagentExecutorReport::from(&disclosure)),
                        });
                    }
                }
                // Shaped from the live disclosure rather than from the request,
                // and whether or not the follow-up named an executor: a second
                // turn on an SCV session is another tool request, and a
                // follow-up that omits `executor` is still going to SCV.
                let disclosure = cx.update(|cx| subagent.executor_disclosure(cx));
                task = match task_for_executor(&disclosure.agent_id, task) {
                    Ok(task) => task,
                    Err(refusal) => {
                        return Err(SpawnAgentToolOutput::Error {
                            session_id: Some(session_id),
                            error: refusal,
                            class: DelegateFailureClass::TaskNotInContract,
                            session_info: None,
                            executor: Some(SubagentExecutorReport::from(&disclosure)),
                        });
                    }
                };
                subagent
            } else {
                // Decide what runs this before creating anything. A named
                // executor that is unavailable must fail by name rather than
                // reaching a fallback further down.
                let executor = match resolve_requested_executor(input.executor.as_deref()) {
                    ExecutorResolution::Resolved(executor) => executor,
                    ExecutorResolution::Refused(reason) => {
                        return Err(SpawnAgentToolOutput::Error {
                            session_id: None,
                            error: reason,
                            class: DelegateFailureClass::NoExecutor,
                            session_info: None,
                            executor: None,
                        });
                    }
                };
                // Before `create_subagent`, deliberately. A task the target
                // cannot parse is refused without spending a process launch, an
                // ACP handshake and a session on it.
                if let Some((agent_id, _)) = executor.external_acp() {
                    task = match task_for_executor(agent_id, task) {
                        Ok(task) => task,
                        Err(refusal) => {
                            return Err(SpawnAgentToolOutput::Error {
                                session_id: None,
                                error: refusal,
                                class: DelegateFailureClass::TaskNotInContract,
                                session_info: None,
                                executor: None,
                            });
                        }
                    };
                }
                if let Some(lane) = executor.engine_lane() {
                    return Err(SpawnAgentToolOutput::Error {
                        session_id: None,
                        error: format!(
                            "Engine lane `{lane}` is unavailable from this thread. \
                             Engine delegation is accepted only through the framed \
                             omega-effectd run authority; Omega did not run it locally."
                        ),
                        class: DelegateFailureClass::EngineUnavailable,
                        session_info: None,
                        executor: None,
                    });
                }
                cx.update(|cx| {
                    self.environment
                        .create_subagent(input.label, executor.clone(), cx)
                })
                .await
                .map_err(|err| SpawnAgentToolOutput::Error {
                    session_id: None,
                    error: err.to_string(),
                    class: classify_execution_failure(&err.to_string()),
                    session_info: None,
                    executor: None,
                })?
            };

            // Asked of the handle, not of the request. A report derived from
            // `input.executor` would still read "codex-acp" on a subagent that
            // ran as something else: it would state the intention, where this
            // states the fact.
            let executor_report = cx.update(|cx| {
                let disclosure = subagent.executor_disclosure(cx);
                debug_assert!(
                    disclosure.is_coherent(),
                    "a subagent disclosed an incoherent record: {disclosure:?}"
                );
                SubagentExecutorReport::from(&disclosure)
            });

            let mut session_info = cx.update(|cx| {
                let session_info = SubagentSessionInfo {
                    session_id: subagent.id(),
                    message_start_index: subagent.num_entries(cx),
                    message_end_index: None,
                };

                event_stream.subagent_spawned(subagent.id());
                event_stream.update_fields_with_meta(
                    acp::ToolCallUpdateFields::new(),
                    Some(acp::Meta::from_iter([(
                        SUBAGENT_SESSION_INFO_META_KEY.into(),
                        serde_json::json!(&session_info),
                    )])),
                );

                session_info
            });

            let send_result = subagent.send(task, cx).await;

            let status = if send_result.is_ok() {
                "completed"
            } else {
                "error"
            };
            telemetry::event!(
                "Subagent Completed",
                subagent_session = session_info.session_id.to_string(),
                status,
                executor = executor_report.agent_id.clone(),
                executor_class = executor_report.class.clone(),
            );

            session_info.message_end_index =
                cx.update(|cx| Some(subagent.num_entries(cx).saturating_sub(1)));

            let meta = Some(acp::Meta::from_iter([(
                SUBAGENT_SESSION_INFO_META_KEY.into(),
                serde_json::json!(&session_info),
            )]));

            let (output, result) = match send_result {
                Ok(output) => (
                    output.clone(),
                    Ok(SpawnAgentToolOutput::Success {
                        session_id: session_info.session_id.clone(),
                        session_info,
                        output,
                        executor: Some(executor_report),
                    }),
                ),
                Err(e) => {
                    let error = e.to_string();
                    let class = classify_execution_failure(&error);
                    (
                        error.clone(),
                        Err(SpawnAgentToolOutput::Error {
                            session_id: Some(session_info.session_id.clone()),
                            error,
                            class,
                            session_info: Some(session_info),
                            executor: Some(executor_report),
                        }),
                    )
                }
            };
            event_stream.update_fields_with_meta(
                acp::ToolCallUpdateFields::new().content(vec![output.into()]),
                meta,
            );
            result
        })
    }

    fn replay(
        &self,
        _input: Self::Input,
        output: Self::Output,
        event_stream: ToolCallEventStream,
        _cx: &mut App,
    ) -> Result<()> {
        let (content, session_info) = match output {
            SpawnAgentToolOutput::Success {
                output,
                session_info,
                ..
            } => (output.into(), Some(session_info)),
            SpawnAgentToolOutput::Error {
                error,
                session_info,
                ..
            } => (error.into(), session_info),
        };

        let meta = session_info.map(|session_info| {
            acp::Meta::from_iter([(
                SUBAGENT_SESSION_INFO_META_KEY.into(),
                serde_json::json!(&session_info),
            )])
        });
        event_stream.update_fields_with_meta(
            acp::ToolCallUpdateFields::new().content(vec![content]),
            meta,
        );

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn deserializes_blank_session_id_as_absent() {
        for session_id in [json!(null), json!(""), json!("   ")] {
            let input: SpawnAgentToolInput = serde_json::from_value(json!({
                "label": "label",
                "message": "message",
                "session_id": session_id,
            }))
            .unwrap();

            assert!(input.session.is_none());
        }

        let input: SpawnAgentToolInput = serde_json::from_value(json!({
            "label": "label",
            "message": "message",
        }))
        .unwrap();
        assert!(input.session.is_none());

        let input: SpawnAgentToolInput = serde_json::from_value(json!({
            "label": "label",
            "message": "message",
            "session_id": "existing-session",
        }))
        .unwrap();
        assert_eq!(input.session.unwrap().to_string(), "existing-session");
    }

    #[test]
    fn model_schema_uses_the_delegate_contract() {
        let schema = serde_json::to_value(schemars::schema_for!(SpawnAgentToolInput))
            .expect("delegate input schema must serialize");
        let properties = schema["properties"]
            .as_object()
            .expect("delegate input must be an object");
        let mut names = properties.keys().map(String::as_str).collect::<Vec<_>>();
        names.sort_unstable();
        assert_eq!(names, ["executor", "label", "session", "task"]);
        assert!(
            schema["required"]
                .as_array()
                .is_some_and(|required| required.iter().any(|name| name == "executor")),
            "{schema}"
        );
    }

    fn session() -> acp::SessionId {
        acp::SessionId::from("sub-1".to_string())
    }

    fn session_info() -> SubagentSessionInfo {
        SubagentSessionInfo {
            session_id: session(),
            message_start_index: 0,
            message_end_index: Some(1),
        }
    }

    fn model_reads(output: SpawnAgentToolOutput) -> serde_json::Value {
        let content: LanguageModelToolResultContent = output.into();
        let LanguageModelToolResultContent::Text(text) = content else {
            panic!("a spawn_agent result must be text the model can read");
        };
        serde_json::from_str(&text).expect("the result must be JSON")
    }

    /// Criterion 6, at the seam the parent actually reads.
    ///
    /// The record reaches the model, with the class and the agent id. A
    /// disclosure the parent cannot see is not a disclosure.
    #[test]
    fn an_external_result_carries_its_executor_record_to_the_model() {
        let report = SubagentExecutorReport::from(&crate::external_acp_disclosure("codex-acp"));
        let value = model_reads(SpawnAgentToolOutput::Success {
            session_id: session(),
            output: "done".into(),
            session_info: session_info(),
            executor: Some(report),
        });

        assert_eq!(value["disclosure"]["class"], "external_acp");
        assert_eq!(value["disclosure"]["agent_id"], "codex-acp");
        assert_eq!(value["session_address"], "session:sub-1");
        // Absent, not empty and not invented.
        assert!(value["disclosure"].get("provider").is_none());
        assert!(value["disclosure"].get("model").is_none());
    }

    /// The failure arm is attributed too.
    ///
    /// In a mixed fan-out this is the case that matters most: three subagents
    /// went out, one came back dead, and an unattributed error does not say
    /// which one.
    #[test]
    fn a_failed_result_names_the_executor_that_failed() {
        let report = SubagentExecutorReport::from(&crate::external_acp_disclosure("claude-acp"));
        let value = model_reads(SpawnAgentToolOutput::Error {
            session_id: Some(session()),
            error: "the Claude subagent failed".into(),
            class: DelegateFailureClass::ExecutionError,
            session_info: Some(session_info()),
            executor: Some(report),
        });

        assert_eq!(value["disclosure"]["class"], "external_acp");
        assert_eq!(value["disclosure"]["agent_id"], "claude-acp");
        assert!(value["error"].is_string());
    }

    /// OMEGA-DELTA-0209. The model is told every structured target's contract.
    ///
    /// Generated from the catalog, so this is not a check that somebody
    /// remembered to write SCV into a doc comment — it is a check that the
    /// generation happens at all and reaches the string the model reads.
    #[test]
    fn the_delegate_description_names_every_structured_contract() {
        let description = SpawnAgentTool::description();

        let mut structured = 0;
        for candidate in omega_agent_detect::CANDIDATES {
            let omega_agent_detect::AgentPromptContract::Structured(contract) = candidate.prompt
            else {
                continue;
            };
            structured += 1;
            assert!(
                description.contains(candidate.id),
                "a delegating model is never told that `{}` takes a structured \
                 request, so it writes prose at it: {description}",
                candidate.id
            );
            assert!(
                description.contains(contract.request),
                "`{}`'s description names no request, and a model handed a \
                 sentence about JSON writes a sentence: {description}",
                candidate.id
            );
        }
        assert!(structured > 0, "the catalog holds no structured target");

        // The written description survives the generation.
        assert!(
            description.contains("Spawn a sub-agent for a well-scoped task."),
            "{description}"
        );
    }

    /// OMEGA-DELTA-0209. The tool describes itself at all.
    ///
    /// `efdc784fa9` replaced this input's derived `JsonSchema` with a
    /// hand-written one, and a hand-written `json_schema!` carries no doc
    /// comment — so the default `AgentTool::description`, which reads the
    /// schema, served an empty string and every word of the delegate guidance
    /// was absent from the tool the model sees. Nothing failed, because an
    /// empty description is a valid description of nothing.
    #[test]
    fn the_delegate_tool_is_described_to_the_model() {
        let description = SpawnAgentTool::description();
        assert!(
            description.len() > 1000,
            "the delegate guidance is missing or truncated: {description:?}"
        );
        for written in [
            "### Designing delegated subtasks",
            "### Choosing what runs the agent",
            "### Output",
            "`auto` is not accepted",
        ] {
            assert!(description.contains(written), "{written} is not served");
        }
        // And the schema this tool pins is still the delegate contract, which is
        // why the description cannot come from it.
        let schema = serde_json::to_value(schemars::schema_for!(SpawnAgentToolInput)).unwrap();
        assert!(
            schema.get("description").is_none(),
            "if the schema ever carries the description, serve it from there \
             instead of from two places: {schema}"
        );
    }

    /// OMEGA-DELTA-0209. The owner's task, at the seam that refuses it.
    ///
    /// Prose is not shaped into a request, and the refusal is the shape rather
    /// than a parser position. The prose case is what the owner sent three
    /// times; the JSON case is what has to keep working.
    #[test]
    fn a_task_a_structured_target_cannot_parse_is_refused_by_naming_the_shape() {
        let refusal = task_for_executor(
            "scv",
            "Perform a read-only test delegation: report the project root path \
             and list one or two top-level entries. Do not modify files."
                .to_owned(),
        )
        .expect_err("SCV has no model and this is prose");

        assert!(
            refusal.contains(omega_agent_detect::SCV_REQUEST),
            "{refusal}"
        );
        assert_eq!(refusal.lines().count(), 1, "one line: {refusal}");
        assert!(
            refusal.contains("SCV"),
            "the refusal names the agent: {refusal}"
        );

        let request = r#"{"tool":"read","arguments":{"path":"/tmp/a"}}"#;
        assert_eq!(
            task_for_executor("scv", request.to_owned()),
            Ok(request.to_owned()),
            "a request in the advertised shape must pass through unchanged"
        );

        // A prose target is untouched, and so is an agent Omega has no
        // declaration for: inventing a contract would be worse than sending
        // what the parent wrote.
        for agent_id in ["codex-acp", "some-custom-agent"] {
            assert_eq!(
                task_for_executor(agent_id, "do the thing".to_owned()),
                Ok("do the thing".to_owned()),
                "{agent_id}"
            );
        }
    }

    /// The refusal is its own class, because it asks for its own fix.
    #[test]
    fn an_unshapeable_task_is_not_an_execution_error() {
        let value = model_reads(SpawnAgentToolOutput::Error {
            session_id: None,
            error: "SCV takes a JSON tool request as the task, not prose.".into(),
            class: DelegateFailureClass::TaskNotInContract,
            session_info: None,
            executor: None,
        });
        assert_eq!(value["class"], "task_not_in_contract", "{value}");
    }

    #[test]
    fn capacity_failures_are_not_flattened() {
        assert_eq!(
            classify_execution_failure("account is at capacity"),
            DelegateFailureClass::AccountExhausted
        );
        assert_eq!(
            classify_execution_failure("insufficient credits for this request"),
            DelegateFailureClass::AccountExhausted
        );
        assert_eq!(
            classify_execution_failure("credit balance exhausted"),
            DelegateFailureClass::AccountExhausted
        );
        assert_eq!(
            classify_execution_failure("billing quota exceeded"),
            DelegateFailureClass::AccountExhausted
        );
        assert_eq!(
            classify_execution_failure("rate_limit exceeded"),
            DelegateFailureClass::AccountRateLimited
        );
    }

    #[test]
    fn credential_failures_are_not_exhausted_accounts() {
        for error in [
            "missing credentials",
            "invalid credentials",
            "credential helper unavailable",
            "billing credentials missing",
            "credit credential invalid",
        ] {
            assert_eq!(
                classify_execution_failure(error),
                DelegateFailureClass::ExecutionError,
                "{error}"
            );
        }
    }

    #[test]
    fn a_follow_up_executor_must_match_the_live_disclosure() {
        let native = crate::native_loop_disclosure("Omega Agent", None, None);
        let codex = crate::external_acp_disclosure("codex-acp");
        let exo = omega_exo_lane::ExoLaneIdentity {
            executor: "claude-code".to_owned(),
            provider: None,
            model: None,
        }
        .disclosure(None);

        assert!(disclosure_matches_requested_executor("native", &native));
        assert!(disclosure_matches_requested_executor("codex-acp", &codex));
        assert!(disclosure_matches_requested_executor("exo", &exo));
        assert!(!disclosure_matches_requested_executor("native", &codex));
        assert!(!disclosure_matches_requested_executor("claude-acp", &codex));
    }

    #[test]
    fn an_exo_result_names_the_hosted_chain() {
        let report = SubagentExecutorReport::from(
            &omega_exo_lane::ExoLaneIdentity {
                executor: "claude-code".to_owned(),
                provider: Some("https://api.anthropic.com".to_owned()),
                model: Some("claude-opus-4".to_owned()),
            }
            .disclosure(None),
        );
        let value = model_reads(SpawnAgentToolOutput::Success {
            session_id: session(),
            output: "done".to_owned(),
            session_info: session_info(),
            executor: Some(report),
        });
        assert_eq!(value["executor_chain"][0]["agent_id"], "Omega Agent");
        assert_eq!(value["executor_chain"][1]["agent_id"], "exo");
        assert_eq!(value["executor_chain"][2]["agent_id"], "exo/claude-code");
        assert_eq!(value["executor_chain"][2]["model"], "claude-opus-4");
    }

    /// Two results from one turn are told apart by what the model receives.
    #[test]
    fn a_mixed_fan_out_reaches_the_model_as_different_executors() {
        let codex = model_reads(SpawnAgentToolOutput::Success {
            session_id: session(),
            output: "codex answer".into(),
            session_info: session_info(),
            executor: Some(SubagentExecutorReport::from(
                &crate::external_acp_disclosure("codex-acp"),
            )),
        });
        let inherited = model_reads(SpawnAgentToolOutput::Success {
            session_id: session(),
            output: "my own answer".into(),
            session_info: session_info(),
            executor: Some(SubagentExecutorReport::from(
                &crate::native_loop_disclosure(
                    "Omega Agent",
                    Some("anthropic".into()),
                    Some("claude-opus-4".into()),
                ),
            )),
        });

        assert_ne!(codex["disclosure"], inherited["disclosure"]);
        assert_eq!(inherited["disclosure"]["class"], "native_loop");
        assert_eq!(inherited["disclosure"]["model"], "claude-opus-4");
    }

    /// The record holds no rendered sentence.
    ///
    /// `OMEGA-DELTA-0021`'s law, applied to the wire: parts, never a line. A
    /// stored rendering cannot be handed to a signer and cannot be re-rendered
    /// for a different reader, which is what makes the owner's identity
    /// decision cheap to reverse.
    #[test]
    fn the_reported_record_is_parts_and_not_a_sentence() {
        let report = SubagentExecutorReport::from(&crate::external_acp_disclosure("codex-acp"));
        let value = serde_json::to_value(&report).unwrap();
        let object = value.as_object().expect("the report must be an object");

        let mut keys: Vec<_> = object.keys().map(String::as_str).collect();
        keys.sort_unstable();
        assert_eq!(
            keys,
            ["agent_id", "class"],
            "the report gained a field. Every field here is a part of \
             `ExecutorDisclosure`; a rendered line under any name is the shape \
             OMEGA-DELTA-0021 forbids."
        );
    }

    /// A tool result written by an older build still loads.
    ///
    /// The first cut of this delta stored a rendered sentence in `executor`.
    /// `SpawnAgentToolOutput` is untagged, so a field that fails to
    /// deserialize takes the whole tool call with it — the thread would replay
    /// missing the delegation, not merely missing its attribution.
    #[test]
    fn a_legacy_rendered_executor_reads_as_not_disclosed() {
        let stored = json!({
            "session_id": "sub-1",
            "output": "done",
            "session_info": {
                "session_id": "sub-1",
                "message_start_index": 0,
                "message_end_index": 1,
            },
            "executor": "Codex (codex-acp, external ACP agent)",
        });

        let output: SpawnAgentToolOutput =
            serde_json::from_value(stored).expect("an older tool result must still load");
        match output {
            SpawnAgentToolOutput::Success {
                output, executor, ..
            } => {
                assert_eq!(output, "done");
                assert_eq!(
                    executor, None,
                    "a sentence cannot be turned back into parts, so it must \
                     read as not disclosed rather than as an invented class"
                );
            }
            SpawnAgentToolOutput::Error { .. } => {
                panic!("a stored success must not load as an error")
            }
        }
    }

    /// And a well-formed record round-trips.
    #[test]
    fn a_stored_executor_record_round_trips() {
        let report = SubagentExecutorReport::from(&crate::external_acp_disclosure("codex-acp"));
        let stored = serde_json::to_value(SpawnAgentToolOutput::Success {
            session_id: session(),
            output: "done".into(),
            session_info: session_info(),
            executor: Some(report.clone()),
        })
        .unwrap();

        let loaded: SpawnAgentToolOutput = serde_json::from_value(stored).unwrap();
        match loaded {
            SpawnAgentToolOutput::Success { executor, .. } => assert_eq!(executor, Some(report)),
            SpawnAgentToolOutput::Error { .. } => panic!("a success must load as a success"),
        }
    }
}
