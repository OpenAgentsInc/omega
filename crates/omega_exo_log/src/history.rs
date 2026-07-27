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

/// Artifacts that were read, keyed by the version that was read.
///
/// Built by the caller from whatever artifact reads it chose to spend. An empty
/// set is legitimate and produces a history that says what it is missing.
///
/// # The key is `(id, version)`, and that is the whole point
///
/// An Exo artifact is a *versioned* record: `artifact_written` carries a
/// version, `ExoArtifactRef` carries the version the event named, and Exo's own
/// scheduler rewrites the same path many times in one conversation. A set keyed
/// by id alone loses that. Insert version 2 and every event that referenced
/// version 1 renders version 2's bytes; hold only version 2 and a version-1
/// reference looks resolved. Either way the row reads as complete while showing
/// a body from a later point in the conversation, which is the durable-replay
/// claim failing silently — the one failure mode a durable log exists to
/// prevent.
#[derive(Clone, Debug, Default)]
pub struct ExoArtifactSet {
    by_version: HashMap<(String, u64), ExoArtifact>,
    /// The highest version held for each id, which is what an unversioned
    /// reference resolves to.
    latest: HashMap<String, u64>,
}

impl ExoArtifactSet {
    /// An empty set: names resolve to nothing.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Add an artifact that was read.
    pub fn insert(&mut self, artifact: ExoArtifact) {
        let artifact_id = artifact.version.artifact_id.clone();
        let version = artifact.version.version;
        self.latest
            .entry(artifact_id.clone())
            .and_modify(|held| {
                if version > *held {
                    *held = version;
                }
            })
            .or_insert(version);
        self.by_version.insert((artifact_id, version), artifact);
    }

    /// Look one up.
    ///
    /// `Some(version)` requires **that** version and resolves to nothing
    /// otherwise: a reference that named a version is answered with the bytes it
    /// named or with nothing at all, never with a neighbouring version's.
    ///
    /// `None` is the latest version this set holds, which is the same policy
    /// Exo applies to a read with `version: null` — so an unversioned reference
    /// means the same thing on both sides of the wire.
    #[must_use]
    pub fn get(&self, artifact_id: &str, version: Option<u64>) -> Option<&ExoArtifact> {
        let version = version.or_else(|| self.latest.get(artifact_id).copied())?;
        self.by_version.get(&(artifact_id.to_owned(), version))
    }

    /// How many artifact versions were read.
    #[must_use]
    pub fn len(&self) -> usize {
        self.by_version.len()
    }

    /// Whether nothing was read.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.by_version.is_empty()
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
    /// The event named an artifact and nobody read it — or read a different
    /// version of it. The body is absent and the row says so, rather than
    /// rendering as if there were nothing to show.
    NotRead {
        artifact_id: String,
        path: Option<String>,
        /// The version the event named, when it named one. Carried so the
        /// caller can fetch *this* body rather than whatever is latest.
        version: Option<u64>,
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
        let Some(artifact) = artifacts.get(&reference.artifact_id, reference.version) else {
            return Self::NotRead {
                artifact_id: reference.artifact_id.clone(),
                path: reference.path.clone(),
                version: reference.version,
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
    ///
    /// References rather than ids, because an id is not enough to fetch with.
    /// An event that named version 1 needs version 1 read; handed only the id,
    /// a caller asks for the latest, gets version 3, and the row it fills in is
    /// the wrong body under the right name. The version travels with the
    /// request so the caller can ask for the bytes it was told were missing.
    #[must_use]
    pub fn unresolved_artifacts(&self, artifacts: &ExoArtifactSet) -> Vec<ExoArtifactRef> {
        let mut wanted: Vec<ExoArtifactRef> = Vec::new();
        for reference in &self.referenced_artifacts {
            if artifacts
                .get(&reference.artifact_id, reference.version)
                .is_some()
            {
                continue;
            }
            if wanted.iter().any(|held| {
                held.artifact_id == reference.artifact_id && held.version == reference.version
            }) {
                continue;
            }
            wanted.push(reference.clone());
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

/// Why no durable history was read. `OMEGA-DELTA-0107`, omega#104.
///
/// Every variant is an absence **with a cause**, and none of them is an empty
/// conversation. That distinction is the whole reason this type exists rather
/// than an `Option<ExoHistory>`: on a machine where the owner never set
/// `EXO_EXOHARNESS_URL` — which is the ordinary machine, and the safe one — the
/// durable log is simply not reachable, and a surface that renders that as a
/// thread with no history has told the reader something false about their own
/// conversation. So each variant carries a sentence, and every sentence ends
/// with [`Self::NOT_AN_EMPTY_HISTORY`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ExoHistoryUnavailable {
    /// `EXO_EXOHARNESS_URL` is unset, so this lane reaches Exo through the CLI
    /// and the state root on disk, and no socket exists to read.
    ///
    /// Omega does not start one. `serve` is in
    /// `omega_deltas::EXO_REDIRECTING_FLAGS` and `ExoCommand` cannot express
    /// it; a second process pointed at one `.exo` root is exactly what
    /// `omega_exo_episode::root` refuses, and Omega cannot know whether the
    /// owner already has one running. Reading a log the owner pointed us at is
    /// a read. Starting a server is process authority, and it is not this
    /// crate's to take.
    NotConfigured,
    /// Exo printed no id for the agent, so there is nothing to address.
    ///
    /// Optional on purpose upstream of here: an Exo that prints no id line
    /// still parses and still runs a turn, because "cannot show the history" is
    /// a smaller failure than "cannot run the agent".
    NoAgentId,
    /// Exo printed no id for the conversation, so there is nothing to address.
    NoConversationId,
}

impl ExoHistoryUnavailable {
    /// The clause every one of these sentences carries.
    ///
    /// Checked by `OMEGA-DELTA-0107` against each variant, because the failure
    /// this type exists to prevent is a *rendering* that reads as emptiness,
    /// and the only defence a leaf crate can offer is that the words it hands
    /// the renderer say otherwise.
    pub const NOT_AN_EMPTY_HISTORY: &'static str =
        "this is not an empty history; Omega did not read one";

    /// Every variant, so a check can drive all of them.
    pub const ALL: &'static [Self] =
        &[Self::NotConfigured, Self::NoAgentId, Self::NoConversationId];
}

impl std::fmt::Display for ExoHistoryUnavailable {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let cause = match self {
            Self::NotConfigured => {
                "this Exo lane runs on the CLI and no reachable Exo server is configured, so \
                 the durable log is not available"
            }
            Self::NoAgentId => {
                "Exo printed no id for this agent, so the durable log has nothing to be \
                 addressed by"
            }
            Self::NoConversationId => {
                "Exo printed no id for this conversation, so the durable log has nothing to \
                 be addressed by"
            }
        };
        write!(formatter, "{cause}: {}", Self::NOT_AN_EMPTY_HISTORY)
    }
}

impl std::error::Error for ExoHistoryUnavailable {}

/// What a durable-log read produced. `OMEGA-DELTA-0107`, omega#104.
///
/// Deliberately without a `Default`. A default would be an empty [`ExoHistory`]
/// — a value that says "this conversation has no durable record" and that
/// nobody had to decide to construct.
#[derive(Clone, Debug)]
pub enum ExoDurableHistory {
    /// Nothing was read, and this is why.
    Unavailable(ExoHistoryUnavailable),
    /// The durable record, rendered.
    Read(ExoHistory),
}

impl ExoDurableHistory {
    /// The history, when one was read.
    #[must_use]
    pub const fn history(&self) -> Option<&ExoHistory> {
        match self {
            Self::Read(history) => Some(history),
            Self::Unavailable(_) => None,
        }
    }

    /// Why nothing was read, when nothing was.
    #[must_use]
    pub const fn unavailable(&self) -> Option<ExoHistoryUnavailable> {
        match self {
            Self::Unavailable(reason) => Some(*reason),
            Self::Read(_) => None,
        }
    }

    /// The rendering: the history, or the sentence saying why there is none.
    #[must_use]
    pub fn to_text(&self) -> String {
        match self {
            Self::Read(history) => history.to_text(),
            Self::Unavailable(reason) => reason.to_string(),
        }
    }
}

fn describe(body: &ExoBody) -> String {
    match body {
        ExoBody::Inline(text) => text.clone(),
        ExoBody::FromArtifact { path, text, .. } => format!("(from {path}) {text}"),
        ExoBody::ArtifactBytes {
            path, size_bytes, ..
        } => format!("({size_bytes} bytes at {path}, not text)"),
        ExoBody::NotRead {
            artifact_id,
            path,
            version,
        } => format!(
            "(body not read: artifact {artifact_id}{}{})",
            version
                .map(|version| format!(" version {version}"))
                .unwrap_or_default(),
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
        assert!(history.unresolved_artifacts(&artifacts).is_empty());
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
        // Two reads for one artifact id, because the two events reference it
        // differently: the tool result named no version and `artifact_written`
        // named version 1. They are different requests and Exo answers them
        // differently, so collapsing them to one id is how a caller ends up
        // fetching the latest for a row that asked for a specific version.
        let wanted = without.unresolved_artifacts(&empty);
        assert_eq!(wanted.len(), 2, "{wanted:?}");
        assert!(
            wanted
                .iter()
                .all(|reference| reference.artifact_id == ARTIFACT)
        );
        let mut versions: Vec<Option<u64>> =
            wanted.iter().map(|reference| reference.version).collect();
        versions.sort_unstable();
        assert_eq!(versions, vec![None, Some(1)]);
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
    ///
    /// Version 1 because that is the version `a_turn`'s `artifact_written`
    /// names. It read as version 4 until `OMEGA-DELTA-0107` keyed the set by
    /// version, at which point the mismatch stopped resolving — correctly, and
    /// this test is about bytes rather than about versions.
    #[test]
    fn a_binary_artifact_renders_as_bytes() {
        let artifact: ExoArtifact = serde_json::from_value(serde_json::json!({
            "artifact_id": ARTIFACT,
            "path": "snapshot.bin",
            "version": 1,
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

    /// `OMEGA-DELTA-0107`. Two versions of one artifact, and each row renders
    /// its own bytes.
    ///
    /// The reviewer's falsifier on `b074ac3986`, run. Keyed by id alone, the
    /// second insert overwrote the first and every version-1 reference rendered
    /// version 2's bytes — a row that looks complete and artifact-backed while
    /// showing a body from a later point in the conversation. That is the
    /// durable-replay claim failing in the one direction nobody would notice.
    #[test]
    fn each_versioned_reference_renders_its_own_version() {
        let write = |version: u64, text: &str| -> ExoArtifact {
            serde_json::from_value(serde_json::json!({
                "artifact_id": ARTIFACT,
                "path": "notes.md",
                "version": version,
                "created_at": "2026-07-26T09:15:00Z",
                "size_bytes": text.len(),
                "contents": text.as_bytes(),
            }))
            .expect("an artifact")
        };
        let events = vec![
            event(serde_json::json!({
                "type": "artifact_written",
                "artifact_id": ARTIFACT, "path": "notes.md", "version": 1,
            })),
            event(serde_json::json!({
                "type": "artifact_written",
                "artifact_id": ARTIFACT, "path": "notes.md", "version": 2,
            })),
        ];

        let both: ExoArtifactSet = [write(1, "first"), write(2, "second")]
            .into_iter()
            .collect();
        let history = ExoHistory::read(&events, &both);
        let bodies: Vec<&ExoBody> = history
            .rows
            .iter()
            .filter_map(|row| match row {
                ExoHistoryRow::Artifact { body, .. } => Some(body),
                _ => None,
            })
            .collect();
        assert_eq!(bodies.len(), 2);
        assert_eq!(bodies[0].text(), Some("first"), "{bodies:?}");
        assert_eq!(bodies[1].text(), Some("second"), "{bodies:?}");
        assert_eq!(history.unread_artifact_rows, 0);
        assert_eq!(both.len(), 2, "two versions are two entries, not one");

        // Only version 2 read. The version-1 row must stay unread and ask for
        // version 1, rather than borrowing the bytes it can reach.
        let later: ExoArtifactSet = [write(2, "second")].into_iter().collect();
        let partial = ExoHistory::read(&events, &later);
        assert_eq!(partial.unread_artifact_rows, 1);
        assert_eq!(
            partial.rows[0],
            ExoHistoryRow::Artifact {
                path: "notes.md".into(),
                version: 1,
                body: ExoBody::NotRead {
                    artifact_id: ARTIFACT.into(),
                    path: Some("notes.md".into()),
                    version: Some(1),
                },
            }
        );
        let wanted = partial.unresolved_artifacts(&later);
        assert_eq!(wanted.len(), 1, "{wanted:?}");
        assert_eq!(
            wanted[0].version,
            Some(1),
            "the caller is told which version is missing, so it can fetch that one"
        );

        // The version-2 row still renders "second" — it referenced version 2
        // and version 2 was read. The claim is about the *version-1* row, which
        // must not borrow the bytes it can reach.
        let first_row = partial
            .to_text()
            .lines()
            .next()
            .expect("the version-1 row")
            .to_owned();
        assert!(!first_row.contains("second"), "{first_row}");
        assert!(!first_row.contains("first"), "{first_row}");
        assert!(first_row.contains("body not read"), "{first_row}");
    }

    /// `OMEGA-DELTA-0107`. An unread durable log says why, and never says the
    /// thread has no history.
    #[test]
    fn an_unavailable_durable_history_names_its_cause() {
        for reason in ExoHistoryUnavailable::ALL {
            let sentence = reason.to_string();
            assert!(
                sentence.contains(ExoHistoryUnavailable::NOT_AN_EMPTY_HISTORY),
                "{reason:?} renders as {sentence:?}, which a surface can show as \
                 an empty conversation"
            );
            let unread = ExoDurableHistory::Unavailable(*reason);
            assert!(unread.history().is_none());
            assert_eq!(unread.unavailable(), Some(*reason));
            assert_eq!(unread.to_text(), sentence);
        }
        assert_eq!(ExoHistoryUnavailable::ALL.len(), 3);

        let read = ExoDurableHistory::Read(ExoHistory::read(
            &a_turn(),
            &[full_run_output()].into_iter().collect(),
        ));
        assert!(read.unavailable().is_none());
        assert!(
            read.history()
                .is_some_and(|history| !history.rows.is_empty())
        );
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
