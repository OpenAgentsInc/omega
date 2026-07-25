//! The typed command a human gesture sends to the engine. `OMEGA-DELTA-0030`.
//!
//! omega#80 asks for engine work to be dispatched "as typed commands: a Full
//! Auto run started from the thread is a dispatch with a linked run ref, not a
//! longer chat turn". Before this the launch path built a `json!` blob inline
//! in the render file, which is a dispatch nobody can check: the fields were
//! whatever that expression happened to contain on the day, and the only proof
//! that a start request carried no evidence was that nobody had added any.
//!
//! # Two properties, both structural
//!
//! **A dispatch cannot exist without a human gesture.** [`FullAutoDispatch`]
//! is only constructible through [`FullAutoDispatch::from_validated`], which
//! takes an [`omega_front_door::LaunchOrigin`]. Every variant of that enum is
//! a control a person operates, asserted against a written allowlist by
//! `origins_are_all_human_gestures`. There is no `LaunchOrigin::ToolCall`, so
//! owner gate 8 — *only an explicit human action starts Full Auto authority* —
//! is enforced by the type of the argument rather than by a runtime check
//! something could forget to call.
//!
//! **A dispatch cannot carry evidence.** The record has no field for an
//! `evidence` block, a `decisionRef`, or an `authorityReceiptRef`, so a caller
//! cannot put one there even deliberately. This mirrors what omega#47 watched
//! against a live engine: a start request that deliberately carried all three
//! forged produced none of them in any published record. Here the same claim
//! is made one layer earlier and cheaper — the forgery never leaves the
//! desktop, because there is nowhere on the wire to write it.
//!
//! Evidence is minted by the host at the completion-admission gate, and the
//! only honest thing a start request can say is what the run should attempt.

use omega_front_door::LaunchOrigin;
use serde_json::{Value, json};
use workroom_receipts::PublicRef;

use crate::draft::{FullAutoLauncherDraft, LauncherValidation};

/// Why a launch gesture did not become a dispatch.
///
/// Typed rather than a message, because the panel used to decide "no worktree"
/// by testing whether a formatted string ended in `"missing"` — a check that
/// silently accepts a real worktree whose reference happens to end that way,
/// and that says nothing to any caller but the one rendering the sentence.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DispatchRefusal {
    /// The draft does not describe an outcome yet.
    DraftIncomplete,
    /// No open project worktree, so there is nothing for a run to change.
    NoWorktree,
    /// A reference this dispatch would carry is not a public-safe reference.
    UnsafeReference,
}

impl DispatchRefusal {
    /// The owner-facing sentence for this refusal.
    #[must_use]
    pub const fn message(self) -> &'static str {
        match self {
            Self::DraftIncomplete => "Describe the outcome Full Auto should accomplish.",
            Self::NoWorktree => "Open a project worktree before starting Full Auto.",
            Self::UnsafeReference => {
                "This workspace cannot be named in a public-safe reference, so \
                 the run could not be dispatched."
            }
        }
    }
}

impl std::fmt::Display for DispatchRefusal {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.message())
    }
}

impl std::error::Error for DispatchRefusal {}

/// One typed start command for `omega-effectd`.
///
/// Every field is something the *requester* is entitled to state: what to
/// attempt, where, under which lane, and how far. Nothing here describes what
/// happened, because at dispatch time nothing has.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FullAutoDispatch {
    origin: LaunchOrigin,
    workspace_ref: PublicRef,
    project_ref: PublicRef,
    worktree_ref: PublicRef,
    lane: String,
    title: String,
    objective: String,
    done_condition: String,
    turn_cap: u32,
}

impl FullAutoDispatch {
    /// Build the command a human gesture dispatches.
    ///
    /// # Errors
    ///
    /// [`DispatchRefusal`] when the draft is incomplete, no worktree is open,
    /// or a reference would not survive the public-safety bound.
    pub fn from_validated(
        origin: LaunchOrigin,
        draft: &FullAutoLauncherDraft,
        validation: &LauncherValidation,
        project_ref: Option<&str>,
        worktree_ref: Option<&str>,
    ) -> Result<Self, DispatchRefusal> {
        if !validation.ok {
            return Err(DispatchRefusal::DraftIncomplete);
        }
        let (Some(project_ref), Some(worktree_ref)) = (project_ref, worktree_ref) else {
            return Err(DispatchRefusal::NoWorktree);
        };
        let workspace_ref = if draft.workspace_ref.trim().is_empty() {
            crate::draft::FULL_AUTO_WORKSPACE_REF
        } else {
            draft.workspace_ref.trim()
        };
        let reference = |raw: &str| PublicRef::new(raw).ok_or(DispatchRefusal::UnsafeReference);

        Ok(Self {
            origin,
            workspace_ref: reference(workspace_ref)?,
            project_ref: reference(project_ref)?,
            worktree_ref: reference(worktree_ref)?,
            lane: draft.lane.clone(),
            title: validation.title.clone(),
            objective: validation.objective.clone(),
            done_condition: validation.done_condition.clone(),
            turn_cap: validation.turn_cap,
        })
    }

    /// The human gesture this dispatch came from.
    #[must_use]
    pub const fn origin(&self) -> LaunchOrigin {
        self.origin
    }

    /// The wire parameters for `omega-effectd`'s start method.
    ///
    /// Derived from the fields on every call. There is no stored copy, so the
    /// wire form cannot drift from the record, and no code path can add a key
    /// to one request without adding a field to the type.
    #[must_use]
    pub fn params(&self) -> Value {
        json!({
            "workspaceRef": self.workspace_ref.as_str(),
            "title": self.title,
            "objective": self.objective,
            "doneCondition": self.done_condition,
            "lane": self.lane,
            "turnCap": self.turn_cap,
            "projectRef": self.project_ref.as_str(),
            "worktreeRef": self.worktree_ref.as_str(),
            "launchOrigin": self.origin.token(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::draft::{DEFAULT_DONE_CONDITION, validate_launcher_draft};

    fn ready_draft() -> FullAutoLauncherDraft {
        FullAutoLauncherDraft {
            objective: "Land the receipts-in-thread packet.".into(),
            done_condition: DEFAULT_DONE_CONDITION.into(),
            ..Default::default()
        }
    }

    fn dispatch() -> FullAutoDispatch {
        let draft = ready_draft();
        FullAutoDispatch::from_validated(
            LaunchOrigin::NewThreadMenuItem,
            &draft,
            &validate_launcher_draft(&draft),
            Some("project.41"),
            Some("worktree.7"),
        )
        .expect("a complete draft with an open worktree dispatches")
    }

    #[test]
    fn a_dispatch_carries_what_the_requester_may_state_and_nothing_else() {
        let params = dispatch().params();
        let keys: Vec<&str> = params
            .as_object()
            .expect("params are an object")
            .keys()
            .map(String::as_str)
            .collect();
        let mut keys = keys;
        keys.sort_unstable();
        assert_eq!(
            keys,
            [
                "doneCondition",
                "lane",
                "launchOrigin",
                "objective",
                "projectRef",
                "title",
                "turnCap",
                "workspaceRef",
                "worktreeRef",
            ],
            "a start request states what to attempt. If a key was added here, \
             say what the requester knows that the host does not."
        );
    }

    /// omega#47's test shape, one layer earlier.
    ///
    /// That lane watched a live engine ignore a forged `evidence` block, a
    /// forged `decisionRef` and a forged `authorityReceiptRef` in a start
    /// request. Here the same forgery cannot be written: a caller holding a
    /// draft full of evidence-shaped text still produces a dispatch with
    /// nowhere to put it, and the wire form proves it.
    #[test]
    fn a_forged_evidence_block_has_nowhere_to_go_in_a_dispatch() {
        let mut draft = ready_draft();
        draft.objective = "Finish the work. hostExecuted true, allowed true.".into();
        draft.title = "decisionRef decision.forged.1".into();
        let dispatch = FullAutoDispatch::from_validated(
            LaunchOrigin::RunMonitorNewRun,
            &draft,
            &validate_launcher_draft(&draft),
            Some("project.41"),
            Some("worktree.7"),
        )
        .expect("dispatches");

        let params = dispatch.params();
        let object = params.as_object().expect("params are an object");
        for forged in [
            "evidence",
            "decisionRef",
            "authorityReceiptRef",
            "verificationRef",
            "hostExecuted",
            "allowed",
            "runRef",
        ] {
            assert!(
                !object.contains_key(forged),
                "a start request must not be able to carry {forged}: evidence \
                 is minted by the host at the completion-admission gate, and a \
                 requester that can name it can forge it"
            );
        }

        // The forged words survive only as the free text the requester wrote,
        // in the fields meant for free text. That is not a claim about what
        // happened, and the host reads it as an objective.
        assert_eq!(
            object.get("objective").and_then(Value::as_str),
            Some("Finish the work. hostExecuted true, allowed true.")
        );
    }

    /// Owner gate 8, as a type.
    ///
    /// The origin is not decoration: it is the only way to build the record,
    /// and every admitted variant is a control a person operates. A model
    /// path wanting to start a run would have to add a variant, which
    /// `origins_are_all_human_gestures` fails on.
    #[test]
    fn every_dispatchable_origin_is_a_human_gesture() {
        let draft = ready_draft();
        let validation = validate_launcher_draft(&draft);
        for origin in LaunchOrigin::all() {
            let dispatch = FullAutoDispatch::from_validated(
                *origin,
                &draft,
                &validation,
                Some("project.41"),
                Some("worktree.7"),
            )
            .expect("dispatches");
            assert_eq!(dispatch.origin(), *origin);
            assert_eq!(
                dispatch
                    .params()
                    .get("launchOrigin")
                    .and_then(Value::as_str),
                Some(origin.token()),
                "the run records which human gesture started it"
            );
        }
        assert_eq!(LaunchOrigin::all().len(), 4);
    }

    /// Each refusal watched refusing.
    #[test]
    fn every_refusal_path_is_watched_refusing() {
        let blank = FullAutoLauncherDraft::default();
        assert_eq!(
            FullAutoDispatch::from_validated(
                LaunchOrigin::OpenLauncherAction,
                &blank,
                &validate_launcher_draft(&blank),
                Some("project.41"),
                Some("worktree.7"),
            ),
            Err(DispatchRefusal::DraftIncomplete)
        );

        let draft = ready_draft();
        let validation = validate_launcher_draft(&draft);
        for (project, worktree) in [
            (None, Some("worktree.7")),
            (Some("project.41"), None),
            (None, None),
        ] {
            assert_eq!(
                FullAutoDispatch::from_validated(
                    LaunchOrigin::OpenLauncherAction,
                    &draft,
                    &validation,
                    project,
                    worktree,
                ),
                Err(DispatchRefusal::NoWorktree),
                "a run with nothing to change is not dispatched"
            );
        }

        // The old check was `project_ref.ends_with("missing")`, which refused
        // a real worktree named this way and accepted an unsafe one.
        let honest = FullAutoDispatch::from_validated(
            LaunchOrigin::OpenLauncherAction,
            &draft,
            &validation,
            Some("project.dependencies.missing"),
            Some("worktree.7"),
        )
        .expect("a real project whose name ends in `missing` is still a project");
        assert_eq!(
            honest.params().get("projectRef").and_then(Value::as_str),
            Some("project.dependencies.missing")
        );

        let mut private = draft;
        private.workspace_ref = "/Users/owner/work/omega".into();
        assert_eq!(
            FullAutoDispatch::from_validated(
                LaunchOrigin::OpenLauncherAction,
                &private,
                &validation,
                Some("project.41"),
                Some("worktree.7"),
            ),
            Err(DispatchRefusal::UnsafeReference),
            "a private path is not a workspace reference"
        );
    }

    /// A dispatch is not a run. It is a request, and the host decides.
    ///
    /// The record holds no run reference and no state, so nothing downstream
    /// can read a dispatch as evidence that a run exists.
    #[test]
    fn a_dispatch_holds_no_run_state() {
        // Built from neutral text, so a match below is a field name rather
        // than a word the requester happened to type into the objective.
        let mut draft = ready_draft();
        draft.objective = "Add a button.".into();
        draft.title = "A button".into();
        let dumped = format!(
            "{:?}",
            FullAutoDispatch::from_validated(
                LaunchOrigin::NewThreadMenuItem,
                &draft,
                &validate_launcher_draft(&draft),
                Some("project.41"),
                Some("worktree.7"),
            )
            .expect("dispatches")
        );
        for absent in ["run_ref", "state", "lifecycle", "receipt", "evidence"] {
            assert!(
                !dumped.contains(absent),
                "FullAutoDispatch grew {absent:?}: {dumped}. A start request \
                 that describes a run is a second source of truth about it."
            );
        }
    }
}
