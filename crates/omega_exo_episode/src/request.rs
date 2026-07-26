//! The four requests an episode sends, and the exact bytes they become.
//! `OMEGA-DELTA-0090`, omega#103.
//!
//! `exo serve` takes one shape on `POST /request`:
//!
//! ```text
//! {"kind":"request","id":<u64>,"request":{"type":"<request_type>", ...}}
//! ```
//!
//! and answers
//!
//! ```text
//! {"kind":"response","id":<u64>,"ok":<bool>,"response":{...}|null,"error":<string>|null}
//! ```
//!
//! [`EpisodeRequest`] is closed at four variants and [`EpisodeRequest::envelope`]
//! is total over it, for the same reason `omega_exo_lane::ExoCommand` is closed:
//! a fifth call does not compile until somebody writes its shape down, and
//! writing it down is the record that a person decided it belonged. Every one
//! of the four is in an admitted family ([`crate::family`]), and
//! `every_episode_request_is_in_an_admitted_family` is what holds that rather
//! than the reader's memory.
//!
//! # Every optional key is written, including the null ones
//!
//! Exo's request structs derive `Deserialize` without `#[serde(default)]` on
//! their `Option` fields. Whether a missing key is accepted is therefore a
//! property of a serde version rather than of Exo's declared contract, and it
//! is not a property this crate wants to depend on. So the encoders write every
//! field of `ForkConversationRequest`, `EventQuery`, and `StartSandboxRequest`,
//! `null` included. Longer bytes, no dependence on a default nobody wrote down.
//!
//! # The fork point is required here and optional upstream
//!
//! `ForkConversationRequest::up_to_inclusive` is an `Option`, and `None` means
//! "copy the whole history". [`EpisodeRequest::ForkAtEvent`] takes an
//! [`EventId`] and has no way to say `None`.
//!
//! That is deliberate, and it is the omega#103 falsifier stated as a type. Fork
//! *after* the mutation and the sibling carries the mutation, so the fork point
//! is the whole mechanism; a fork with no point is a fork at "now", which is
//! the same mistake by omission. An episode that could forget its fork point
//! would fail exactly the way the manual loop already failed: quietly, in the
//! direction of a green check.

use crate::family::{RequestFamily, family_of};
use crate::ids::{AgentId, ConversationId, EventId, SandboxId, SnapshotId};
use crate::reset::FilesystemReset;

/// A conversation Omega knows is a fork, because it read a fork response.
///
/// The reset half of an episode ([`EpisodeRequest::RestoreSandbox`]) takes one
/// of these rather than a bare [`ConversationId`]. There is one constructor and
/// it is [`ForkedConversation::read_fork_response`], so a restore cannot be
/// aimed at the conversation the episode forked *from*. Restoring a snapshot
/// into the parent would put the working state of a candidate into the history
/// the episode is measuring against, which is the falsification loop damaging
/// the thing it was protecting.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ForkedConversation {
    agent: AgentId,
    conversation: ConversationId,
}

/// Why a fork response could not be read.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ForkReadError {
    /// The server said `ok: false`, and this is what it said.
    Refused(String),
    /// The envelope did not answer the request this reader was given.
    WrongRequestId { expected: u64, found: Option<u64> },
    /// The response was not the `conversation` shape a fork answers with.
    NotAConversation,
    /// The response carried an id this build cannot read.
    UnreadableId,
}

impl std::fmt::Display for ForkReadError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Refused(error) => write!(formatter, "Exo refused the fork: {error}"),
            Self::WrongRequestId { expected, found } => write!(
                formatter,
                "that answer is for request {found:?}, not for request {expected}"
            ),
            Self::NotAConversation => {
                formatter.write_str("a fork answers with a conversation, and that is not one")
            }
            Self::UnreadableId => {
                formatter.write_str("the forked conversation carried an id Omega cannot read")
            }
        }
    }
}

impl std::error::Error for ForkReadError {}

impl ForkedConversation {
    /// Read `exo serve`'s answer to a `conversation_fork`.
    ///
    /// Exo answers a fork with `Response::Conversation { conversation }`, whose
    /// payload is a `ConversationHandleInfo` — an `agent_id` beside a
    /// `record`. Both halves are read, because a fork that reported the right
    /// conversation under the wrong agent would address every later request to
    /// somebody else's history.
    ///
    /// # Errors
    ///
    /// [`ForkReadError`] for a refusal, a mismatched request id, or a shape
    /// this build cannot read.
    pub fn read_fork_response(
        request_id: u64,
        envelope: &serde_json::Value,
    ) -> Result<Self, ForkReadError> {
        let answered = envelope.get("id").and_then(serde_json::Value::as_u64);
        if answered != Some(request_id) {
            return Err(ForkReadError::WrongRequestId {
                expected: request_id,
                found: answered,
            });
        }
        if envelope.get("ok").and_then(serde_json::Value::as_bool) != Some(true) {
            let error = envelope
                .get("error")
                .and_then(serde_json::Value::as_str)
                .unwrap_or("no reason given")
                .to_owned();
            return Err(ForkReadError::Refused(error));
        }
        let response = envelope
            .get("response")
            .ok_or(ForkReadError::NotAConversation)?;
        if response.get("type").and_then(serde_json::Value::as_str) != Some("conversation") {
            return Err(ForkReadError::NotAConversation);
        }
        let conversation = response
            .get("conversation")
            .filter(|value| !value.is_null())
            .ok_or(ForkReadError::NotAConversation)?;
        let agent = conversation
            .get("agent_id")
            .and_then(serde_json::Value::as_str)
            .ok_or(ForkReadError::NotAConversation)?;
        let id = conversation
            .get("record")
            .and_then(|record| record.get("id"))
            .and_then(serde_json::Value::as_str)
            .ok_or(ForkReadError::NotAConversation)?;
        Ok(Self {
            agent: AgentId::parse(agent).map_err(|_| ForkReadError::UnreadableId)?,
            conversation: ConversationId::parse(id).map_err(|_| ForkReadError::UnreadableId)?,
        })
    }

    /// The agent the fork belongs to.
    #[must_use]
    pub const fn agent(&self) -> &AgentId {
        &self.agent
    }

    /// The forked conversation's id.
    #[must_use]
    pub const fn conversation(&self) -> &ConversationId {
        &self.conversation
    }
}

/// A slug Omega proposes for a fork.
///
/// Exo derives one (`fork`, `fork-2`, …) when none is given, and refuses a
/// duplicate. A slug reaches Exo's storage as a directory-adjacent name, so the
/// admitted shape is narrow on purpose: lowercase letters, digits, and hyphens.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ForkSlug(String);

impl ForkSlug {
    /// Read a slug.
    ///
    /// # Errors
    ///
    /// The rejected string, when it is empty, over 64 characters, or carries
    /// anything but `a-z`, `0-9`, and `-`.
    pub fn parse(value: &str) -> Result<Self, &'static str> {
        let value = value.trim();
        if value.is_empty() {
            return Err("a fork slug cannot be empty");
        }
        if value.len() > 64 {
            return Err("a fork slug is at most 64 characters");
        }
        if !value.chars().all(|character| {
            character.is_ascii_lowercase() || character.is_ascii_digit() || character == '-'
        }) {
            return Err("a fork slug carries only lowercase letters, digits, and hyphens");
        }
        Ok(Self(value.to_owned()))
    }

    /// The slug, for the wire.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Every request an episode may send.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum EpisodeRequest {
    /// Fork a conversation at an exact event. The episode reset itself.
    ForkAtEvent {
        /// The agent that owns the conversation.
        agent: AgentId,
        /// The conversation to fork.
        conversation: ConversationId,
        /// The last event the fork carries. Required; see the module docs.
        up_to_inclusive: EventId,
        /// A slug for the fork, or Exo's derived one.
        slug: Option<ForkSlug>,
    },
    /// Read a conversation's durable events, oldest first.
    ReadEvents {
        /// The agent that owns the conversation.
        agent: AgentId,
        /// The conversation to read.
        conversation: ConversationId,
        /// A page size, or Exo's default.
        limit: Option<u32>,
        /// Where to resume from, for a conversation longer than one page.
        after: Option<EventId>,
    },
    /// Read a conversation's record.
    ShowConversation {
        /// The agent that owns the conversation.
        agent: AgentId,
        /// The conversation to read.
        conversation: ConversationId,
    },
    /// Restore a sandbox's filesystem from a snapshot, inside a fork.
    ///
    /// Only constructible with a [`FilesystemReset`] witness, which
    /// [`crate::reset::admit_filesystem_reset`] issues and which it refuses to
    /// issue for the shapes that do not work at this pin. See that module.
    RestoreSandbox {
        /// The fork the restore happens in.
        fork: ForkedConversation,
        /// The sandbox record to restore into.
        sandbox: SandboxId,
        /// The snapshot to restore from.
        snapshot: SnapshotId,
        /// Proof that this shape of reset is one that can work.
        admitted: FilesystemReset,
    },
}

impl EpisodeRequest {
    /// Exo's request type name for this request.
    #[must_use]
    pub const fn request_type(&self) -> &'static str {
        match self {
            Self::ForkAtEvent { .. } => "conversation_fork",
            Self::ReadEvents { .. } => "conversation_get_events",
            Self::ShowConversation { .. } => "get_conversation",
            Self::RestoreSandbox { .. } => "start_sandbox",
        }
    }

    /// The family this request belongs to.
    ///
    /// # Panics
    ///
    /// Never at runtime for a request this enum can build:
    /// `every_episode_request_is_in_an_admitted_family` fails the build first
    /// if a variant's type name is not in [`crate::family`]'s table.
    #[must_use]
    pub fn family(&self) -> RequestFamily {
        family_of(self.request_type()).expect("every episode request type is classified")
    }

    /// The full JSON body of `POST /request`, ready to send.
    #[must_use]
    pub fn envelope(&self, request_id: u64) -> serde_json::Value {
        serde_json::json!({
            "kind": "request",
            "id": request_id,
            "request": self.request_body(),
        })
    }

    fn request_body(&self) -> serde_json::Value {
        match self {
            Self::ForkAtEvent {
                agent,
                conversation,
                up_to_inclusive,
                slug,
            } => serde_json::json!({
                "type": "conversation_fork",
                "agent_id": agent.as_str(),
                "conversation_id": conversation.as_str(),
                "request": {
                    "up_to_inclusive": up_to_inclusive.as_str(),
                    "slug": slug.as_ref().map(ForkSlug::as_str),
                    // Exo derives a display name from the slug. Omega proposes
                    // none: a name is prose, and prose Omega wrote inside
                    // somebody else's record is prose a later reader would
                    // mistake for Exo's own.
                    "name": serde_json::Value::Null,
                },
            }),
            Self::ReadEvents {
                agent,
                conversation,
                limit,
                after,
            } => serde_json::json!({
                "type": "conversation_get_events",
                "agent_id": agent.as_str(),
                "conversation_id": conversation.as_str(),
                "query": {
                    "cursor": after.as_ref().map(EventId::as_str),
                    // Stated rather than inherited. Exo's default is ascending
                    // today; an episode comparison that silently flipped to
                    // descending would compare two reversed logs and find them
                    // equal, which is a green check that read nothing.
                    "direction": "asc",
                    "limit": limit,
                    "session_id": serde_json::Value::Null,
                    "turn_id": serde_json::Value::Null,
                    "types": serde_json::Value::Null,
                },
            }),
            Self::ShowConversation {
                agent,
                conversation,
            } => serde_json::json!({
                "type": "get_conversation",
                "agent_id": agent.as_str(),
                "conversation_id": conversation.as_str(),
            }),
            Self::RestoreSandbox {
                fork,
                sandbox,
                snapshot,
                admitted,
            } => serde_json::json!({
                "type": "start_sandbox",
                "scope": admitted.scope_json(fork),
                "request": {
                    "id": sandbox.as_str(),
                    "snapshot_id": snapshot.as_str(),
                    "idle_seconds": serde_json::Value::Null,
                    "provider": serde_json::Value::Null,
                },
            }),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::reset::{EpisodeShape, SandboxScopeKind, admit_filesystem_reset};

    const AGENT: &str = "019e5782-0000-7000-8000-000000000001";
    const CONVERSATION: &str = "019e5782-0000-7000-8000-000000000002";
    const FORK: &str = "019e5782-0000-7000-8000-000000000003";
    const EVENT: &str = "019e5782-0000-7000-8000-000000000004";
    const SNAPSHOT: &str = "019e5782-0000-7000-8000-000000000005";

    fn agent() -> AgentId {
        AgentId::parse(AGENT).expect("a v7 uuid")
    }

    fn conversation() -> ConversationId {
        ConversationId::parse(CONVERSATION).expect("a v7 uuid")
    }

    fn fork_response(request_id: u64) -> serde_json::Value {
        serde_json::json!({
            "kind": "response",
            "id": request_id,
            "ok": true,
            "response": {
                "type": "conversation",
                "conversation": {
                    "agent_id": AGENT,
                    "record": {
                        "id": FORK,
                        "slug": "fork",
                        "name": "Fork",
                        "latest_event_id": EVENT,
                    },
                },
            },
            "error": serde_json::Value::Null,
        })
    }

    #[test]
    fn every_episode_request_is_in_an_admitted_family() {
        let fork = ForkedConversation::read_fork_response(1, &fork_response(1)).expect("a fork");
        let admitted = admit_filesystem_reset(SandboxScopeKind::Agent, EpisodeShape::SingleEpisode)
            .expect("the one shape that can work");
        let requests = [
            EpisodeRequest::ForkAtEvent {
                agent: agent(),
                conversation: conversation(),
                up_to_inclusive: EventId::parse(EVENT).expect("a v7 uuid"),
                slug: None,
            },
            EpisodeRequest::ReadEvents {
                agent: agent(),
                conversation: conversation(),
                limit: None,
                after: None,
            },
            EpisodeRequest::ShowConversation {
                agent: agent(),
                conversation: conversation(),
            },
            EpisodeRequest::RestoreSandbox {
                fork,
                sandbox: SandboxId::parse("sandbox-1").expect("a sandbox id"),
                snapshot: SnapshotId::parse(SNAPSHOT).expect("a v7 uuid"),
                admitted,
            },
        ];
        assert_eq!(requests.len(), 4, "the closed set is four requests");
        for request in &requests {
            assert!(
                request.family().is_admitted(),
                "{} is in the {} family",
                request.request_type(),
                request.family()
            );
        }
        assert!(requests.iter().all(|request| !matches!(
            request.family(),
            RequestFamily::Write | RequestFamily::Secret
        )));
    }

    #[test]
    fn a_fork_request_names_its_event_and_writes_every_key() {
        let request = EpisodeRequest::ForkAtEvent {
            agent: agent(),
            conversation: conversation(),
            up_to_inclusive: EventId::parse(EVENT).expect("a v7 uuid"),
            slug: Some(ForkSlug::parse("episode-a").expect("a slug")),
        };
        let envelope = request.envelope(7);
        assert_eq!(envelope["kind"], "request");
        assert_eq!(envelope["id"], 7);
        let body = &envelope["request"];
        assert_eq!(body["type"], "conversation_fork");
        assert_eq!(body["agent_id"], AGENT);
        assert_eq!(body["conversation_id"], CONVERSATION);
        assert_eq!(body["request"]["up_to_inclusive"], EVENT);
        assert_eq!(body["request"]["slug"], "episode-a");
        assert!(
            body["request"].get("name").is_some(),
            "every key of ForkConversationRequest is written, null included"
        );
        assert!(body["request"]["name"].is_null());
    }

    #[test]
    fn reading_events_states_its_direction_rather_than_inheriting_one() {
        let request = EpisodeRequest::ReadEvents {
            agent: agent(),
            conversation: conversation(),
            limit: Some(500),
            after: None,
        };
        let body = request.envelope(1)["request"].clone();
        assert_eq!(body["type"], "conversation_get_events");
        assert_eq!(
            body["query"]["direction"], "asc",
            "two reversed logs compare equal, so the order is never left to a default"
        );
        assert_eq!(body["query"]["limit"], 500);
        for key in ["cursor", "session_id", "turn_id", "types"] {
            assert!(
                body["query"].get(key).is_some(),
                "EventQuery::{key} is written even when it is null"
            );
        }
    }

    #[test]
    fn a_restore_names_the_fork_and_never_the_parent() {
        let fork = ForkedConversation::read_fork_response(3, &fork_response(3)).expect("a fork");
        assert_eq!(fork.conversation().as_str(), FORK);
        assert_ne!(
            fork.conversation().as_str(),
            CONVERSATION,
            "the fork is not the conversation it came from"
        );
        let admitted = admit_filesystem_reset(SandboxScopeKind::Agent, EpisodeShape::SingleEpisode)
            .expect("the one shape that can work");
        let request = EpisodeRequest::RestoreSandbox {
            fork,
            sandbox: SandboxId::parse("sandbox-019e5782-2a46-7970-a5bf-62900a2233e8")
                .expect("a sandbox id"),
            snapshot: SnapshotId::parse(SNAPSHOT).expect("a v7 uuid"),
            admitted,
        };
        let body = request.envelope(9)["request"].clone();
        assert_eq!(body["type"], "start_sandbox");
        assert_eq!(body["scope"]["type"], "agent");
        assert_eq!(body["scope"]["agent_id"], AGENT);
        assert_eq!(
            body["request"]["id"],
            "sandbox-019e5782-2a46-7970-a5bf-62900a2233e8"
        );
        assert_eq!(body["request"]["snapshot_id"], SNAPSHOT);
        assert!(body["request"].get("idle_seconds").is_some());
        assert!(body["request"].get("provider").is_some());
    }

    #[test]
    fn a_refused_fork_is_not_read_as_a_fork() {
        let refusal = serde_json::json!({
            "kind": "response",
            "id": 4,
            "ok": false,
            "response": serde_json::Value::Null,
            "error": "conversation slug already exists for agent: fork",
        });
        assert_eq!(
            ForkedConversation::read_fork_response(4, &refusal),
            Err(ForkReadError::Refused(
                "conversation slug already exists for agent: fork".to_owned()
            ))
        );
    }

    #[test]
    fn an_answer_to_a_different_request_is_not_read_as_this_one() {
        assert_eq!(
            ForkedConversation::read_fork_response(5, &fork_response(6)),
            Err(ForkReadError::WrongRequestId {
                expected: 5,
                found: Some(6)
            }),
            "one connection carries many requests; an off-by-one here forks the wrong thing"
        );
    }

    #[test]
    fn a_response_that_is_not_a_conversation_is_refused() {
        for shape in [
            serde_json::json!({"kind":"response","id":1,"ok":true,"response":{"type":"unit"},"error":null}),
            serde_json::json!({"kind":"response","id":1,"ok":true,"response":{"type":"conversation","conversation":null},"error":null}),
            serde_json::json!({"kind":"response","id":1,"ok":true,"error":null}),
        ] {
            assert_eq!(
                ForkedConversation::read_fork_response(1, &shape),
                Err(ForkReadError::NotAConversation),
                "{shape} was read as a fork"
            );
        }
    }

    #[test]
    fn a_slug_that_could_be_a_path_is_refused() {
        assert!(ForkSlug::parse("episode-a").is_ok());
        assert!(ForkSlug::parse("").is_err());
        assert!(ForkSlug::parse("../escape").is_err());
        assert!(ForkSlug::parse("Episode A").is_err());
        assert!(ForkSlug::parse(&"a".repeat(65)).is_err());
    }
}
