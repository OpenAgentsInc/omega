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

use omega_front_door::ExecutorClass;
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
            Self::ExternalAcp { .. } => ExecutorClass::ExternalAcp,
        }
    }

    #[must_use]
    pub fn is_external(&self) -> bool {
        matches!(self, Self::ExternalAcp { .. })
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

/// [`resolve_subagent_executor`] against the agents actually on this machine.
///
/// The installed set comes from `omega_agent_detect`, a `PATH` probe, and not
/// from `AllAgentServersSettings`. Settings record what is *configured*, which
/// is not evidence of presence: a fresh `--user-data-dir` has no settings
/// written whatever is installed, so a settings-based check would offer
/// nothing on exactly the machine a new person is using.
#[must_use]
pub fn resolve_requested_executor(requested: Option<&str>) -> ExecutorResolution {
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
