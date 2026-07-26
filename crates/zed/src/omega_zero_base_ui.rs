//! The zero-base surface: the gate that refuses and the palette restriction.
//!
//! omega#99. The mode itself lives in `crates/omega_zero_base`, which reads the
//! process command line and nothing else. This module is what a person sees and
//! touches: what the palette lists and what a refused key binding says.
//!
//! `OMEGA-DELTA-0052`, omega#100. This module used to own a third thing — the
//! status-bar control that put the editor back. The owner asked for it to go:
//! *"remove the 'zero base / leave zero base' buttons. they must be stuck in
//! zero base with no way out if it was started in this mode."* So the control,
//! the `Leave` action, and the runtime unwind that put the panels back are all
//! removed. What is left is the half that was always the load-bearing half:
//! a hidden surface is only safe while something refuses it at dispatch.

use gpui::App;
use omega_zero_base::{ADMITTED_ACTIONS, ADMITTED_NAMESPACES};
use workspace::{Toast, Workspace, notifications::NotificationId};

/// Install the refusal gate and the palette restriction, once, at app init.
///
/// Only called when the process started in zero base. Without that, nothing
/// here runs and Omega's behaviour is byte-identical to a build that never had
/// this module.
pub fn init(cx: &mut App) {
    restrict_command_palette(cx);
    install_action_gate(cx);
}

/// Restrict the palette to the admitted set.
///
/// `hide_namespace` is what `agent.enabled` uses, and it stays exactly where it
/// was: this adds a separate admitted set rather than replacing the denylist,
/// so a settings change that hides the agent namespace still hides it here.
fn restrict_command_palette(cx: &mut App) {
    command_palette_hooks::CommandPaletteFilter::update_global(cx, |filter, _| {
        filter.restrict_to(ADMITTED_NAMESPACES, ADMITTED_ACTIONS);
    });
}

/// Refuse every action outside the admitted set, with a sentence.
///
/// The gate runs before any listener, so this is what makes "not rendered" safe:
/// a surface that is only visually absent is still one key press away, and the
/// key press lands here instead.
fn install_action_gate(cx: &mut App) {
    cx.set_action_gate(|action, cx| {
        if !omega_zero_base::is_active() {
            return true;
        }
        let name = action.name();
        if omega_zero_base::admits_action(name) {
            return true;
        }
        report_refusal(name, cx);
        false
    });
}

/// Say why, where a person reads it.
///
/// Deferred because the gate runs inside a window update and a toast is a
/// second one. A refusal a person cannot read is a silent no-op with extra
/// steps, so this also logs at `info` for the case where no window is up.
fn report_refusal(action_name: &'static str, cx: &mut App) {
    let sentence = omega_zero_base::refusal(action_name);
    log::info!("{sentence}");
    cx.defer(move |cx| {
        let Some(workspace) = cx
            .active_window()
            .and_then(|window| window.downcast::<Workspace>())
        else {
            return;
        };
        workspace
            .update(cx, |workspace, _window, cx| {
                struct ZeroBaseRefusal;
                workspace.show_toast(
                    Toast::new(NotificationId::unique::<ZeroBaseRefusal>(), sentence).autohide(),
                    cx,
                );
            })
            .ok();
    });
}

// OMEGA-DELTA-0052. The way out used to live below this line: a status-bar
// item, the workspace installer that added it, the action it dispatched, and
// the unwind that lifted the palette restriction and the action gate and
// un-zoomed the panel. All of it is gone rather than hidden.
//
// Hiding the control would have been the cheap version — one `when(false)` and
// the button disappears — and it would have left the mode's exit on the crate,
// the action in the registry, and the restriction still liftable at runtime.
// The owner asked for no way out, and a way out that is merely not drawn is
// still one dispatch away. That is the exact reasoning `OMEGA-DELTA-0048` uses
// about every other hidden surface, and the answer here is different only
// because this one is not hidden: it does not exist.
//
// Nothing else was deleted. The mode still hides by filter and refusal and
// still removes no keymap binding, because the removed action was Omega's own
// with no shipped binding — `keymaps_name_no_deleted_action` covers the rest.
