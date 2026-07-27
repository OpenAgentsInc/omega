use acp_thread::{SUBAGENT_SESSION_INFO_META_KEY, SubagentSessionInfo};
use agent_client_protocol::schema::v1 as acp;
use anyhow::Result;
use gpui::{App, SharedString, Task};
use language_model::LanguageModelToolResultContent;
use schemars::JsonSchema;
use serde::{Deserialize, Deserializer, Serialize};
use std::rc::Rc;
use std::sync::Arc;

use omega_front_door::ExecutorDisclosure;

use crate::{
    AgentTool, ExecutorResolution, ThreadEnvironment, ToolCallEventStream, ToolInput,
    resolve_requested_executor,
};

/// Spawn a sub-agent for a well-scoped task.
///
/// ### Designing delegated subtasks
/// - An agent does not see your conversation history. Include all relevant context (file paths, requirements, constraints) in the message.
/// - Subtasks must be concrete, well-defined, and self-contained.
/// - Delegated subtasks must materially advance the main task.
/// - Do not duplicate work between your work and delegated subtasks.
/// - Do not use this tool for tasks you could accomplish directly with one or two tool calls. For example, don't ask the agent to read a single file and return the contents, you can do this yourself.
/// - When you delegate work, focus on coordinating and synthesizing results instead of duplicating the same work yourself.
/// - Avoid issuing multiple delegate calls for the same unresolved subproblem unless the new delegated task is genuinely different and necessary.
/// - Narrow the delegated ask to the concrete output you need next.
/// - For code-edit subtasks, decompose work so each delegated task has a disjoint write set.
/// - When sending a follow-up using an existing agent session_id, the agent already has the context from the previous turn. Send only a short, direct message. Do NOT repeat the original task or context.
///
/// ### Parallel delegation patterns
/// - Run multiple independent information-seeking subtasks in parallel when you have distinct questions that can be answered independently.
/// - Split implementation into disjoint codebase slices and spawn multiple agents for them in parallel when the write scopes do not overlap.
/// - When a plan has multiple independent steps, prefer delegating those steps in parallel rather than serializing them unnecessarily.
/// - Reuse the returned session_id when you want to follow up on the same delegated subproblem instead of creating a duplicate session.
///
/// ### Choosing what runs the agent
/// - By default a sub-agent runs on the same model you are running on. Omit `executor` for that; it is the right choice for most delegation.
/// - Set `executor` to run the sub-agent as a *different* agent entirely — Codex or Claude Code, each with its own login, its own tools and its own loop. Use this when the task suits another agent better, or when you want a second opinion from a genuinely independent one.
/// - Accepted values are agent ids: `codex-acp` (Codex) and `claude-acp` (Claude Code). Only agents actually installed on this machine can be used; asking for one that is not installed fails and tells you which agents are available, so you can retry with one of those.
/// - `executor` names an agent, not a language model. A model name such as `gpt-5` is not accepted and will fail rather than silently running on your own model.
/// - You may give different sub-agents different executors in the same turn, and they run concurrently.
///
/// ### Output
/// - You will receive only the agent's final message as output.
/// - The result also carries an `executor` record naming what actually produced it — its `class` (`native_loop` or `external_acp`) and its `agent_id` — so you can tell which agent gave you which answer. An external agent does not report its model, and `provider`/`model` are absent rather than guessed when it does not.
/// - Successful calls return a session_id that you can use for follow-up messages.
/// - Error results may also include a session_id if a session was already created.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub struct SpawnAgentToolInput {
    /// Short label displayed in the UI while the agent runs (e.g., "Researching alternatives")
    pub label: String,
    /// The prompt for the agent. For new sessions, include full context needed for the task. For follow-ups (with session_id), you can rely on the agent already having the previous message.
    pub message: String,
    /// Session ID of an existing agent session to continue instead of creating a new one. Omit to create a new agent.
    #[serde(default, deserialize_with = "deserialize_session_id")]
    pub session_id: Option<acp::SessionId>,
    /// Which agent should run this sub-agent: `codex-acp` or `claude-acp`. Omit to run it on your own model, which is the default. This names an agent, not a language model.
    #[serde(default)]
    pub executor: Option<String>,
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
    /// The executor's own identifier: `codex-acp`, `claude-acp`, or Omega's own
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
            } => serde_json::to_string(
                // The executor *is* shown to the model. Without it a mixed
                // fan-out comes back as three anonymous answers.
                &serde_json::json!({ "session_id": session_id, "output": output, "executor": executor }),
            )
            .unwrap_or_else(|e| format!("Failed to serialize spawn_agent output: {e}"))
            .into(),
            SpawnAgentToolOutput::Error {
                session_id,
                error,
                session_info: _, // Don't show this to the model
                executor,
            } => serde_json::to_string(
                &serde_json::json!({ "session_id": session_id, "error": error, "executor": executor }),
            )
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
                    session_info: None,
                    executor: None,
                })?;

            // Decide what runs this before creating anything. A named agent
            // that is not installed must fail here, naming itself, rather than
            // reaching a fallback further down.
            let executor = match resolve_requested_executor(input.executor.as_deref()) {
                ExecutorResolution::Resolved(executor) => executor,
                ExecutorResolution::Refused(reason) => {
                    return Err(SpawnAgentToolOutput::Error {
                        session_id: None,
                        error: reason,
                        session_info: None,
                        executor: None,
                    });
                }
            };

            // Resuming runs on whatever already created that session. Honouring
            // `executor` here is impossible, so accepting it silently would drop
            // the request — the same silent-fallback defect, arriving by a
            // different door.
            if input.session_id.is_some() && executor.is_external() {
                return Err(SpawnAgentToolOutput::Error {
                    session_id: input.session_id.clone(),
                    error: "Cannot set `executor` when continuing an existing \
                            session: the session already belongs to the agent \
                            that created it. Omit `executor` to follow up on \
                            this session, or omit `session_id` to start a new \
                            subagent on the executor you named."
                        .to_owned(),
                    session_info: None,
                    executor: None,
                });
            }

            let subagent = if let Some(session_id) = input.session_id {
                cx.update(|cx| self.environment.resume_subagent(session_id, cx))
                    .map_err(|err| SpawnAgentToolOutput::Error {
                        session_id: None,
                        error: err.to_string(),
                        session_info: None,
                        executor: None,
                    })?
            } else {
                cx.update(|cx| {
                    self.environment
                        .create_subagent(input.label, executor.clone(), cx)
                })
                .await
                .map_err(|err| SpawnAgentToolOutput::Error {
                    session_id: None,
                    error: err.to_string(),
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

            let send_result = subagent.send(input.message, cx).await;

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
                    (
                        error.clone(),
                        Err(SpawnAgentToolOutput::Error {
                            session_id: Some(session_info.session_id.clone()),
                            error,
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

            assert!(input.session_id.is_none());
        }

        let input: SpawnAgentToolInput = serde_json::from_value(json!({
            "label": "label",
            "message": "message",
        }))
        .unwrap();
        assert!(input.session_id.is_none());

        let input: SpawnAgentToolInput = serde_json::from_value(json!({
            "label": "label",
            "message": "message",
            "session_id": "existing-session",
        }))
        .unwrap();
        assert_eq!(input.session_id.unwrap().to_string(), "existing-session");
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

        assert_eq!(value["executor"]["class"], "external_acp");
        assert_eq!(value["executor"]["agent_id"], "codex-acp");
        // Absent, not empty and not invented.
        assert!(value["executor"].get("provider").is_none());
        assert!(value["executor"].get("model").is_none());
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
            session_info: Some(session_info()),
            executor: Some(report),
        });

        assert_eq!(value["executor"]["class"], "external_acp");
        assert_eq!(value["executor"]["agent_id"], "claude-acp");
        assert!(value["error"].is_string());
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

        assert_ne!(codex["executor"], inherited["executor"]);
        assert_eq!(inherited["executor"]["class"], "native_loop");
        assert_eq!(inherited["executor"]["model"], "claude-opus-4");
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
