use std::any::TypeId;
use std::collections::BTreeMap;

use command_palette_hooks::CommandPaletteFilter;
use component::{ComponentMetadata, ComponentStatus};
use gpui::{
    App, Entity, EventEmitter, FocusHandle, Focusable, SharedString, Window, actions, prelude::*,
};
use ui::{Divider, Label, LabelSize, prelude::*};
use ui_input::InputField;
use workspace::{Item, Workspace};

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
    if !ComponentLibraryGate::from_process_environment().enabled() {
        CommandPaletteFilter::update_global(cx, |filter, _| {
            filter.hide_action_types(&[TypeId::of::<OpenComponentLibrary>()]);
        });
        return;
    }
    cx.observe_new(
        |workspace: &mut Workspace, _window, _cx: &mut Context<Workspace>| {
            workspace.register_action(|workspace, _: &OpenComponentLibrary, window, cx| {
                let library = Box::new(cx.new(|cx| ComponentLibrary::new(window, cx)));
                workspace.add_item_to_active_pane(library, None, true, window, cx);
            });
        },
    )
    .detach();
}

pub struct ComponentLibrary {
    focus_handle: FocusHandle,
    filter: Entity<InputField>,
}

pub enum ComponentLibraryEvent {}

impl ComponentLibrary {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let filter = cx.new(|cx| InputField::new(window, cx, "Filter by name, scope, or text"));
        Self {
            focus_handle: cx.focus_handle(),
            filter,
        }
    }

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

    fn render_component_card(
        &self,
        metadata: &ComponentMetadata,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let status = metadata.status();
        v_flex()
            .w_full()
            .gap_2()
            .p_3()
            .border_1()
            .border_color(cx.theme().colors().border_variant)
            .rounded_md()
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
    }
}

impl Render for ComponentLibrary {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let by_scope = self.filtered_components_by_scope(cx);
        let total: usize = by_scope.values().map(Vec::len).sum();
        let mut sections = Vec::new();
        for (scope, components) in by_scope {
            let mut cards = Vec::new();
            for metadata in &components {
                cards.push(
                    self.render_component_card(metadata, window, cx)
                        .into_any_element(),
                );
            }
            sections.push(
                v_flex()
                    .gap_2()
                    .child(
                        h_flex()
                            .gap_2()
                            .child(Label::new(scope).size(LabelSize::Small).color(Color::Muted))
                            .child(Divider::horizontal()),
                    )
                    .children(cards),
            );
        }

        v_flex()
            .track_focus(&self.focus_handle)
            .size_full()
            .bg(cx.theme().colors().editor_background)
            .debug_selector(|| "omega.component_library".into())
            .child(
                v_flex()
                    .w_full()
                    .px_3()
                    .py_2()
                    .gap_2()
                    .border_b_1()
                    .border_color(cx.theme().colors().border)
                    .child(
                        h_flex()
                            .w_full()
                            .justify_between()
                            .child(Label::new("Component Library"))
                            .child(
                                Label::new(format!(
                                    "development surface · not production UI · {total} components"
                                ))
                                .size(LabelSize::XSmall)
                                .color(Color::Muted),
                            ),
                    )
                    .child(self.filter.clone()),
            )
            .child(
                div()
                    .id("component-library-scroll")
                    .size_full()
                    .overflow_y_scroll()
                    .child(v_flex().p_3().gap_4().children(sections)),
            )
    }
}

impl Focusable for ComponentLibrary {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl EventEmitter<ComponentLibraryEvent> for ComponentLibrary {}

impl Item for ComponentLibrary {
    type Event = ComponentLibraryEvent;

    fn tab_content_text(&self, _detail: usize, _cx: &App) -> SharedString {
        "Component Library".into()
    }

    fn tab_icon(&self, _window: &Window, _cx: &App) -> Option<Icon> {
        Some(Icon::new(IconName::Blocks))
    }
}
