//! Who executed a thread. OMEGA-DELTA-0021.
//!
//! Omega presents one chat surface over three executor classes: its own
//! in-process agent loop, out-of-process ACP agents such as `codex-acp`, and
//! `omega-effectd` engine lanes. Without disclosure all three look identical to
//! the reader — output appears in an Omega window and reads as Omega's own
//! work. That is a false attribution claim, made silently.
//!
//! # The record lives in `omega_front_door`, and this is only the binding
//!
//! [`omega_front_door::ExecutorDisclosure`] is the typed record. That is
//! omega#74's binding condition on the owner's identity decision: the agent
//! does not sign with its own principal, *on the condition that disclosure is
//! stored as a typed record that a label renders, never as a label string.*
//! A record of parts can be handed to a signer later; a rendered line cannot,
//! so storing the line would silently convert a reversible owner decision into
//! an irreversible one.
//!
//! This module holds the part that record deliberately cannot: reading a live
//! thread. `omega_front_door` is a leaf that depends on nothing, so it cannot
//! see GPUI, `acp_thread`, or a connection. The extension trait below is that
//! bridge, and it lives here rather than in `crates/acp_thread` so the upstream
//! thread type carries no Omega state and a rebase cannot quietly drop it.
//!
//! # Why nothing new is persisted
//!
//! omega#77's falsifier names a *new* GPUI-owned durable store as a failure,
//! and its deliverable says the disclosure persists through existing thread
//! persistence. Every part of the record already has a durable home, so the
//! record is a projection over them rather than a fourth copy that can drift:
//!
//! | Part | Durable home |
//! | --- | --- |
//! | class, agent id | the connection, rebuilt from `sidebar_threads.agent_id` |
//! | provider, model | `DbThread.model`, restored by `Thread::from_db` |
//! | run ref | `full-auto-host-correlation.json`, reloaded at startup |
//!
//! A projection cannot disagree with the thread it describes. A cached copy
//! can, and a disclosure that disagrees with its executor is worse than none.

use std::rc::Rc;

use acp_thread::{AcpThread, AgentConnection};
use agent::NativeAgentConnection;
use agent_client_protocol::schema::v1 as acp;
use agent_servers::AcpConnection;
use gpui::App;
use omega_front_door::{ExecutorClass, ExecutorDisclosure};

/// Read a thread's executor disclosure.
///
/// An Omega extension trait over the shared thread type, rather than an
/// inherent method on it and rather than a fork of it. A rebase that reshapes
/// `AcpThread` cannot silently drop the disclosure: it would drop the `impl`
/// below, and this crate would stop compiling.
pub trait ThreadExecutorDisclosure {
    /// Who executed this thread, before any engine-lane re-attribution.
    fn omega_executor_disclosure(&self, cx: &App) -> ExecutorDisclosure;
}

impl ThreadExecutorDisclosure for AcpThread {
    fn omega_executor_disclosure(&self, cx: &App) -> ExecutorDisclosure {
        classify_connection(self.connection().clone(), self.session_id(), cx)
    }
}

/// Re-attribute a disclosure to the engine run that owns the thread.
///
/// The executing agent's identity is kept. A host-bridge thread really is a
/// `codex-acp` process underneath, and the honest reading is "this run
/// delegated to this agent", not "a run did it".
#[must_use]
pub fn delegated_to_run(mut disclosure: ExecutorDisclosure, run_ref: String) -> ExecutorDisclosure {
    disclosure.class = ExecutorClass::EngineLane;
    disclosure.run_ref = Some(run_ref);
    disclosure
}

/// Classify a connection by its concrete type.
///
/// A checked downcast, not a name comparison. `agent_id()` is a display-facing
/// identifier — omega#75 is renaming Omega's own, and an extension can set its
/// own to anything — so deciding *what ran* from it would make the disclosure a
/// string match on a label, which is the failure mode this whole surface exists
/// to avoid.
///
/// The fallback is [`ExecutorClass::ExternalAcp`] rather than the native loop,
/// and that direction is deliberate: `NativeLoop` is the first-party claim, and
/// an unrecognised connection defaulting to it would present somebody else's
/// output as Omega's own. Guessing wrong towards "not ours" costs precision;
/// guessing wrong towards "ours" is the dishonest attribution omega#77 exists
/// to stop.
fn classify_connection(
    connection: Rc<dyn AgentConnection>,
    session_id: &acp::SessionId,
    cx: &App,
) -> ExecutorDisclosure {
    let agent_id = connection.agent_id().0.to_string();

    if let Some(native) = connection.clone().downcast::<NativeAgentConnection>() {
        let (provider, model) = native
            .thread(session_id, cx)
            .and_then(|thread| {
                thread.read(cx).model().map(|model| {
                    (
                        Some(model.provider_id().0.to_string()),
                        Some(model.id().0.to_string()),
                    )
                })
            })
            .unwrap_or((None, None));
        return ExecutorDisclosure {
            class: ExecutorClass::NativeLoop,
            agent_id,
            provider,
            model,
            run_ref: None,
        };
    }

    // `AcpConnection` shares the fallback, but is recognised explicitly so an
    // unrecognised connection type leaves a trace instead of passing silently.
    if connection.downcast::<AcpConnection>().is_none() {
        log::debug!(
            "OMEGA-DELTA-0021: disclosing {agent_id} as an external ACP agent; \
             its connection type is not one this build recognises"
        );
    }

    ExecutorDisclosure {
        class: ExecutorClass::ExternalAcp,
        agent_id,
        provider: None,
        model: None,
        run_ref: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// OMEGA-DELTA-0021. Delegating to a run keeps the agent that actually ran
    /// the work, and produces a coherent record.
    #[test]
    fn delegating_to_a_run_keeps_the_executing_agent() {
        let routed = ExecutorDisclosure {
            class: ExecutorClass::ExternalAcp,
            agent_id: "codex-acp".into(),
            provider: None,
            model: None,
            run_ref: None,
        };
        assert!(routed.is_coherent());

        let delegated = delegated_to_run(routed, "operation.full-auto.77".into());
        assert_eq!(delegated.class, ExecutorClass::EngineLane);
        assert_eq!(delegated.agent_id, "codex-acp");
        assert_eq!(delegated.run_ref.as_deref(), Some("operation.full-auto.77"));
        assert!(delegated.is_coherent());

        let line = delegated.label();
        assert!(line.contains("codex-acp"), "{line:?}");
        assert!(line.contains("engine_lane"), "{line:?}");
        assert!(line.contains("operation.full-auto.77"), "{line:?}");
    }

    /// The honest-attribution rule as an oracle: only the native loop is
    /// Omega's own output, so no routed or delegated record may carry the
    /// first-party class.
    #[test]
    fn a_routed_or_delegated_record_never_claims_the_native_loop() {
        for class in [ExecutorClass::ExternalAcp, ExecutorClass::EngineLane] {
            let run_ref = (class == ExecutorClass::EngineLane).then(|| "run.1".to_string());
            let disclosure = ExecutorDisclosure {
                class,
                agent_id: "codex-acp".into(),
                provider: None,
                model: None,
                run_ref,
            };
            assert!(disclosure.is_coherent(), "{class:?}");
            assert_ne!(disclosure.class, ExecutorClass::NativeLoop);
            assert!(
                disclosure.label().starts_with(class.token()),
                "the line must lead with the class that ran the work: {}",
                disclosure.label()
            );
        }
    }
}
