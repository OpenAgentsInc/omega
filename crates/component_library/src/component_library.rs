//! A gated development screen that renders the `component` registry.
//!
//! This is the successor to the deleted `component_preview` crate
//! (OMEGA-DELTA-0022 / OMEGA-DELTA-0186): that surface shipped ungated in a
//! release command palette and rendered unreviewed artwork. This one uses new
//! crate and action names, is admitted through the `omega_workbench`
//! namespace, and is doubly gated — a runtime gate (`debug_assertions` plus
//! `OMEGA_COMPONENT_LIBRARY=1`) and compile-time omission, so release
//! binaries carry no trace of the screen. See OMEGA-DELTA-0233 and omega#247.

use gpui::App;

/// The environment variable that requests the component library in a
/// development build.
pub const COMPONENT_LIBRARY_ENV: &str = "OMEGA_COMPONENT_LIBRARY";

/// The dual development gate for the component library surface.
///
/// A runtime check alone is not enough (omega#220's lesson for fixtures):
/// the screen module itself is also compiled out of release builds, so the
/// runtime half only ever runs where the compile-time half admitted the code.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ComponentLibraryGate {
    debug_assertions: bool,
    requested: bool,
}

impl ComponentLibraryGate {
    pub fn from_process_environment() -> Self {
        Self::from_runtime_state(
            cfg!(debug_assertions),
            std::env::var(COMPONENT_LIBRARY_ENV).as_deref() == Ok("1"),
        )
    }

    pub const fn from_runtime_state(debug_assertions: bool, requested: bool) -> Self {
        Self {
            debug_assertions,
            requested,
        }
    }

    pub const fn enabled(self) -> bool {
        self.debug_assertions && self.requested
    }
}

#[cfg(any(debug_assertions, test))]
mod library;

#[cfg(any(debug_assertions, test))]
pub use library::{ComponentLibrary, OpenComponentLibrary};

#[cfg(any(debug_assertions, test))]
pub fn init(cx: &mut App) {
    library::init(cx);
}

#[cfg(not(any(debug_assertions, test)))]
pub fn init(_cx: &mut App) {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_gate_requires_both_debug_assertions_and_the_request() {
        assert!(!ComponentLibraryGate::from_runtime_state(false, false).enabled());
        assert!(!ComponentLibraryGate::from_runtime_state(true, false).enabled());
        assert!(!ComponentLibraryGate::from_runtime_state(false, true).enabled());
        assert!(ComponentLibraryGate::from_runtime_state(true, true).enabled());
    }

    #[test]
    fn a_release_binary_compiles_the_screen_out() {
        let source = include_str!("component_library.rs");
        assert!(
            source.contains("#[cfg(any(debug_assertions, test))]\nmod library;"),
            "the library module must be compile-time gated, not only runtime gated"
        );
    }
}
