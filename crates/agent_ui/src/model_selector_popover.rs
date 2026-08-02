use std::rc::Rc;

use acp_thread::{AgentModelIcon, AgentModelId, AgentModelInfo, AgentModelSelector};
use anyhow::Result;
use gpui::{Entity, FocusHandle, Task};
use picker::popover_menu::PickerPopoverMenu;
use ui::{PopoverMenuHandle, Tooltip, prelude::*};

use crate::ui::ModelSelectorTooltip;
use crate::{ModelSelector, model_selector::acp_model_selector};

pub struct ModelSelectorPopover {
    selector: Entity<ModelSelector>,
    /// The ACP selector used to apply tier choices without opening the picker.
    agent_selector: Rc<dyn AgentModelSelector>,
    menu_handle: PopoverMenuHandle<ModelSelector>,
}

impl ModelSelectorPopover {
    pub(crate) fn new(
        selector: Rc<dyn AgentModelSelector>,
        menu_handle: PopoverMenuHandle<ModelSelector>,
        focus_handle: FocusHandle,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let agent_selector = selector.clone();
        Self {
            selector: cx
                .new(move |cx| acp_model_selector(selector, focus_handle.clone(), window, cx)),
            agent_selector,
            menu_handle,
        }
    }

    pub fn toggle(&self, window: &mut Window, cx: &mut Context<Self>) {
        self.menu_handle.toggle(window, cx);
    }

    pub fn active_model<'a>(&self, cx: &'a App) -> Option<&'a AgentModelInfo> {
        self.selector.read(cx).delegate.active_model()
    }

    pub fn available_models(&self, cx: &App) -> Vec<AgentModelInfo> {
        self.selector.read(cx).delegate.available_models()
    }

    /// The underlying ACP model selector (for Flash/Pro tier application).
    pub fn agent_selector(&self) -> Rc<dyn AgentModelSelector> {
        self.agent_selector.clone()
    }

    /// Select a model by id (e.g. `google/gemini-3.6-flash` or `openagents/kimi-k3`).
    pub fn select_model_id(&self, model_id: AgentModelId, cx: &mut App) -> Task<Result<()>> {
        self.agent_selector.select_model(model_id, cx)
    }

    pub fn cycle_favorite_models(&self, window: &mut Window, cx: &mut Context<Self>) {
        self.selector.update(cx, |selector, cx| {
            selector.delegate.cycle_favorite_models(window, cx);
        });
    }
}

impl Render for ModelSelectorPopover {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let selector = self.selector.read(cx);
        let model = selector.delegate.active_model();
        let model_name = model
            .as_ref()
            .map(|model| model.name.clone())
            .unwrap_or_else(|| SharedString::from("Select a Model"));

        let model_icon = model.as_ref().and_then(|model| model.icon.clone());

        let (color, icon) = if self.menu_handle.is_deployed() {
            (Color::Accent, IconName::ChevronUp)
        } else {
            (Color::Muted, IconName::ChevronDown)
        };

        let show_cycle_row = selector.delegate.favorites_count() > 1;

        let tooltip = Tooltip::element({
            move |_, _cx| {
                ModelSelectorTooltip::new()
                    .show_cycle_row(show_cycle_row)
                    .into_any_element()
            }
        });

        PickerPopoverMenu::new(
            self.selector.clone(),
            Button::new("active-model", model_name)
                .label_size(LabelSize::Small)
                .color(color)
                .when_some(model_icon, |this, icon| {
                    this.start_icon(
                        match icon {
                            AgentModelIcon::Path(path) => Icon::from_external_svg(path),
                            AgentModelIcon::Named(icon_name) => Icon::new(icon_name),
                        }
                        .color(color)
                        .size(IconSize::XSmall),
                    )
                })
                .end_icon(Icon::new(icon).color(Color::Muted).size(IconSize::XSmall)),
            tooltip,
            gpui::Anchor::BottomRight,
            cx,
        )
        .with_handle(self.menu_handle.clone())
        .render(window, cx)
    }
}
