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

#[derive(Clone)]
pub struct ComposerTraitOption {
    pub id: SharedString,
    pub label: SharedString,
    pub selected: bool,
}

#[derive(Clone)]
pub struct ComposerTraitGroup {
    pub id: SharedString,
    pub label: SharedString,
    pub options: Vec<ComposerTraitOption>,
    pub on_select: Rc<dyn Fn(SharedString, &mut Window, &mut App)>,
}

#[derive(Clone)]
pub struct ComposerModelPicker {
    pub label: SharedString,
    pub current_model: Option<acp_thread::AgentModelId>,
    pub models: Vec<ComposerModelOption>,
    pub traits: Vec<ComposerTraitGroup>,
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
            traits: Vec::new(),
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
struct GlobalComposerExecutorMenuHandles {
    handles: HashMap<
        EntityId,
        (
            PopoverMenuHandle<OmegaComposerModelMenu>,
            WeakEntity<AgentPanel>,
        ),
    >,
    keep_open: HashMap<EntityId, bool>,
    reopen_scheduled: HashMap<EntityId, bool>,
}

impl Global for GlobalComposerExecutorMenuHandles {}

/// Register the dropdown's popover handle and owning panel for
/// `workspace_id`.
///
/// Called from `AgentPanel::new`. Re-registering replaces the entry, which is
/// what a rebuilt panel wants.
pub(crate) fn register_menu_handle(
    workspace_id: EntityId,
    handle: PopoverMenuHandle<OmegaComposerModelMenu>,
    panel: WeakEntity<AgentPanel>,
    cx: &mut App,
) {
    cx.default_global::<GlobalComposerExecutorMenuHandles>()
        .handles
        .insert(workspace_id, (handle, panel));
}

fn menu_handle(
    workspace_id: EntityId,
    cx: &App,
) -> Option<(
    PopoverMenuHandle<OmegaComposerModelMenu>,
    WeakEntity<AgentPanel>,
)> {
    cx.try_global::<GlobalComposerExecutorMenuHandles>()
        .and_then(|handles| handles.handles.get(&workspace_id).cloned())
}

fn keep_menu_open(workspace_id: EntityId, keep_open: bool, cx: &mut App) {
    cx.default_global::<GlobalComposerExecutorMenuHandles>()
        .keep_open
        .insert(workspace_id, keep_open);
}

fn menu_should_stay_open(workspace_id: EntityId, cx: &App) -> bool {
    cx.try_global::<GlobalComposerExecutorMenuHandles>()
        .and_then(|handles| handles.keep_open.get(&workspace_id))
        .copied()
        .unwrap_or(false)
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
    composer_focus_handle: FocusHandle,
    model_picker: ComposerModelPicker,
    window: &mut Window,
    cx: &mut App,
) -> Option<AnyElement> {
    let workspace_id = workspace.entity_id();
    let (handle, panel) = menu_handle(workspace_id, cx)?;

    if let Some(menu) = handle.deployed_menu() {
        let current_label = current_label.clone();
        let composer_focus_handle = composer_focus_handle.clone();
        let model_picker = model_picker.clone();
        menu.update(cx, |menu, cx| {
            menu.sync_active_composer(
                current_label,
                conversation_is_bound,
                composer_focus_handle,
                model_picker,
                cx,
            );
        });
    } else if menu_should_stay_open(workspace_id, cx) {
        let should_schedule = {
            let handles = cx.default_global::<GlobalComposerExecutorMenuHandles>();
            !handles
                .reopen_scheduled
                .get(&workspace_id)
                .copied()
                .unwrap_or(false)
        };
        if should_schedule {
            cx.default_global::<GlobalComposerExecutorMenuHandles>()
                .reopen_scheduled
                .insert(workspace_id, true);
            let handle = handle.clone();
            window.on_next_frame(move |window, cx| {
                cx.default_global::<GlobalComposerExecutorMenuHandles>()
                    .reopen_scheduled
                    .insert(workspace_id, false);
                if menu_should_stay_open(workspace_id, cx) {
                    handle.show(window, cx);
                }
            });
        }
    }

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
            .on_open(Rc::new(move |_, cx| {
                keep_menu_open(workspace_id, true, cx);
            }))
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
                    OmegaComposerModelMenu::new(
                        panel,
                        workspace_id,
                        rows,
                        conversation_is_bound,
                        composer_focus_handle.clone(),
                        model_picker.current_model.clone(),
                        model_picker.models.clone(),
                        model_picker.traits.clone(),
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

pub(crate) struct OmegaComposerModelMenu {
    panel: gpui::Entity<AgentPanel>,
    workspace_id: EntityId,
    rows: Vec<ComposerExecutorRow>,
    conversation_is_bound: bool,
    composer_focus_handle: FocusHandle,
    current_model: Option<acp_thread::AgentModelId>,
    models: Vec<ComposerModelOption>,
    traits: Vec<ComposerTraitGroup>,
    model_picker_enabled: bool,
    empty_message: SharedString,
    on_model_select: Rc<dyn Fn(acp_thread::AgentModelId, &mut Window, &mut App)>,
    focus_handle: FocusHandle,
    selected_index: usize,
    preserve_open_on_blur: bool,
}

impl OmegaComposerModelMenu {
    fn new(
        panel: gpui::Entity<AgentPanel>,
        workspace_id: EntityId,
        rows: Vec<ComposerExecutorRow>,
        conversation_is_bound: bool,
        composer_focus_handle: FocusHandle,
        current_model: Option<acp_thread::AgentModelId>,
        models: Vec<ComposerModelOption>,
        traits: Vec<ComposerTraitGroup>,
        model_picker_enabled: bool,
        empty_message: SharedString,
        on_model_select: Rc<dyn Fn(acp_thread::AgentModelId, &mut Window, &mut App)>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let focus_handle = cx.focus_handle();
        cx.on_blur(&focus_handle, window, |this, _, cx| {
            if this.preserve_open_on_blur {
                this.preserve_open_on_blur = false;
            } else {
                keep_menu_open(this.workspace_id, false, cx);
            }
            cx.emit(DismissEvent);
        })
        .detach();
        let selected_index = rows.iter().position(|row| row.is_current).unwrap_or(0);
        Self {
            panel,
            workspace_id,
            rows,
            conversation_is_bound,
            composer_focus_handle,
            current_model,
            models,
            traits,
            model_picker_enabled,
            empty_message,
            on_model_select,
            focus_handle,
            selected_index,
            preserve_open_on_blur: false,
        }
    }

    fn sync_active_composer(
        &mut self,
        current_label: SharedString,
        conversation_is_bound: bool,
        composer_focus_handle: FocusHandle,
        model_picker: ComposerModelPicker,
        cx: &mut Context<Self>,
    ) {
        self.conversation_is_bound = conversation_is_bound;
        self.composer_focus_handle = composer_focus_handle;
        self.current_model = model_picker.current_model;
        self.models = model_picker.models;
        self.traits = model_picker.traits;
        self.model_picker_enabled = model_picker.enabled;
        self.empty_message = model_picker.empty_message;
        self.on_model_select = model_picker.on_select;
        for row in &mut self.rows {
            row.is_current = row.label == current_label;
        }
        if let Some(index) = self.rows.iter().position(|row| row.is_current) {
            self.selected_index = index;
        }
        cx.notify();
    }

    fn item_count(&self) -> usize {
        self.rows.len() + self.models.len() + self.trait_option_count() + 1
    }

    fn trait_option_count(&self) -> usize {
        self.traits.iter().map(|group| group.options.len()).sum()
    }

    fn trait_option_at(&self, index: usize) -> Option<(usize, usize)> {
        let mut remaining = index;
        for (group_index, group) in self.traits.iter().enumerate() {
            if remaining < group.options.len() {
                return Some((group_index, remaining));
            }
            remaining -= group.options.len();
        }
        None
    }

    fn item_is_selectable(&self, index: usize) -> bool {
        if let Some(row) = self.rows.get(index) {
            return row.is_selectable();
        }
        let model_index = index.saturating_sub(self.rows.len());
        if self
            .models
            .get(model_index)
            .is_some_and(|model| self.model_picker_enabled && !model.disabled)
        {
            return true;
        }
        let trait_index = model_index.saturating_sub(self.models.len());
        self.trait_option_at(trait_index).is_some() || trait_index == self.trait_option_count()
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
        let trait_index = model_index.saturating_sub(self.models.len());
        if let Some((group_index, option_index)) = self.trait_option_at(trait_index) {
            self.choose_trait(group_index, option_index, window, cx);
            return;
        }
        keep_menu_open(self.workspace_id, false, cx);
        window.dispatch_action(Box::new(omega_actions::AcpRegistry), cx);
        cx.emit(DismissEvent);
    }

    fn cancel(&mut self, _: &menu::Cancel, window: &mut Window, cx: &mut Context<Self>) {
        self.dismiss_to_composer(window, cx);
    }

    fn choose_agent(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(row) = self.rows.get(index).cloned() else {
            return;
        };
        if !row.is_selectable() {
            return;
        }
        if row.is_current {
            return;
        }
        keep_menu_open(self.workspace_id, true, cx);
        self.preserve_open_on_blur = true;
        for row in &mut self.rows {
            row.is_current = false;
        }
        if let Some(row) = self.rows.get_mut(index) {
            row.is_current = true;
        }
        self.selected_index = index;
        self.current_model = None;
        self.models.clear();
        self.traits.clear();
        self.model_picker_enabled = false;
        self.empty_message = "Loading models…".into();
        self.panel.update(cx, |panel, cx| {
            panel.compose_on_executor(row.target, window, cx);
        });
        cx.emit(DismissEvent);
        if let Some((handle, _)) = menu_handle(self.workspace_id, cx) {
            let workspace_id = self.workspace_id;
            window.on_next_frame(move |window, cx| {
                if menu_should_stay_open(workspace_id, cx) {
                    handle.show(window, cx);
                }
            });
        }
    }

    fn choose_trait(
        &mut self,
        group_index: usize,
        option_index: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(group) = self.traits.get(group_index).cloned() else {
            return;
        };
        let Some(option) = group.options.get(option_index).cloned() else {
            return;
        };
        (group.on_select)(option.id.clone(), window, cx);
        if let Some(group) = self.traits.get_mut(group_index) {
            for candidate in &mut group.options {
                candidate.selected = candidate.id == option.id;
            }
        }
        keep_menu_open(self.workspace_id, true, cx);
        cx.notify();
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
            self.dismiss_to_composer(window, cx);
            return;
        }
        (self.on_model_select)(model.id.clone(), window, cx);
        self.current_model = Some(model.id);
        self.dismiss_to_composer(window, cx);
    }

    fn dismiss_to_composer(&self, window: &mut Window, cx: &mut Context<Self>) {
        keep_menu_open(self.workspace_id, false, cx);
        self.composer_focus_handle.focus(window, cx);
        cx.emit(DismissEvent);
    }
}

impl Focusable for OmegaComposerModelMenu {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl EventEmitter<DismissEvent> for OmegaComposerModelMenu {}

impl Render for OmegaComposerModelMenu {
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
                    .id(("omega-agent-row", index))
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
                    .id(("omega-model-row", index))
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

        let mut trait_offset = 0usize;
        let mut trait_groups = Vec::new();
        for (group_index, group) in self.traits.clone().into_iter().enumerate() {
            let group_offset = trait_offset;
            trait_offset += group.options.len();
            let mut options = Vec::new();
            for (option_index, option) in group.options.into_iter().enumerate() {
                let is_focused = self.selected_index
                    == self.rows.len() + self.models.len() + group_offset + option_index;
                let selected_option = option.selected;
                options.push(
                    h_flex()
                        .id((
                            "omega-trait-option",
                            group_index.saturating_mul(1_000) + option_index,
                        ))
                        .h(px(26.))
                        .px_2()
                        .rounded(px(7.))
                        .border_1()
                        .border_color(if selected_option || is_focused {
                            colors.border_selected
                        } else {
                            divider
                        })
                        .bg(if selected_option || is_focused {
                            selected
                        } else {
                            gpui::transparent_black()
                        })
                        .text_size(px(11.))
                        .text_color(if selected_option {
                            colors.text
                        } else {
                            colors.text_muted
                        })
                        .cursor_pointer()
                        .hover(|style| style.bg(colors.element_hover))
                        .on_click(cx.listener(move |this, _, window, cx| {
                            this.choose_trait(group_index, option_index, window, cx);
                        }))
                        .child(option.label)
                        .into_any_element(),
                );
            }
            trait_groups.push(
                v_flex()
                    .gap_1()
                    .child(
                        Label::new(group.label)
                            .size(LabelSize::XSmall)
                            .color(Color::Muted),
                    )
                    .child(h_flex().flex_wrap().gap_1().children(options))
                    .into_any_element(),
            );
        }

        let add_agents_index = self.rows.len() + self.models.len() + self.trait_option_count();

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
                                    .id("omega-agent-scroll")
                                    .flex_1()
                                    .min_h_0()
                                    .overflow_y_scroll()
                                    .children(agents),
                            )
                            .child(
                                h_flex()
                                    .id("omega-model-add-agents")
                                    .mt_auto()
                                    .h(px(30.))
                                    .px_2()
                                    .rounded(px(8.))
                                    .text_size(px(12.))
                                    .text_color(colors.text_muted)
                                    .cursor_pointer()
                                    .hover(|style| style.bg(colors.element_hover))
                                    .when(self.selected_index == add_agents_index, |this| {
                                        this.bg(selected)
                                    })
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
                                    .p_1()
                                    .child(
                                        Label::new("Models")
                                            .size(LabelSize::XSmall)
                                            .color(Color::Muted),
                                    )
                                    .child(
                                        v_flex()
                                            .id("omega-model-scroll")
                                            .flex_1()
                                            .min_h_0()
                                            .gap_0p5()
                                            .overflow_y_scroll()
                                            .children(models),
                                    ),
                            )
                            .child(
                                v_flex()
                                    .id("omega-traits-scroll")
                                    .flex_none()
                                    .max_h(px(190.))
                                    .overflow_y_scroll()
                                    .gap_1()
                                    .border_t_1()
                                    .border_color(divider)
                                    .p_2()
                                    .when(self.traits.is_empty(), |this| {
                                        this.child(
                                            Label::new("Selection")
                                                .size(LabelSize::XSmall)
                                                .color(Color::Muted),
                                        )
                                        .child(
                                            Label::new(self.empty_message.clone())
                                                .size(LabelSize::Small),
                                        )
                                    })
                                    .children(trait_groups),
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
