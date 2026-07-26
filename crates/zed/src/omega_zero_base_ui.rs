//! The zero-base surface: the gate that refuses, the palette restriction, and
//! the visible control that leaves.
//!
//! omega#99. The mode itself lives in `crates/omega_zero_base`, which reads the
//! process command line and nothing else. This module is what a person sees and
//! touches: what the palette lists, what a refused key binding says, and the
//! one control that puts the editor back.

use gpui::{
    Action, App, AppContext as _, Context, IntoElement, ParentElement, Render, Styled, Window,
    actions,
};
use omega_zero_base::{ADMITTED_ACTIONS, ADMITTED_NAMESPACES, BANNER_LABEL, LEAVE_LABEL};
use ui::{
    Button, ButtonCommon, Clickable, Color, FluentBuilder as _, Label, LabelCommon, LabelSize,
    Tooltip, h_flex,
};
use workspace::{
    HideStatusItem, StatusItemView, Toast, Workspace, item::ItemHandle,
    notifications::NotificationId,
};

actions!(
    omega_zero_base,
    [
        /// Leaves zero base and puts the full Omega surface back in this window.
        Leave
    ]
);

/// Install the refusal gate and the palette restriction, once, at app init.
///
/// Only called when the process was started with the flag. Without it nothing
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

/// Put the zero-base control on this workspace and register the way out.
///
/// Called from `initialize_workspace` in place of the ordinary status-bar
/// items, so in zero base the status bar carries this one control and nothing
/// else.
///
/// `restore_panels` is the caller's, because the two binaries that build this
/// surface load panels differently: the app calls `initialize_panels`, and the
/// visual runner adds the one panel it photographs by hand.
pub fn install_on_workspace(
    workspace: &mut Workspace,
    window: &mut Window,
    cx: &mut Context<Workspace>,
    restore_panels: impl Fn(&mut Workspace, &mut Window, &mut Context<Workspace>) + 'static,
) {
    let restore_panels = std::rc::Rc::new(restore_panels);
    workspace.register_action(move |workspace, _: &Leave, window, cx| {
        leave(workspace, window, cx);
        restore_panels(workspace, window, cx);
    });

    let control = cx.new(|_| ZeroBaseStatusItem);
    workspace.status_bar().update(cx, |status_bar, cx| {
        status_bar.add_left_item(control, window, cx);
    });
}

/// Leave zero base inside the window a person is already looking at.
///
/// The mode goes off, the palette restriction is lifted, the gate stops
/// refusing, and the zoomed Exo panel goes back to being one panel among the
/// rest. Entry is not re-readable, so nothing re-enters.
fn leave(workspace: &mut Workspace, window: &mut Window, cx: &mut Context<Workspace>) {
    omega_zero_base::leave();
    command_palette_hooks::CommandPaletteFilter::update_global(cx, |filter, _| {
        filter.clear_restriction();
    });
    cx.clear_action_gate();

    if let Some(panel) = workspace.panel::<agent_ui::AgentPanel>(cx) {
        panel.update(cx, |panel, cx| {
            use workspace::dock::Panel as _;
            panel.set_zoomed(false, window, cx);
        });
    }
    cx.notify();
}

/// The visible way out, on the status bar.
struct ZeroBaseStatusItem;

impl Render for ZeroBaseStatusItem {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let active = omega_zero_base::is_active();
        h_flex()
            .gap_2()
            .when(active, |this| {
                this.child(
                    Label::new(BANNER_LABEL)
                        .size(LabelSize::Small)
                        .color(Color::Accent),
                )
                .child(
                    Button::new("omega-zero-base-leave", LEAVE_LABEL)
                        .label_size(LabelSize::Small)
                        .tooltip(Tooltip::text(format!(
                            "{BANNER_LABEL} shows one Exo thread and nothing else. \
                             This puts the rest of Omega back in this window."
                        )))
                        .on_click(cx.listener(|_, _, window, cx| {
                            window.dispatch_action(Leave.boxed_clone(), cx);
                        })),
                )
            })
    }
}

impl StatusItemView for ZeroBaseStatusItem {
    fn set_active_pane_item(
        &mut self,
        _active_pane_item: Option<&dyn ItemHandle>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) {
    }

    /// `None`: this control is conditional on the process command line, and a
    /// settings file must not be able to hide the way out of the mode.
    fn hide_setting(&self, _cx: &App) -> Option<HideStatusItem> {
        None
    }
}
