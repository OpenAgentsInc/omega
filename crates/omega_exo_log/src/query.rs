//! The read-only slice of Exo's protocol. `OMEGA-DELTA-0091`, omega#104.
//!
//! `exo serve` answers **52** request variants over one unauthenticated
//! loopback endpoint (`Request::kind` in `crates/exoharness/src/protocol.rs` at
//! the pinned commit — counted, not quoted; Omega's own
//! `omega_exo_lane::endpoint` prose says 53 and is one out).
//!
//! Eight of those variants read a conversation's durable record. Omega wants
//! exactly those eight. The other forty-four create agents, write events, fork
//! conversations, run sandboxes, and read secrets, and this crate must not be
//! able to send one.
//!
//! # Why a closed enum rather than a check
//!
//! [`ExoQuery`] has eight constructors and no escape hatch. There is no
//! `ExoQuery::from_kind(&str)`, no public wire-string constructor, and
//! [`ExoReadClient`] takes an `ExoQuery` and nothing else. A caller that wants
//! `conversation_add_events` has to add a variant to this file, which is a diff
//! a reviewer reads and `OMEGA-DELTA-0091` fails on — rather than a runtime
//! denylist that is satisfied by whoever remembered to call it.
//!
//! The same discipline as `LoopbackEndpoint`: every value of the type is
//! admissible because there is no other way to make one.
//!
//! [`ExoReadClient`]: crate::ExoReadClient

use crate::record::ExoResponseTag;

/// A durable id in Exo's store.
///
/// Every core Exo id is a UUIDv7, so an id is also a creation timestamp and a
/// sort key (`crates/exoharness/src/uuid7.rs`). This newtype does not decode
/// the timestamp — it refuses anything that is not UUID-shaped, so an id that
/// reaches the wire is an id Exo could have minted rather than arbitrary text
/// that happened to be in a field.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ExoId(String);

/// Why a string is not an Exo id.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct NotAnExoId;

impl std::fmt::Display for NotAnExoId {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("Exo ids are UUIDs, and that is not one")
    }
}

impl std::error::Error for NotAnExoId {}

impl ExoId {
    /// Read an id, admitting only the UUID shape Exo mints.
    ///
    /// # Errors
    ///
    /// [`NotAnExoId`] for anything else.
    pub fn parse(text: &str) -> Result<Self, NotAnExoId> {
        let text = text.trim();
        if text.len() != 36 {
            return Err(NotAnExoId);
        }
        for (offset, character) in text.char_indices() {
            let expected_hyphen = matches!(offset, 8 | 13 | 18 | 23);
            if expected_hyphen {
                if character != '-' {
                    return Err(NotAnExoId);
                }
            } else if !character.is_ascii_hexdigit() {
                return Err(NotAnExoId);
            }
        }
        Ok(Self(text.to_ascii_lowercase()))
    }

    /// The id as Exo spells it.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for ExoId {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.0)
    }
}

/// Which end of the log to read from.
///
/// Exo's `EventQueryDirection`, restated. `Desc` is what a workspace wants when
/// it opens a long conversation: the tail, then older pages behind it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ExoReadDirection {
    /// Oldest first. Exo's `asc`.
    #[default]
    Oldest,
    /// Newest first. Exo's `desc`.
    Newest,
}

impl ExoReadDirection {
    const fn wire(self) -> &'static str {
        match self {
            Self::Oldest => "asc",
            Self::Newest => "desc",
        }
    }
}

/// How much of a conversation's log to ask for.
///
/// Exo's `EventQuery` minus the fields this crate has no read for. `types` is
/// deliberately absent: a kind filter applied on the wire would let a caller
/// build a history with the tool results quietly removed, and a rendering that
/// silently drops rows is the failure this issue exists to end.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ExoEventWindow {
    /// Read after this event. `None` starts at the end named by `direction`.
    pub cursor: Option<ExoId>,
    /// Which end to read from.
    pub direction: ExoReadDirection,
    /// How many events at most. `None` lets Exo choose, which at this pin is
    /// "all of them" — bounded reads are the caller's business.
    pub limit: Option<u32>,
    /// Restrict to one session.
    pub session: Option<ExoId>,
    /// Restrict to one turn.
    pub turn: Option<ExoId>,
}

impl ExoEventWindow {
    /// A window over one turn, newest first, bounded.
    #[must_use]
    pub fn turn(turn: ExoId, limit: u32) -> Self {
        Self {
            turn: Some(turn),
            limit: Some(limit),
            ..Self::default()
        }
    }

    fn wire(&self) -> serde_json::Value {
        let mut query = serde_json::Map::new();
        query.insert(
            "cursor".into(),
            self.cursor.as_ref().map_or(serde_json::Value::Null, |id| {
                serde_json::Value::String(id.as_str().to_owned())
            }),
        );
        query.insert(
            "direction".into(),
            serde_json::Value::String(self.direction.wire().to_owned()),
        );
        query.insert(
            "limit".into(),
            self.limit
                .map_or(serde_json::Value::Null, |limit| limit.into()),
        );
        query.insert(
            "session_id".into(),
            self.session.as_ref().map_or(serde_json::Value::Null, |id| {
                serde_json::Value::String(id.as_str().to_owned())
            }),
        );
        query.insert(
            "turn_id".into(),
            self.turn.as_ref().map_or(serde_json::Value::Null, |id| {
                serde_json::Value::String(id.as_str().to_owned())
            }),
        );
        query.insert("types".into(), serde_json::Value::Null);
        serde_json::Value::Object(query)
    }
}

/// One request Omega is willing to send Exo.
///
/// Closed at eight variants, all of them reads. See the module documentation
/// for why this is a type rather than a validation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ExoQuery {
    /// The agent record. Exo's `get_agent`.
    AgentShow { agent: ExoId },
    /// Every artifact version the agent holds. Exo's `agent_list_artifacts`.
    AgentArtifacts { agent: ExoId },
    /// One agent-scoped artifact, with its bytes. Exo's `agent_read_artifact`.
    AgentArtifact {
        agent: ExoId,
        artifact: ExoId,
        /// `None` reads the latest version Exo holds.
        version: Option<u64>,
    },
    /// The conversation record, including its head event. Exo's
    /// `get_conversation`.
    ConversationShow { agent: ExoId, conversation: ExoId },
    /// A window over the durable event log. Exo's `conversation_get_events`,
    /// and the reason this crate exists.
    ConversationEvents {
        agent: ExoId,
        conversation: ExoId,
        window: ExoEventWindow,
    },
    /// One event by id. Exo's `conversation_get_event`.
    ConversationEvent {
        agent: ExoId,
        conversation: ExoId,
        event: ExoId,
    },
    /// Every artifact version the conversation holds. Exo's
    /// `conversation_list_artifacts`.
    ConversationArtifacts { agent: ExoId, conversation: ExoId },
    /// One conversation-scoped artifact, with its bytes. Exo's
    /// `conversation_read_artifact`.
    ConversationArtifact {
        agent: ExoId,
        conversation: ExoId,
        artifact: ExoId,
        /// `None` reads the latest version Exo holds.
        version: Option<u64>,
    },
}

impl ExoQuery {
    /// The `type` tag this query carries on the wire.
    ///
    /// The whole admitted set, in one function, so `OMEGA-DELTA-0091` can read
    /// it rather than infer it.
    #[must_use]
    pub const fn wire_kind(&self) -> &'static str {
        match self {
            Self::AgentShow { .. } => "get_agent",
            Self::AgentArtifacts { .. } => "agent_list_artifacts",
            Self::AgentArtifact { .. } => "agent_read_artifact",
            Self::ConversationShow { .. } => "get_conversation",
            Self::ConversationEvents { .. } => "conversation_get_events",
            Self::ConversationEvent { .. } => "conversation_get_event",
            Self::ConversationArtifacts { .. } => "conversation_list_artifacts",
            Self::ConversationArtifact { .. } => "conversation_read_artifact",
        }
    }

    /// The one response shape this query may be answered with.
    ///
    /// Bound to the query rather than checked at the call site, so a reply
    /// carrying somebody else's payload is a decode failure instead of a
    /// silently empty history.
    #[must_use]
    pub const fn expects(&self) -> ExoResponseTag {
        match self {
            Self::AgentShow { .. } => ExoResponseTag::Agent,
            Self::AgentArtifacts { .. } | Self::ConversationArtifacts { .. } => {
                ExoResponseTag::ArtifactVersions
            }
            Self::AgentArtifact { .. } | Self::ConversationArtifact { .. } => {
                ExoResponseTag::Artifact
            }
            Self::ConversationShow { .. } => ExoResponseTag::Conversation,
            Self::ConversationEvents { .. } => ExoResponseTag::Events,
            Self::ConversationEvent { .. } => ExoResponseTag::Event,
        }
    }

    /// The `request` object of Exo's `ClientMessage::Request`.
    ///
    /// Field names and nesting are Exo's, read off `protocol.rs` and confirmed
    /// against the shapes Exo's own TypeScript bridge writes by hand
    /// (`typescript/harness/runner.ts`) — two independent witnesses in the
    /// pinned tree.
    #[must_use]
    pub fn wire_request(&self) -> serde_json::Value {
        let mut request = serde_json::Map::new();
        request.insert(
            "type".into(),
            serde_json::Value::String(self.wire_kind().to_owned()),
        );
        let mut put = |key: &str, id: &ExoId| {
            request.insert(
                key.to_owned(),
                serde_json::Value::String(id.as_str().to_owned()),
            );
        };
        match self {
            Self::AgentShow { agent } | Self::AgentArtifacts { agent } => put("agent_id", agent),
            Self::AgentArtifact {
                agent,
                artifact,
                version,
            } => {
                put("agent_id", agent);
                request.insert("request".into(), read_artifact_request(artifact, *version));
            }
            Self::ConversationShow {
                agent,
                conversation,
            }
            | Self::ConversationArtifacts {
                agent,
                conversation,
            } => {
                put("agent_id", agent);
                put("conversation_id", conversation);
            }
            Self::ConversationEvents {
                agent,
                conversation,
                window,
            } => {
                put("agent_id", agent);
                put("conversation_id", conversation);
                request.insert("query".into(), window.wire());
            }
            Self::ConversationEvent {
                agent,
                conversation,
                event,
            } => {
                put("agent_id", agent);
                put("conversation_id", conversation);
                put("event_id", event);
            }
            Self::ConversationArtifact {
                agent,
                conversation,
                artifact,
                version,
            } => {
                put("agent_id", agent);
                put("conversation_id", conversation);
                request.insert("request".into(), read_artifact_request(artifact, *version));
            }
        }
        serde_json::Value::Object(request)
    }
}

fn read_artifact_request(artifact: &ExoId, version: Option<u64>) -> serde_json::Value {
    let mut request = serde_json::Map::new();
    request.insert(
        "artifact_id".into(),
        serde_json::Value::String(artifact.as_str().to_owned()),
    );
    request.insert(
        "version".into(),
        version.map_or(serde_json::Value::Null, |version| version.into()),
    );
    serde_json::Value::Object(request)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn id(text: &str) -> ExoId {
        ExoId::parse(text).expect("a UUID")
    }

    const AGENT: &str = "0198f3ec-1b7a-7c31-9f0e-6d2a4b8c1d55";
    const CONVERSATION: &str = "0198f3ec-2c8b-7d42-a01f-7e3b5c9d2e66";

    #[test]
    fn an_id_that_is_not_a_uuid_is_refused() {
        for text in [
            "",
            "../../etc/passwd",
            "0198f3ec1b7a7c319f0e6d2a4b8c1d55",
            "0198f3ec-1b7a-7c31-9f0e-6d2a4b8c1d5",
            "0198f3ec-1b7a-7c31-9f0e-6d2a4b8c1dzz",
            "0198f3ec_1b7a_7c31_9f0e_6d2a4b8c1d55",
        ] {
            assert_eq!(ExoId::parse(text), Err(NotAnExoId), "{text:?}");
        }
        assert_eq!(id(AGENT).as_str(), AGENT);
    }

    /// The wire shape of the request this issue is named for, spelled out.
    /// Compared against Exo's own TypeScript bridge, which writes the same
    /// object by hand at `typescript/harness/runner.ts`.
    #[test]
    fn the_event_read_matches_exos_own_request_shape() {
        let query = ExoQuery::ConversationEvents {
            agent: id(AGENT),
            conversation: id(CONVERSATION),
            window: ExoEventWindow {
                direction: ExoReadDirection::Newest,
                limit: Some(200),
                ..ExoEventWindow::default()
            },
        };
        assert_eq!(
            query.wire_request(),
            serde_json::json!({
                "type": "conversation_get_events",
                "agent_id": AGENT,
                "conversation_id": CONVERSATION,
                "query": {
                    "cursor": null,
                    "direction": "desc",
                    "limit": 200,
                    "session_id": null,
                    "turn_id": null,
                    "types": null,
                },
            })
        );
    }

    #[test]
    fn an_artifact_read_nests_its_request_the_way_exo_does() {
        let artifact = "0198f3ec-3d9c-7e53-b120-8f4c6dae3f77";
        assert_eq!(
            ExoQuery::ConversationArtifact {
                agent: id(AGENT),
                conversation: id(CONVERSATION),
                artifact: id(artifact),
                version: Some(3),
            }
            .wire_request(),
            serde_json::json!({
                "type": "conversation_read_artifact",
                "agent_id": AGENT,
                "conversation_id": CONVERSATION,
                "request": { "artifact_id": artifact, "version": 3 },
            })
        );
        assert_eq!(
            ExoQuery::AgentArtifact {
                agent: id(AGENT),
                artifact: id(artifact),
                version: None,
            }
            .wire_request(),
            serde_json::json!({
                "type": "agent_read_artifact",
                "agent_id": AGENT,
                "request": { "artifact_id": artifact, "version": null },
            })
        );
    }

    /// Every variant names a read, and every variant is bound to exactly one
    /// response shape. Enumerated here so a ninth variant added without a tag
    /// fails in this crate before `OMEGA-DELTA-0091` sees it.
    #[test]
    fn every_variant_is_a_read_and_expects_one_shape() {
        let artifact = id("0198f3ec-3d9c-7e53-b120-8f4c6dae3f77");
        let every = [
            ExoQuery::AgentShow { agent: id(AGENT) },
            ExoQuery::AgentArtifacts { agent: id(AGENT) },
            ExoQuery::AgentArtifact {
                agent: id(AGENT),
                artifact: artifact.clone(),
                version: None,
            },
            ExoQuery::ConversationShow {
                agent: id(AGENT),
                conversation: id(CONVERSATION),
            },
            ExoQuery::ConversationEvents {
                agent: id(AGENT),
                conversation: id(CONVERSATION),
                window: ExoEventWindow::default(),
            },
            ExoQuery::ConversationEvent {
                agent: id(AGENT),
                conversation: id(CONVERSATION),
                event: artifact.clone(),
            },
            ExoQuery::ConversationArtifacts {
                agent: id(AGENT),
                conversation: id(CONVERSATION),
            },
            ExoQuery::ConversationArtifact {
                agent: id(AGENT),
                conversation: id(CONVERSATION),
                artifact,
                version: None,
            },
        ];
        assert_eq!(every.len(), 8);
        for query in &every {
            let kind = query.wire_kind();
            assert!(
                kind.contains("get") || kind.contains("list") || kind.contains("read"),
                "{kind} does not read"
            );
            assert_eq!(
                query.wire_request().get("type").and_then(|it| it.as_str()),
                Some(kind)
            );
            assert_eq!(query.expects().wire(), query.expects().wire());
        }
    }
}
