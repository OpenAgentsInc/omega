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
//! # Which way the default points
//!
//! `OMEGA-DELTA-0052`, omega#100. Zero base is what `omega` does with no
//! arguments, and the editor is what [`FULL_EDITOR_FLAG`] asks for. The owner
//! said it in as many words: *"they must be stuck in zero base with no way out
//! if it was started in this mode. which must be the default starting now.
//! booting the full editor must require a separate flag."*
//!
//! The reader did not move. `OMEGA-DELTA-0047` is about *where* the mode is
//! read from — the parsed command line, once, never a settings key — and that
//! is unchanged. What changed is which way the boolean points when nobody says
//! anything.
//!
//! # There is no way out
//!
//! `OMEGA-DELTA-0052`. A process that starts in zero base stays in zero base
//! until it exits. There is no `leave`, no status-bar control, and no action
//! that puts the editor back, because the owner asked for exactly that. Ending
//! the process is still a complete repair: the mode is never written to disk.
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
//!   that names the mode and how to start Omega without it. Never a silent
//!   no-op.
//! - **Removed.** Nothing. The same binary is a full editor with
//!   [`FULL_EDITOR_FLAG`]. An unresolvable key binding panics Omega before any
//!   window opens, so zero base deletes no action and edits no keymap file.

#![deny(missing_docs)]

use std::sync::atomic::{AtomicBool, Ordering};

/// The command-line flag that used to enter zero base, as a person types it.
///
/// `OMEGA-DELTA-0052`. Zero base is the default now, so this flag asks for what
/// it already gets. It is still accepted, and it still does nothing else, so
/// commands and scripts that carry it keep working.
pub const FLAG: &str = "--zero-base";

/// The command-line flag that asks for the editor around the thread.
///
/// `OMEGA-DELTA-0052`. The one way to start Omega as a full editor.
pub const FULL_EDITOR_FLAG: &str = "--full-editor";

/// What a person sees when zero base refuses something.
pub const MODE_NAME: &str = "zero base";

/// Is this process in zero base? Written once, by the argument parser.
///
/// One-way for the life of the process. There is no companion static for
/// leaving, because `OMEGA-DELTA-0052` removed leaving.
static ENTERED: AtomicBool = AtomicBool::new(false);

/// Has zero base's own surface taken the window over yet?
///
/// `OMEGA-DELTA-0053`. Written once, by the workspace initialiser, after the
/// identity gate has been answered and the thread is open. Before that the
/// ordinary workspace has to render, because `OMEGA-DELTA-0040`'s identity
/// onboarding is a centre-pane item and a window with no centre pane could
/// never show it — which would be a worse dead end than the one
/// `OMEGA-DELTA-0051` just repaired.
static SEALED: AtomicBool = AtomicBool::new(false);

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
///
/// `OMEGA-DELTA-0052` removed `omega_zero_base` from this list. That namespace
/// held one action, `Leave`, and there is no leaving now. An admitted namespace
/// with nothing in it is a door left standing where the room was demolished.
pub const ADMITTED_NAMESPACES: &[&str] = &[
    "agent",
    "command_palette",
    "editor",
    "markdown",
    "menu",
    "picker",
];

/// The individually admitted actions, by full name.
///
/// Window management, font size, and the one control that finishes identity
/// onboarding. Admitting the whole `omega` namespace would admit the extensions
/// and settings surfaces with it, and section 4 of the design note records that
/// those reach nothing in Omega today.
///
/// # Why `onboarding::Finish` is here
///
/// omega#99. Without it, zero base is a **dead end on a fresh profile**.
/// `OMEGA-DELTA-0040` puts a first-ever launch on identity onboarding and parks
/// startup on `await_identity_ready`; the only thing that releases that wait is
/// the first-run branch of `on_finish`, which runs from `onboarding::Finish`.
/// The gate refused that action, so a new user reached the identity page,
/// created an identity, pressed "Finish Setup", and nothing happened —
/// forever, across restarts, because identity never became ready. Observed
/// directly: the log line reads `onboarding::Finish is off in zero base`.
///
/// This admits *completing* the identity gate, which is the opposite of
/// bypassing it. `SignIn` and `OpenAccount` stay refused — they are the hosted
/// account path `OMEGA-DELTA-0010` and `OMEGA-DELTA-0011` removed — and
/// `ResetHints` stays refused because it belongs to the welcome-screen journey
/// zero base does not render.
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
    "onboarding::Finish",
    // OMEGA-DELTA-0054, omega#100. The one control that gives the thread a
    // folder to work in.
    //
    // Zero base opens the directory it was started in, and from Finder or the
    // Dock there is no such directory. A thread whose file tools have nothing
    // to read is the state the owner met, and a person must be able to leave it
    // without restarting Omega from a shell. This admits a system folder
    // picker, which is not a setup page: nothing is asked before the thread
    // opens, and the control is beside the composer only while there is no
    // folder.
    //
    // `workspace::OpenFiles` and the rest of the namespace stay refused. The
    // point is choosing what the thread can see, not opening the editor's file
    // surfaces inside a mode that does not render them.
    "workspace::Open",
];

/// Enter zero base, from the parsed command line and from nowhere else.
///
/// Idempotent, and one-way for the life of the process: `OMEGA-DELTA-0052`
/// removed the way out, so nothing turns this off again. The mode is never
/// written to disk, so ending the process is always a complete repair.
///
/// The name still says where the mode comes from, and that is still true after
/// `OMEGA-DELTA-0052`: the caller is the argument parser, and it calls this when
/// the parsed command line asked for no editor. A default is a decision made
/// from the command line as much as a flag is.
pub fn enter_from_command_line() {
    ENTERED.store(true, Ordering::SeqCst);
}

/// Is zero base on right now?
#[must_use]
pub fn is_active() -> bool {
    ENTERED.load(Ordering::SeqCst)
}

/// Did this process start in zero base?
///
/// The same answer as [`is_active`] since `OMEGA-DELTA-0052`, and kept separate
/// because the two questions are asked for different reasons: this one decides
/// what to install once at startup, and [`is_active`] is asked on every render.
#[must_use]
pub fn entered_from_command_line() -> bool {
    ENTERED.load(Ordering::SeqCst)
}

/// Zero base's surface now owns the window. One-way, like entry.
///
/// `OMEGA-DELTA-0053`. Called by the workspace initialiser once the identity
/// gate is answered and the thread is open. It does nothing outside zero base,
/// so a build started with the editor flag cannot be sealed by a stray call.
pub fn seal() {
    if is_active() {
        SEALED.store(true, Ordering::SeqCst);
    }
}

/// Has zero base's surface taken the window over?
///
/// `OMEGA-DELTA-0053`. While this is true the workspace renders no editor pane,
/// no tab bar, no title bar and no status bar — they are **not rendered**,
/// rather than covered by a zoomed panel. Hiding the editor by zooming a panel
/// over it was the previous mechanism, and one sidebar toggle released the zoom
/// and put Zed's whole "Welcome back" surface on the screen, with New File,
/// Open Project, Open Settings and Explore Extensions on it, inside a mode
/// whose premise is that those are not present.
#[must_use]
pub fn is_sealed() -> bool {
    is_active() && SEALED.load(Ordering::SeqCst)
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
/// It names the action, the mode, and how to start Omega without the mode. A
/// refusal a person cannot read is the same thing as a silent no-op.
///
/// `OMEGA-DELTA-0052` changed the second half of the sentence. It used to name
/// a control in the window, and that control is gone: there is no way out of a
/// running zero base, so a sentence that offered one would be a lie a person
/// would go looking for.
#[must_use]
pub fn refusal(action_name: &str) -> String {
    format!(
        "{action_name} is off in {MODE_NAME}, which shows one Exo thread and \
         nothing else. Start Omega with {FULL_EDITOR_FLAG} for the editor."
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The mode reader names the command line, and once it is on it stays on.
    ///
    /// `OMEGA-DELTA-0052`. This test used to end by leaving and proving nothing
    /// re-entered. There is no leaving now, so it ends by proving the opposite
    /// property: the mode is one-way for the life of the process.
    ///
    /// The static starts `false` here even though zero base is the shipped
    /// default, and that is the point of the split: the default lives in the
    /// argument parser in `crates/zed/src/main.rs`, not in this crate. A crate
    /// that defaulted itself to on would be a second reader of the mode, which
    /// `OMEGA-DELTA-0047` refuses.
    #[test]
    fn the_mode_is_off_until_the_command_line_turns_it_on() {
        // The statics are process-wide, so this test owns the transitions and
        // the others below only read derived pure functions.
        assert!(!is_active(), "zero base must be off in a fresh process");
        assert!(!entered_from_command_line());

        enter_from_command_line();
        assert!(is_active());
        assert!(entered_from_command_line());

        enter_from_command_line();
        assert!(
            is_active(),
            "entering twice is the same as entering once, and nothing turns the \
             mode off inside a process"
        );

        // OMEGA-DELTA-0053. Sealing is a second one-way step, after the
        // identity gate, and it is what stops the editor being one un-zoom
        // away. It happens inside this test because the statics are
        // process-wide and this test owns the transitions.
        assert!(
            !is_sealed(),
            "entering zero base does not seal the window on its own; the \
             identity gate renders in the ordinary workspace first"
        );
        seal();
        assert!(is_sealed());
    }

    #[test]
    fn the_admitted_set_is_the_exo_surface_and_nothing_else() {
        for admitted in [
            "agent::Chat",
            "agent::ToggleFocus",
            "editor::Backspace",
            "menu::Confirm",
            "command_palette::Toggle",
            "omega::Quit",
            // omega#99. The action that releases `await_identity_ready`.
            // Without it a fresh profile can reach identity onboarding and
            // never leave it, which is a dead end rather than a subtraction.
            "onboarding::Finish",
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
            // The hosted-account path OMEGA-DELTA-0010 and OMEGA-DELTA-0011
            // removed. Admitting `onboarding::Finish` must not drag the rest of
            // its namespace in with it.
            "onboarding::SignIn",
            "onboarding::OpenAccount",
            "onboarding::ResetHints",
            // OMEGA-DELTA-0052. The namespace that held the way out. There is
            // no `Leave` action any more, and admitting a namespace with
            // nothing in it would leave the door frame standing.
            "omega_zero_base::Leave",
        ] {
            assert!(!admits_action(refused), "{refused} must be refused");
        }
    }

    /// `OMEGA-DELTA-0052`. The sentence names how to start Omega differently,
    /// and never a control in the window.
    #[test]
    fn a_refusal_is_one_readable_sentence_naming_the_editor_flag() {
        let sentence = refusal("project_panel::ToggleFocus");
        assert!(sentence.contains("project_panel::ToggleFocus"));
        assert!(sentence.contains(MODE_NAME));
        assert!(sentence.contains(FULL_EDITOR_FLAG));
        assert!(
            !sentence.contains("Leave"),
            "there is no way out of a running zero base, so the refusal must \
             not send a person looking for a button that does not exist: {sentence}"
        );
    }
}
