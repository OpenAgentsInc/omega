//! Zero base — one Exo thread and nothing else.
//!
//! omega#99. The owner asked for one capability to work first: a window that
//! shows Exo, the controls that operate Exo, and nothing else. Zero base is a
//! subtraction of the editor around that thread. It is not a second product,
//! and it is not a second code path through the Exo lane.
//!
//! # Where the mode comes from
//!
//! The process command line, once, at start. Nothing else. It is deliberately
//! *not* a setting: a settings value is writable by a project settings file and
//! by anything else that can write settings, so a mode that hides
//! authority-bearing surfaces would be settable by something that is not the
//! person at the keyboard. `OMEGA-DELTA-0020` records the same objection
//! against a composer mode flag. It is also not a release channel and not a
//! second binary — `OMEGA-DELTA-0038` requires the packaged gate to open every
//! executable that ships, and a second binary doubles that surface.
//!
//! This crate therefore reads no file, no environment variable and no settings
//! store. [`enter_from_command_line`] is the only way in, and the only caller
//! is the argument parser in `crates/zed/src/main.rs`.
//!
//! # What the mode does
//!
//! Three mechanisms, and the distinction decides what breaks:
//!
//! - **Not rendered.** The panels, the status-bar items and the Full Auto entry
//!   are never built. Cheapest, and on its own the most dangerous: the
//!   capability behind an unrendered surface is still one key press away.
//! - **Disabled.** Everything outside [`ADMITTED_NAMESPACES`] and
//!   [`ADMITTED_ACTIONS`] is refused at dispatch with [`refusal`] — one sentence
//!   that names the mode and the way out of it. Never a silent no-op.
//! - **Removed.** Nothing. The same binary is a full editor without the flag.
//!   An unresolvable key binding panics Omega before any window opens, so zero
//!   base deletes no action and edits no keymap file.

#![deny(missing_docs)]

use std::sync::atomic::{AtomicBool, Ordering};

/// The command-line flag that enters zero base, as a person types it.
pub const FLAG: &str = "--zero-base";

/// What a person sees when zero base refuses something, and how they leave.
pub const MODE_NAME: &str = "zero base";

/// The label on the visible control that leaves zero base inside the window.
pub const LEAVE_LABEL: &str = "Leave zero base";

/// The one line the surface shows to say where it is.
pub const BANNER_LABEL: &str = "Zero base";

/// Was the process started with the flag? Written once, by the argument parser.
static ENTERED: AtomicBool = AtomicBool::new(false);

/// Has a person used the visible control to leave? Never written back to true.
static LEFT: AtomicBool = AtomicBool::new(false);

/// The namespaces zero base admits.
///
/// `&'static str` on purpose: the palette filter's admitted set is this exact
/// list, not a list some other code assembles at runtime, so the palette and
/// the action gate cannot drift apart.
///
/// - `agent` is the panel, the threads, the composer's send and the transcript
///   controls. It contains no Full Auto action; those are `full_auto_panel`.
/// - `editor` is the composer, which is an ordinary Omega editor.
/// - `menu`, `picker` and `command_palette` are how the palette opens and how a
///   person moves inside it.
/// - `markdown` is copying out of the transcript.
/// - `omega_zero_base` is the way out.
pub const ADMITTED_NAMESPACES: &[&str] = &[
    "agent",
    "command_palette",
    "editor",
    "markdown",
    "menu",
    "omega_zero_base",
    "picker",
];

/// The individually admitted actions, by full name.
///
/// Window management and font size only. Admitting the whole `omega` namespace
/// would admit the extensions and settings surfaces with it, and section 4 of
/// the design note records that those reach nothing in Omega today.
pub const ADMITTED_ACTIONS: &[&str] = &[
    "omega::DecreaseBufferFontSize",
    "omega::DecreaseUiFontSize",
    "omega::Hide",
    "omega::HideOthers",
    "omega::IncreaseBufferFontSize",
    "omega::IncreaseUiFontSize",
    "omega::Minimize",
    "omega::Quit",
    "omega::ResetBufferFontSize",
    "omega::ResetUiFontSize",
    "omega::ToggleFullScreen",
];

/// Enter zero base, from the parsed command line and from nowhere else.
///
/// Idempotent, and one-way within a process: a person leaves with
/// [`leave`], and nothing re-enters. The mode is never written to disk, so
/// ending the process is always a complete repair.
pub fn enter_from_command_line() {
    ENTERED.store(true, Ordering::SeqCst);
}

/// Leave zero base, from the visible control in the window.
///
/// The full surface comes back in the running window. A viewer who cannot leave
/// a demonstration will not trust the demonstration.
pub fn leave() {
    LEFT.store(true, Ordering::SeqCst);
}

/// Is zero base on right now?
#[must_use]
pub fn is_active() -> bool {
    ENTERED.load(Ordering::SeqCst) && !LEFT.load(Ordering::SeqCst)
}

/// Did this process start in zero base, whether or not a person has left?
#[must_use]
pub fn entered_from_command_line() -> bool {
    ENTERED.load(Ordering::SeqCst)
}

/// Does zero base admit this action name?
///
/// Names are `namespace::Name`. An action with no namespace separator is
/// treated as its own namespace, which is how `Action::name` and the palette
/// filter already split them.
#[must_use]
pub fn admits_action(name: &str) -> bool {
    let namespace = name.split("::").next().unwrap_or(name);
    ADMITTED_NAMESPACES.contains(&namespace) || ADMITTED_ACTIONS.contains(&name)
}

/// The one sentence a refused action answers with.
///
/// It names the action, the mode, and the way out. A refusal a person cannot
/// read is the same thing as a silent no-op.
#[must_use]
pub fn refusal(action_name: &str) -> String {
    format!(
        "{action_name} is off in {MODE_NAME}, which shows one Exo thread and \
         nothing else. Choose \u{201c}{LEAVE_LABEL}\u{201d} in the window, or \
         start Omega without {FLAG}."
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The mode reader names the command line. Nothing here reads a settings
    /// store, an environment variable or a file, and the source check in
    /// `crates/omega_deltas/` asserts the same thing about this file's text.
    #[test]
    fn the_mode_is_off_until_the_command_line_turns_it_on() {
        // The statics are process-wide, so this test owns the transitions and
        // the others below only read derived pure functions.
        assert!(!is_active(), "zero base must be off in a fresh process");
        assert!(!entered_from_command_line());

        enter_from_command_line();
        assert!(is_active());
        assert!(entered_from_command_line());

        leave();
        assert!(!is_active(), "the visible control must leave the mode");
        assert!(
            entered_from_command_line(),
            "leaving does not rewrite how the process started"
        );

        enter_from_command_line();
        assert!(
            !is_active(),
            "nothing re-enters zero base inside a process that left it"
        );
    }

    #[test]
    fn the_admitted_set_is_the_exo_surface_and_the_way_out() {
        for admitted in [
            "agent::Chat",
            "agent::ToggleFocus",
            "editor::Backspace",
            "menu::Confirm",
            "command_palette::Toggle",
            "omega_zero_base::Leave",
            "omega::Quit",
        ] {
            assert!(admits_action(admitted), "{admitted} must be admitted");
        }

        for refused in [
            "full_auto_panel::OpenLauncher",
            "full_auto_panel::ToggleFocus",
            "agent_computer::OpenPanel",
            "project_panel::ToggleFocus",
            "terminal_panel::ToggleFocus",
            "git_panel::ToggleFocus",
            "debugger::Start",
            "workroom::Toggle",
            "omega::Extensions",
            "omega::OpenSettings",
            "workspace::ToggleLeftDock",
        ] {
            assert!(!admits_action(refused), "{refused} must be refused");
        }
    }

    #[test]
    fn a_refusal_is_one_readable_sentence_with_the_way_out() {
        let sentence = refusal("project_panel::ToggleFocus");
        assert!(sentence.contains("project_panel::ToggleFocus"));
        assert!(sentence.contains(MODE_NAME));
        assert!(sentence.contains(LEAVE_LABEL));
        assert!(sentence.contains(FLAG));
    }
}
