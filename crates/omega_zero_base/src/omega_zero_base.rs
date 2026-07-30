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
/// identity gate has been answered and the thread is open. The gate is silent
/// now — omega#164 provisions the Nostr identity in the background — but the
/// order is unchanged: the seal happens only after the readiness wait resolves
/// and the thread renders, so nothing is sealed over a window that has not
/// finished becoming the product.
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
/// - `omega_workbench` owns the desktop activity rail and work-surface dock.
///
/// Terminal, pane, buffer-search, and workspace actions remain absent here.
/// The workbench terminal admits terminal input and search actions
/// individually below. Terminal creation uses `omega_workbench`'s
/// thread-bound wrapper instead of admitting a workspace action. Pane actions
/// stay refused because this process-wide gate cannot prove they were
/// dispatched from the terminal.
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
    "omega_workbench",
    "picker",
];

/// The individually admitted actions, by full name.
///
/// Window management, font size, and settings navigation. The settings actions
/// are admitted individually so provider errors can open the real settings
/// window without admitting the extensions surface or the rest of the `omega`
/// namespace. Sarah voice controls are likewise admitted individually: the
/// composer owns those five controls in zero base, while opening or focusing
/// the unrendered workroom remains refused.
///
/// # Why `onboarding::Finish` is no longer here
///
/// omega#99 admitted it because zero base was a **dead end on a fresh
/// profile**: `OMEGA-DELTA-0040` parked startup on `await_identity_ready`, and
/// only the first-run branch of `on_finish` released that wait. omega#164
/// (owner direction 2026-07-29) removed the ceremony the admission completed:
/// startup now provisions the Nostr identity silently in the background and no
/// UI action releases any startup wait, so the dead-end class the admission
/// repaired is structurally impossible and the admission retires with the
/// gate. The whole `onboarding` namespace stays refused — `SignIn` and
/// `OpenAccount` are the hosted account path `OMEGA-DELTA-0010` and
/// `OMEGA-DELTA-0011` removed, and `ResetHints` belongs to the welcome-screen
/// journey zero base does not render.
pub const ADMITTED_ACTIONS: &[&str] = &[
    // OMEGA-DELTA-0175. Clean-profile Vim editor actions, mechanically checked
    // against the shipped keymap. Helix and workspace-management actions stay refused.
    "vim::AngleBrackets",
    "vim::Argument",
    "vim::AutoIndent",
    "vim::BackQuotes",
    "vim::ChangeCase",
    "vim::ChangeListNewer",
    "vim::ChangeListOlder",
    "vim::ChangeToEndOfLine",
    "vim::Class",
    "vim::ClearExchange",
    "vim::ClearOperators",
    "vim::ColumnLeft",
    "vim::ColumnRight",
    "vim::Comment",
    "vim::ConvertToLowerCase",
    "vim::ConvertToRot13",
    "vim::ConvertToUpperCase",
    "vim::CountCommand",
    "vim::CurlyBrackets",
    "vim::CurrentLine",
    "vim::Decrement",
    "vim::DeleteLeft",
    "vim::DeleteRight",
    "vim::DeleteToEndOfLine",
    "vim::DoubleQuotes",
    "vim::Down",
    "vim::EndOfDocument",
    "vim::EndOfLine",
    "vim::EndOfLineDownward",
    "vim::EndOfParagraph",
    "vim::Enter",
    "vim::EntireFile",
    "vim::Exchange",
    "vim::FirstNonWhitespace",
    "vim::GoToColumn",
    "vim::GoToNextReference",
    "vim::GoToPercentage",
    "vim::GoToPreviousReference",
    "vim::HalfPageLeft",
    "vim::HalfPageRight",
    "vim::Increment",
    "vim::Indent",
    "vim::IndentObj",
    "vim::InsertAfter",
    "vim::InsertAtPrevious",
    "vim::InsertBefore",
    "vim::InsertEmptyLineAbove",
    "vim::InsertEmptyLineBelow",
    "vim::InsertEndOfLine",
    "vim::InsertFirstNonWhitespace",
    "vim::InsertFromAbove",
    "vim::InsertFromBelow",
    "vim::InsertLineAbove",
    "vim::InsertLineBelow",
    "vim::JoinLines",
    "vim::JoinLinesNoWhitespace",
    "vim::Left",
    "vim::LineDown",
    "vim::LineUp",
    "vim::Literal",
    "vim::Matching",
    "vim::Method",
    "vim::MiddleOfLine",
    "vim::MiniQuotes",
    "vim::MoveToNext",
    "vim::MoveToNextMatch",
    "vim::MoveToPrevious",
    "vim::MoveToPreviousMatch",
    "vim::NextComment",
    "vim::NextGreaterIndent",
    "vim::NextLesserIndent",
    "vim::NextLineStart",
    "vim::NextMethodEnd",
    "vim::NextMethodStart",
    "vim::NextSameIndent",
    "vim::NextSectionEnd",
    "vim::NextSectionStart",
    "vim::NextWordEnd",
    "vim::NextWordStart",
    "vim::NormalBefore",
    "vim::Number",
    "vim::OtherEnd",
    "vim::OtherEndRowAware",
    "vim::Outdent",
    "vim::PageDown",
    "vim::PageUp",
    "vim::Paragraph",
    "vim::Parentheses",
    "vim::Paste",
    "vim::PreviousComment",
    "vim::PreviousGreaterIndent",
    "vim::PreviousLesserIndent",
    "vim::PreviousLineStart",
    "vim::PreviousMethodEnd",
    "vim::PreviousMethodStart",
    "vim::PreviousSameIndent",
    "vim::PreviousSectionEnd",
    "vim::PreviousSectionStart",
    "vim::PreviousWordEnd",
    "vim::PreviousWordStart",
    "vim::PushAddSurrounds",
    "vim::PushAutoIndent",
    "vim::PushChange",
    "vim::PushChangeSurrounds",
    "vim::PushDelete",
    "vim::PushDeleteSurrounds",
    "vim::PushDigraph",
    "vim::PushFindBackward",
    "vim::PushFindForward",
    "vim::PushForcedMotion",
    "vim::PushIndent",
    "vim::PushJump",
    "vim::PushLiteral",
    "vim::PushLowercase",
    "vim::PushMark",
    "vim::PushObject",
    "vim::PushOppositeCase",
    "vim::PushOutdent",
    "vim::PushRegister",
    "vim::PushReplace",
    "vim::PushReplaceWithRegister",
    "vim::PushReplayRegister",
    "vim::PushRewrap",
    "vim::PushRot13",
    "vim::PushShellCommand",
    "vim::PushToggleBlockComments",
    "vim::PushToggleComments",
    "vim::PushUppercase",
    "vim::PushYank",
    "vim::Quotes",
    "vim::Redo",
    "vim::Repeat",
    "vim::RepeatFind",
    "vim::RepeatFindReversed",
    "vim::ReplayLastRecording",
    "vim::RestoreVisualSelection",
    "vim::Rewrap",
    "vim::Right",
    "vim::ScrollDown",
    "vim::ScrollUp",
    "vim::Search",
    "vim::SearchSubmit",
    "vim::SelectLargerSyntaxNode",
    "vim::SelectNext",
    "vim::SelectNextMatch",
    "vim::SelectPrevious",
    "vim::SelectPreviousMatch",
    "vim::SelectSmallerSyntaxNode",
    "vim::Sentence",
    "vim::SentenceBackward",
    "vim::SentenceForward",
    "vim::ShellCommand",
    "vim::ShowLocation",
    "vim::SquareBrackets",
    "vim::StartOfDocument",
    "vim::StartOfLine",
    "vim::StartOfLineDownward",
    "vim::StartOfParagraph",
    "vim::Substitute",
    "vim::SubstituteLine",
    "vim::SwitchToInsertMode",
    "vim::SwitchToNormalMode",
    "vim::Tab",
    "vim::Tag",
    "vim::TemporaryNormal",
    "vim::ToggleBlockComments",
    "vim::ToggleComments",
    "vim::ToggleRecord",
    "vim::ToggleReplace",
    "vim::ToggleVisual",
    "vim::ToggleVisualBlock",
    "vim::ToggleVisualLine",
    "vim::Undo",
    "vim::UndoLastLine",
    "vim::UndoReplace",
    "vim::UnmatchedBackward",
    "vim::UnmatchedForward",
    "vim::Up",
    "vim::VerticalBars",
    "vim::VisualCommand",
    "vim::VisualDelete",
    "vim::VisualDeleteLine",
    "vim::VisualInsertEndOfLine",
    "vim::VisualInsertFirstNonWhiteSpace",
    "vim::VisualYank",
    "vim::VisualYankLine",
    "vim::WindowBottom",
    "vim::WindowMiddle",
    "vim::WindowTop",
    "vim::Word",
    "vim::WrappingLeft",
    "vim::WrappingRight",
    "vim::YankLine",
    "buffer_search::Deploy",
    "buffer_search::Dismiss",
    "buffer_search::FocusEditor",
    "buffer_search::UseSelectionForFind",
    "omega::About",
    "omega::AcpRegistry",
    "omega::DecreaseBufferFontSize",
    "omega::DecreaseUiFontSize",
    "omega::Hide",
    "omega::HideOthers",
    "omega::IncreaseBufferFontSize",
    "omega::IncreaseUiFontSize",
    "omega::Minimize",
    "omega::OpenDocs",
    "omega::OpenLicenses",
    "omega::OpenSettings",
    "omega::OpenLegacySettings",
    "omega::OpenSettingsAt",
    "omega::OpenSettingsPage",
    "omega::Quit",
    "omega::ResetBufferFontSize",
    "omega::ResetUiFontSize",
    "omega::ToggleFullScreen",
    "search::SelectNextMatch",
    "search::SelectPreviousMatch",
    "terminal::Clear",
    "terminal::Copy",
    "terminal::Paste",
    "terminal::PasteText",
    "terminal::RenameTerminal",
    "terminal::RerunTask",
    "terminal::ScrollLineDown",
    "terminal::ScrollLineUp",
    "terminal::ScrollPageDown",
    "terminal::ScrollPageUp",
    "terminal::ScrollToBottom",
    "terminal::ScrollToTop",
    "terminal::SelectAll",
    "terminal::SendKeystroke",
    "terminal::SendText",
    "terminal::ShowCharacterPalette",
    "terminal::ToggleViMode",
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
    // point is choosing what the thread can see, not opening arbitrary editor
    // surfaces.
    "workspace::Open",
    // OMEGA-DELTA-0175, omega#149. Vim is a kept part of the default surface.
    // Its toggle lives in the workspace namespace so it remains available
    // while modal editing is off; admit that exact action without admitting
    // Helix's toggle or the rest of Workspace.
    "workspace::ToggleVimMode",
    // The scoped Files surface emits this instead of Workspace's global
    // OpenTerminal action. Agent Panel validates it against the active thread
    // binding; a centre-pane context menu therefore cannot bypass ownership.
    "project_panel::OpenInThreadTerminal",
    // OMEGA-DELTA-0139, omega#119. A plain transcript file-link click reveals
    // the ordinary editable centre pane. Admit its standard save action
    // without admitting the rest of the workspace namespace.
    "workspace::Save",
    "workroom::PrepareVoiceAdmission",
    "workroom::PrepareVoiceAdmission",
    "workroom::StartVoice",
    "workroom::ApproveSarahVoiceCommand",
    "workroom::RejectSarahVoiceCommand",
    "workroom::ToggleVoiceMute",
    "workroom::InterruptVoice",
    "workroom::EndVoice",
    "workroom::RetryVoice",
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
/// `OMEGA-DELTA-0053`. While this is true the workspace starts with no editor
/// pane, tab bar, title bar, or status bar. They are not merely covered by a
/// zoomed panel. `OMEGA-DELTA-0139` introduced the explicit editable-center
/// exception, and `OMEGA-DELTA-0174` applies it to user-triggered center opens:
/// the center remains beside the agent surface until its final tab closes.
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
            "omega_workbench::SelectFiles",
            "omega_workbench::CollapseWorkSurfaceDock",
            "omega_workbench::NewTerminalForThread",
            // OMEGA-DELTA-0175. This is the complete unique action inventory
            // from the rc session that exposed the broken gate. Every one is
            // admitted by the scoped Vim action table.
            "vim::PushFindForward",
            "vim::InsertBefore",
            "vim::Down",
            "vim::InsertLineBelow",
            "vim::Substitute",
            "vim::PushDelete",
            "vim::InsertAfter",
            "vim::Number",
            "vim::Up",
            "vim::PreviousLineStart",
            "vim::InsertLineAbove",
            "project_panel::OpenInThreadTerminal",
            "terminal::SendKeystroke",
            "terminal::Copy",
            "buffer_search::Deploy",
            "omega::About",
            "omega::AcpRegistry",
            "omega::OpenDocs",
            "omega::OpenLicenses",
            "omega::Quit",
            "omega::OpenSettings",
            "omega::OpenLegacySettings",
            "omega::OpenSettingsAt",
            "omega::OpenSettingsPage",
            "workroom::PrepareVoiceAdmission",
            "workroom::PrepareVoiceAdmission",
            "workroom::StartVoice",
            "workroom::ApproveSarahVoiceCommand",
            "workroom::RejectSarahVoiceCommand",
            "workroom::ToggleVoiceMute",
            "workroom::InterruptVoice",
            "workroom::EndVoice",
            "workroom::RetryVoice",
            "workspace::ToggleVimMode",
            "workspace::Save",
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
            "workroom::OpenPanel",
            "omega::Extensions",
            "workspace::ToggleLeftDock",
            "workspace::ToggleRightDock",
            "workspace::ToggleBottomDock",
            "workspace::CloseWindow",
            "pane::ActivateItem",
            "pane::ActivateLastItem",
            "pane::ActivateNextItem",
            "pane::ActivatePreviousItem",
            "pane::CloseActiveItem",
            "pane::CloseAllItems",
            "pane::SplitDown",
            "pane::SplitLeft",
            "pane::SplitRight",
            "pane::SplitUp",
            "terminal_panel::Toggle",
            "terminal::SearchTest",
            "buffer_search::DeployReplace",
            "workspace::NewTerminal",
            "workspace::OpenTerminal",
            "workspace::ToggleHelixMode",
            // omega#164, owner direction 2026-07-29. The identity gate is
            // silent: startup provisions the Nostr identity in the background
            // and no UI action releases any startup wait, so the omega#99
            // `onboarding::Finish` admission retired with the ceremony it
            // completed. The whole namespace is refused — `SignIn` and
            // `OpenAccount` are the hosted account path OMEGA-DELTA-0010 and
            // OMEGA-DELTA-0011 removed.
            "onboarding::Finish",
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

    #[test]
    fn workbench_terminal_keymaps_win_without_admitting_pane_actions() {
        const AGENT_TERMINAL_CONTEXT: &str = "\"context\": \"AgentPanel > Terminal\"";
        const WORKBENCH_TERMINAL_CONTEXT: &str = "\"context\": \"WorkbenchTerminal > Terminal\"";

        for (platform, keymap) in [
            (
                "linux",
                include_str!("../../../assets/keymaps/default-linux.json"),
            ),
            (
                "macos",
                include_str!("../../../assets/keymaps/default-macos.json"),
            ),
            (
                "windows",
                include_str!("../../../assets/keymaps/default-windows.json"),
            ),
        ] {
            let Some(agent_context_offset) = keymap.find(AGENT_TERMINAL_CONTEXT) else {
                panic!("{platform} keymap must contain the agent terminal context");
            };
            let Some(workbench_context_offset) = keymap.find(WORKBENCH_TERMINAL_CONTEXT) else {
                panic!("{platform} keymap must contain the workbench terminal context");
            };
            assert!(
                workbench_context_offset > agent_context_offset,
                "{platform} must place workbench terminal bindings after the broader agent-panel \
                 terminal bindings so new-terminal and search are not shadowed"
            );

            let Some(workbench_tail) = keymap.get(workbench_context_offset..) else {
                panic!("{platform} workbench terminal context offset must be in bounds");
            };
            let workbench_section = workbench_tail
                .split("\n  },")
                .next()
                .unwrap_or(workbench_tail);
            assert!(
                workbench_section.contains("omega_workbench::NewTerminalForThread"),
                "{platform} must create terminals through the thread-bound workbench action"
            );
            for action in [
                "omega_workbench::ActivatePreviousTerminalTab",
                "omega_workbench::ActivateNextTerminalTab",
                "omega_workbench::CloseActiveTerminalTab",
            ] {
                assert!(
                    workbench_section.contains(action),
                    "{platform} must route native terminal tab shortcuts through {action}"
                );
            }
            assert!(
                !workbench_section.contains("workspace::NewTerminal")
                    && !workbench_section.contains("pane::"),
                "{platform} workbench terminal bindings must not rely on globally admitted \
                 workspace or pane actions"
            );
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
