//! The composer's new-conversation executor dropdown. `OMEGA-DELTA-0184`.
//!
//! omega#165. The owner hit #151's full-screen "Start a new conversation"
//! chooser in a release build and called it horrible friction: anything
//! between `+` and a blinking cursor fails the product. So the interstitial
//! died and its selection surface moved here — a dropdown in the composer
//! bar, beside the Flash/Pro tier control, the same visual weight, reachable
//! but never in the way.
//!
//! # What survives from the front door
//!
//! The typed model is #151's, unchanged: [`ConversationTarget`] names the
//! executor exactly, `ModeReadiness` carries the four honest states, and a
//! `Ready` row can only be minted through a `PreparationReceipt` bound to a
//! created session. Rows that cannot run here render disabled with the
//! reason — never hidden, never fake-enabled.
//!
//! # One selection authority
//!
//! `OMEGA-DELTA-0131` records the lie this control must not restate: a
//! selector that says Exo over a thread Codex answers. The repair is one
//! authority, not zero selectors. The trigger's face reads the **active
//! conversation's own owner** from `AgentPanel::composer_executor_rows`, and
//! choosing a row goes through `AgentPanel::compose_on_executor`, which
//! replaces a blank draft or starts a new conversation. There is no second
//! selection store to disagree with the transcript: switching swaps the
//! conversation entity itself, so the label and the thread cannot diverge.
//!
//! # The ownership law, restated in menu terms
//!
//! Selection is free until the first send. After the first send the
//! conversation is bound (`OMEGA-DELTA-0178`), so picking a different row
//! starts a **new** thread on that executor — the existing transcript's
//! executor never changes underneath its entries (`OMEGA-DELTA-0150`).

use std::collections::HashMap;

use gpui::{AnyElement, App, EntityId, Global, WeakEntity, Window};
use omega_front_door::ModeReadiness;
use ui::{Button, ContextMenu, ContextMenuEntry, PopoverMenu, PopoverMenuHandle, prelude::*};
use util::ResultExt as _;
use workspace::Workspace;

use crate::agent_panel::{AgentPanel, ComposerExecutorRow};

/// The popover handles behind every composer executor dropdown, one per
/// workspace.
///
/// The dropdown renders inside the conversation view, which renders inside
/// the Agent Panel's own render pass — so the render path may not read the
/// panel entity (that would double-lease it). The panel registers its handle
/// here at construction instead, and the render path reads this global
/// immutably.
#[derive(Default)]
struct GlobalComposerExecutorMenuHandles(
    HashMap<EntityId, (PopoverMenuHandle<ContextMenu>, WeakEntity<AgentPanel>)>,
);

impl Global for GlobalComposerExecutorMenuHandles {}

/// Register the dropdown's popover handle and owning panel for
/// `workspace_id`.
///
/// Called from `AgentPanel::new`. Re-registering replaces the entry, which is
/// what a rebuilt panel wants.
pub fn register_menu_handle(
    workspace_id: EntityId,
    handle: PopoverMenuHandle<ContextMenu>,
    panel: WeakEntity<AgentPanel>,
    cx: &mut App,
) {
    cx.default_global::<GlobalComposerExecutorMenuHandles>()
        .0
        .insert(workspace_id, (handle, panel));
}

fn menu_handle(
    workspace_id: EntityId,
    cx: &App,
) -> Option<(PopoverMenuHandle<ContextMenu>, WeakEntity<AgentPanel>)> {
    cx.try_global::<GlobalComposerExecutorMenuHandles>()
        .and_then(|handles| handles.0.get(&workspace_id).cloned())
}

/// The menu header over an unbound conversation: choosing re-homes this
/// conversation onto the picked executor before anything is said in it.
pub const UNBOUND_MENU_HEADER: &str = "Run this conversation on";

/// The menu header over a bound conversation: the transcript keeps its
/// executor, and choosing starts a new conversation instead.
pub const BOUND_MENU_HEADER: &str = "Start a new conversation on";

/// The fixed label for a named direct agent whose install registry carries
/// no display name. The trio the issue names, by exact ACP id.
#[must_use]
pub fn named_direct_agent_label(agent_id: &str) -> Option<&'static str> {
    match agent_id {
        agent_servers::CODEX_ID => Some("Codex"),
        agent_servers::CLAUDE_AGENT_ID => Some("Claude Code"),
        "grok-build" => Some("Grok Build"),
        _ => None,
    }
}

/// The composer's executor dropdown.
///
/// `current_label` names the surrounding conversation's own owner and comes
/// from the caller's local state — the loading composer reads its own agent
/// key, the thread bar its own resolved agent name. It is deliberately not
/// read back out of the panel here: this function runs inside the
/// conversation's render, where reading that same entity would double-lease
/// it, and a face fed by the entity it sits in cannot disagree with it.
///
/// `conversation_is_bound` is whether the surrounding conversation has
/// content — the fact that flips the menu from re-homing the draft to
/// starting a new thread. It comes from the caller's own thread state so the
/// two bars (loading composer and zero-base bar) answer it from what they
/// are actually rendering.
pub fn render_composer_executor_menu(
    workspace: WeakEntity<Workspace>,
    current_label: SharedString,
    conversation_is_bound: bool,
    cx: &App,
) -> Option<AnyElement> {
    let (handle, panel) = menu_handle(workspace.entity_id(), cx)?;

    let trigger = Button::new("omega-composer-executor-trigger", current_label)
        .debug_selector(|| "omega.composer.executor-menu".into())
        .label_size(LabelSize::XSmall)
        .color(Color::Muted)
        .end_icon(
            Icon::new(IconName::ChevronDown)
                .size(IconSize::XSmall)
                .color(Color::Muted),
        );

    Some(
        PopoverMenu::new("omega-composer-executor")
            // Keyboard-reachable: `agent::ToggleComposerExecutorMenu` opens
            // this handle, and the context menu itself is arrow-key driven.
            //
            // No trigger tooltip. The owner's law (omega#160, 2026-07-30,
            // `OMEGA-DELTA-0189`): a dropdown labeled with the executor name
            // needs no essay about internal mechanics.
            .with_handle(handle)
            .trigger(trigger)
            .anchor(gpui::Anchor::BottomRight)
            .menu(move |window, cx| {
                // The rows are read at open time, in an event context:
                // readiness can change between the trigger's draw and the
                // click, and reading the panel or the active conversation
                // during their own render would double-lease them. The panel
                // comes from the registration, not from the workspace, so
                // opening from inside a workspace action cannot re-enter the
                // leased workspace either.
                let panel = panel.upgrade()?;
                let rows = panel.read(cx).composer_executor_rows(cx);
                Some(build_menu(panel, rows, conversation_is_bound, window, cx))
            })
            .into_any_element(),
    )
}

fn build_menu(
    panel: gpui::Entity<AgentPanel>,
    rows: Vec<ComposerExecutorRow>,
    conversation_is_bound: bool,
    window: &mut Window,
    cx: &mut App,
) -> gpui::Entity<ContextMenu> {
    ContextMenu::build(window, cx, move |mut menu, _window, _cx| {
        menu = menu.header(if conversation_is_bound {
            BOUND_MENU_HEADER
        } else {
            UNBOUND_MENU_HEADER
        });

        for row in rows {
            let selectable = row.is_selectable();
            let is_current = row.is_current;
            if selectable {
                let panel = panel.downgrade();
                let target = row.target.clone();
                let readiness_note = match &row.readiness {
                    // "Ready" as a badge would restate the checkmark; the
                    // states a person acts on are the non-ready ones.
                    ModeReadiness::Ready { .. } => None,
                    readiness => readiness.reason().map(str::to_owned),
                };
                let mut entry = ContextMenuEntry::new(row.label.clone())
                    .toggleable(IconPosition::End, is_current)
                    .handler(move |window, cx| {
                        // Re-picking what already owns the conversation would
                        // rebuild a draft for no change.
                        if is_current {
                            return;
                        }
                        panel
                            .update(cx, |panel, cx| {
                                panel.compose_on_executor(target.clone(), window, cx);
                            })
                            .log_err();
                    });
                if let Some(note) = readiness_note {
                    let note = SharedString::from(note);
                    entry = entry.documentation_aside(ui::DocumentationSide::Left, move |_| {
                        Label::new(note.clone()).into_any_element()
                    });
                }
                menu.push_item(entry);
            } else {
                // The reason lives in the label, not in a documentation
                // aside: a disabled entry's aside is never shown (the same
                // component fact `OMEGA-DELTA-0123` recorded), and a reason
                // that cannot be reached is not a reason.
                let reason = row
                    .readiness
                    .reason()
                    .map_or_else(|| row.readiness.label().to_owned(), str::to_owned);
                menu.push_item(
                    ContextMenuEntry::new(SharedString::from(format!("{} — {reason}", row.label)))
                        .disabled(true),
                );
            }
        }

        // The install path for everything the disabled rows name.
        menu = menu.separator();
        menu.push_item(
            ContextMenuEntry::new("Add More Agents…")
                // omega#170. This entry doubles as the deployed popup's
                // position probe, because "the menu opened" and "the menu
                // opened at its trigger" once diverged: the pre-first-send
                // composer deployed this menu at the window's bottom-left.
                .debug_selector("omega.composer.executor-menu.popup")
                .handler(move |window, cx| {
                    window.dispatch_action(Box::new(omega_actions::AcpRegistry), cx);
                }),
        );

        menu.key_context("OmegaComposerExecutorMenu")
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The two headers say different things because they do different
    /// things: re-homing an unbound draft is not starting a second thread.
    #[test]
    fn the_menu_headers_separate_rehoming_from_starting_anew() {
        assert_ne!(UNBOUND_MENU_HEADER, BOUND_MENU_HEADER);
        assert!(BOUND_MENU_HEADER.contains("new conversation"));
    }

    #[test]
    fn conversation_target_equality_backs_the_current_row_check() {
        use omega_front_door::{ConversationTarget, DirectAgentId};
        let codex = ConversationTarget::DirectAgent {
            agent_id: DirectAgentId::new("codex-acp").expect("non-empty id"),
        };
        assert_eq!(codex, codex.clone());
        assert_ne!(codex, ConversationTarget::OmegaAgent);
    }
}
