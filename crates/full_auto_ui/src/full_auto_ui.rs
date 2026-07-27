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
    DEFAULT_DONE_CONDITION, DEFAULT_TURN_CAP, FULL_AUTO_ACTIVE_LIMIT, FULL_AUTO_WORKSPACE_REF,
    FullAutoLauncherDraft, LauncherValidation, validate_launcher_draft,
};
pub use evidence_chain::FullAutoEvidenceView;
pub use issue31_adjunct::{
    Issue31FullAutoLiveSources, Issue31FullAutoProjectionError, Issue31HostIdentitySource,
    Issue31HostProjectionError, Issue31HostPublication, project_issue31_full_auto_adjunct,
    publish_issue31_host_snapshot,
};
pub use issue31_delivery::{
    Issue31DeviceMirrorReading, Issue31FullAutoReading, issue31_device_mirror_reading,
    issue31_device_mirror_text, issue31_device_mirror_text_is_safe,
    issue31_host_projection_documents, issue31_host_projection_source,
    issue31_provider_roster_source, latest_issue31_live_reading, set_issue31_live_reading,
};
pub use issue31_observation::{
    Issue31ObservationError, MAX_ISSUE31_PROJECTED_RUNS, observe_issue31_full_auto,
};
pub use panel::FullAutoPanel;
pub use provider_roster::{ProviderAccountRow, parse_provider_accounts};
pub use thread_run_link::{
    THREAD_RUN_LINK_MAX_AGE_MS, ThreadRunLink, ThreadRunRecords, project_thread_run_link,
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
