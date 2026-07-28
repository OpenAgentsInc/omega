//! Episode reset over Exo: fork the log, compare the state, falsify a check.
//! `OMEGA-DELTA-0090`, omega#103.
//!
//! Delta discipline says: watch a check fail before you trust it. On
//! 2026-07-26 that was done by hand about fifteen times across four lanes, and
//! the manual loop cost real damage. One lane reverted its mutation with `git
//! checkout --` and wiped uncommitted work in two files. One check "passed"
//! while testing nothing, because the mutated string also appeared in a second
//! code path. One suite flaked and there was no way to re-run it from an
//! identical start, so the answer was to run it again and hope.
//!
//! Every one of those is an episode-reset problem, and Exo already has the
//! primitive: `conversation_fork` replays a conversation's whole event log at a
//! chosen event into a new conversation, so a mutation lands in a copy and the
//! thing it was copied from is never touched. **The working tree is never
//! mutated, so a failed revert cannot destroy anything** — and this crate
//! cannot revert anything even by mistake, because it has no filesystem and no
//! process.
//!
//! # What this crate is
//!
//! A leaf law, in the shape `omega_exo_lane` established: no GPUI, no process,
//! no filesystem, no clock, no network. It decides
//!
//! * which of Exo's 52 requests an episode may send ([`family`]),
//! * what those requests look like on the wire ([`request`]),
//! * that one `.exo` root has one writer ([`root`]),
//! * whether two episodes are the same state ([`state`]),
//! * and which shapes of filesystem reset actually work ([`reset`]).
//!
//! The half that opens a socket and sends the bytes is not here, for the same
//! reason the router's dispatch half is not in `omega_front_door`: a law that
//! needs a running Exo to check is a law nobody checks.
//!
//! # It has now been run against a real Exo
//!
//! `OMEGA-DELTA-0120`. `examples/live_episode.rs` walks [`FALSIFICATION_LOOP`]
//! against a running `exo serve`, and `script/exo-episode-live` watches the
//! root and the process table while it does. Two forks of one event were taken
//! and compared identical by value while their raw reads differed in every
//! event id; a named check that passed on the control failed on the candidate;
//! the source conversation's digest and `latest_event_id` were unchanged
//! afterwards; every file that appeared under the root was inside a fork the
//! run took, nothing vanished, and `exo serve` gained no child process.
//!
//! The live server also contradicted this crate once, which is the whole point
//! of running it: a page's `cursor` is the id of its last event and not a
//! promise of another page, so the reader had been refusing every complete read
//! of every non-empty conversation. See [`PageBound`].
//!
//! # The honest state of the mechanism
//!
//! omega#103 and `docs/teardowns/2026-07-25-exoharness-exo-teardown.md` §11.5
//! both say fork plus `start_sandbox { snapshot_id }` is a complete episode
//! reset needing no upstream change. Reading the pinned source, that is half
//! right, and [`reset`] is where the other half is written down:
//!
//! * **The conversation reset is real and complete today.** Fork replays every
//!   event, copies bindings, secrets, artifacts and sandbox records, and never
//!   touches the source.
//! * **The filesystem reset does not compose with fork at this pin.** Fork
//!   copies four prefixes and `snapshots` is not one of them, so a
//!   conversation-scoped snapshot taken before the fork does not exist inside
//!   it. An agent-scoped one does exist inside it, and is one sandbox shared by
//!   every conversation of the agent, so two siblings restoring it are not two
//!   episodes.
//!
//! That is a finding about somebody else's code, so it is stated with its
//! evidence rather than asserted: see [`reset`] for the file, the four copied
//! prefixes, Exo's own error text, and Exo's own documentation saying snapshots
//! are "not a conversation rewind".
//!
//! # Boundaries
//!
//! Loopback only — [`EpisodeSession`] cannot be built without an
//! `omega_exo_lane::LoopbackEndpoint`, and that type has one constructor and it
//! refuses anything but this machine. Query and fork families only, never write
//! or secret ([`family`]). One writer per root ([`root`]). And
//! self-modification stays out entirely: `guardian_action`, agent-authored
//! tools, and the read-write source mount are gated owner decisions that
//! `omega_exo_lane::capability` already refuses, and nothing here relaxes,
//! re-implements, or routes around that gate.

pub mod family;
pub mod ids;
pub mod request;
pub mod reset;
pub mod root;
pub mod state;

pub use family::{
    RequestFamily, exo_request_families, family_of, family_of_wire, is_admitted, is_admitted_wire,
};
pub use ids::{AgentId, ConversationId, EventId, ExoIdError, SandboxId, SnapshotId};
pub use request::{EpisodeRequest, ForkReadError, ForkSlug, ForkedConversation};
pub use reset::{
    CheckOutcome, EpisodeShape, FALSIFICATION_LOOP, FilesystemReset, ProbeOutcome, ResetRefusal,
    SandboxScopeKind, SnapshotEvidence, SnapshotReach, Step, Verdict, admit_filesystem_reset,
    snapshot_reach, verdict,
};
pub use root::{ExoRoots, RootClaim, RootRefusal};
pub use state::{Divergence, EpisodeState, IDENTITY_FIELDS, PageBound, StateReadError};

use omega_exo_lane::LoopbackEndpoint;

/// The Exo revision every shape and every claim in this crate was read from.
///
/// The commit `omega_exo_lane::EXO_PIN` names — the maintained
/// `OpenAgentsInc/exo` fork, which is the Exo Omega actually drives. Exo's own
/// house rule is that it does not keep backwards compatibility, so a wire shape
/// here is only true of a revision, and the revision is a value rather than a
/// sentence in a commit message.
pub const EXO_PROTOCOL_PIN: &str = "cd7c0d29db869e953fb7261d8390ca93007d36a6";

/// The upstream revision the teardown read, which carries the same protocol.
///
/// `docs/teardowns/2026-07-25-exoharness-exo-teardown.md` was written against
/// `exoharness/exo` at this commit, and the lane drives the fork at
/// [`EXO_PROTOCOL_PIN`]. The five files every shape and finding in this crate
/// comes from — `crates/exoharness/src/protocol.rs`, `basic.rs`, `types.rs`,
/// `http/server.rs`, and `docs/sandbox-snapshots.md` — are byte-identical
/// between the two, so the teardown's protocol reading and this crate's apply
/// to both. The fork's 1,063 changed lines are in the executor, the ACP
/// transport, and secrets; none of them are in the substrate this crate speaks
/// to. That is recorded here because "the fork is the same" is exactly the kind
/// of claim that stops being true without anybody noticing.
pub const EXO_PROTOCOL_UPSTREAM_PIN: &str = "baa07f6785547080d99bd2a7d3eab6d76b984e35";

/// The path `exo serve` answers requests on.
pub const EXO_SERVE_REQUEST_PATH: &str = "/request";

/// The path `exo serve` answers liveness on.
pub const EXO_SERVE_HEALTH_PATH: &str = "/health";

/// One episode client: one Exo, on this machine, holding one root.
///
/// Both halves are types that refuse rather than checks a caller performs. The
/// endpoint cannot be off loopback because [`LoopbackEndpoint`] has no other
/// constructor, and the root cannot be a second writer because
/// [`root::ExoRoots`] issued the claim.
#[derive(Debug)]
pub struct EpisodeSession {
    endpoint: LoopbackEndpoint,
    root: RootClaim,
    next_request_id: u64,
}

impl EpisodeSession {
    /// Open a session against an Exo on this machine.
    #[must_use]
    pub const fn open(endpoint: LoopbackEndpoint, root: RootClaim) -> Self {
        Self {
            endpoint,
            root,
            next_request_id: 1,
        }
    }

    /// Where this session talks to Exo.
    #[must_use]
    pub const fn endpoint(&self) -> &LoopbackEndpoint {
        &self.endpoint
    }

    /// The root this session holds.
    #[must_use]
    pub const fn root(&self) -> &RootClaim {
        &self.root
    }

    /// The full URL of the request endpoint.
    #[must_use]
    pub fn request_url(&self) -> String {
        format!("http://{}{EXO_SERVE_REQUEST_PATH}", self.endpoint)
    }

    /// Prepare a request: the next id, and the body to post.
    ///
    /// Ids are handed out in order and never reused within a session, because
    /// every reader in this crate refuses an answer whose id is not the one it
    /// asked about. Reusing an id would make those refusals unable to fire.
    pub fn prepare(&mut self, request: &EpisodeRequest) -> (u64, serde_json::Value) {
        let id = self.next_request_id;
        self.next_request_id = self.next_request_id.saturating_add(1);
        (id, request.envelope(id))
    }

    /// Give the root back.
    pub fn close(self, roots: &mut ExoRoots) {
        roots.release(self.root);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_session_only_reaches_this_machine() {
        let mut roots = ExoRoots::new();
        let claim = roots.claim("/Users/someone/.exo").expect("a free root");
        let endpoint = LoopbackEndpoint::parse("127.0.0.1:4766").expect("loopback");
        let session = EpisodeSession::open(endpoint, claim);
        assert_eq!(session.request_url(), "http://127.0.0.1:4766/request");
        assert_eq!(session.root().as_str(), "/Users/someone/.exo");

        // The refusal that matters: an unauthenticated Exo with a shell,
        // published on every interface of a laptop that joins a Tailnet.
        assert!(LoopbackEndpoint::parse("0.0.0.0:4766").is_err());
        assert!(LoopbackEndpoint::parse("100.64.0.1:4766").is_err());
    }

    #[test]
    fn request_ids_are_handed_out_in_order_and_not_reused() {
        let mut roots = ExoRoots::new();
        let claim = roots.claim("/Users/someone/.exo").expect("a free root");
        let mut session = EpisodeSession::open(
            LoopbackEndpoint::parse("localhost:4766").expect("loopback"),
            claim,
        );
        let request = EpisodeRequest::ShowConversation {
            agent: AgentId::parse("019e5782-0000-7000-8000-000000000001").expect("a v7 uuid"),
            conversation: ConversationId::parse("019e5782-0000-7000-8000-000000000002")
                .expect("a v7 uuid"),
        };
        let (first, first_body) = session.prepare(&request);
        let (second, second_body) = session.prepare(&request);
        assert_eq!((first, second), (1, 2));
        assert_eq!(first_body["id"], 1);
        assert_eq!(second_body["id"], 2);
        assert_ne!(first_body, second_body, "the id is part of the body");
    }

    #[test]
    fn closing_a_session_frees_the_root_for_the_next_one() {
        let mut roots = ExoRoots::new();
        let claim = roots.claim("/Users/someone/.exo").expect("a free root");
        let session = EpisodeSession::open(
            LoopbackEndpoint::parse("127.0.0.1").expect("loopback"),
            claim,
        );
        assert!(roots.holds("/Users/someone/.exo"));
        assert!(
            roots.claim("/Users/someone/.exo").is_err(),
            "a second session on a live root is the interleaving that makes a fork \
             a copy of a history that never existed"
        );
        session.close(&mut roots);
        assert!(!roots.holds("/Users/someone/.exo"));
        roots.claim("/Users/someone/.exo").expect("free again");
    }

    #[test]
    fn the_pin_is_the_one_the_lane_admits() {
        assert_eq!(
            EXO_PROTOCOL_PIN,
            omega_exo_lane::EXO_PIN.source_commit,
            "the wire shapes in this crate were read at one revision, and the lane \
             drives another"
        );
        assert_ne!(
            EXO_PROTOCOL_PIN, EXO_PROTOCOL_UPSTREAM_PIN,
            "these are two different commits, and the point of recording both is that \
             they are"
        );
    }
}
