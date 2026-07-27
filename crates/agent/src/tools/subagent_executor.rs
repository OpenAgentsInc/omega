//! Choosing what runs a subagent, per spawn.
//!
//! Before this, every subagent was the parent wearing the same face:
//! `Thread::new_subagent` copies the parent's model, and `subagent_model` is
//! one global setting for all of them. It could not say "this one is Codex and
//! that one is Claude".
//!
//! The choice is an **executor**, not a model. Codex and Claude Code are not
//! language models — they are external ACP agents reached through
//! `crates/agent_servers`, each with its own login, tools and loop. Treating
//! the choice as a model swap only ever reaches the native loop.
//!
//! The whole of the risk here is in one place: [`resolve_subagent_executor`].
//! It is a pure function of what was asked for and what is actually installed,
//! and it has exactly one job — **never resolve to something other than what
//! was named**. A request for `codex-acp` either runs Codex or fails saying it
//! could not. It does not quietly become the parent's model, because a
//! subagent that reports as Codex and is not is the same defect class as an
//! undisclosed provider handoff.

use omega_front_door::{ExecutorClass, ExecutorDisclosure};
use std::fmt::Write as _;

/// An agent Omega found on this machine.
///
/// Deliberately not `omega_agent_detect::DetectedAgent`. This module's rules
/// are about ids and names, and re-stating the two fields it needs keeps the
/// resolution testable without a `PATH`, and keeps it from breaking when the
/// detector's own shape changes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InstalledAgent {
    pub id: String,
    pub name: String,
}

impl InstalledAgent {
    pub fn new(id: impl Into<String>, name: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            name: name.into(),
        }
    }
}

/// What should run a subagent.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SubagentExecutor {
    /// Today's behaviour: the subagent inherits the parent's model and runs on
    /// the native loop. This is what an omitted field means, and it is the only
    /// thing an omitted field may ever mean.
    InheritParent,
    /// An external ACP agent, run as its own session against its own agent
    /// server.
    ExternalAcp { id: String, name: String },
    /// The installed Exo lane, reached through its ACP stdin transport.
    Exo(omega_agent_detect::exo::DerivedExoLane),
    /// A named `omega-effectd` lane. The tool layer hands this to the engine
    /// authority; it must never be interpreted as a local subagent.
    EngineLane { lane: String },
}

impl SubagentExecutor {
    /// The admitted executor class this choice runs as.
    ///
    /// Borrowed from `omega_front_door` rather than restated, so there is one
    /// answer to what the three classes are.
    #[must_use]
    pub const fn class(&self) -> ExecutorClass {
        match self {
            Self::InheritParent => ExecutorClass::NativeLoop,
            Self::ExternalAcp { .. } | Self::Exo(_) => ExecutorClass::ExternalAcp,
            Self::EngineLane { .. } => ExecutorClass::EngineLane,
        }
    }

    #[must_use]
    pub fn is_external(&self) -> bool {
        matches!(self, Self::ExternalAcp { .. } | Self::Exo(_))
    }

    #[must_use]
    pub fn engine_lane(&self) -> Option<&str> {
        match self {
            Self::EngineLane { lane } => Some(lane),
            _ => None,
        }
    }
}

/// The outcome of asking for an executor.
///
/// Two variants, and the refusal carries the sentence the model reads. There is
/// deliberately no third variant meaning "could not honour this, used something
/// else" — that shape is the silent fallback this module exists to prevent.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExecutorResolution {
    Resolved(SubagentExecutor),
    Refused(String),
}

impl ExecutorResolution {
    #[must_use]
    pub fn resolved(&self) -> Option<&SubagentExecutor> {
        match self {
            Self::Resolved(executor) => Some(executor),
            Self::Refused(_) => None,
        }
    }

    #[must_use]
    pub fn refusal(&self) -> Option<&str> {
        match self {
            Self::Resolved(_) => None,
            Self::Refused(reason) => Some(reason),
        }
    }
}

/// Render the installed set for a refusal message.
///
/// An empty set is *said*, not left blank. "No agents were found on PATH" is a
/// different problem from "you asked for the wrong one", and a caller that
/// cannot tell them apart will go looking in the wrong place.
fn describe_available(installed: &[InstalledAgent]) -> String {
    if installed.is_empty() {
        return "No external agents were found on this machine's PATH, so only \
                the default (omit the field) is available here."
            .to_owned();
    }
    let mut out = String::from("Installed and available here: ");
    for (index, agent) in installed.iter().enumerate() {
        if index > 0 {
            out.push_str(", ");
        }
        let _ = write!(out, "`{}` ({})", agent.id, agent.name);
    }
    out.push('.');
    out
}

/// Decide what should run a subagent.
///
/// - `requested` is the tool input's field, `None` when omitted.
/// - `known` is every executor id Omega understands, installed or not.
/// - `installed` is what is actually present on this machine.
///
/// The two lists are separate on purpose. Being *known* and being *present* are
/// different facts, and collapsing them produces the wrong sentence for the
/// most common mistake: asking for a real agent that simply is not installed
/// here. `AllAgentServersSettings` records what is configured, which is a third
/// thing again and is not evidence of presence — a fresh `--user-data-dir` has
/// no settings written whatever is on disk.
///
/// **An unrecognised name is refused, never guessed.** The tempting reading of
/// "or a model for the native loop" is to treat anything that is not an agent
/// id as a model name. That makes the typo `codex-acpp` a silent inherit — the
/// parent asked for Codex, got its own model, and is told nothing. Per-spawn
/// native model selection is therefore not accepted here at all; it is a
/// separate change that needs a validated model lookup, not a fallthrough.
#[must_use]
pub fn resolve_subagent_executor(
    requested: Option<&str>,
    known: &[InstalledAgent],
    installed: &[InstalledAgent],
) -> ExecutorResolution {
    let Some(requested) = requested else {
        return ExecutorResolution::Resolved(SubagentExecutor::InheritParent);
    };

    let requested = requested.trim();
    if requested.is_empty() {
        // An empty string is not a request for anything. Treating it as one
        // would refuse a spawn over whitespace.
        return ExecutorResolution::Resolved(SubagentExecutor::InheritParent);
    }

    if requested == "native" {
        return ExecutorResolution::Resolved(SubagentExecutor::InheritParent);
    }

    if requested == "auto" {
        return ExecutorResolution::Refused(
            "`auto` is not a delegate executor. Name `native`, an installed \
             external agent, `exo`, or an `engine:<lane>` explicitly."
                .to_owned(),
        );
    }

    if let Some(lane) = requested.strip_prefix("engine:") {
        let lane = lane.trim();
        if lane.is_empty() {
            return ExecutorResolution::Refused(
                "An engine executor must name its lane as `engine:<lane>`.".to_owned(),
            );
        }
        return ExecutorResolution::Resolved(SubagentExecutor::EngineLane {
            lane: lane.to_owned(),
        });
    }

    if let Some(agent) = installed.iter().find(|agent| agent.id == requested) {
        return ExecutorResolution::Resolved(SubagentExecutor::ExternalAcp {
            id: agent.id.clone(),
            name: agent.name.clone(),
        });
    }

    if let Some(agent) = known.iter().find(|agent| agent.id == requested) {
        return ExecutorResolution::Refused(format!(
            "Cannot spawn a `{}` subagent: {} is an agent Omega knows how to \
             drive, but its binary was not found on this machine's PATH, so it \
             is not installed here. {} Omega will not silently run this on the \
             parent's own model instead — a subagent that reported as {} and \
             was not would be a lie about who did the work.",
            requested,
            agent.name,
            describe_available(installed),
            agent.name,
        ));
    }

    ExecutorResolution::Refused(format!(
        "Cannot spawn a `{requested}` subagent: that is not an executor Omega \
         knows. {} This field names an external agent to run the subagent, not \
         a language model — omit it to run the subagent on the parent's own \
         model, which is the default.",
        describe_available(installed),
    ))
}

/// The disclosure record for a subagent that ran as an external ACP agent.
///
/// `OMEGA-DELTA-0021` fixed the shape of executor disclosure: a **typed record
/// that a label renders**, never a label string. That is the binding condition
/// of the owner's 2026-07-25 identity decision, and it applies to a subagent
/// for the same reason it applies to a thread — output that appeared inside
/// Omega and was produced by somebody else is a false attribution claim made
/// silently, whether the reader is a person or the parent agent.
///
/// The first cut of this delta disclosed subagents with a hand-written
/// sentence, `"Codex (codex-acp, external ACP agent)"`, which is exactly the
/// stored rendering that shape forbids: it cannot be handed to a signer, it
/// cannot be re-rendered for a different reader, and nothing stops it drifting
/// from what actually ran. So the parts are the record, and the sentence is
/// derived from it.
///
/// `provider` and `model` are `None` because they are genuinely **not
/// disclosed**, not because nobody looked. `AcpConnection` does not implement
/// `AgentConnection::model_selector`, so an external agent does not tell Omega
/// which model served the turn — Codex chooses that inside its own loop. An
/// invented model here would be worse than an absent one: it would read as a
/// disclosure and be a guess. `ExecutorDisclosure::label` says "model not
/// disclosed" for exactly this case.
///
/// `run_ref` is `None` because only an engine lane has run authority to
/// reference, and `route` is `None` because the router did not put this
/// subagent here — the parent named it, in the tool call. Saying "not routed"
/// is different from claiming a reason nobody recorded.
#[must_use]
pub fn external_acp_disclosure(agent_id: &str) -> ExecutorDisclosure {
    ExecutorDisclosure {
        class: ExecutorClass::ExternalAcp,
        agent_id: agent_id.to_owned(),
        provider: None,
        model: None,
        run_ref: None,
        route: None,
    }
}

/// The disclosure record for a subagent that ran on Omega's own loop.
///
/// The counterpart to [`external_acp_disclosure`], and the reason a mixed
/// fan-out is attributable: three results carry three records, and an inherited
/// one says `native_loop` with Omega's agent id where an external one says
/// `external_acp` with the agent's.
///
/// The model is the **subagent's** model, read from the subagent's own thread,
/// not the parent's. Those are usually the same and are not always: the
/// `subagent_model` setting overrides the inherited model for every subagent,
/// and a record that reported the parent's model would be wrong on exactly the
/// machine where that setting is set.
#[must_use]
pub fn native_loop_disclosure(
    agent_id: &str,
    provider: Option<String>,
    model: Option<String>,
) -> ExecutorDisclosure {
    ExecutorDisclosure {
        class: ExecutorClass::NativeLoop,
        agent_id: agent_id.to_owned(),
        provider,
        model,
        run_ref: None,
        route: None,
    }
}

/// [`resolve_subagent_executor`] against the agents actually on this machine.
///
/// The installed set comes from `omega_agent_detect`, a `PATH` probe, and not
/// from `AllAgentServersSettings`. Settings record what is *configured*, which
/// is not evidence of presence: a fresh `--user-data-dir` has no settings
/// written whatever is installed, so a settings-based check would offer
/// nothing on exactly the machine a new person is using.
#[must_use]
pub fn resolve_requested_executor(requested: Option<&str>) -> ExecutorResolution {
    if requested.is_some_and(|requested| requested.trim() == "exo") {
        return match omega_agent_detect::exo::derive_lane_from_env() {
            Ok(lane) => ExecutorResolution::Resolved(SubagentExecutor::Exo(lane)),
            Err(error) => ExecutorResolution::Refused(format!(
                "Cannot delegate to `exo`: {} ({error}). Omega will not \
                 substitute a different executor.",
                error.summary()
            )),
        };
    }
    let known: Vec<InstalledAgent> = omega_agent_detect::CANDIDATES
        .iter()
        .map(|candidate| InstalledAgent::new(candidate.id, candidate.name))
        .collect();
    let installed: Vec<InstalledAgent> = omega_agent_detect::detected()
        .iter()
        .map(|agent| InstalledAgent::new(agent.id, agent.name))
        .collect();
    resolve_subagent_executor(requested, &known, &installed)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn codex() -> InstalledAgent {
        InstalledAgent::new("codex-acp", "Codex")
    }
    fn claude() -> InstalledAgent {
        InstalledAgent::new("claude-acp", "Claude")
    }
    fn cursor() -> InstalledAgent {
        InstalledAgent::new("cursor", "Cursor")
    }

    fn known() -> Vec<InstalledAgent> {
        vec![codex(), claude(), cursor()]
    }

    #[test]
    fn omitting_the_field_inherits_the_parent() {
        let resolution = resolve_subagent_executor(None, &known(), &[codex()]);
        assert_eq!(
            resolution,
            ExecutorResolution::Resolved(SubagentExecutor::InheritParent)
        );
        // Even with agents available, silence means today's behaviour. This is
        // the compatibility promise: existing spawns do not change.
        assert_eq!(
            resolve_subagent_executor(None, &known(), &known()),
            ExecutorResolution::Resolved(SubagentExecutor::InheritParent)
        );
    }

    #[test]
    fn an_installed_agent_resolves_to_itself() {
        let resolution = resolve_subagent_executor(Some("codex-acp"), &known(), &[codex()]);
        assert_eq!(
            resolution,
            ExecutorResolution::Resolved(SubagentExecutor::ExternalAcp {
                id: "codex-acp".into(),
                name: "Codex".into()
            })
        );
        assert_eq!(
            resolution.resolved().map(SubagentExecutor::class),
            Some(ExecutorClass::ExternalAcp)
        );
    }

    #[test]
    fn native_is_explicit_and_auto_is_not_an_executor() {
        assert_eq!(
            resolve_subagent_executor(Some("native"), &known(), &[codex()]),
            ExecutorResolution::Resolved(SubagentExecutor::InheritParent)
        );
        let auto = resolve_subagent_executor(Some("auto"), &known(), &[codex()]);
        assert!(
            auto.refusal()
                .is_some_and(|reason| reason.contains("not a delegate executor")),
            "{auto:?}"
        );
    }

    #[test]
    fn an_engine_lane_keeps_its_exact_name() {
        assert_eq!(
            resolve_subagent_executor(Some("engine:claude-local"), &known(), &[]),
            ExecutorResolution::Resolved(SubagentExecutor::EngineLane {
                lane: "claude-local".to_owned()
            })
        );
        assert!(
            resolve_subagent_executor(Some("engine: "), &known(), &[])
                .refusal()
                .is_some()
        );
    }

    #[test]
    fn two_different_requests_resolve_to_two_different_executors() {
        // The point of the whole issue: a mixed fan-out. If these ever came
        // back equal, attribution would be meaningless.
        let installed = vec![codex(), claude()];
        let first = resolve_subagent_executor(Some("codex-acp"), &known(), &installed);
        let second = resolve_subagent_executor(Some("claude-acp"), &known(), &installed);
        assert_ne!(first, second);
        assert_eq!(
            first.resolved().unwrap(),
            &SubagentExecutor::ExternalAcp {
                id: "codex-acp".into(),
                name: "Codex".into()
            }
        );
        assert_eq!(
            second.resolved().unwrap(),
            &SubagentExecutor::ExternalAcp {
                id: "claude-acp".into(),
                name: "Claude".into()
            }
        );
    }

    #[test]
    fn a_known_agent_that_is_not_installed_fails_naming_it() {
        let resolution = resolve_subagent_executor(Some("codex-acp"), &known(), &[claude()]);
        let refusal = resolution.refusal().expect("must refuse");

        // Names what was asked for.
        assert!(refusal.contains("codex-acp"));
        assert!(refusal.contains("Codex"));
        // Says it is a real agent that is missing, not an unknown name.
        assert!(refusal.contains("not found on this machine's PATH"));
        // Says what *is* here, so the caller can pick again.
        assert!(refusal.contains("claude-acp"));
        // And states that it did not fall back.
        assert!(refusal.contains("will not silently run this on the parent's own model"));

        assert!(resolution.resolved().is_none());
    }

    #[test]
    fn an_unknown_name_is_refused_and_never_guessed_as_a_model() {
        // The typo case. `codex-acpp` must not become an inherit.
        for requested in ["codex-acpp", "gpt-5", "claude-sonnet-4", "nonsense"] {
            let resolution = resolve_subagent_executor(Some(requested), &known(), &known());
            let refusal = resolution
                .refusal()
                .unwrap_or_else(|| panic!("`{requested}` must be refused, not guessed"));
            assert!(
                refusal.contains(requested),
                "the refusal for `{requested}` must name it"
            );
            assert!(
                refusal.contains("not an executor Omega knows"),
                "`{requested}` must be refused as unknown"
            );
            assert!(
                resolution.resolved().is_none(),
                "`{requested}` resolved to {:?} instead of refusing — a \
                 mistyped executor became a silent inherit",
                resolution.resolved()
            );
        }
    }

    #[test]
    fn nothing_installed_offers_nothing_external() {
        // Detection pointed at an empty PATH. Every named agent must refuse,
        // and the message must say the machine is empty rather than implying
        // the caller picked the wrong name.
        for requested in ["codex-acp", "claude-acp", "cursor"] {
            let resolution = resolve_subagent_executor(Some(requested), &known(), &[]);
            let refusal = resolution.refusal().expect("must refuse with empty PATH");
            assert!(refusal.contains("No external agents were found"));
            assert!(refusal.contains(requested));
        }
        // But the default still works with nothing installed.
        assert_eq!(
            resolve_subagent_executor(None, &known(), &[]),
            ExecutorResolution::Resolved(SubagentExecutor::InheritParent)
        );
    }

    #[test]
    fn a_blank_request_is_not_a_request() {
        for requested in ["", "   ", "\t"] {
            assert_eq!(
                resolve_subagent_executor(Some(requested), &known(), &known()),
                ExecutorResolution::Resolved(SubagentExecutor::InheritParent),
                "a blank executor field must mean the same as omitting it"
            );
        }
    }

    #[test]
    fn resolution_is_by_exact_id_not_by_prefix_or_name() {
        // Bounded exact matching against a closed set. A prefix or fuzzy match
        // would let `codex` resolve to `codex-acp` and, worse, let one agent's
        // name capture another's request.
        for requested in ["codex", "CODEX-ACP", "Codex", "codex-acp "] {
            let resolution = resolve_subagent_executor(Some(requested), &known(), &known());
            if requested.trim() == "codex-acp" {
                assert!(resolution.resolved().is_some(), "trimmed exact id resolves");
            } else {
                assert!(
                    resolution.refusal().is_some(),
                    "`{requested}` must not match `codex-acp` by prefix or case"
                );
            }
        }
    }

    /// Criterion 6. An external subagent discloses **itself**.
    ///
    /// The record is coherent on `ExecutorDisclosure`'s own terms, which is the
    /// check that catches a record built out of missing values: an empty
    /// `agent_id`, or a `provider` that is `Some("")` because something turned
    /// an absent value into a present empty one.
    #[test]
    fn an_external_subagent_discloses_its_own_executor() {
        let disclosure = external_acp_disclosure("codex-acp");

        assert_eq!(disclosure.class, ExecutorClass::ExternalAcp);
        assert_eq!(disclosure.agent_id, "codex-acp");
        // Not disclosed, and said so rather than invented. `AcpConnection` has
        // no `model_selector`; Codex picks the model inside its own loop.
        assert_eq!(disclosure.provider, None);
        assert_eq!(disclosure.model, None);
        // No run authority, and the router did not put it here.
        assert_eq!(disclosure.run_ref, None);
        assert_eq!(disclosure.route, None);
        assert!(
            disclosure.is_coherent(),
            "an external subagent's record must be coherent: {disclosure:?}"
        );

        let label = disclosure.label();
        assert!(label.contains("codex-acp"), "{label}");
        assert!(
            label.contains("model not disclosed"),
            "an undisclosed model must be said, not skipped: {label}"
        );
    }

    /// Criterion 6, the failure it exists to prevent. An external subagent must
    /// never disclose as Omega's own loop.
    ///
    /// This is the same law `OMEGA-DELTA-0021` states for threads: only the
    /// native loop is Omega's own output, so anything else claiming it is
    /// presenting somebody else's work as Omega's.
    #[test]
    fn an_external_subagent_never_discloses_as_the_native_loop() {
        for agent_id in ["codex-acp", "claude-acp", "cursor"] {
            let disclosure = external_acp_disclosure(agent_id);
            assert_ne!(disclosure.class, ExecutorClass::NativeLoop);
            assert!(
                !disclosure
                    .label()
                    .contains(ExecutorClass::NativeLoop.token()),
                "`{agent_id}` must not read as the native loop"
            );
        }
    }

    /// Criterion 3 and 4, as an oracle over the records themselves.
    ///
    /// One turn, three subagents, two of them Codex and one Claude, and a
    /// fourth on the parent's own model. Every record must be distinguishable
    /// from the ones that ran on something else, or the parent cannot attribute
    /// a result and the whole point of mixing executors is gone.
    ///
    /// The two Codex subagents disclose the *same* record on purpose: they are
    /// the same executor, and attribution is to an executor, not to a spawn.
    /// What must differ is Codex from Claude, and either from the parent.
    #[test]
    fn a_mixed_fan_out_is_attributable_record_by_record() {
        let first_codex = external_acp_disclosure("codex-acp");
        let second_codex = external_acp_disclosure("codex-acp");
        let claude = external_acp_disclosure("claude-acp");
        let inherited = native_loop_disclosure(
            "Omega Agent",
            Some("anthropic".to_owned()),
            Some("claude-opus-4".to_owned()),
        );

        assert_eq!(
            first_codex, second_codex,
            "two subagents on the same executor disclose the same executor"
        );
        assert_ne!(first_codex, claude);
        assert_ne!(first_codex.class, inherited.class);
        assert_ne!(claude.class, inherited.class);

        // And the rendered lines differ too, because the line is what a reader
        // actually compares.
        let lines = [first_codex.label(), claude.label(), inherited.label()];
        assert_ne!(lines[0], lines[1]);
        assert_ne!(lines[0], lines[2]);
        assert_ne!(lines[1], lines[2]);

        for disclosure in [&first_codex, &claude, &inherited] {
            assert!(disclosure.is_coherent(), "{disclosure:?}");
        }
    }

    /// An inherited subagent discloses the model that actually served it.
    ///
    /// `subagent_model` overrides the inherited model for every subagent, so
    /// "the parent's model" and "the subagent's model" are not the same fact.
    /// The record must be able to carry the second one.
    #[test]
    fn an_inherited_subagent_discloses_its_own_model() {
        let parent_model = native_loop_disclosure(
            "Omega Agent",
            Some("anthropic".to_owned()),
            Some("claude-opus-4".to_owned()),
        );
        let overridden = native_loop_disclosure(
            "Omega Agent",
            Some("openai".to_owned()),
            Some("gpt-5".to_owned()),
        );
        assert_ne!(parent_model, overridden);
        assert!(overridden.label().contains("gpt-5"));

        // A subagent whose model is not known yet says so, and stays coherent.
        let unknown = native_loop_disclosure("Omega Agent", None, None);
        assert!(unknown.is_coherent(), "{unknown:?}");
        assert!(unknown.label().contains("model not disclosed"));
    }

    #[test]
    fn the_class_of_an_inherited_subagent_is_the_native_loop() {
        assert_eq!(
            SubagentExecutor::InheritParent.class(),
            ExecutorClass::NativeLoop
        );
        assert!(!SubagentExecutor::InheritParent.is_external());
        assert!(
            SubagentExecutor::ExternalAcp {
                id: "codex-acp".into(),
                name: "Codex".into()
            }
            .is_external()
        );
    }
}
