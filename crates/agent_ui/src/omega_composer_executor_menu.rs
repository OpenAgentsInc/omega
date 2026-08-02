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

use std::{collections::HashMap, rc::Rc};

use gpui::{
    AnyElement, App, DismissEvent, EntityId, EventEmitter, FocusHandle, Focusable, Global, Render,
    WeakEntity, Window,
};
use ui::{ButtonLike, PopoverMenu, PopoverMenuHandle, prelude::*};
use workspace::Workspace;

use crate::agent_panel::{AgentPanel, ComposerExecutorRow};
use crate::omega_model_tier::{ModelTier, RoutedFace};

#[derive(Clone)]
pub struct ComposerModelOption {
    pub id: acp_thread::AgentModelId,
    pub name: SharedString,
    pub description: Option<SharedString>,
    pub disabled: bool,
}

pub struct ComposerModelPicker {
    pub label: SharedString,
    pub current_model: Option<acp_thread::AgentModelId>,
    pub models: Vec<ComposerModelOption>,
    pub enabled: bool,
    pub empty_message: SharedString,
    pub on_select: Rc<dyn Fn(acp_thread::AgentModelId, &mut Window, &mut App)>,
}

impl ComposerModelPicker {
    pub fn omega(
        face: RoutedFace,
        enabled: bool,
        on_select: Rc<dyn Fn(ModelTier, &mut Window, &mut App)>,
    ) -> Self {
        let current_model = face
            .tier
            .map(|tier| acp_thread::AgentModelId::new(tier.agent_model_id()));
        let models = ModelTier::ALL
            .iter()
            .copied()
            .map(|tier| ComposerModelOption {
                id: acp_thread::AgentModelId::new(tier.agent_model_id()),
                name: tier.model_name().into(),
                description: Some(tier.description().into()),
                disabled: false,
            })
            .collect();
        Self {
            label: face.label,
            current_model,
            models,
            enabled,
            empty_message: "Choose a model for the next turn.".into(),
            on_select: Rc::new(move |model_id, window, cx| {
                if let Some(tier) = ModelTier::ALL
                    .iter()
                    .copied()
                    .find(|tier| tier.agent_model_id() == model_id.as_str())
                {
                    on_select(tier, window, cx);
                }
            }),
        }
    }
}

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
    HashMap<
        EntityId,
        (
            PopoverMenuHandle<CometComposerModelMenu>,
            WeakEntity<AgentPanel>,
        ),
    >,
);

impl Global for GlobalComposerExecutorMenuHandles {}

/// Register the dropdown's popover handle and owning panel for
/// `workspace_id`.
///
/// Called from `AgentPanel::new`. Re-registering replaces the entry, which is
/// what a rebuilt panel wants.
pub(crate) fn register_menu_handle(
    workspace_id: EntityId,
    handle: PopoverMenuHandle<CometComposerModelMenu>,
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
) -> Option<(
    PopoverMenuHandle<CometComposerModelMenu>,
    WeakEntity<AgentPanel>,
)> {
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
        agent_servers::CLAUDE_AGENT_ID => Some("Claude"),
        agent_servers::GROK_ID => Some("Grok"),
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
    model_picker: ComposerModelPicker,
    cx: &App,
) -> Option<AnyElement> {
    let (handle, panel) = menu_handle(workspace.entity_id(), cx)?;

    let model_label = model_picker.label.clone();
    let trigger_icon = executor_icon(&current_label);
    let trigger = ButtonLike::new("omega-composer-executor-trigger")
        .style(ButtonStyle::Transparent)
        .size(ButtonSize::None)
        .height(px(32.).into())
        .aria_label("Choose agent and model")
        .aria_value(SharedString::from(format!(
            "{} {}",
            current_label, model_label
        )))
        .child(
            h_flex()
                .debug_selector(|| "omega.composer.executor-menu".into())
                .h(px(32.))
                .max_w(px(208.))
                .min_w_0()
                .gap_1p5()
                .px_2p5()
                .rounded_lg()
                .text_size(px(12.))
                .font_weight(gpui::FontWeight::MEDIUM)
                .text_color(cx.theme().colors().text.opacity(0.9))
                .child(
                    Icon::new(trigger_icon)
                        .size(IconSize::Small)
                        .color(Color::Accent),
                )
                .child(Label::new(model_label).size(LabelSize::XSmall).truncate()),
        );

    Some(
        PopoverMenu::new("omega-composer-executor")
            // Keyboard-reachable: `agent::ToggleComposerExecutorMenu` opens
            // this handle, and the selector itself is arrow-key driven.
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
                Some(cx.new(|cx| {
                    CometComposerModelMenu::new(
                        panel,
                        rows,
                        conversation_is_bound,
                        model_picker.current_model.clone(),
                        model_picker.models.clone(),
                        model_picker.enabled,
                        model_picker.empty_message.clone(),
                        model_picker.on_select.clone(),
                        window,
                        cx,
                    )
                }))
            })
            .into_any_element(),
    )
}

fn executor_icon(label: &str) -> IconName {
    match label {
        "Codex" => IconName::AiOpenAi,
        "Claude" => IconName::AiClaude,
        "Grok" => IconName::AiXAi,
        _ => IconName::OmegaAgent,
    }
}

pub(crate) struct CometComposerModelMenu {
    panel: gpui::Entity<AgentPanel>,
    rows: Vec<ComposerExecutorRow>,
    conversation_is_bound: bool,
    current_model: Option<acp_thread::AgentModelId>,
    models: Vec<ComposerModelOption>,
    model_picker_enabled: bool,
    empty_message: SharedString,
    on_model_select: Rc<dyn Fn(acp_thread::AgentModelId, &mut Window, &mut App)>,
    focus_handle: FocusHandle,
    selected_index: usize,
}

impl CometComposerModelMenu {
    fn new(
        panel: gpui::Entity<AgentPanel>,
        rows: Vec<ComposerExecutorRow>,
        conversation_is_bound: bool,
        current_model: Option<acp_thread::AgentModelId>,
        models: Vec<ComposerModelOption>,
        model_picker_enabled: bool,
        empty_message: SharedString,
        on_model_select: Rc<dyn Fn(acp_thread::AgentModelId, &mut Window, &mut App)>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let focus_handle = cx.focus_handle();
        cx.on_blur(&focus_handle, window, |_, _, cx| cx.emit(DismissEvent))
            .detach();
        let selected_index = rows.iter().position(|row| row.is_current).unwrap_or(0);
        Self {
            panel,
            rows,
            conversation_is_bound,
            current_model,
            models,
            model_picker_enabled,
            empty_message,
            on_model_select,
            focus_handle,
            selected_index,
        }
    }

    fn item_count(&self) -> usize {
        self.rows.len() + self.models.len() + 1
    }

    fn item_is_selectable(&self, index: usize) -> bool {
        if let Some(row) = self.rows.get(index) {
            return row.is_selectable();
        }
        let model_index = index.saturating_sub(self.rows.len());
        self.models
            .get(model_index)
            .is_some_and(|model| self.model_picker_enabled && !model.disabled)
            || model_index == self.models.len()
    }

    fn move_selection(&mut self, delta: isize, cx: &mut Context<Self>) {
        let count = self.item_count();
        if count == 0 {
            return;
        }
        for _ in 0..count {
            self.selected_index = if delta < 0 {
                self.selected_index.checked_sub(1).unwrap_or(count - 1)
            } else {
                (self.selected_index + 1) % count
            };
            if self.item_is_selectable(self.selected_index) {
                cx.notify();
                return;
            }
        }
    }

    fn select_next(&mut self, _: &menu::SelectNext, _: &mut Window, cx: &mut Context<Self>) {
        self.move_selection(1, cx);
    }

    fn select_previous(
        &mut self,
        _: &menu::SelectPrevious,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.move_selection(-1, cx);
    }

    fn confirm(&mut self, _: &menu::Confirm, window: &mut Window, cx: &mut Context<Self>) {
        if self.selected_index < self.rows.len() {
            self.choose_agent(self.selected_index, window, cx);
            return;
        }
        let model_index = self.selected_index - self.rows.len();
        if let Some(model) = self.models.get(model_index).cloned() {
            self.choose_model(model, window, cx);
            return;
        }
        window.dispatch_action(Box::new(omega_actions::AcpRegistry), cx);
        cx.emit(DismissEvent);
    }

    fn cancel(&mut self, _: &menu::Cancel, _: &mut Window, cx: &mut Context<Self>) {
        cx.emit(DismissEvent);
    }

    fn choose_agent(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(row) = self.rows.get(index).cloned() else {
            return;
        };
        if !row.is_selectable() {
            return;
        }
        if row.is_current {
            cx.emit(DismissEvent);
            return;
        }
        self.panel.update(cx, |panel, cx| {
            panel.compose_on_executor(row.target, window, cx);
        });
        cx.emit(DismissEvent);
    }

    fn choose_model(
        &mut self,
        model: ComposerModelOption,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.model_picker_enabled || model.disabled {
            return;
        }
        if self.current_model.as_ref() == Some(&model.id) {
            cx.emit(DismissEvent);
            return;
        }
        (self.on_model_select)(model.id.clone(), window, cx);
        self.current_model = Some(model.id);
        cx.emit(DismissEvent);
    }
}

impl Focusable for CometComposerModelMenu {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl EventEmitter<DismissEvent> for CometComposerModelMenu {}

impl Render for CometComposerModelMenu {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let colors = cx.theme().colors();
        let divider = colors.border_variant.opacity(0.45);
        let selected = colors.element_selected.opacity(0.72);
        let header = if self.conversation_is_bound {
            BOUND_MENU_HEADER
        } else {
            UNBOUND_MENU_HEADER
        };

        let agents = self
            .rows
            .clone()
            .into_iter()
            .enumerate()
            .map(|(index, row)| {
                let selectable = row.is_selectable();
                let is_current = row.is_current;
                let reason = (!selectable).then(|| {
                    row.readiness
                        .reason()
                        .map_or_else(|| row.readiness.label().to_owned(), str::to_owned)
                });
                let icon = executor_icon(&row.label);
                let is_focused = self.selected_index == index;
                h_flex()
                    .id(("comet-agent-row", index))
                    .h(px(30.))
                    .min_w_0()
                    .gap_2()
                    .px_2()
                    .rounded(px(8.))
                    .text_size(px(12.))
                    .font_weight(gpui::FontWeight::MEDIUM)
                    .text_color(if is_current {
                        colors.text
                    } else {
                        colors.text_muted
                    })
                    .when(is_current || is_focused, |this| this.bg(selected))
                    .when(!selectable, |this| this.opacity(0.35))
                    .when(selectable, |this| {
                        this.cursor_pointer()
                            .hover(|style| style.bg(colors.element_hover))
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.choose_agent(index, window, cx);
                            }))
                    })
                    .child(Icon::new(icon).size(IconSize::Small).color(if is_current {
                        Color::Accent
                    } else {
                        Color::Muted
                    }))
                    .child(
                        v_flex()
                            .min_w_0()
                            .child(Label::new(row.label).size(LabelSize::XSmall).truncate())
                            .when_some(reason, |this, reason| {
                                this.child(
                                    Label::new(reason)
                                        .size(LabelSize::XSmall)
                                        .color(Color::Muted)
                                        .truncate(),
                                )
                            }),
                    )
            });

        let models = self
            .models
            .clone()
            .into_iter()
            .enumerate()
            .map(|(index, model)| {
                let is_current = self.current_model.as_ref() == Some(&model.id);
                let is_focused = self.selected_index == self.rows.len() + index;
                let selectable = self.model_picker_enabled && !model.disabled;
                let click_model = model.clone();
                v_flex()
                    .id(("comet-model-row", index))
                    .min_h(px(48.))
                    .justify_center()
                    .gap_0p5()
                    .px_2()
                    .rounded(px(8.))
                    .when(is_current || is_focused, |this| this.bg(selected))
                    .when(!selectable, |this| this.opacity(0.35))
                    .when(selectable, |this| {
                        this.cursor_pointer()
                            .hover(|style| style.bg(colors.element_hover))
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.choose_model(click_model.clone(), window, cx);
                            }))
                    })
                    .child(
                        h_flex()
                            .w_full()
                            .justify_between()
                            .child(Label::new(model.name.clone()).size(LabelSize::Small))
                            .when(is_current, |this| {
                                this.child(
                                    Icon::new(IconName::Check)
                                        .size(IconSize::XSmall)
                                        .color(Color::Accent),
                                )
                            }),
                    )
                    .when_some(model.description, |this, description| {
                        this.child(
                            Label::new(description)
                                .size(LabelSize::XSmall)
                                .color(Color::Muted)
                                .truncate(),
                        )
                    })
            });

        v_flex()
            .id("omega-composer-model-menu")
            .debug_selector(|| "omega.composer.executor-menu.popup".into())
            .track_focus(&self.focus_handle)
            .key_context("OmegaComposerExecutorMenu")
            .on_action(cx.listener(Self::select_next))
            .on_action(cx.listener(Self::select_previous))
            .on_action(cx.listener(Self::confirm))
            .on_action(cx.listener(Self::cancel))
            .occlude()
            .w(px(460.))
            .h(px(420.))
            .overflow_hidden()
            .rounded(px(12.))
            .border_1()
            .border_color(colors.border)
            .bg(colors.elevated_surface_background)
            .shadow_lg()
            .aria_label(header)
            .child(
                h_flex()
                    .flex_1()
                    .min_h_0()
                    .items_stretch()
                    .child(
                        v_flex()
                            .w(px(148.))
                            .flex_none()
                            .gap_0p5()
                            .p_1()
                            .border_r_1()
                            .border_color(divider)
                            .child(
                                Label::new("Agents")
                                    .size(LabelSize::XSmall)
                                    .color(Color::Muted),
                            )
                            .child(
                                v_flex()
                                    .id("comet-agent-scroll")
                                    .flex_1()
                                    .min_h_0()
                                    .overflow_y_scroll()
                                    .children(agents),
                            )
                            .child(
                                h_flex()
                                    .id("comet-model-add-agents")
                                    .mt_auto()
                                    .h(px(30.))
                                    .px_2()
                                    .rounded(px(8.))
                                    .text_size(px(12.))
                                    .text_color(colors.text_muted)
                                    .cursor_pointer()
                                    .hover(|style| style.bg(colors.element_hover))
                                    .when(
                                        self.selected_index == self.rows.len() + self.models.len(),
                                        |this| this.bg(selected),
                                    )
                                    .on_click(|_, window, cx| {
                                        window.dispatch_action(
                                            Box::new(omega_actions::AcpRegistry),
                                            cx,
                                        );
                                    })
                                    .child("Add More Agents…"),
                            ),
                    )
                    .child(
                        v_flex()
                            .flex_1()
                            .min_w_0()
                            .min_h_0()
                            .child(
                                v_flex()
                                    .flex_1()
                                    .min_h_0()
                                    .gap_0p5()
                                    .p_1()
                                    .child(
                                        Label::new("Models")
                                            .size(LabelSize::XSmall)
                                            .color(Color::Muted),
                                    )
                                    .children(models),
                            )
                            .child(
                                v_flex()
                                    .max_h(px(190.))
                                    .flex_none()
                                    .gap_1()
                                    .border_t_1()
                                    .border_color(divider)
                                    .p_2()
                                    .child(
                                        Label::new("Selection")
                                            .size(LabelSize::XSmall)
                                            .color(Color::Muted),
                                    )
                                    .child(
                                        Label::new(self.empty_message.clone())
                                            .size(LabelSize::Small),
                                    ),
                            ),
                    ),
            )
            .child(
                h_flex()
                    .h(px(38.))
                    .flex_none()
                    .gap_3()
                    .px_3()
                    .border_t_1()
                    .border_color(divider)
                    .bg(colors.surface_background.opacity(0.72))
                    .text_size(px(11.))
                    .text_color(colors.text_muted)
                    .child(
                        h_flex()
                            .gap_1()
                            .child(Icon::new(IconName::ArrowUp).size(IconSize::XSmall))
                            .child(Icon::new(IconName::ArrowDown).size(IconSize::XSmall))
                            .child("Navigate"),
                    )
                    .child(
                        h_flex()
                            .gap_1()
                            .child(Icon::new(IconName::Return).size(IconSize::XSmall))
                            .child("Select"),
                    ),
            )
    }
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
