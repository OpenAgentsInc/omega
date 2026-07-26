//! Omega GPUI Full Auto launcher and concurrent run monitor (`OMEGA-FA-03`).
//!
//! Full Auto is a dedicated run surface. It is never a composer toggle.
//! Durable mutation goes only through supervised `omega_effectd`.

mod dispatch;
mod draft;
mod evidence_chain;
mod issue31_adjunct;
mod issue31_delivery;
mod issue31_observation;
mod panel;
mod provider_roster;
mod thread_run_link;

pub use dispatch::{DispatchRefusal, FullAutoDispatch};
pub use draft::{
    validate_launcher_draft, FullAutoLauncherDraft, LauncherValidation, DEFAULT_DONE_CONDITION,
    DEFAULT_TURN_CAP, FULL_AUTO_ACTIVE_LIMIT, FULL_AUTO_WORKSPACE_REF,
};
pub use evidence_chain::FullAutoEvidenceView;
pub use issue31_adjunct::{
    publish_issue31_host_snapshot, Issue31HostIdentitySource, Issue31HostProjectionError,
    Issue31HostPublication,
    project_issue31_full_auto_adjunct, Issue31FullAutoLiveSources, Issue31FullAutoProjectionError,
};
pub use issue31_delivery::{
    issue31_host_projection_documents, issue31_host_projection_source,
    issue31_provider_roster_source,
    latest_issue31_live_reading, set_issue31_live_reading, Issue31FullAutoReading,
};
pub use issue31_observation::{
    observe_issue31_full_auto, Issue31ObservationError, MAX_ISSUE31_PROJECTED_RUNS,
};
pub use panel::FullAutoPanel;
pub use provider_roster::{parse_provider_accounts, ProviderAccountRow};
pub use thread_run_link::{
    project_thread_run_link, ThreadRunLink, ThreadRunRecords, THREAD_RUN_LINK_MAX_AGE_MS,
};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn launcher_requires_objective_and_rejects_blank_mission() {
        let mut draft = FullAutoLauncherDraft::default();
        assert!(!validate_launcher_draft(&draft).ok);
        draft.objective = "Ship the Omega Full Auto launcher.".into();
        draft.done_condition = DEFAULT_DONE_CONDITION.into();
        assert!(validate_launcher_draft(&draft).ok);
    }

    /// Product law, restated for `OMEGA-DELTA-0020`.
    ///
    /// The owner asked for Full Auto to be folded into the Omega chat UI, so
    /// "dedicated panel" is no longer the right half of this law. The half
    /// that survives is the one with teeth: Full Auto authority is a dedicated
    /// *entry*, never a flag on a chat draft. A flag is a boolean the send
    /// path reads, so anything that can set it can start a run — a slash
    /// command, a restored draft, a model-authored composer insertion. Owner
    /// gate 8 forbids exactly that.
    ///
    /// The earlier version of this test asserted `module_path!()` contains
    /// `full_auto_ui`, which is true of any test in this crate and therefore
    /// checked nothing. It is replaced with a check that can fail: the draft
    /// carries no field the send path could read as "run this automatically".
    #[test]
    fn full_auto_is_not_a_composer_mode_flag() {
        let draft = format!("{:?}", FullAutoLauncherDraft::default());
        for flag in [
            "full_auto: ",
            "auto: true",
            "autonomous",
            "composer_mode",
            "send_starts_run",
        ] {
            assert!(
                !draft.contains(flag),
                "FullAutoLauncherDraft grew {flag:?}: {draft}. Full Auto is \
                 reached by a dedicated entry and started by a dedicated \
                 button, never by a flag a send path reads."
            );
        }
        assert_eq!(FULL_AUTO_ACTIVE_LIMIT, 8);
    }
}
