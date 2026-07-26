//! Rendering a durable Exo conversation. `OMEGA-DELTA-0091`, omega#104.
//!
//! ACP carries the live turn. This carries what the turn *was*: every message,
//! every tool call, every tool result, every artifact, every sandbox and
//! snapshot record, after the turn ended and for as long as Exo keeps the log.
//!
//! # The artifact read is load-bearing
//!
//! Exo's event log names artifacts and never contains them. `artifact_written`
//! carries an id, a path, and a version; the bytes live in a separate versioned
//! record reachable only through `agent_read_artifact` or
//! `conversation_read_artifact`. Exo's own scheduler writes a run's whole stdout
//! into an artifact and leaves a preview in the transcript.
//!
//! So a history built from events alone has the *names* of its tool results and
//! none of the bodies, and this type says so in the row rather than rendering an
//! empty one: [`ExoBody::NotRead`]. Remove the artifact read and every
//! artifact-backed result degrades to that variant — which is the falsifier for
//! this issue, run as a test.
//!
//! # This is a projection, not a surface
//!
//! No GPUI here, for the same reason `omega_exo_lane` has none: a law that
//! needs a window to check is a law nobody checks. [`ExoHistory`] produces rows
//! and a plain-text rendering; the workspace decides how they look.

use std::collections::HashMap;

use crate::record::{ExoArtifact, ExoArtifactRef, ExoEvent, ExoEventBody, HarnessReportedUsage};

/// Artifacts that were read, keyed by id.
///
/// Built by the caller from whatever artifact reads it chose to spend. An empty
/// set is legitimate and produces a history that says what it is missing.
#[derive(Clone, Debug, Default)]
pub struct ExoArtifactSet {
    by_id: HashMap<String, ExoArtifact>,
}

impl ExoArtifactSet {
    /// An empty set: names resolve to nothing.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Add an artifact that was read.
    pub fn insert(&mut self, artifact: ExoArtifact) {
        self.by_id
            .insert(artifact.version.artifact_id.clone(), artifact);
    }

    /// Look one up.
    #[must_use]
    pub fn get(&self, artifact_id: &str) -> Option<&ExoArtifact> {
        self.by_id.get(artifact_id)
    }

    /// How many artifacts were read.
    #[must_use]
    pub fn len(&self) -> usize {
        self.by_id.len()
    }

    /// Whether nothing was read.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.by_id.is_empty()
    }
}

impl FromIterator<ExoArtifact> for ExoArtifactSet {
    fn from_iter<T: IntoIterator<Item = ExoArtifact>>(artifacts: T) -> Self {
        let mut set = Self::new();
        for artifact in artifacts {
            set.insert(artifact);
        }
        set
    }
}

/// Where a rendered body came from.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ExoBody {
    /// The event carried the whole thing.
    Inline(String),
    /// The event named an artifact and the artifact was read.
    FromArtifact {
        path: String,
        version: u64,
        text: String,
    },
    /// The event named an artifact whose bytes are not text.
    ArtifactBytes {
        path: String,
        version: u64,
        size_bytes: u64,
    },
    /// The event named an artifact and nobody read it. The body is absent and
    /// the row says so, rather than rendering as if there were nothing to show.
    NotRead {
        artifact_id: String,
        path: Option<String>,
    },
}

impl ExoBody {
    /// The text, when there is any.
    #[must_use]
    pub fn text(&self) -> Option<&str> {
        match self {
            Self::Inline(text) | Self::FromArtifact { text, .. } => Some(text),
            Self::ArtifactBytes { .. } | Self::NotRead { .. } => None,
        }
    }

    /// Whether this body is missing because no artifact read paid for it.
    #[must_use]
    pub const fn is_unread_artifact(&self) -> bool {
        matches!(self, Self::NotRead { .. })
    }

    fn resolve(reference: &ExoArtifactRef, artifacts: &ExoArtifactSet, inline: String) -> Self {
        let Some(artifact) = artifacts.get(&reference.artifact_id) else {
            return Self::NotRead {
                artifact_id: reference.artifact_id.clone(),
                path: reference.path.clone(),
            };
        };
        artifact.text().map_or_else(
            || Self::ArtifactBytes {
                path: artifact.version.path.clone(),
                version: artifact.version.version,
                size_bytes: artifact.version.size_bytes,
            },
            |text| Self::FromArtifact {
                path: artifact.version.path.clone(),
                version: artifact.version.version,
                text: if text.is_empty() {
                    inline
                } else {
                    text.to_owned()
                },
            },
        )
    }
}

/// One line of a durable history.
#[derive(Clone, Debug, PartialEq)]
pub enum ExoHistoryRow {
    /// A message in the conversation.
    Message {
        role: String,
        text: String,
        /// What Exo said the call cost. Never accounting truth.
        usage: Option<HarnessReportedUsage>,
    },
    /// A tool call.
    ToolCall {
        tool_call_id: String,
        function_name: String,
        arguments: String,
    },
    /// A tool result — the row ACP summarised, here in full when the artifact
    /// behind it was read.
    ToolResult { tool_call_id: String, body: ExoBody },
    /// An artifact version written during the conversation.
    Artifact {
        path: String,
        version: u64,
        body: ExoBody,
    },
    /// A sandbox or snapshot record.
    Sandbox {
        tag: String,
        sandbox_id: Option<String>,
        snapshot_id: Option<String>,
    },
    /// An error Exo recorded.
    Error { message: String },
    /// Lifecycle, custom, or a variant this build does not know. Kept so the
    /// history is complete rather than tidy.
    Note { tag: String },
}

/// A conversation's durable record, rendered.
#[derive(Clone, Debug, Default)]
pub struct ExoHistory {
    /// The rows, in the order the events arrived.
    pub rows: Vec<ExoHistoryRow>,
    /// Every artifact the log named, whether or not it was read.
    pub referenced_artifacts: Vec<ExoArtifactRef>,
    /// Rows whose body is missing because no artifact read paid for it.
    pub unread_artifact_rows: usize,
}

impl ExoHistory {
    /// Render a page of events against the artifacts that were read.
    #[must_use]
    pub fn read(events: &[ExoEvent], artifacts: &ExoArtifactSet) -> Self {
        let mut history = Self::default();
        for event in events {
            let row = match event.body() {
                ExoEventBody::Messages { messages, usage } => {
                    for message in &messages {
                        history.rows.push(ExoHistoryRow::Message {
                            role: field(message, "role"),
                            text: message_text(message),
                            usage: usage.clone(),
                        });
                    }
                    continue;
                }
                ExoEventBody::ToolRequested {
                    tool_call_id,
                    function_name,
                    arguments,
                } => ExoHistoryRow::ToolCall {
                    tool_call_id,
                    function_name,
                    arguments: compact(&arguments),
                },
                ExoEventBody::ToolResult {
                    tool_call_id,
                    result,
                    artifact,
                } => {
                    let body = artifact.as_ref().map_or_else(
                        || ExoBody::Inline(compact(&result)),
                        |reference| {
                            history.referenced_artifacts.push(reference.clone());
                            ExoBody::resolve(reference, artifacts, compact(&result))
                        },
                    );
                    if body.is_unread_artifact() {
                        history.unread_artifact_rows += 1;
                    }
                    ExoHistoryRow::ToolResult { tool_call_id, body }
                }
                ExoEventBody::ArtifactWritten { artifact } => {
                    history.referenced_artifacts.push(artifact.clone());
                    let body = ExoBody::resolve(&artifact, artifacts, String::new());
                    if body.is_unread_artifact() {
                        history.unread_artifact_rows += 1;
                    }
                    ExoHistoryRow::Artifact {
                        path: artifact.path.clone().unwrap_or_default(),
                        version: artifact.version.unwrap_or_default(),
                        body,
                    }
                }
                ExoEventBody::Sandbox {
                    tag,
                    sandbox_id,
                    snapshot_id,
                } => ExoHistoryRow::Sandbox {
                    tag,
                    sandbox_id,
                    snapshot_id,
                },
                ExoEventBody::Error { message } => ExoHistoryRow::Error { message },
                ExoEventBody::Lifecycle { tag } | ExoEventBody::Unrecognized { tag } => {
                    ExoHistoryRow::Note { tag }
                }
                ExoEventBody::Custom { event_type } => ExoHistoryRow::Note { tag: event_type },
            };
            history.rows.push(row);
        }
        history
    }

    /// The artifact reads this history still needs, without duplicates.
    ///
    /// The caller spends the reads; this says which ones would change the
    /// rendering. A caller that spends none gets a history that admits it.
    #[must_use]
    pub fn unresolved_artifact_ids(&self, artifacts: &ExoArtifactSet) -> Vec<String> {
        let mut wanted: Vec<String> = Vec::new();
        for reference in &self.referenced_artifacts {
            if artifacts.get(&reference.artifact_id).is_none()
                && !wanted.contains(&reference.artifact_id)
            {
                wanted.push(reference.artifact_id.clone());
            }
        }
        wanted
    }

    /// The history as plain text.
    ///
    /// Usage is always prefixed with its provenance, because a token count
    /// beside a message reads as a measurement unless it says otherwise.
    #[must_use]
    pub fn to_text(&self) -> String {
        let mut out = String::new();
        for row in &self.rows {
            match row {
                ExoHistoryRow::Message { role, text, usage } => {
                    out.push_str(&format!("{role}: {text}\n"));
                    if let Some(usage) = usage {
                        out.push_str(&format!(
                            "  [{} usage: {} prompt {:?} completion {:?}]\n",
                            HarnessReportedUsage::PROVENANCE,
                            usage.model,
                            usage.prompt_tokens,
                            usage.completion_tokens
                        ));
                    }
                }
                ExoHistoryRow::ToolCall {
                    function_name,
                    arguments,
                    ..
                } => out.push_str(&format!("tool call {function_name}({arguments})\n")),
                ExoHistoryRow::ToolResult { body, .. } => {
                    out.push_str(&format!("tool result {}\n", describe(body)));
                }
                ExoHistoryRow::Artifact {
                    path,
                    version,
                    body,
                } => out.push_str(&format!(
                    "artifact {path} version {version} {}\n",
                    describe(body)
                )),
                ExoHistoryRow::Sandbox {
                    tag,
                    sandbox_id,
                    snapshot_id,
                } => out.push_str(&format!(
                    "{tag} sandbox {} snapshot {}\n",
                    sandbox_id.as_deref().unwrap_or("-"),
                    snapshot_id.as_deref().unwrap_or("-")
                )),
                ExoHistoryRow::Error { message } => out.push_str(&format!("error: {message}\n")),
                ExoHistoryRow::Note { tag } => out.push_str(&format!("- {tag}\n")),
            }
        }
        out
    }
}

fn describe(body: &ExoBody) -> String {
    match body {
        ExoBody::Inline(text) => text.clone(),
        ExoBody::FromArtifact { path, text, .. } => format!("(from {path}) {text}"),
        ExoBody::ArtifactBytes {
            path, size_bytes, ..
        } => format!("({size_bytes} bytes at {path}, not text)"),
        ExoBody::NotRead { artifact_id, path } => format!(
            "(body not read: artifact {artifact_id}{})",
            path.as_ref()
                .map(|path| format!(" at {path}"))
                .unwrap_or_default()
        ),
    }
}

fn field(value: &serde_json::Value, key: &str) -> String {
    value
        .get(key)
        .and_then(serde_json::Value::as_str)
        .unwrap_or_default()
        .to_owned()
}

/// A Lingua message's text.
///
/// Content is either a string or a list of parts. Anything else renders as
/// compact JSON rather than as nothing, so an unmodelled shape is visible.
fn message_text(message: &serde_json::Value) -> String {
    match message.get("content") {
        Some(serde_json::Value::String(text)) => text.clone(),
        Some(serde_json::Value::Array(parts)) => parts
            .iter()
            .map(|part| match part.get("text") {
                Some(serde_json::Value::String(text)) => text.clone(),
                _ => compact(part),
            })
            .collect::<Vec<_>>()
            .join(""),
        Some(other) => compact(other),
        None => String::new(),
    }
}

fn compact(value: &serde_json::Value) -> String {
    match value {
        serde_json::Value::Null => String::new(),
        serde_json::Value::String(text) => text.clone(),
        other => other.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const CONVERSATION: &str = "0198f3ec-2c8b-7d42-a01f-7e3b5c9d2e66";
    const ARTIFACT: &str = "0198f3ec-3d9c-7e53-b120-8f4c6dae3f77";

    fn event(data: serde_json::Value) -> ExoEvent {
        serde_json::from_value(serde_json::json!({
            "id": "0198f3ec-4eaf-7f64-c231-9a5d7ebf4088",
            "conversation_id": CONVERSATION,
            "session_id": null,
            "turn_id": null,
            "created_at": "2026-07-26T09:15:00Z",
            "data": data,
        }))
        .expect("an event")
    }

    fn full_run_output() -> ExoArtifact {
        serde_json::from_value(serde_json::json!({
            "artifact_id": ARTIFACT,
            "path": "scheduled-tasks/nightly/run-1.json",
            "version": 1,
            "created_at": "2026-07-26T09:15:00Z",
            "size_bytes": 27,
            "contents": "every line of the output".as_bytes(),
        }))
        .expect("an artifact")
    }

    /// A turn as Exo records it: the model spoke, called a tool, and the tool's
    /// full result went to an artifact with a preview left behind.
    fn a_turn() -> Vec<ExoEvent> {
        vec![
            event(serde_json::json!({ "type": "turn_started" })),
            event(serde_json::json!({
                "type": "messages",
                "messages": [{ "role": "assistant", "content": "running the suite" }],
                "usage": { "model": "gpt-5-mini", "prompt_tokens": 900, "completion_tokens": 40 },
            })),
            event(serde_json::json!({
                "type": "tool_requested",
                "tool_call_id": "call-1",
                "request": { "function_name": "bash", "arguments": { "command": "cargo test" } },
            })),
            event(serde_json::json!({
                "type": "tool_result",
                "tool_call_id": "call-1",
                "result": { "artifact_id": ARTIFACT, "preview": "…", "truncated": true },
            })),
            event(serde_json::json!({
                "type": "artifact_written",
                "artifact_id": ARTIFACT,
                "path": "scheduled-tasks/nightly/run-1.json",
                "version": 1,
            })),
            event(serde_json::json!({
                "type": "sandbox_snapshotted",
                "sandbox_id": "sbx-1",
                "snapshot_id": "snap-1",
            })),
            event(serde_json::json!({ "type": "turn_ended" })),
        ]
    }

    /// The acceptance: the turn is over, and the record still has the tool
    /// result ACP summarised.
    #[test]
    fn a_finished_turn_renders_its_tool_result_from_the_artifact() {
        let artifacts: ExoArtifactSet = [full_run_output()].into_iter().collect();
        let history = ExoHistory::read(&a_turn(), &artifacts);
        let text = history.to_text();

        // Asserted on the tool-result row itself, not on the rendered page. An
        // earlier version of this test looked for the artifact's text anywhere
        // in the output, which the `artifact_written` row satisfies on its own —
        // so a build that stopped resolving tool results entirely still passed.
        // The acceptance is about the row ACP summarised.
        let body = history
            .rows
            .iter()
            .find_map(|row| match row {
                ExoHistoryRow::ToolResult { tool_call_id, body } if tool_call_id == "call-1" => {
                    Some(body)
                }
                _ => None,
            })
            .expect("the tool result is a row");
        assert_eq!(
            body,
            &ExoBody::FromArtifact {
                path: "scheduled-tasks/nightly/run-1.json".into(),
                version: 1,
                text: "every line of the output".into(),
            }
        );

        assert!(text.contains("every line of the output"), "{text}");
        assert!(text.contains("tool call bash"), "{text}");
        assert!(text.contains("snapshot snap-1"), "{text}");
        assert_eq!(history.unread_artifact_rows, 0);
        assert!(history.unresolved_artifact_ids(&artifacts).is_empty());
    }

    /// The falsifier, run. Remove the artifact read and the history keeps every
    /// name and loses every body — which is what proves the artifact path is
    /// what carries tool results, rather than the event log.
    #[test]
    fn without_the_artifact_read_the_history_loses_its_tool_results() {
        let events = a_turn();
        let empty = ExoArtifactSet::new();
        let without = ExoHistory::read(&events, &empty);

        assert!(!without.to_text().contains("every line of the output"));
        assert_eq!(without.unread_artifact_rows, 2);
        assert_eq!(
            without.unresolved_artifact_ids(&empty),
            vec![ARTIFACT.to_owned()]
        );
        assert!(
            without.to_text().contains("body not read"),
            "{}",
            without.to_text()
        );

        // Same events, same row count: nothing was dropped, only unresolved.
        let with: ExoHistory =
            ExoHistory::read(&events, &[full_run_output()].into_iter().collect());
        assert_eq!(without.rows.len(), with.rows.len());
        assert_ne!(without.rows, with.rows);
    }

    /// Usage never renders as a bare number.
    #[test]
    fn harness_reported_usage_says_that_it_is_harness_reported() {
        let history = ExoHistory::read(&a_turn(), &ExoArtifactSet::new());
        let text = history.to_text();
        assert!(text.contains("harness-reported usage"), "{text}");
        assert!(text.contains("gpt-5-mini"), "{text}");
    }

    /// An artifact whose bytes are not text renders as bytes, not as an empty
    /// tool result.
    #[test]
    fn a_binary_artifact_renders_as_bytes() {
        let artifact: ExoArtifact = serde_json::from_value(serde_json::json!({
            "artifact_id": ARTIFACT,
            "path": "snapshot.bin",
            "version": 4,
            "created_at": "2026-07-26T09:15:00Z",
            "size_bytes": 2,
            "contents": [0xff, 0xfe],
        }))
        .expect("an artifact");
        let history = ExoHistory::read(&a_turn(), &[artifact].into_iter().collect());
        assert!(
            history.to_text().contains("not text"),
            "{}",
            history.to_text()
        );
        assert_eq!(history.unread_artifact_rows, 0);
    }

    /// An event this build does not know still occupies a row.
    #[test]
    fn an_unknown_event_still_takes_a_row() {
        let history = ExoHistory::read(
            &[event(serde_json::json!({ "type": "wormhole_opened" }))],
            &ExoArtifactSet::new(),
        );
        assert_eq!(
            history.rows,
            vec![ExoHistoryRow::Note {
                tag: "wormhole_opened".into()
            }]
        );
    }
}
