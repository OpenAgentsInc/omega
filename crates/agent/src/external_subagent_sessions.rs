//! Where an external ACP subagent's live session can be found from outside the
//! tool that spawned it.
//!
//! omega#109. `spawn_agent` with an `executor` opens a real session against a
//! real agent server, and the parent's panel could not show it. The reason is
//! narrow and worth stating exactly, because it is not "the panel forgot to
//! render something": the subagent card resolves its thread out of the **native
//! connection's** session map, and an external subagent is deliberately not in
//! that map. `ExternalAcpSubagentHandle` holds only an `AcpThread`, there is no
//! native `Thread` behind it, and `NativeAgent::sessions` never learns of it —
//! the session belongs to the external agent server, which runs its own loop
//! with its own login and its own tools. That is the whole point of the
//! executor, and it is also why the panel had nowhere to look.
//!
//! So this is the other place to look, and it holds exactly one fact: **which
//! `AcpThread` an external subagent's session id names**. It is not a second
//! session map. It does not decide anything, it does not own the session, and
//! it is not consulted by anything that runs a subagent — the handle already
//! holds what it needs. It exists so that a reader who has only a session id,
//! which is all `AcpThreadEvent::SubagentSpawned` carries, can find the thread.
//!
//! # Why the entities are weak
//!
//! The subagent's lifetime belongs to the handle, which drops the connection
//! and therefore the child process when the tool call ends. A strong reference
//! here would quietly extend that: a spawn map that never forgets would keep
//! every external session's `AcpThread` — and everything it retains — alive for
//! the life of the process. A `WeakEntity` records where the thread is without
//! having an opinion about how long it lives, and a session whose thread is
//! gone reads as absent, which is the truth.
//!
//! # Why this is a global and not a field
//!
//! The two ends are in different crates and neither owns the other: `crates/agent`
//! opens the session inside a tool call, and `crates/agent_ui` needs it while
//! rendering a card in a view that has no path to that tool call. Threading a
//! handle between them would mean widening `SubagentHandle`, `ThreadEnvironment`
//! and the tool's result to carry a UI concern through three layers that do not
//! otherwise have one. The registry is per-`App`, so a test's registry is its
//! own and there is no process-wide state to reset between them.

use agent_client_protocol::schema::v1 as acp;
use collections::HashMap;
use gpui::{App, Entity, Global, WeakEntity};

use acp_thread::AcpThread;

/// Every external ACP subagent session opened in this `App`, while its thread
/// is still alive.
#[derive(Default)]
pub struct ExternalSubagentSessions {
    threads: HashMap<acp::SessionId, WeakEntity<AcpThread>>,
}

impl Global for ExternalSubagentSessions {}

impl ExternalSubagentSessions {
    /// Every session id whose thread is still alive, for tests and diagnostics.
    ///
    /// Ordered by nothing in particular — this is a map, and a caller that
    /// wants an order must impose one. Dead entries are skipped rather than
    /// reported as present, for the same reason [`lookup`] returns `None` for
    /// them: a session whose thread is gone cannot be shown.
    ///
    /// [`lookup`]: external_subagent_thread
    #[must_use]
    pub fn live_session_ids(&self) -> Vec<acp::SessionId> {
        self.threads
            .iter()
            .filter(|(_, thread)| thread.upgrade().is_some())
            .map(|(session_id, _)| session_id.clone())
            .collect()
    }
}

/// Record that `session_id` names `thread`, an external ACP subagent's session.
///
/// Called once, where the session is opened. Registering here rather than in
/// `ExternalAcpSubagentHandle::new` is deliberate: the handle is also
/// constructed directly by the live tests, which drive a real agent server
/// without a panel, and a registry that filled up from those would be recording
/// sessions nothing is looking for.
///
/// Dead entries are swept on every insert. There is no other sweep and there
/// does not need to be one: the map only grows when a subagent is spawned, so
/// the cost of a stale entry is bounded by the next spawn.
pub fn register_external_subagent_session(
    session_id: acp::SessionId,
    thread: &Entity<AcpThread>,
    cx: &mut App,
) {
    let sessions = cx.default_global::<ExternalSubagentSessions>();
    sessions
        .threads
        .retain(|_, thread| thread.upgrade().is_some());
    sessions.threads.insert(session_id, thread.downgrade());
}

/// The `AcpThread` an external ACP subagent's session id names, if it is still
/// running.
///
/// `None` means one of two things and deliberately does not distinguish them:
/// the session was never an external subagent's, or its thread has been dropped.
/// Both answer the only question a caller has — "is there a thread to show" —
/// with no, and a caller that acted differently on the two would be acting on
/// the difference between "not ours" and "over", which is not a difference the
/// panel can render.
#[must_use]
pub fn external_subagent_thread(
    session_id: &acp::SessionId,
    cx: &App,
) -> Option<Entity<AcpThread>> {
    cx.try_global::<ExternalSubagentSessions>()?
        .threads
        .get(session_id)?
        .upgrade()
}
