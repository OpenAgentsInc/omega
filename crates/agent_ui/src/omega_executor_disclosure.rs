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
//! | native provider, model | `DbThread.model`, restored by `Thread::from_db` |
//! | external ACP model | the live session's model config option, when advertised |
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
        // `OMEGA-DELTA-0202`. `active_turn_model` and not `model`: while a turn
        // has fallen onto a rung of the `OMEGA-DELTA-0201` chain, the rung is
        // what is answering, and this record is what every label is derived
        // from. Reading the configured model here would put the whole surface
        // back to naming a model that is not serving the turn.
        //
        // `OMEGA-DELTA-0207`. `routed_model_pair` and not `active_turn_model`:
        // the latter answers only once the model is `Ready`, so a thread that
        // already knows it will dispatch on `openagents/gemini-3.6-flash` —
        // and is merely waiting for the provider to register — reported "not
        // disclosed" here, and every label fell through to the standing choice,
        // which is `Luna` at every launch.
        let (provider, model) = native
            .thread(session_id, cx)
            .and_then(|thread| {
                thread
                    .read(cx)
                    .routed_model_pair()
                    .map(|(provider, model)| (Some(provider), Some(model)))
            })
            .unwrap_or((None, None));
        return ExecutorDisclosure {
            class: ExecutorClass::NativeLoop,
            agent_id,
            provider,
            model,
            run_ref: None,
            route: crate::omega_router::recorded_route(session_id),
        };
    }

    if let Some(acp_connection) = connection.clone().downcast::<AcpConnection>() {
        let model = acp_connection
            .session_config_options(session_id, cx)
            .and_then(|options| selected_acp_model(&options.config_options()))
            .or_else(|| (agent_id == agent_servers::GROK_ID).then(|| "grok".to_string()));
        return ExecutorDisclosure {
            class: ExecutorClass::ExternalAcp,
            agent_id,
            provider: None,
            model,
            run_ref: None,
            route: crate::omega_router::recorded_route(session_id),
        };
    }

    // `OMEGA-DELTA-0042`, omega#87. The Exo harness lane. Recognised by its
    // concrete type for the same reason the native loop is: `agent_id()` on
    // this connection is *derived from what Exo said about itself*, so
    // classifying by it would let an Exo install decide its own class.
    //
    // Exo reports `ExternalAcp`. The reasoning — and the argument against a
    // fourth class — is in `omega_exo_lane`'s module documentation. What is
    // added here over the shared fallback below is the model: Exo does tell
    // Omega which model served, so the lane says so instead of "not disclosed".
    if let Some(exo) = connection
        .clone()
        .downcast::<crate::omega_exo_connection::ExoHarnessConnection>()
    {
        // `None` before Exo has been asked. An identity nobody observed is
        // absent, never invented.
        let identity = exo.identity();
        return ExecutorDisclosure {
            class: omega_exo_lane::EXO_EXECUTOR_CLASS,
            agent_id: identity.as_ref().map_or_else(
                || agent_id.clone(),
                omega_exo_lane::ExoLaneIdentity::agent_id,
            ),
            provider: identity
                .as_ref()
                .and_then(|identity| identity.provider.clone()),
            model: identity.and_then(|identity| identity.model),
            run_ref: None,
            route: crate::omega_router::recorded_route(session_id),
        };
    }

    log::debug!(
        "OMEGA-DELTA-0021: disclosing {agent_id} as an external ACP agent; \
         its connection type is not one this build recognises"
    );

    ExecutorDisclosure {
        class: ExecutorClass::ExternalAcp,
        agent_id,
        provider: None,
        model: None,
        run_ref: None,
        route: crate::omega_router::recorded_route(session_id),
    }
}

fn selected_acp_model(options: &[acp::SessionConfigOption]) -> Option<String> {
    let option = options.iter().find(|option| {
        matches!(
            option.category.as_ref(),
            Some(acp::SessionConfigOptionCategory::Model)
        ) || option.id.0.as_ref() == "model"
    })?;
    let acp::SessionConfigKind::Select(select) = &option.kind else {
        return None;
    };

    let selected_name = match &select.options {
        acp::SessionConfigSelectOptions::Ungrouped(options) => options
            .iter()
            .find(|option| option.value == select.current_value)
            .map(|option| option.name.clone()),
        acp::SessionConfigSelectOptions::Grouped(groups) => groups
            .iter()
            .flat_map(|group| &group.options)
            .find(|option| option.value == select.current_value)
            .map(|option| option.name.clone()),
        _ => None,
    };

    selected_name
        .filter(|name| !name.trim().is_empty())
        .or_else(|| {
            let current_value = select.current_value.0.to_string();
            (!current_value.trim().is_empty()).then_some(current_value)
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn external_acp_model_uses_the_adapters_human_readable_selection() {
        let options = vec![
            acp::SessionConfigOption::select(
                "model",
                "Model",
                "claude-opus-4-6",
                vec![
                    acp::SessionConfigSelectOption::new("claude-sonnet-4-6", "Sonnet 4.6"),
                    acp::SessionConfigSelectOption::new("claude-opus-4-6", "Opus 4.6"),
                ],
            )
            .category(acp::SessionConfigOptionCategory::Model),
        ];

        assert_eq!(selected_acp_model(&options).as_deref(), Some("Opus 4.6"));
    }

    #[test]
    fn external_acp_model_keeps_an_out_of_picker_model_id_visible() {
        let options = vec![
            acp::SessionConfigOption::select(
                "model",
                "Model",
                "claude-opus-5",
                Vec::<acp::SessionConfigSelectOption>::new(),
            )
            .category(acp::SessionConfigOptionCategory::Model),
        ];

        assert_eq!(
            selected_acp_model(&options).as_deref(),
            Some("claude-opus-5")
        );
    }

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
            route: Some(omega_front_door::RouteReason::PinHonored),
        };
        assert!(routed.is_coherent());

        let delegated = delegated_to_run(routed, "operation.full-auto.77".into());
        assert_eq!(delegated.class, ExecutorClass::EngineLane);
        assert_eq!(delegated.agent_id, "codex-acp");
        assert_eq!(delegated.run_ref.as_deref(), Some("operation.full-auto.77"));
        assert!(delegated.is_coherent());

        let line = delegated.label();
        assert!(line.contains("codex-acp"), "{line:?}");
        // omega#100. The class is no longer rendered, so what the reader is
        // told is *who* ran it and *which run* it belongs to. The class stays
        // on the record and is asserted above.
        assert!(line.contains("operation.full-auto.77"), "{line:?}");
        assert!(
            !line.contains(ExecutorClass::NativeLoop.token()),
            "a delegated line must never read as the native loop: {line:?}"
        );
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
                route: Some(omega_front_door::RouteReason::PinHonored),
            };
            assert!(disclosure.is_coherent(), "{class:?}");
            assert_ne!(disclosure.class, ExecutorClass::NativeLoop);
            // omega#100. The record still carries the class; the line no
            // longer leads with it.
            //
            // This asserted `label().starts_with(class.token())`, which was
            // right while the wire token was rendered. The owner asked for the
            // token to go — `ExecutorClass::token` documents that it is "never
            // shown to a user on its own", and the line was leading with one.
            // What the test is really for survives: a routed or delegated
            // record must not read as the native loop. So it now asserts the
            // agent id is named and the native-loop token appears nowhere.
            let label = disclosure.label();
            assert!(
                label.starts_with("codex-acp"),
                "the line must name the agent that ran the work: {label}"
            );
            assert!(
                !label.contains(ExecutorClass::NativeLoop.token()),
                "a {class:?} record must never read as the native loop: {label}"
            );
        }
    }
}
