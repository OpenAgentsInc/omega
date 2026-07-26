//! What Exo's durable record decodes to. `OMEGA-DELTA-0091`, omega#104.
//!
//! Exo's own types, restated in Omega's vocabulary, with three deliberate
//! differences:
//!
//! * **Unknown is a value, not an error.** Exo declares itself unstable and
//!   writes "do not write fallback code" into its own `AGENTS.md`, so it will
//!   add event variants. An event this build does not know becomes
//!   [`ExoEventBody::Unrecognized`] carrying its tag, and the history says so.
//!   A decoder that failed the whole page on one unknown row would lose the
//!   forty rows around it, which is the opposite of reading the durable record.
//! * **Usage is typed as unattested.** [`HarnessReportedUsage`] is what Exo
//!   said about a model call Exo made. Exo's own cost design doc calls it
//!   "agent-reported telemetry, not an attested ledger", and Omega's receipts
//!   must mark it as harness-reported. There is deliberately no conversion from
//!   this type into any Omega usage, cost, or credit type.
//! * **Bytes stay bytes.** An artifact's contents are `Vec<u8>` on the wire and
//!   here. Rendering decides whether they are text.

use serde::Deserialize;

/// The `type` tag on Exo's `Response`.
///
/// Only the shapes the eight admitted queries can be answered with. A reply
/// carrying any other tag is a decode failure, which is how a mis-routed
/// response becomes an error rather than an empty history.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ExoResponseTag {
    /// `get_agent`.
    Agent,
    /// `get_conversation`.
    Conversation,
    /// `conversation_get_events`.
    Events,
    /// `conversation_get_event`.
    Event,
    /// `agent_list_artifacts`, `conversation_list_artifacts`.
    ArtifactVersions,
    /// `agent_read_artifact`, `conversation_read_artifact`.
    Artifact,
}

impl ExoResponseTag {
    /// The tag as Exo spells it.
    #[must_use]
    pub const fn wire(self) -> &'static str {
        match self {
            Self::Agent => "agent",
            Self::Conversation => "conversation",
            Self::Events => "events",
            Self::Event => "event",
            Self::ArtifactVersions => "artifact_versions",
            Self::Artifact => "artifact",
        }
    }
}

/// Exo's `AgentRecord`.
#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
pub struct ExoAgentRecord {
    pub id: String,
    pub slug: String,
    pub name: String,
}

/// Exo's `ConversationRecord`, inside the `ConversationHandleInfo` the protocol
/// wraps it in.
#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
pub struct ExoConversationRecord {
    pub id: String,
    pub slug: String,
    pub name: String,
    /// The head of the log. Exo's turn writers compare against this, and a
    /// reader uses it to tell a stale page from a complete one.
    #[serde(default)]
    pub latest_event_id: Option<String>,
}

/// Exo's `ConversationHandleInfo`.
#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
pub struct ExoConversation {
    pub agent_id: String,
    pub record: ExoConversationRecord,
}

/// Exo's `ArtifactVersion`: an artifact named, not an artifact read.
///
/// The event log carries these. It never carries contents — which is why the
/// artifact read is load-bearing rather than decorative.
#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
pub struct ExoArtifactVersion {
    pub artifact_id: String,
    pub path: String,
    pub version: u64,
    pub created_at: String,
    pub size_bytes: u64,
}

/// Exo's `Artifact`: a version plus its bytes.
///
/// `#[serde(flatten)]` on Exo's side, so the version fields are siblings of
/// `contents` on the wire rather than nested.
#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
pub struct ExoArtifact {
    #[serde(flatten)]
    pub version: ExoArtifactVersion,
    pub contents: Vec<u8>,
}

impl ExoArtifact {
    /// The contents as text, when they are valid UTF-8.
    ///
    /// `None` for anything else rather than a lossy string: a snapshot payload
    /// rendered as replacement characters reads like a corrupt transcript.
    #[must_use]
    pub fn text(&self) -> Option<&str> {
        std::str::from_utf8(&self.contents).ok()
    }
}

/// Exo's `GetEventsResult`.
#[derive(Clone, Debug, Deserialize)]
pub struct ExoEventPage {
    pub events: Vec<ExoEvent>,
    /// Where the next page starts. `None` means this page reached the end.
    #[serde(default)]
    pub cursor: Option<String>,
}

/// One durable event.
#[derive(Clone, Debug, Deserialize)]
pub struct ExoEvent {
    pub id: String,
    pub conversation_id: String,
    #[serde(default)]
    pub session_id: Option<String>,
    #[serde(default)]
    pub turn_id: Option<String>,
    pub created_at: String,
    pub data: serde_json::Value,
}

impl ExoEvent {
    /// The event's `type` tag, as Exo wrote it.
    #[must_use]
    pub fn tag(&self) -> &str {
        self.data
            .get("type")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("")
    }

    /// The event, read into the shapes this build understands.
    #[must_use]
    pub fn body(&self) -> ExoEventBody {
        ExoEventBody::read(&self.data)
    }
}

/// What Exo said a model call cost, as Exo reported it.
///
/// **Not accounting truth.** Exo never makes the model call through an attested
/// path; its own cost design doc says these numbers are agent-reported
/// telemetry. Any Omega receipt that carries them marks them as
/// harness-reported, and this type deliberately converts into nothing.
#[derive(Clone, Debug, Default, Deserialize, PartialEq)]
pub struct HarnessReportedUsage {
    pub model: String,
    #[serde(default)]
    pub prompt_tokens: Option<i64>,
    #[serde(default)]
    pub completion_tokens: Option<i64>,
    #[serde(default)]
    pub completion_reasoning_tokens: Option<i64>,
    /// Exo's own price table applied to Exo's own token counts.
    #[serde(default)]
    pub cost_usd: Option<f64>,
    #[serde(default)]
    pub ttft_ms: Option<u64>,
    #[serde(default)]
    pub duration_ms: Option<u64>,
}

impl HarnessReportedUsage {
    /// The provenance every rendering of these numbers must carry.
    pub const PROVENANCE: &'static str = "harness-reported";
}

/// An artifact an event names but does not contain.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExoArtifactRef {
    pub artifact_id: String,
    /// The path, when the event carried one.
    pub path: Option<String>,
    /// The version, when the event carried one.
    pub version: Option<u64>,
}

/// An event, read into the shapes this build understands.
///
/// Not every one of Exo's twenty-two variants: the ones a durable history has
/// to render, plus [`Self::Unrecognized`] for the rest, which is honest about
/// an upstream that adds variants without notice.
#[derive(Clone, Debug, PartialEq)]
pub enum ExoEventBody {
    /// Conversation, session, or turn lifecycle. The tag is kept so the render
    /// can say which.
    Lifecycle { tag: String },
    /// Model traffic. `messages` is Lingua's shape, kept as JSON: Omega renders
    /// role and text and does not re-model somebody else's message type.
    Messages {
        messages: Vec<serde_json::Value>,
        usage: Option<HarnessReportedUsage>,
    },
    /// A tool call Exo's agent made.
    ToolRequested {
        tool_call_id: String,
        function_name: String,
        arguments: serde_json::Value,
    },
    /// The result of a tool call. This is the row ACP summarised.
    ToolResult {
        tool_call_id: String,
        result: serde_json::Value,
        /// Set when the result names an artifact, which is where Exo puts a
        /// body too big for an event.
        artifact: Option<ExoArtifactRef>,
    },
    /// An artifact version was written. The bytes are not here.
    ArtifactWritten { artifact: ExoArtifactRef },
    /// Sandbox lifecycle: created, started, stopped, snapshotted, or a process
    /// event. The sandbox and any snapshot are named because snapshot identity
    /// anchored to a transcript position is what makes environment time travel
    /// legible.
    Sandbox {
        tag: String,
        sandbox_id: Option<String>,
        snapshot_id: Option<String>,
    },
    /// Exo recorded an error.
    Error { message: String },
    /// Exo's `custom` namespace, which carries real weight upstream: model
    /// config, host lifecycle, and the fork's durable turn cancellation.
    Custom { event_type: String },
    /// A variant this build does not know. Kept, not dropped.
    Unrecognized { tag: String },
}

impl ExoEventBody {
    fn read(data: &serde_json::Value) -> Self {
        let tag = data
            .get("type")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("")
            .to_owned();
        match tag.as_str() {
            "conversation_created"
            | "conversation_updated"
            | "conversation_deleted"
            | "conversation_forked"
            | "session_started"
            | "session_ended"
            | "turn_started"
            | "turn_ended" => Self::Lifecycle { tag },
            "messages" => Self::Messages {
                messages: data
                    .get("messages")
                    .and_then(serde_json::Value::as_array)
                    .cloned()
                    .unwrap_or_default(),
                usage: data
                    .get("usage")
                    .and_then(|usage| serde_json::from_value(usage.clone()).ok()),
            },
            "tool_requested" => Self::ToolRequested {
                tool_call_id: string_at(data, "tool_call_id"),
                function_name: data
                    .get("request")
                    .map(|request| string_at(request, "function_name"))
                    .unwrap_or_default(),
                arguments: data
                    .get("request")
                    .and_then(|request| request.get("arguments"))
                    .cloned()
                    .unwrap_or(serde_json::Value::Null),
            },
            "tool_result" => {
                let result = data
                    .get("result")
                    .cloned()
                    .unwrap_or(serde_json::Value::Null);
                Self::ToolResult {
                    tool_call_id: string_at(data, "tool_call_id"),
                    artifact: artifact_ref(&result),
                    result,
                }
            }
            "artifact_written" => Self::ArtifactWritten {
                artifact: ExoArtifactRef {
                    artifact_id: string_at(data, "artifact_id"),
                    path: data
                        .get("path")
                        .and_then(serde_json::Value::as_str)
                        .map(ToOwned::to_owned),
                    version: data.get("version").and_then(serde_json::Value::as_u64),
                },
            },
            "sandbox_created"
            | "sandbox_started"
            | "sandbox_stopped"
            | "sandbox_snapshotted"
            | "sandbox_process_started"
            | "sandbox_process_state_updated"
            | "sandbox_process_event" => Self::Sandbox {
                tag,
                sandbox_id: data
                    .get("sandbox_id")
                    .and_then(serde_json::Value::as_str)
                    .map(ToOwned::to_owned),
                snapshot_id: data
                    .get("snapshot_id")
                    .and_then(serde_json::Value::as_str)
                    .map(ToOwned::to_owned),
            },
            "error" => Self::Error {
                message: string_at(data, "message"),
            },
            "custom" => Self::Custom {
                event_type: string_at(data, "event_type"),
            },
            _ => Self::Unrecognized { tag },
        }
    }
}

fn string_at(value: &serde_json::Value, key: &str) -> String {
    value
        .get(key)
        .and_then(serde_json::Value::as_str)
        .unwrap_or_default()
        .to_owned()
}

/// Read an artifact reference out of a tool result.
///
/// Exo puts a body too large for an event into a versioned artifact and leaves
/// the id behind — its scheduler does exactly this with a run's stdout. The
/// reference is only recognised when the result names an `artifact_id` that is
/// a string, so an unrelated field of that name in a tool's own output cannot
/// promote arbitrary text into an artifact read.
fn artifact_ref(result: &serde_json::Value) -> Option<ExoArtifactRef> {
    let artifact_id = result.get("artifact_id")?.as_str()?;
    if crate::ExoId::parse(artifact_id).is_err() {
        return None;
    }

    Some(ExoArtifactRef {
        artifact_id: artifact_id.to_owned(),
        path: result
            .get("path")
            .and_then(serde_json::Value::as_str)
            .map(ToOwned::to_owned),
        version: result.get("version").and_then(serde_json::Value::as_u64),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_artifact_decodes_exos_flattened_shape() {
        let artifact: ExoArtifact = serde_json::from_value(serde_json::json!({
            "artifact_id": "0198f3ec-3d9c-7e53-b120-8f4c6dae3f77",
            "path": "scheduled-tasks/nightly/run-1.json",
            "version": 2,
            "created_at": "2026-07-26T09:15:00Z",
            "size_bytes": 5,
            "contents": [104, 101, 108, 108, 111],
        }))
        .expect("Exo's artifact shape");
        assert_eq!(artifact.version.version, 2);
        assert_eq!(artifact.text(), Some("hello"));
    }

    #[test]
    fn invalid_utf8_contents_are_not_rendered_as_text() {
        let artifact: ExoArtifact = serde_json::from_value(serde_json::json!({
            "artifact_id": "0198f3ec-3d9c-7e53-b120-8f4c6dae3f77",
            "path": "snapshot.bin",
            "version": 1,
            "created_at": "2026-07-26T09:15:00Z",
            "size_bytes": 2,
            "contents": [0xff, 0xfe],
        }))
        .expect("Exo's artifact shape");
        assert_eq!(artifact.text(), None);
    }

    /// An event variant added upstream after this build must not take the page
    /// down with it.
    #[test]
    fn an_unknown_event_is_kept_rather_than_dropped() {
        let event = ExoEvent {
            id: "0198f3ec-1b7a-7c31-9f0e-6d2a4b8c1d55".into(),
            conversation_id: "0198f3ec-2c8b-7d42-a01f-7e3b5c9d2e66".into(),
            session_id: None,
            turn_id: None,
            created_at: "2026-07-26T09:15:00Z".into(),
            data: serde_json::json!({ "type": "quantum_entangled", "payload": {} }),
        };
        assert_eq!(
            event.body(),
            ExoEventBody::Unrecognized {
                tag: "quantum_entangled".into()
            }
        );
        assert_eq!(event.tag(), "quantum_entangled");
    }

    #[test]
    fn usage_decodes_and_carries_its_provenance() {
        let ExoEventBody::Messages { usage, .. } = ExoEventBody::read(&serde_json::json!({
            "type": "messages",
            "messages": [],
            "usage": { "model": "gpt-5-mini", "prompt_tokens": 900, "cost_usd": 0.0031 },
        })) else {
            panic!("messages")
        };
        let usage = usage.expect("Exo reported usage");
        assert_eq!(usage.prompt_tokens, Some(900));
        assert_eq!(HarnessReportedUsage::PROVENANCE, "harness-reported");
    }

    /// A tool whose own output has a field called `artifact_id` holding prose
    /// must not become an artifact read.
    #[test]
    fn only_a_uuid_shaped_artifact_id_becomes_a_reference() {
        let body = ExoEventBody::read(&serde_json::json!({
            "type": "tool_result",
            "tool_call_id": "call-1",
            "result": { "artifact_id": "the one from yesterday" },
        }));
        let ExoEventBody::ToolResult { artifact, .. } = body else {
            panic!("tool result")
        };
        assert_eq!(artifact, None);
    }
}
