//! The Markets panel's palette entry point stays reachable (omega#244).
//!
//! The sealed interface renders no status-bar panel buttons, so the palette
//! toggle is the panel's only entry point. This test failed before
//! `market::ToggleFocus` was admitted: the zero-base restriction hid the
//! action from the palette and the gate refused its dispatch.

use command_palette_hooks::CommandPaletteFilter;
use gpui::Action as _;
use market_ui::{Reconnect, ToggleFocus};
use omega_zero_base::{ADMITTED_ACTIONS, ADMITTED_NAMESPACES, admits_action};

#[test]
fn the_palette_does_not_hide_the_panel_toggle() {
    let mut filter = CommandPaletteFilter::default();
    filter.restrict_to(ADMITTED_NAMESPACES, ADMITTED_ACTIONS);
    let toggle = ToggleFocus;
    let reconnect = Reconnect;
    assert!(
        !filter.is_hidden(&toggle),
        "the zero-base palette restriction hides market::ToggleFocus"
    );
    assert!(
        !filter.is_hidden(&reconnect),
        "the zero-base palette restriction hides market::Reconnect"
    );
    assert!(admits_action(ToggleFocus.name()));
    assert!(admits_action(Reconnect.name()));
}
