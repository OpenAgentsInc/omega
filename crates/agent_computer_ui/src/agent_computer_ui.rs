//! Minimal Omega Agent Computer launch surface (`OMEGA-AC-02`).
//!
//! One operator-reachable panel starts a cloud turn through supervised
//! `omega_effectd`. GPUI stays projection-only. Credentials stay runtime-only.
//! This is not a Full Auto surface and does not own a second cloud thread store.

mod panel;

pub use panel::{init, AgentComputerPanel};

pub const DEFAULT_CONTROL_PLANE_BASE_URL: &str = "https://openagents.com";
pub const DEFAULT_REPO_REF: &str = "OpenAgentsInc/openagents";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn agent_computer_surface_is_not_full_auto() {
        let names = module_path!();
        assert!(names.contains("agent_computer_ui"));
        assert!(!names.contains("full_auto"));
        assert_eq!(DEFAULT_CONTROL_PLANE_BASE_URL, "https://openagents.com");
        assert_eq!(DEFAULT_REPO_REF, "OpenAgentsInc/openagents");
    }
}
