use std::any::TypeId;
use std::collections::BTreeMap;

use command_palette_hooks::CommandPaletteFilter;
use component::{ComponentMetadata, ComponentStatus};
use gpui::{
    App, Entity, EventEmitter, FocusHandle, Focusable, KeyDownEvent, TitlebarOptions, Window,
    WindowBounds, WindowOptions, actions, point, prelude::*, px, size,
};
use ui::{Divider, Label, LabelSize, prelude::*};
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
    if !ComponentLibraryGate::from_process_environment().enabled() {
        CommandPaletteFilter::update_global(cx, |filter, _| {
            filter.hide_action_types(&[TypeId::of::<OpenComponentLibrary>()]);
        });
        return;
    }
    // The sealed zero-base interface does not render the workspace's center
    // pane, so a pane item would be invisible; the library opens as its own
    // window instead, like the settings editor.
    cx.on_action(|_: &OpenComponentLibrary, cx| open_component_library_window(cx));
}

fn open_component_library_window(cx: &mut App) {
    let existing_window = cx
        .windows()
        .into_iter()
        .find_map(|window| window.downcast::<ComponentLibrary>());
    if let Some(existing_window) = existing_window {
        existing_window
            .update(cx, |_, window, _| window.activate_window())
            .ok();
        return;
    }

    cx.defer(move |cx| {
        let bounds = size(px(1080.), px(760.));
        if cx
            .open_window(
                WindowOptions {
                    titlebar: Some(TitlebarOptions {
                        title: Some("Component Library".into()),
                        appears_transparent: false,
                        traffic_light_position: Some(point(px(12.0), px(12.0))),
                    }),
                    focus: true,
                    show: true,
                    is_movable: true,
                    kind: gpui::WindowKind::Normal,
                    window_background: cx.theme().window_background_appearance(),
                    window_min_size: Some(size(px(480.), px(320.))),
                    window_bounds: Some(WindowBounds::centered(bounds, cx)),
                    ..Default::default()
                },
                |window, cx| {
                    let library = cx.new(|cx| ComponentLibrary::new(window, cx));
                    let focus_handle = library.focus_handle(cx);
                    window.focus(&focus_handle, cx);
                    library
                },
            )
            .is_err()
        {
            log::error!("failed to open the component library window");
        }
    });
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
            .key_context("ComponentLibrary")
            .on_key_down(cx.listener(|_, event: &KeyDownEvent, window, _cx| {
                if event.keystroke.key == "escape" {
                    window.remove_window();
                }
            }))
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
