use std::any::TypeId;
use std::collections::BTreeMap;

use command_palette_hooks::CommandPaletteFilter;
use component::{ComponentId, ComponentMetadata, ComponentStatus};
use gpui::{
    App, Entity, EventEmitter, FocusHandle, Focusable, KeyDownEvent, Window, actions, prelude::*,
    px,
};
use ui::{Label, LabelSize, ListItem, prelude::*};
use ui_input::InputField;

use crate::ComponentLibraryGate;

actions!(
    omega_workbench,
    [
        /// Opens the component library, a development-only gallery of the
        /// registered UI component previews.
        OpenComponentLibrary
    ]
);

pub fn init(cx: &mut App) {
    // The open handler lives on the agent panel, which embeds the library as
    // the shell's main pane; init only keeps the action out of the palette
    // when the gate is closed.
    if !ComponentLibraryGate::from_process_environment().enabled() {
        CommandPaletteFilter::update_global(cx, |filter, _| {
            filter.hide_action_types(&[TypeId::of::<OpenComponentLibrary>()]);
        });
    }
}

pub struct ComponentLibrary {
    focus_handle: FocusHandle,
    filter: Entity<InputField>,
    selected: Option<ComponentId>,
}

pub enum ComponentLibraryEvent {
    Close,
}

impl ComponentLibrary {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let filter = cx.new(|cx| InputField::new(window, cx, "Filter components"));
        Self {
            focus_handle: cx.focus_handle(),
            filter,
            selected: None,
        }
    }

    /// Only names and metadata for the nav; previews render one at a time,
    /// because painting all registered previews at once blows the element
    /// budget and drops frames.
    fn filtered_components_by_scope(&self, cx: &App) -> BTreeMap<String, Vec<ComponentMetadata>> {
        let filter = self.filter.read(cx).text(cx).trim().to_lowercase();
        let mut by_scope: BTreeMap<String, Vec<ComponentMetadata>> = BTreeMap::new();
        for metadata in component::components().sorted_components() {
            let scope = metadata.scope().to_string();
            if !filter.is_empty() {
                let matches = metadata.name().to_lowercase().contains(&filter)
                    || scope.to_lowercase().contains(&filter)
                    || metadata.description().to_lowercase().contains(&filter);
                if !matches {
                    continue;
                }
            }
            by_scope.entry(scope).or_default().push(metadata);
        }
        by_scope
    }

    fn render_nav(
        &self,
        by_scope: &BTreeMap<String, Vec<ComponentMetadata>>,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let mut rows: Vec<gpui::AnyElement> = Vec::new();
        let mut row_index = 0usize;
        for (scope, components) in by_scope {
            rows.push(
                div()
                    .px_2()
                    .pt_2()
                    .child(
                        Label::new(scope.clone())
                            .size(LabelSize::XSmall)
                            .color(Color::Muted),
                    )
                    .into_any_element(),
            );
            for metadata in components {
                let id = metadata.id();
                let selected = self.selected.as_ref() == Some(&id);
                rows.push(
                    ListItem::new(("component-library-row", row_index))
                        .inset(true)
                        .toggle_state(selected)
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.selected = Some(id.clone());
                            cx.notify();
                        }))
                        .child(Label::new(metadata.scopeless_name()).size(LabelSize::Small))
                        .into_any_element(),
                );
                row_index += 1;
            }
        }

        v_flex()
            .w(px(280.))
            .h_full()
            .flex_none()
            .border_r_1()
            .border_color(cx.theme().colors().border)
            .child(div().p_2().child(self.filter.clone()))
            .child(
                div()
                    .id("component-library-nav")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .child(v_flex().pb_2().children(rows)),
            )
    }

    fn render_selected(
        &self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<gpui::AnyElement> {
        let metadata = component::components()
            .get(self.selected.as_ref()?)?
            .clone();
        let status = metadata.status();
        Some(
            v_flex()
                .gap_2()
                .p_3()
                .child(
                    h_flex()
                        .gap_2()
                        .items_baseline()
                        .child(Label::new(metadata.scopeless_name()))
                        .when(status != ComponentStatus::Live, |this| {
                            this.child(
                                Label::new(status.to_string())
                                    .size(LabelSize::XSmall)
                                    .color(Color::Muted),
                            )
                        }),
                )
                .child(
                    Label::new(metadata.description())
                        .size(LabelSize::Small)
                        .color(Color::Muted),
                )
                .child((metadata.preview())(window, cx))
                .into_any_element(),
        )
    }
}

impl Render for ComponentLibrary {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let by_scope = self.filtered_components_by_scope(cx);
        let total: usize = by_scope.values().map(Vec::len).sum();

        let selection_visible = self.selected.as_ref().is_some_and(|selected| {
            by_scope
                .values()
                .flatten()
                .any(|metadata| metadata.id() == *selected)
        });
        if !selection_visible {
            self.selected = by_scope
                .values()
                .flatten()
                .next()
                .map(|metadata| metadata.id());
        }

        let selected_preview = self.render_selected(window, cx);

        v_flex()
            .track_focus(&self.focus_handle)
            .key_context("ComponentLibrary")
            .on_key_down(cx.listener(|_, event: &KeyDownEvent, _window, cx| {
                if event.keystroke.key == "escape" {
                    cx.emit(ComponentLibraryEvent::Close);
                }
            }))
            .size_full()
            .bg(cx.theme().colors().editor_background)
            .debug_selector(|| "omega.component_library".into())
            .child(
                h_flex()
                    .w_full()
                    .px_3()
                    .py_2()
                    .justify_between()
                    .border_b_1()
                    .border_color(cx.theme().colors().border)
                    .child(Label::new("Component Library"))
                    .child(
                        h_flex()
                            .gap_2()
                            .child(
                                Label::new(format!(
                                    "development surface · not production UI · {total} components"
                                ))
                                .size(LabelSize::XSmall)
                                .color(Color::Muted),
                            )
                            .child(
                                IconButton::new("component-library-close", IconName::Close)
                                    .icon_size(IconSize::Small)
                                    .on_click(cx.listener(|_, _, _, cx| {
                                        cx.emit(ComponentLibraryEvent::Close);
                                    })),
                            ),
                    ),
            )
            .child(
                h_flex()
                    .flex_1()
                    .min_h_0()
                    .items_stretch()
                    .child(self.render_nav(&by_scope, cx))
                    .child(
                        div()
                            .id("component-library-preview")
                            .flex_1()
                            .min_w_0()
                            .h_full()
                            .overflow_y_scroll()
                            .children(selected_preview),
                    ),
            )
    }
}

impl Focusable for ComponentLibrary {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl EventEmitter<ComponentLibraryEvent> for ComponentLibrary {}
