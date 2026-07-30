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
///
/// # The window root is `MultiWorkspace`, and this used to miss it
///
/// `OMEGA-DELTA-0118`. Every Omega window is opened with a `MultiWorkspace`
/// root wrapping the `Workspace`, so `downcast::<Workspace>()` on the active
/// window handle answered `None` — always, on every machine. The `let else`
/// returned, no toast was ever shown, and **every refusal this gate has ever
/// made has been silent**. `OMEGA-DELTA-0053` records the owner pressing a
/// denied status-bar control and reporting that "nothing happened"; that is
/// this, and so is the "Toggle Threads Sidebar" entry that `OMEGA-DELTA-0118`
/// repairs. The mode's whole safety argument is that a hidden surface is safe
/// *because* something refuses it out loud, and the out-loud half was off.
fn report_refusal(action_name: &'static str, _cx: &mut App) {
    // omega#119. The log, and nowhere else.
    //
    // This used to raise a toast. Two lanes had independently found that it
    // never worked — it downcast the window to `Workspace`, and every Omega
    // window roots on `MultiWorkspace` — and both fixed it, so refusals became
    // visible for the first time.
    //
    // They were visible for about a minute. The gate refuses actions the
    // *application* dispatches, not only ones a person chose: the owner was
    // typing and got a toast reading "workspace::ActivatePane is off in zero
    // base". He never asked for `ActivatePane`; something incidental did, and
    // announcing it told him nothing he could act on while covering the thing
    // he was doing.
    //
    // A refusal is worth a line in the log, where someone debugging can find
    // it. It is not worth interrupting a person who did not ask the question.
    // The controls that a person *can* deliberately reach are hidden in zero
    // base (`OMEGA-DELTA-0125`), so the loud case this was meant to explain no
    // longer exists.
    log::info!("{}", omega_zero_base::refusal(action_name));
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
