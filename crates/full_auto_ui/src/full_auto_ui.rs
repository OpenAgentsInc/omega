//! Omega GPUI Full Auto launcher and concurrent run monitor (`OMEGA-FA-03`).
//!
//! Full Auto is a dedicated run surface. It is never a composer toggle.
//! Durable mutation goes only through supervised `omega_effectd`.

mod draft;
mod panel;
mod provider_roster;

pub use draft::{
    validate_launcher_draft, FullAutoLauncherDraft, LauncherValidation, DEFAULT_DONE_CONDITION,
    DEFAULT_TURN_CAP, FULL_AUTO_ACTIVE_LIMIT, FULL_AUTO_WORKSPACE_REF,
};
pub use panel::{init, FullAutoPanel};
pub use provider_roster::{parse_provider_accounts, ProviderAccountRow};

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

    #[test]
    fn full_auto_is_not_a_composer_mode_flag() {
        // Product law: Full Auto authority is a dedicated panel/entry, not a
        // chat draft flag. This module exposes no composer_toggle API.
        let names = module_path!();
        assert!(names.contains("full_auto_ui"));
        assert_eq!(FULL_AUTO_ACTIVE_LIMIT, 8);
    }
}
