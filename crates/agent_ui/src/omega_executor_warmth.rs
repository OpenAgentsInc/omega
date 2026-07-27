//! Adapters kept started, so switching executor is not a download. omega#112.
//!
//! `OMEGA-DELTA-0117`. The owner asked for this while switching executors in a
//! running build, in as many words: *"that acp shit is ideally preloaded in the
//! background so user doesnt have to sit there waiting for that bullshit"*.
//!
//! `OMEGA-DELTA-0115` made the switch work by rebuilding the connection, which
//! means every switch pays a full adapter start: `npm exec --yes` resolving an
//! npx package, Node booting it, and the ACP `initialize` handshake. That is
//! the same wait `OMEGA-DELTA-0114` bounded and named — and naming a wait does
//! not shorten it. This module removes it for the executors that are already
//! offerable on this machine, by having their adapters already started when the
//! person picks one.
//!
//! # What is warm is a process, because the package already was
//!
//! Two things could be preloaded and only one of them is worth anything.
//!
//! The npx *package* is the obvious candidate, and it is the one the delta
//! prose points at: a cold resolve is tens of megabytes. It is also already
//! warm on any machine that has connected once — npm keeps the resolved package
//! under `<cache>/_npx`, and that cache is Omega's own directory and persists
//! across launches. Measured on the owner's machine, against his real caches
//! and the exact command `LocalRegistryNpxAgent` builds: a cold resolve of
//! `codex-acp` costs **4.6s**, and every resolve after it costs **0.55–0.67s**
//! (median 0.63s over five runs; `claude-acp` 0.55–0.60s, median 0.59s). So
//! warming the package buys a first launch several seconds and buys the owner's
//! actual complaint — the *second* switch, and the third — nothing at all.
//!
//! What is left in that 0.6s is npm's own startup, Node's boot, and the
//! handshake, and none of it can be cached: it is work, not bytes. The only way
//! to not pay it at switch time is to have paid it already. So what this warms
//! is the whole thing `CustomAgentServer::connect` returns — a started adapter
//! process with `initialize` complete — and the switch becomes a lookup.
//!
//! # What it costs while nobody is using it
//!
//! A warmed adapter is two live processes: the `npm exec` wrapper and the Node
//! child it spawns. Measured, idle, immediately after `initialize`:
//! **~100 MB RSS each, ~200 MB the pair**, per warmed executor.
//!
//! That is the real price and it is why [`warmable`] is as narrow as it is. On
//! a machine with both agents installed, one of them is already attached, so
//! the standing cost is one warm adapter — around 200 MB — for the one name in
//! the menu the person has not chosen. On a machine sitting on Omega's own loop
//! or an Exo lane, both are warm and it is around 400 MB. A person who never
//! opens the menu pays that for nothing, which is the honest statement of the
//! trade, and [`WARM_LIFETIME`] is what stops them paying it forever.
//!
//! # Exactly two names are ever warmed, and Exo is not one of them
//!
//! [`warmable`] is a subset of what the selector offers, never a superset:
//! warming something the menu does not list would be Omega starting a process
//! for an executor a person cannot even choose.
//!
//! Within that, only the two names that *are* an adapter:
//!
//! - **Omega** is compiled in. There is no process to start.
//! - **Exo** must never be started. `OMEGA-DELTA-0107` took route A: Omega
//!   reads an `exo serve` the owner already runs and starts none, because a
//!   second process pointed at one `.exo` root is the write interleaving that
//!   makes a fork a copy of a history that never existed. Warming is precisely
//!   the kind of well-meant convenience that would reach for a `serve`, so the
//!   Exo lane is filtered out here by name and a check holds it.
//! - **The one already attached** is not warmed, because it is already running.
//!
//! # It cannot be seen unless it worked
//!
//! A warm attempt is given no loading-status channel. That is not only so it
//! stays quiet — `AgentServerDelegate`'s channel has exactly one holder and
//! `CustomAgentServer::connect` installs it into a slot on the store that is
//! keyed by agent id, so a warm connect carrying one could take the channel a
//! person's own connect is ticking on and leave them looking at the silent
//! unbounded `Loading…` that `OMEGA-DELTA-0114` exists to have removed. Passing
//! `None` means `connect` never touches that slot.
//!
//! A warm attempt that fails records nothing. It does not call
//! `record_unreachable`, so the **Run on Omega's Own Loop** button is not
//! offered for a failure nobody saw; and it does not call
//! `run_on_omegas_own_loop`, which only a person may call at all. The entry is
//! simply not there, and the person who then picks that executor gets the
//! ordinary attach: the ticking label, [`ADAPTER_START_TIMEOUT`], and the same
//! failure they would have had if this module did not exist.
//!
//! [`ADAPTER_START_TIMEOUT`]: crate::omega_agent_attach::ADAPTER_START_TIMEOUT
//!
//! # A warm connection expires, and is checked before it is handed over
//!
//! Two ways a warm entry can be a lie, and both are refused rather than
//! detected late:
//!
//! - **It got old.** [`WARM_LIFETIME`] caps how long a connection may sit
//!   before it is thrown away rather than given to somebody. The adapter was
//!   started against a network, a registry, and an agent installation that were
//!   all true half an hour ago.
//! - **It died.** Nothing observes that. `AcpConnection`'s wait task fans a
//!   `LoadError::Exited` out to the connection's *sessions*, and a connection
//!   held in reserve has none, so its process can exit into complete silence.
//!   [`agent_servers::AcpConnection::agent_server_process_has_exited`] is asked
//!   on the way out, every time.
//!
//! A refused entry is not merely dropped: its process is ended by name, because
//! `Drop` runs only when the last `Rc` goes and a leaked reference would leave
//! 200 MB of Node running with nothing pointing at it.
//!
//! Handing over a stale handle is worse than being slow. A slow start is a
//! bounded wait with a label on it; a dead handle is a failure that arrives
//! after the person has typed their message, from a place the composer cannot
//! explain.

use std::rc::Rc;
use std::time::{Duration, Instant};

use acp_thread::AgentConnection;
use gpui::{AsyncApp, Entity, EntityId, Global, WeakEntity};
use omega_agent_detect::DetectedAgent;
use project::{Project, agent_server_store::AgentServerStore};

use crate::omega_executor_selector::SelectableExecutor;

/// How long after a connection is established before its neighbours are warmed.
///
/// A connect returning is not the window being finished with it: the session is
/// opened next, the transcript draws, and the person reads it. Starting two
/// Node processes into that competes with the thing they are actually looking
/// at, which would move the wait rather than remove it.
///
/// A settle delay, not a correctness device — nothing breaks if it is zero,
/// the first turn is just less smooth.
pub const WARM_START_DELAY: Duration = Duration::from_secs(2);

/// How long a warmed connection may sit before it is thrown away unused.
///
/// Ten minutes. The adapter was started against a network, an ACP registry, and
/// an agent installation that were all true when it started, and none of those
/// is guaranteed an hour later. Handing a person a handle from a machine state
/// that no longer exists produces a failure after they have typed, which is
/// strictly worse than the bounded, labelled start they would otherwise have
/// had.
///
/// It is also what stops an idle window holding ~200 MB of Node forever for a
/// menu entry nobody opened.
pub const WARM_LIFETIME: Duration = Duration::from_secs(600);

/// How long a failed warm attempt is remembered before another is made.
///
/// Warming is only ever triggered by a connection being established, and
/// connections are established because a person did something — so there is no
/// loop here to run away. This exists for the one case that is not
/// person-driven: `ConversationView` re-drives a failed connect whenever the
/// ACP registry reloads, and a flapping registry would otherwise start a fresh
/// npx resolve on every flap.
pub const WARM_RETRY_AFTER: Duration = Duration::from_secs(600);

/// What is known about one adapter's warmth, for one project.
enum Warmth {
    /// An attempt is running. Nothing may start a second one.
    Starting,
    /// A started adapter, waiting for somebody to choose it.
    Ready {
        connection: Rc<dyn AgentConnection>,
        warmed_at: Instant,
    },
    /// An attempt finished without a connection, at this time.
    Cold { since: Instant },
}

/// One adapter's warmth, for one project.
struct WarmEntry {
    adapter_id: &'static str,
    /// The project the adapter was started for.
    ///
    /// Part of the key, not decoration. `AcpConnection::stdio` sets the child's
    /// working directory from the project's first visible worktree and gives it
    /// that directory's captured environment, so a connection warmed for one
    /// project is a process sitting in another project's folder. Handing it
    /// over would be an agent quietly working in the wrong tree.
    project: EntityId,
    warmth: Warmth,
}

/// Every adapter this process has warmed, is warming, or has failed to warm.
///
/// A `Vec` rather than a map because it holds at most two entries per open
/// project, and because iterating it has an order that does not depend on a
/// hash seed.
#[derive(Default)]
struct WarmAdapters {
    entries: Vec<WarmEntry>,
}

impl Global for WarmAdapters {}

/// The executors worth warming, given what the selector offers and what is
/// already attached.
///
/// A pure function of the two facts, so the rule can be checked without a
/// machine that happens to have the right things installed — the same reason
/// [`crate::omega_executor_selector::ready`] takes what was detected rather
/// than going and looking.
///
/// `attached_adapter` is the agent id filling the router's one external-ACP
/// slot right now, or `None` when the native loop is running it.
#[must_use]
pub fn warmable(
    ready: &[SelectableExecutor],
    attached_adapter: Option<&str>,
) -> Vec<SelectableExecutor> {
    ready
        .iter()
        .copied()
        .filter(|choice| {
            // Omega is compiled in and Exo must never be started
            // (`OMEGA-DELTA-0107`), and both of those are exactly the choices
            // with no adapter of their own. Asking for the adapter id is
            // therefore the whole filter, rather than a list of names that
            // could fall out of step with what an adapter is.
            let Some(adapter) = choice.adapter_id() else {
                return false;
            };
            Some(adapter) != attached_adapter
        })
        .collect()
}

/// Start the adapters for everything [`warmable`] names, in the background.
///
/// Called once a connection has been established, which is the earliest moment
/// this is allowed to happen: a connection exists because a panel asked for
/// one, so the window is up and drawing, and the connect the person was
/// actually waiting on has already returned. Warming before that would move
/// their wait rather than remove it.
///
/// Does nothing an observer could see. Every attempt is silent, bounded by the
/// same [`crate::omega_agent_attach::ADAPTER_START_TIMEOUT`] a person's own
/// attach is bounded by, and records nothing on failure.
pub fn warm_the_others(
    detected: &[DetectedAgent],
    attached_adapter: Option<&'static str>,
    project: Entity<Project>,
    agent_server_store: WeakEntity<AgentServerStore>,
    cx: &mut AsyncApp,
) {
    cx.update(sweep_expired);
    let ready = crate::omega_executor_selector::ready_here();
    for choice in warmable(&ready, attached_adapter) {
        let Some(adapter_id) = choice.adapter_id() else {
            continue;
        };
        let Some(agent) = detected
            .iter()
            .find(|agent| agent.id == adapter_id)
            .cloned()
        else {
            continue;
        };
        let project = project.clone();
        let agent_server_store = agent_server_store.clone();
        if !cx.update(|cx| claim(adapter_id, project.entity_id(), cx)) {
            continue;
        }
        cx.spawn(async move |cx| {
            cx.background_executor().timer(WARM_START_DELAY).await;
            let started = crate::omega_agent_attach::start_adapter_silently(
                &agent,
                project.clone(),
                agent_server_store,
                cx,
            )
            .await;
            cx.update(|cx| match started {
                Ok(connection) => {
                    log::info!(
                        "OMEGA-DELTA-0117: `{}` is warm, so choosing {} is a lookup",
                        agent.id,
                        agent.name
                    );
                    settle(
                        agent.id,
                        project.entity_id(),
                        Warmth::Ready {
                            connection,
                            warmed_at: Instant::now(),
                        },
                        cx,
                    );
                }
                // Not recorded anywhere a person can see, and deliberately not
                // through `record_unreachable`: nobody asked for this adapter,
                // so a failure here must not light up a callout about one.
                Err(error) => {
                    log::info!(
                        "OMEGA-DELTA-0117: `{}` did not warm, so choosing {} will \
                         start it the ordinary way: {error:#}",
                        agent.id,
                        agent.name
                    );
                    settle(
                        agent.id,
                        project.entity_id(),
                        Warmth::Cold {
                            since: Instant::now(),
                        },
                        cx,
                    );
                }
            });
        })
        .detach();
    }
}

/// Take the warm connection for `adapter_id`, if there is a live fresh one.
///
/// Removes what it returns: a connection has one holder, and handing the same
/// one to two callers would be two threads sharing an agent process without
/// either knowing.
///
/// `None` is the ordinary answer and means only *start this the usual way* —
/// never that anything is wrong.
#[must_use]
pub fn take_warm(
    adapter_id: &str,
    project: &Entity<Project>,
    cx: &mut AsyncApp,
) -> Option<Rc<dyn AgentConnection>> {
    let project = project.entity_id();
    cx.update(|cx| {
        let taken = {
            let warm = cx.default_global::<WarmAdapters>();
            let position = warm
                .entries
                .iter()
                .position(|entry| entry.adapter_id == adapter_id && entry.project == project)?;
            let ready = warm
                .entries
                .get(position)
                .is_some_and(|entry| matches!(entry.warmth, Warmth::Ready { .. }));
            // An attempt still running is left running rather than awaited or
            // cancelled. Awaiting it would put a person behind a start with no
            // channel and no tick; cancelling it drops a `Child` that nothing
            // else kills, which is 200 MB of Node with no owner.
            if !ready {
                return None;
            }
            warm.entries.remove(position)
        };
        let Warmth::Ready {
            connection,
            warmed_at,
        } = taken.warmth
        else {
            return None;
        };

        // Both refusals end the process rather than dropping the last `Rc` and
        // hoping: `Drop` runs when every owner is released, and a warm
        // connection's owners are whatever this function did not manage to
        // hand over.
        if warmed_at.elapsed() >= WARM_LIFETIME {
            log::info!(
                "OMEGA-DELTA-0117: the warm `{adapter_id}` is {}s old, past the \
                 {}s it may be trusted for, so it is ended and started fresh",
                warmed_at.elapsed().as_secs(),
                WARM_LIFETIME.as_secs()
            );
            end(&connection);
            return None;
        }
        if has_exited(&connection) {
            log::info!(
                "OMEGA-DELTA-0117: the warm `{adapter_id}` process is gone, so it \
                 is started fresh rather than handed over dead"
            );
            end(&connection);
            return None;
        }

        log::info!("OMEGA-DELTA-0117: attaching the already-started `{adapter_id}`");
        Some(connection)
    })
}

/// End and forget every warm connection that is past [`WARM_LIFETIME`].
///
/// [`take_warm`] refuses an expired entry, but only when somebody asks for that
/// adapter — and the entry most likely to expire is the one for the executor
/// nobody chose, which is exactly the one nobody will ask for. Without this, a
/// warm connection for a project whose window has since closed holds its ~200 MB
/// until the process exits.
///
/// Runs where warming is triggered, so a machine that has stopped connecting has
/// also stopped accumulating.
fn sweep_expired(cx: &mut gpui::App) {
    let warm = cx.default_global::<WarmAdapters>();
    let mut expired = Vec::new();
    warm.entries.retain(|entry| match &entry.warmth {
        Warmth::Ready {
            connection,
            warmed_at,
        } if warmed_at.elapsed() >= WARM_LIFETIME => {
            expired.push(connection.clone());
            false
        }
        _ => true,
    });
    for connection in expired {
        log::info!("OMEGA-DELTA-0117: ending a warm adapter nobody chose within its lifetime");
        end(&connection);
    }
}

/// Claim the right to warm `adapter_id` for `project`, if nothing else has.
///
/// `false` means an attempt is already running, a connection is already warm,
/// or a recent attempt failed and [`WARM_RETRY_AFTER`] has not passed.
fn claim(adapter_id: &'static str, project: EntityId, cx: &mut gpui::App) -> bool {
    let warm = cx.default_global::<WarmAdapters>();
    if let Some(existing) = warm
        .entries
        .iter()
        .position(|entry| entry.adapter_id == adapter_id && entry.project == project)
    {
        match warm.entries[existing].warmth {
            Warmth::Starting | Warmth::Ready { .. } => return false,
            Warmth::Cold { since } if since.elapsed() < WARM_RETRY_AFTER => return false,
            Warmth::Cold { .. } => {
                warm.entries.remove(existing);
            }
        }
    }
    warm.entries.push(WarmEntry {
        adapter_id,
        project,
        warmth: Warmth::Starting,
    });
    true
}

/// Replace a claimed entry with what the attempt produced.
fn settle(adapter_id: &'static str, project: EntityId, warmth: Warmth, cx: &mut gpui::App) {
    let warm = cx.default_global::<WarmAdapters>();
    match warm
        .entries
        .iter_mut()
        .find(|entry| entry.adapter_id == adapter_id && entry.project == project)
    {
        Some(entry) => entry.warmth = warmth,
        // The claim was taken over or the project went away while the adapter
        // was starting. Whatever started is nobody's, so it is ended here
        // rather than added back under a key nothing will look up.
        None => {
            if let Warmth::Ready { connection, .. } = &warmth {
                end(connection);
            }
        }
    }
}

/// Whether the process behind a connection is gone.
///
/// A connection that is not an `AcpConnection` cannot be asked, and is treated
/// as gone: everything this module warms is one, so anything else is a state
/// nobody intended and is not something to hand a person.
fn has_exited(connection: &Rc<dyn AgentConnection>) -> bool {
    connection
        .clone()
        .downcast::<agent_servers::AcpConnection>()
        .is_none_or(|acp| acp.agent_server_process_has_exited())
}

/// End the adapter process behind a connection nobody is going to be given.
fn end(connection: &Rc<dyn AgentConnection>) {
    if let Some(acp) = connection
        .clone()
        .downcast::<agent_servers::AcpConnection>()
    {
        acp.end_agent_server_process();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const EVERYTHING: &[SelectableExecutor] = &[
        SelectableExecutor::Omega,
        SelectableExecutor::Exo,
        SelectableExecutor::Codex,
        SelectableExecutor::Claude,
    ];

    /// `OMEGA-DELTA-0107`, held from the other side. Warming is exactly the
    /// convenience that would reach for an `exo serve`.
    #[test]
    fn the_exo_lane_is_never_warmed() {
        assert!(
            !warmable(EVERYTHING, None).contains(&SelectableExecutor::Exo),
            "Omega reads an `exo serve` the owner already runs and starts none. \
             A second process pointed at one `.exo` root is the write \
             interleaving that makes a fork a copy of a history that never \
             existed, and a preload is no less a second process for being a \
             convenience"
        );
    }

    /// Omega is compiled in. There is nothing to start, and starting something
    /// would mean this had stopped being about adapters.
    #[test]
    fn omegas_own_loop_is_never_warmed() {
        assert!(!warmable(EVERYTHING, None).contains(&SelectableExecutor::Omega));
    }

    /// The two that are an adapter, and only those.
    #[test]
    fn the_two_adapters_are_what_gets_warmed() {
        assert_eq!(
            warmable(EVERYTHING, None),
            vec![SelectableExecutor::Codex, SelectableExecutor::Claude],
        );
    }

    /// What is running is not warmed. It is already running.
    #[test]
    fn the_attached_executor_is_not_warmed_again() {
        assert_eq!(
            warmable(EVERYTHING, Some(agent_servers::CODEX_ID)),
            vec![SelectableExecutor::Claude],
        );
        assert_eq!(
            warmable(EVERYTHING, Some(agent_servers::CLAUDE_AGENT_ID)),
            vec![SelectableExecutor::Codex],
        );
    }

    /// An Exo lane fills the same single external slot, and warming both
    /// adapters underneath it is right: neither is running.
    #[test]
    fn an_exo_lane_leaves_both_adapters_worth_warming() {
        assert_eq!(
            warmable(EVERYTHING, Some(omega_exo_lane::EXO_HARNESS_ID)),
            vec![SelectableExecutor::Codex, SelectableExecutor::Claude],
        );
    }

    /// The falsifier for the whole module: warming may only ever be a subset of
    /// what the selector offers.
    ///
    /// A warm adapter the menu does not list is Omega starting a process for an
    /// executor nobody can choose — which is both a cost with no benefit and a
    /// process running for a reason no window explains.
    #[test]
    fn nothing_is_warmed_that_the_selector_does_not_offer() {
        for offered in [
            vec![SelectableExecutor::Omega],
            vec![SelectableExecutor::Omega, SelectableExecutor::Claude],
            vec![
                SelectableExecutor::Omega,
                SelectableExecutor::Exo,
                SelectableExecutor::Codex,
            ],
            EVERYTHING.to_vec(),
        ] {
            for attached in [None, Some(agent_servers::CODEX_ID)] {
                for warmed in warmable(&offered, attached) {
                    assert!(
                        offered.contains(&warmed),
                        "{warmed:?} was warmed and is not offered by {offered:?}"
                    );
                }
            }
        }
    }

    /// A machine with nothing installed warms nothing, and does not fail to.
    #[test]
    fn a_bare_machine_warms_nothing() {
        assert!(warmable(&[SelectableExecutor::Omega], None).is_empty());
    }
}
