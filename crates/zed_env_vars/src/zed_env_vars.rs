pub use env_var::{EnvVar, bool_env_var, env_var};
use std::sync::LazyLock;

/// Prefer an `OMEGA_*` env var; fall back to the inherited `ZED_*` name.
///
/// OMEGA-DELTA-0190 / omega#174: bundle scripts and local launchers set both
/// during the transition. Application code always reads through this helper so
/// a pure `OMEGA_*` environment is enough and a pure `ZED_*` environment still
/// works for legacy tooling.
fn omega_or_zed(omega_name: &str, zed_name: &str) -> EnvVar {
    EnvVar::new(omega_name.into()).or(EnvVar::new(zed_name.into()))
}

fn omega_or_zed_bool(omega_name: &str, zed_name: &str) -> bool {
    omega_or_zed(omega_name, zed_name).value.is_some()
}

/// Whether Omega is running in stateless mode.
/// When true, Omega will use in-memory databases instead of persistent storage.
///
/// Honors `OMEGA_STATELESS` first, then `ZED_STATELESS`.
pub static ZED_STATELESS: LazyLock<bool> =
    LazyLock::new(|| omega_or_zed_bool("OMEGA_STATELESS", "ZED_STATELESS"));

/// Alias preferred by new callers.
pub static OMEGA_STATELESS: LazyLock<bool> = LazyLock::new(|| *ZED_STATELESS);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn omega_env_takes_precedence_over_zed() {
        // Safety: tests run single-threaded for this module and restore env.
        unsafe {
            std::env::remove_var("OMEGA_STATELESS");
            std::env::remove_var("ZED_STATELESS");
        }
        // Cannot re-init LazyLock; just unit-test the helper.
        unsafe {
            std::env::set_var("ZED_STATELESS", "1");
            std::env::set_var("OMEGA_STATELESS", "");
        }
        // empty OMEGA counts as unset via EnvVar::new
        let only_zed = omega_or_zed("OMEGA_STATELESS", "ZED_STATELESS");
        assert!(only_zed.value.is_some());

        unsafe {
            std::env::set_var("OMEGA_STATELESS", "1");
            std::env::set_var("ZED_STATELESS", "should-not-win");
        }
        let preferred = omega_or_zed("OMEGA_STATELESS", "ZED_STATELESS");
        assert_eq!(preferred.value.as_deref(), Some("1"));

        unsafe {
            std::env::remove_var("OMEGA_STATELESS");
            std::env::remove_var("ZED_STATELESS");
        }
    }
}
