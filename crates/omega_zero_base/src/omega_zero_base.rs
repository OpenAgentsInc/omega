//! The Omega surface — one agent thread and the controls that operate it.
//!
//! omega#99 built this as a mode: a subtraction of the editor around the
//! thread, entered from the command line. omega#100 made it the default and
//! omega#161 removed the editor split entirely. The primary Omega scaffold
//! remains inside this seal: it begins with only the canvas and composer, then
//! admits ported UI one surface at a time. This crate owns that process choice
//! and the shared action inventory.
//!
//! # Why the legacy implementation name survives
//!
//! The admitted set is consulted by the palette filter, the dispatch gate, and
//! every crate that needs to know whether a control belongs on the surface.
//! One list, one authority, checked by `crates/omega_deltas` — that job did
//! not end when the launch-mode split did.
//!
//! # What the tripwire does
//!
//! Two mechanisms, and the distinction decides what breaks:
//!
//! - **Not rendered.** The editor panels, the status-bar items and the Full
//!   Auto entry are never built. Cheapest, and on its own the most dangerous:
//!   the capability behind an unrendered surface is still one key press away.
//! - **Refused and reported.** Everything outside [`ADMITTED_NAMESPACES`] and
//!   [`ADMITTED_ACTIONS`] is refused at dispatch with [`refusal`] — one
//!   sentence that names the action. Visual proofs and `--omega-send` then
//!   fail the run. Never a silent no-op or an accepted refusal.
//!
//! omega#162 deleted the legacy editor crates behind the gate. An unresolvable
//! built-in key binding can still panic Omega before any window opens, so crate
//! deletion, keymap cleanup, and the admitted-action inventory remain one
//! mechanically checked contract.

#![deny(missing_docs)]

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

/// The command-line flag that used to name this surface, as a person types it.
///
/// `OMEGA-DELTA-0052`. Zero base has been the default since omega#100 and the
/// only surface since omega#161, so this flag asks for what it already gets.
/// It is still accepted, and it still does nothing, so commands and scripts
/// that carry it keep working.
pub const FLAG: &str = "--zero-base";

/// Has the surface taken the window over yet?
///
/// `OMEGA-DELTA-0053`. Written once, at process start, by
/// `crates/omega/src/main.rs`. The seal used to wait for the identity gate's
/// centre-pane onboarding to be answered; omega#164 removed that page and
/// provisions the Nostr identity silently in the background, so there is
/// nothing left that needs the unsealed editor chrome and the seal moved to
/// startup with omega#161. Proof harnesses that build their own windows call
/// [`seal`] themselves when a scene photographs the sealed surface.
static SEALED: AtomicBool = AtomicBool::new(false);

/// Whether this process is rendering the primary Omega interface.
///
/// The primary interface begins with only the conversation canvas and composer. New
/// surfaces must be admitted deliberately instead of inheriting the Omega
/// workbench shell.
static PRIMARY_INTERFACE: AtomicBool = AtomicBool::new(false);

/// Number of action refusals logged by this process.
///
/// Visual proofs and `--omega-send` smoke runs read this counter and fail when
/// it advances. Keeping the counter beside the refusal sentence prevents a
/// logging-site-specific sweep from missing a newly added refusal path.
static LOGGED_REFUSALS: AtomicU64 = AtomicU64::new(0);

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
/// Terminal, pane, buffer-search, and workspace namespaces remain absent here.
/// The workbench terminal admits terminal input and search actions
/// individually below. Terminal creation uses `omega_workbench`'s
/// thread-bound wrapper instead of admitting a workspace action. The standard
/// close-active-item action is admitted individually for editor items revealed
/// beside the agent surface; other pane actions remain refused.
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
    // OMEGA-DELTA-0221. These actions operate only the selected public
    // channel's bounded community-room model. Admitting them individually
    // keeps the rest of the community and workspace namespaces closed.
    "community_sarah::JoinRoom",
    "community_sarah::LeaveRoom",
    "community_sarah::ToggleMute",
    "community_sarah::SummonSarah",
    "community_sarah::RemoveSarah",
    "community_sarah::TalkToSarah",
    "community_sarah::ModeratorStop",
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
    // The center editor is a supported auxiliary surface. Its standard close
    // action also routes correctly while the agent dock retains focus, and
    // closing the final item restores the agent-only surface.
    "pane::CloseActiveItem",
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
    // omega#242. Omega's own Settings route is dispatched, not called: the
    // admitted `omega::OpenSettings` handler re-dispatches
    // `omega::OpenEmbeddedSettings`, and the Back control at the foot of the
    // drawn Settings sidebar dispatches `omega::CloseEmbeddedSettings`. Both
    // are in the `omega` namespace, which is refused wholesale, so the gate
    // stopped them before any listener ran and reported it only to the log.
    // The route could still be entered by clicking the Effective Principal
    // footer, which calls the panel method directly — so Settings opened, and
    // then nothing closed it. A control this surface draws must not be refused
    // by the surface's own gate.
    "omega::OpenEmbeddedSettings",
    "omega::CloseEmbeddedSettings",
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
    // OMEGA-DELTA-0211. The composer microphone's own start path: it opens
    // audio without navigating to the admission page, so it must be admitted
    // beside the page's `StartVoice`.
    "workroom::StartVoiceFromComposer",
    "workroom::ApproveSarahVoiceCommand",
    "workroom::RejectSarahVoiceCommand",
    "workroom::ToggleVoiceMute",
    "workroom::InterruptVoice",
    "workroom::EndVoice",
    "workroom::RetryVoice",
];

/// Is the zero-base seal active? Constant `true` since omega#161.
///
/// The editor split is gone. The primary interface is a stricter presentation inside the
/// same zero-base gate, so callers that protect removed editor capabilities
/// continue to receive `true` in either presentation.
#[must_use]
pub fn is_active() -> bool {
    true
}

/// Select the primary Omega interface for this process.
///
/// This is one-way because changing the surface after a window has opened
/// could briefly expose controls that the launch contract excludes.
pub fn enable_primary_interface() {
    PRIMARY_INTERFACE.store(true, Ordering::SeqCst);
}

/// Is this process rendering the primary Omega interface?
#[must_use]
pub fn is_primary_interface() -> bool {
    PRIMARY_INTERFACE.load(Ordering::SeqCst)
}

/// The surface owns the window. One-way for the life of the process.
///
/// `OMEGA-DELTA-0053`, amended by omega#161. Called once at process start by
/// `crates/omega/src/main.rs`, before any window opens, so the editor chrome is
/// never drawn — not even for a frame. The seal used to wait for
/// `OMEGA-DELTA-0040`'s centre-pane identity onboarding; omega#164 deleted
/// that page, so nothing renders in the centre before the thread and sealing
/// at startup is safe. Test processes and proof harnesses start unsealed and
/// call this themselves when a scene needs the sealed render.
pub fn seal() {
    SEALED.store(true, Ordering::SeqCst);
}

/// Has the surface taken the window over?
///
/// `OMEGA-DELTA-0053`. While this is true the workspace starts with no editor
/// pane, tab bar, title bar, or status bar. They are not merely covered by a
/// zoomed panel. `OMEGA-DELTA-0139` introduced the explicit editable-center
/// exception, and `OMEGA-DELTA-0174` applies it to user-triggered center opens:
/// the center remains beside the agent surface until its final tab closes.
#[must_use]
pub fn is_sealed() -> bool {
    SEALED.load(Ordering::SeqCst)
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
/// It names the action and says what Omega is instead. A refusal a person
/// cannot read is the same thing as a silent no-op.
///
/// `OMEGA-DELTA-0052` removed the control in the window the sentence used to
/// name, and omega#161 removed the flag it named after that: there is no
/// editor to start any more, so the sentence offers nothing. A sentence that
/// offered a way to the refused surface would be a lie a person would go
/// looking for.
#[must_use]
pub fn refusal(action_name: &str) -> String {
    format!(
        "{action_name} is not part of Omega, which shows one agent thread and \
         the controls that operate it."
    )
}

/// Record one refused action and return its canonical log sentence.
///
/// Product logging sites call this instead of [`refusal`] directly so proof
/// processes can treat any refusal as a failed run without scraping log text.
#[must_use]
pub fn record_refusal(action_name: &str) -> String {
    LOGGED_REFUSALS.fetch_add(1, Ordering::SeqCst);
    refusal(action_name)
}

/// Return the number of refused actions logged by this process.
#[must_use]
pub fn logged_refusal_count() -> u64 {
    LOGGED_REFUSALS.load(Ordering::SeqCst)
}

/// Whether a proof run observed the required empty refusal log.
#[must_use]
pub const fn proof_is_refusal_free(logged_refusals: u64) -> bool {
    logged_refusals == 0
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The surface is always on, and sealing is the one remaining one-way step.
    ///
    /// omega#161. This test used to prove the mode was off until the command
    /// line turned it on. There is no mode any more: `is_active` is constant
    /// `true`, and the only process state left is the seal, which starts off
    /// so tests and proof harnesses can exercise the unsealed render, and
    /// which production sets once at startup.
    #[test]
    fn the_surface_is_always_on_and_the_seal_is_one_way() {
        assert!(is_active(), "there is only one surface since omega#161");

        // OMEGA-DELTA-0053. The seal is process-wide, so this test owns the
        // transition and the others below only read derived pure functions.
        assert!(
            !is_sealed(),
            "a fresh test process starts unsealed so harnesses can build the \
             unsealed render before opting in"
        );
        seal();
        assert!(is_sealed());
        seal();
        assert!(
            is_sealed(),
            "sealing twice is the same as sealing once, and nothing unseals \
             inside a process"
        );
    }

    #[test]
    fn a_seeded_refusal_trips_the_proof_counter() {
        let before = logged_refusal_count();
        let sentence = record_refusal("seeded::Refusal");
        let logged_refusals = logged_refusal_count().saturating_sub(before);

        assert_eq!(logged_refusals, 1);
        assert!(!proof_is_refusal_free(logged_refusals));
        assert!(sentence.contains("seeded::Refusal"));
        assert!(sentence.contains("is not part of Omega"));
        assert!(sentence.contains("controls that operate it"));
    }

    #[test]
    fn the_admitted_set_is_the_exo_surface_and_nothing_else() {
        for admitted in [
            "community_sarah::JoinRoom",
            "community_sarah::LeaveRoom",
            "community_sarah::ToggleMute",
            "community_sarah::SummonSarah",
            "community_sarah::RemoveSarah",
            "community_sarah::TalkToSarah",
            "community_sarah::ModeratorStop",
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
            "omega::OpenEmbeddedSettings",
            "omega::CloseEmbeddedSettings",
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
            "pane::CloseActiveItem",
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

    #[test]
    fn omega_thread_tab_keymaps_win_over_workspace_pane_bindings() {
        const OMEGA_CONTEXT: &str = "\"context\": \"AgentPanel && OmegaInterface\"";
        const FORENSICS_CONTEXT: &str = "\"context\": \"WorkbenchForensics\"";

        for (platform, keymap, first_shortcut) in [
            (
                "linux",
                include_str!("../../../assets/keymaps/default-linux.json"),
                "\"ctrl-1\": [\"agent::ActivateOmegaThreadTab\", 0]",
            ),
            (
                "macos",
                include_str!("../../../assets/keymaps/default-macos.json"),
                "\"cmd-1\": [\"agent::ActivateOmegaThreadTab\", 0]",
            ),
            (
                "windows",
                include_str!("../../../assets/keymaps/default-windows.json"),
                "\"ctrl-1\": [\"agent::ActivateOmegaThreadTab\", 0]",
            ),
        ] {
            let omega_offset = keymap
                .rfind(OMEGA_CONTEXT)
                .unwrap_or_else(|| panic!("{platform} keymap must contain the Omega context"));
            let forensics_offset = keymap
                .rfind(FORENSICS_CONTEXT)
                .unwrap_or_else(|| panic!("{platform} keymap must contain the Forensics context"));
            let pane_offset = keymap.rfind("workspace::ActivatePane").unwrap_or_else(|| {
                panic!("{platform} keymap must contain workspace pane bindings")
            });

            assert!(
                omega_offset > pane_offset,
                "{platform} must place Omega tab bindings after workspace pane bindings so the scoped action wins"
            );
            assert!(
                forensics_offset > pane_offset,
                "{platform} must place Forensics bindings after workspace pane bindings so closing the route wins"
            );
            let forensics_section = keymap[forensics_offset..]
                .split("\n  },")
                .next()
                .unwrap_or(&keymap[forensics_offset..]);
            assert!(
                forensics_section.contains("omega_workbench::CloseActiveThreadTab"),
                "{platform} Forensics context must close the active Omega route"
            );
            assert!(
                keymap[omega_offset..].contains(first_shortcut),
                "{platform} Omega context must bind its first positional tab shortcut"
            );
        }
    }

    /// `OMEGA-DELTA-0052`, amended by omega#161. The sentence names the action
    /// and offers nothing: no control in the window, and no flag — the editor
    /// the old sentence pointed at no longer exists.
    #[test]
    fn a_refusal_is_one_readable_sentence_offering_no_way_out() {
        let sentence = refusal("project_panel::ToggleFocus");
        assert!(sentence.contains("project_panel::ToggleFocus"));
        assert!(sentence.contains("Omega"));
        assert!(
            !sentence.contains("Leave") && !sentence.contains("--full-editor"),
            "the refusal must not send a person looking for a button or a flag \
             that does not exist: {sentence}"
        );
    }
}
