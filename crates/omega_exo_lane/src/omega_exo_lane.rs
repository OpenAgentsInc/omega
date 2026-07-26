//! The Exo harness lane. `OMEGA-DELTA-0042`, omega#87.
//!
//! Omega drives **`exoharness/exo`** — the recursive-self-improvement agent
//! harness from the Braintrust orbit — as one more executor lane beneath Omega
//! Agent, beside the native loop and the ACP agents. It is **not** exo labs'
//! `exo-explore/exo` cluster-inference appliance; omega#86 was closed for
//! targeting the wrong one, and [`EXO_PIN`] names the repository so the
//! distinction is a field rather than a sentence.
//!
//! This crate is the **law**, and it is a leaf: no GPUI, no process, no
//! filesystem, no clock. It decides which Exo is admitted ([`pin`]), which
//! command lines exist ([`command`]), where Exo may be reached ([`endpoint`]),
//! which Exo agents may be run ([`capability`]), and what a turn's output means
//! ([`turn`]). The half that spawns a process and builds a thread lives in
//! `crates/agent_ui/src/omega_exo_connection.rs`, for the same reason the
//! router's decision half lives here and its dispatch half lives there: a law
//! that needs GPUI to check is a law nobody checks.
//!
//! # Streaming and self-modification authority
//!
//! Exo sends live text, tool calls, tool results, and completion records through
//! its ACP standard-input transport. Omega attaches that transport through
//! `crates/agent_servers`.
//!
//! Self-modification is off by default. A person can authorize one exact turn
//! in a dedicated confirmation dialog. The grant binds the source, binary,
//! agent, conversation, tool modules, mounts, draft, generation, and expiry.
//! The send path consumes the grant once and writes a durable receipt.
//!
//! # Which executor class an Exo thread reports, and why
//!
//! [`ExecutorClass`] is closed at three variants by `OMEGA-AGENT-AC-04`, and it
//! answers one question: **who ran the work.** An Exo thread reports
//! [`ExecutorClass::ExternalAcp`].
//!
//! * Not [`ExecutorClass::NativeLoop`]. That is the first-party claim — Omega's
//!   own in-process loop in `crates/agent`. Presenting somebody else's output
//!   as Omega's own is the dishonest attribution omega#77 exists to stop, and
//!   this would be its worst case: Exo's agent has an unrestricted networked
//!   shell.
//! * Not [`ExecutorClass::EngineLane`]. An engine lane *is* Full Auto
//!   authority. `ExecutorDisclosure::is_coherent` requires a `run_ref` on one,
//!   and the router will not route an unpinned thread to one because owner gate
//!   8 admits only an explicit human action into that authority. Exo has no
//!   engine run, no run reference, and no receipt. A lane that reported
//!   `EngineLane` would be a new door into Full Auto authority reachable by
//!   adding an executor — a fourth of exactly the kind that were removed from
//!   OpenAgents Desktop on 2026-07-25. An Exo agent with a shell is precisely
//!   the caller that gate exists for.
//! * So [`ExecutorClass::ExternalAcp`]. Its documentation says "an external ACP
//!   agent reached through `crates/agent_servers`", and Tier A reaches Exo over
//!   its CLI rather than over ACP — so the fit is worth stating rather than
//!   assuming. The class is about the *executor*, not the *wire*: a separate
//!   process, outside Omega, that Omega does not own, that reports no model
//!   through `AgentConnection::model_selector`, and that must carry no
//!   `run_ref` — which is exactly what `is_coherent` requires of this class and
//!   exactly what is true of Exo. Tier B swaps the CLI for ACP and **the class
//!   does not change**, which is the test that the class was about the right
//!   thing.
//!
//! A fourth variant was considered and not taken. It would need a revision of
//! `OMEGA-AGENT-AC-04`, and the argument for it would have to be that the wire
//! is part of who ran the work. It is not: the wire is why the lane is coarse,
//! and the coarseness is visible to the reader without a class, because no text
//! delta ever arrives. That argument is written here so a later reader can
//! disagree with something concrete rather than re-derive it.
//!
//! # The disclosure names Exo, its executor, and its model
//!
//! [`ExecutorDisclosure`] is a record of parts a label renders, never a stored
//! label — omega#74's binding condition, held mechanically by
//! `EXECUTOR_DISCLOSURE_FIELDS`. So the lane adds no field. It fills the ones
//! that exist:
//!
//! | Part | Exo |
//! | --- | --- |
//! | `class` | `ExternalAcp`, for the reasons above |
//! | `agent_id` | `exo/<executor>` — the process and the executor inside it |
//! | `provider` | the model binding's base URL, or *not disclosed* |
//! | `model` | the upstream model the binding resolves to |
//! | `run_ref` | `None`; Exo holds no Omega run authority |
//!
//! `provider` is genuinely `None` for a default binding, because Exo's LLM
//! binding has no provider field at all — only an optional base URL. Saying
//! "not disclosed" is the honest third option omega#77 added for exactly this,
//! and it is better than deriving "openai" from the absence of a URL, which
//! would be Omega inventing a fact about somebody else's configuration.

pub mod authority;
pub mod capability;
pub mod command;
pub mod endpoint;
pub mod pin;
pub mod turn;

pub use capability::{ExoAgent, ExoAgentReadError, ExoConversation, ExoMount, SelfModification};
pub use command::{ADMITTED_LANE_ARGV, ARGUMENT_TERMINATOR, ExoArg, ExoCommand, ExoRoot};
pub use endpoint::{EXO_SERVE_DEFAULT_BIND, LoopbackEndpoint, OffLoopback};
pub use pin::{EXO_HARNESS_ID, EXO_PIN, ExoPin, ExoPinMismatch, ObservedExoCheckout, admits_bytes};
pub use turn::{ExoToolActivity, ExoTurn, NotATurn};

use omega_front_door::{ExecutorClass, ExecutorDisclosure, RouteReason};

/// The executor class an Exo thread reports. See the module documentation for
/// why this is the one, and why it is not a fourth.
pub const EXO_EXECUTOR_CLASS: ExecutorClass = ExecutorClass::ExternalAcp;

/// One row of `exo model list`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExoModelBinding {
    /// The local binding name, which is what an agent record refers to.
    pub name: String,
    /// The model the binding actually resolves to upstream.
    pub upstream_model: String,
    /// The base URL, when the binding names one. `None` for Exo's default,
    /// which carries no provider information at all.
    pub base_url: Option<String>,
}

impl ExoModelBinding {
    /// Read `exo model list` output.
    ///
    /// The table is elastic-tab-expanded, so columns are separated by runs of
    /// two or more spaces. A row this build cannot split into four columns is
    /// skipped rather than guessed at: a half-read binding would produce a
    /// disclosure naming a model nobody configured.
    #[must_use]
    pub fn read_table(stdout: &str) -> Vec<Self> {
        stdout
            .lines()
            .skip(1)
            .filter_map(|line| {
                let columns: Vec<&str> = line
                    .split("  ")
                    .map(str::trim)
                    .filter(|column| !column.is_empty())
                    .collect();
                let [name, upstream_model, _secret, base_url] = columns.as_slice() else {
                    return None;
                };
                Some(Self {
                    name: (*name).to_owned(),
                    upstream_model: (*upstream_model).to_owned(),
                    base_url: (*base_url != "default").then(|| (*base_url).to_owned()),
                })
            })
            .collect()
    }
}

/// Everything a thread on this lane discloses.
///
/// Assembled from what Exo reported about itself: [`ExoAgent`] from
/// `agent show`, and the matching [`ExoModelBinding`] from `model list`. A
/// binding Exo did not report is *absent*, never invented.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExoLaneIdentity {
    /// The executor Exo runs the turn with: `basic`, `rlm`, `typescript`,
    /// `codex`, `claude-code`, `cursor`, or a module path.
    pub executor: String,
    /// The model the agent's binding resolves to upstream, when Exo reported a
    /// binding for it.
    pub model: Option<String>,
    /// The binding's base URL, when it has one.
    pub provider: Option<String>,
}

impl ExoLaneIdentity {
    /// Bind an agent to the model bindings Exo reported.
    #[must_use]
    pub fn resolve(agent: &ExoAgent, bindings: &[ExoModelBinding]) -> Self {
        let binding = bindings.iter().find(|binding| binding.name == agent.model);
        Self {
            executor: agent.harness.clone(),
            model: binding.map(|binding| binding.upstream_model.clone()),
            provider: binding.and_then(|binding| binding.base_url.clone()),
        }
    }

    /// The identifier a thread on this lane presents.
    ///
    /// One string, because [`ExecutorDisclosure`] has one field for it and the
    /// field list is closed on purpose. Both halves are facts Exo reported: the
    /// process is `exo`, and the executor inside it is what `agent show` said.
    #[must_use]
    pub fn agent_id(&self) -> String {
        format!("exo/{}", self.executor)
    }

    /// The disclosure record for a thread on this lane.
    #[must_use]
    pub fn disclosure(&self, route: Option<RouteReason>) -> ExecutorDisclosure {
        ExecutorDisclosure {
            class: EXO_EXECUTOR_CLASS,
            agent_id: self.agent_id(),
            provider: self.provider.clone(),
            model: self.model.clone(),
            run_ref: None,
            route,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Real `exo model list` output from the pinned Exo, 2026-07-25.
    const DRIVEN_MODELS: &str = "\
MODEL     UPSTREAM_MODEL  SECRET  BASE_URL
gpt5mini  gpt-5-mini      openai  default
";

    fn driven_agent() -> ExoAgent {
        ExoAgent {
            slug: "omega-lane".into(),
            harness: "basic".into(),
            model: "gpt5mini".into(),
            agent_authored_tools: false,
            tool_modules: 0,
            tool_module_paths: Vec::new(),
            read_write_mount: false,
            mounts: Vec::new(),
            networking: false,
        }
    }

    #[test]
    fn the_model_table_that_was_driven_reads_back() {
        let bindings = ExoModelBinding::read_table(DRIVEN_MODELS);
        assert_eq!(
            bindings,
            vec![ExoModelBinding {
                name: "gpt5mini".into(),
                upstream_model: "gpt-5-mini".into(),
                base_url: None,
            }]
        );
    }

    /// The disclosure line, on the identity that was actually driven. It names
    /// Exo, the executor inside it, and the model — and it says the provider is
    /// not disclosed rather than inventing one.
    #[test]
    fn an_exo_thread_names_exo_its_executor_and_its_model() {
        let identity =
            ExoLaneIdentity::resolve(&driven_agent(), &ExoModelBinding::read_table(DRIVEN_MODELS));
        let disclosure = identity.disclosure(Some(RouteReason::PinHonored));
        assert!(disclosure.is_coherent());

        let line = disclosure.label();
        assert!(line.contains("exo/"), "{line}");
        assert!(line.contains("basic"), "{line}");
        assert!(line.contains("gpt-5-mini"), "{line}");
        assert!(line.contains("provider not disclosed"), "{line}");
        assert!(line.starts_with(EXO_EXECUTOR_CLASS.token()), "{line}");
    }

    /// The class decision, as an oracle rather than a comment. An Exo thread
    /// must never claim Omega's own loop, and must never claim engine-lane
    /// authority.
    #[test]
    fn an_exo_thread_claims_neither_omegas_own_loop_nor_full_auto_authority() {
        assert_ne!(EXO_EXECUTOR_CLASS, ExecutorClass::NativeLoop);
        assert_ne!(EXO_EXECUTOR_CLASS, ExecutorClass::EngineLane);
        let identity =
            ExoLaneIdentity::resolve(&driven_agent(), &ExoModelBinding::read_table(DRIVEN_MODELS));
        let disclosure = identity.disclosure(Some(RouteReason::PinHonored));
        assert_eq!(disclosure.run_ref, None, "Exo holds no Omega run authority");

        // The same record with engine-lane authority claimed is incoherent, so
        // the coherence law is what stops it rather than this lane's manners.
        let forged = ExecutorDisclosure {
            class: ExecutorClass::EngineLane,
            ..disclosure
        };
        assert!(!forged.is_coherent());
    }

    /// A binding Exo did not report leaves the model absent. The alternative —
    /// falling back to the agent's local alias — would put `gpt5mini` in a
    /// field that means "the model that served the turn".
    #[test]
    fn an_unreported_binding_leaves_the_model_undisclosed() {
        let identity = ExoLaneIdentity::resolve(&driven_agent(), &[]);
        assert_eq!(identity.model, None);
        let disclosure = identity.disclosure(None);
        assert!(disclosure.is_coherent());
        assert!(disclosure.label().contains("model not disclosed"));
    }

    #[test]
    fn a_binding_with_a_base_url_discloses_it_as_the_provider() {
        let bindings = ExoModelBinding::read_table(
            "MODEL  UPSTREAM_MODEL  SECRET  BASE_URL\n\
             glm    z-ai/glm-5.2    or      https://openrouter.ai/api/v1\n",
        );
        let agent = ExoAgent {
            model: "glm".into(),
            ..driven_agent()
        };
        let identity = ExoLaneIdentity::resolve(&agent, &bindings);
        assert_eq!(
            identity.provider.as_deref(),
            Some("https://openrouter.ai/api/v1")
        );
        assert_eq!(identity.model.as_deref(), Some("z-ai/glm-5.2"));
        assert!(identity.disclosure(None).is_coherent());
    }
}
pub use authority::{
    ExoGrantRefusal, ExoSelfModificationCapability, ExoSelfModificationConsentOrigin,
    ExoSelfModificationGrant, ExoSelfModificationGrantRequest, ExoSelfModificationReceipt,
    ObservedExoCapabilityState, ObservedReadWriteMount, ObservedToolModule,
};
