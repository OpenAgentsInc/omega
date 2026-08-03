use std::ops::Range;

use editor::{Editor, EditorElement, EditorStyle};
use gpui::{
    AnyElement, App, Context, Entity, EventEmitter, FocusHandle, Focusable, IntoElement,
    KeyContext, Render, Styled, TextStyle, UniformListScrollHandle, Window, point, uniform_list,
};
use omega_effectd::all_work_contract::WorkState;
use omega_work_index::{
    AttentionGroup, WorkIndex, WorkIndexHealth, WorkIndexItem, WorkIndexQuery, WorkIndexView,
};
use settings::Settings as _;
use theme_settings::ThemeSettings;
use ui::{
    Button, ButtonSize, ButtonStyle, Color, Icon, IconName, Label, LabelSize, ScrollableHandle,
    ToggleButtonGroup, ToggleButtonGroupSize, ToggleButtonGroupStyle, ToggleButtonSimple,
    prelude::*,
};

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
enum SurfaceFilter {
    #[default]
    All,
    NeedsAttention,
    Active,
    Completed,
}

#[derive(Clone, Debug)]
enum DisplayRow {
    Group(AttentionGroup),
    Item(WorkIndexItem),
}

#[derive(Clone, Debug)]
pub enum WorkIndexSurfaceEvent {
    Open(WorkIndexItem),
    Inspect(WorkIndexItem),
    Refresh,
    SelectionChanged(Option<String>),
}

pub struct WorkIndexSurface {
    focus_handle: FocusHandle,
    query_editor: Entity<Editor>,
    list: UniformListScrollHandle,
    index: WorkIndex,
    view: WorkIndexView,
    filter: SurfaceFilter,
    rows: Vec<DisplayRow>,
    selected_work_ref: Option<String>,
    _query_subscription: gpui::Subscription,
}

impl WorkIndexSurface {
    pub fn new(
        view: WorkIndexView,
        index: WorkIndex,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let query_editor = cx.new(|cx| {
            let mut input = Editor::single_line(window, cx);
            input.set_placeholder_text("Search work…", window, cx);
            input
        });
        let subscription = cx.subscribe(&query_editor, |this, _, event, cx| {
            if matches!(event, editor::EditorEvent::Edited { .. }) {
                this.rebuild_rows(cx);
                this.list.set_offset(point(px(0.), px(0.)));
            }
        });
        let selected_work_ref = index.selected_work_ref().map(ToOwned::to_owned);
        let mut surface = Self {
            focus_handle: cx.focus_handle(),
            query_editor,
            list: UniformListScrollHandle::new(),
            index,
            view,
            filter: SurfaceFilter::All,
            rows: Vec::new(),
            selected_work_ref,
            _query_subscription: subscription,
        };
        surface.rebuild_rows(cx);
        surface
    }

    pub fn set_index(&mut self, index: WorkIndex, cx: &mut Context<Self>) {
        self.index = index;
        if self
            .selected_work_ref
            .as_deref()
            .is_some_and(|work_ref| self.index.item(work_ref).is_none())
        {
            self.selected_work_ref = None;
        }
        self.rebuild_rows(cx);
    }

    pub fn set_view(&mut self, view: WorkIndexView, cx: &mut Context<Self>) {
        if self.view == view {
            return;
        }
        self.view = view;
        self.rebuild_rows(cx);
        self.list.set_offset(point(px(0.), px(0.)));
    }

    pub fn view(&self) -> WorkIndexView {
        self.view
    }

    fn search_query(&self, cx: &App) -> Option<String> {
        let text = self.query_editor.read(cx).text(cx);
        (!text.trim().is_empty()).then_some(text)
    }

    fn query(&self, cx: &App) -> WorkIndexQuery {
        let mut query = WorkIndexQuery {
            view: self.view,
            search: self.search_query(cx),
            ..WorkIndexQuery::default()
        };
        match self.filter {
            SurfaceFilter::All => {}
            SurfaceFilter::NeedsAttention => {
                query.attention = vec![
                    AttentionGroup::Question,
                    AttentionGroup::Recoverable,
                    AttentionGroup::Blocked,
                    AttentionGroup::Failed,
                    AttentionGroup::Stale,
                ];
            }
            SurfaceFilter::Active => {
                query.states = vec![WorkState::Active, WorkState::Waiting, WorkState::Blocked];
            }
            SurfaceFilter::Completed => {
                query.states = vec![
                    WorkState::Completed,
                    WorkState::Canceled,
                    WorkState::Archived,
                ];
            }
        }
        query
    }

    fn rebuild_rows(&mut self, cx: &mut Context<Self>) {
        let items = self.index.query(&self.query(cx));
        let mut rows = Vec::with_capacity(items.len().saturating_add(12));
        let mut group = None;
        for item in items {
            if group != Some(item.attention) {
                group = Some(item.attention);
                rows.push(DisplayRow::Group(item.attention));
            }
            rows.push(DisplayRow::Item(item));
        }
        self.rows = rows;
        cx.notify();
    }

    fn item_rows(&self) -> Vec<&WorkIndexItem> {
        self.rows
            .iter()
            .filter_map(|row| match row {
                DisplayRow::Item(item) => Some(item),
                DisplayRow::Group(_) => None,
            })
            .collect()
    }

    fn select_relative(&mut self, delta: isize, cx: &mut Context<Self>) {
        let items = self.item_rows();
        if items.is_empty() {
            self.selected_work_ref = None;
            cx.emit(WorkIndexSurfaceEvent::SelectionChanged(None));
            cx.notify();
            return;
        }
        let current = self
            .selected_work_ref
            .as_deref()
            .and_then(|selected| items.iter().position(|item| item.work_ref() == selected));
        let next = match current {
            Some(current) => current
                .saturating_add_signed(delta)
                .min(items.len().saturating_sub(1)),
            None if delta < 0 => items.len().saturating_sub(1),
            None => 0,
        };
        let work_ref = items[next].work_ref().to_string();
        self.selected_work_ref = Some(work_ref.clone());
        cx.emit(WorkIndexSurfaceEvent::SelectionChanged(Some(work_ref)));
        cx.notify();
    }

    fn open_selected(&self, cx: &mut Context<Self>) {
        let Some(work_ref) = self.selected_work_ref.as_deref() else {
            return;
        };
        if let Some(item) = self.index.item(work_ref) {
            cx.emit(WorkIndexSurfaceEvent::Open(item));
        }
    }

    fn inspect_selected(&self, cx: &mut Context<Self>) {
        let Some(work_ref) = self.selected_work_ref.as_deref() else {
            return;
        };
        if let Some(item) = self.index.item(work_ref) {
            cx.emit(WorkIndexSurfaceEvent::Inspect(item));
        }
    }

    fn select_item(&mut self, item: &WorkIndexItem, cx: &mut Context<Self>) {
        let work_ref = item.work_ref().to_string();
        if self.selected_work_ref.as_deref() == Some(work_ref.as_str()) {
            return;
        }
        self.selected_work_ref = Some(work_ref.clone());
        cx.emit(WorkIndexSurfaceEvent::SelectionChanged(Some(work_ref)));
        cx.notify();
    }

    fn render_text_input(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let settings = ThemeSettings::get_global(cx);
        let text_style = TextStyle {
            color: cx.theme().colors().text,
            font_family: settings.ui_font.family.clone(),
            font_features: settings.ui_font.features.clone(),
            font_fallbacks: settings.ui_font.fallbacks.clone(),
            font_size: rems(0.875).into(),
            font_weight: settings.ui_font.weight,
            line_height: relative(1.3),
            ..Default::default()
        };
        EditorElement::new(
            &self.query_editor,
            EditorStyle {
                background: cx.theme().colors().editor_background,
                local_player: cx.theme().players().local(),
                text: text_style,
                ..Default::default()
            },
        )
    }

    fn render_rows(
        &mut self,
        range: Range<usize>,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Vec<AnyElement> {
        let selected_background = cx.theme().colors().element_selected;
        let hover_background = cx.theme().colors().ghost_element_hover;
        let border_selected = cx.theme().colors().border_selected;
        let range_start = range.start;
        self.rows[range]
            .iter()
            .cloned()
            .enumerate()
            .map(|(offset, row)| match row {
                DisplayRow::Group(group) => h_flex()
                    .id(("omega-work-group", range_start + offset))
                    .h(px(44.))
                    .px_3()
                    .items_end()
                    .pb_1()
                    .role(gpui::Role::Heading)
                    .aria_level(2)
                    .aria_label(format!("{} work", group.label()))
                    .child(
                        Label::new(group.label())
                            .size(LabelSize::XSmall)
                            .color(Color::Muted),
                    )
                    .into_any_element(),
                DisplayRow::Item(item) => {
                    let selected = self.selected_work_ref.as_deref() == Some(item.work_ref());
                    let click_item = item.clone();
                    let keyboard_item = item.clone();
                    let inspect_item = item.clone();
                    let source = source_label(&item).to_string();
                    h_flex()
                        .id(("omega-work-row", range_start + offset))
                        .debug_selector({
                            let work_ref = item.work_ref().to_string();
                            move || format!("omega.omega.work-index.row.{work_ref}")
                        })
                        .h(px(44.))
                        .px_3()
                        .gap_3()
                        .rounded_md()
                        .cursor_pointer()
                        .role(gpui::Role::ListItem)
                        .tab_index(0)
                        .aria_label(format!(
                            "{} · {} · {}",
                            item.summary.title.0,
                            item.attention.label(),
                            source
                        ))
                        .when(selected, |row| {
                            row.bg(selected_background)
                                .border_1()
                                .border_color(border_selected)
                        })
                        .when(!selected, |row| {
                            row.hover(move |style| style.bg(hover_background))
                        })
                        .on_click(cx.listener(move |this, event: &gpui::ClickEvent, _, cx| {
                            this.select_item(&click_item, cx);
                            if event.click_count() > 1 {
                                cx.emit(WorkIndexSurfaceEvent::Open(click_item.clone()));
                            }
                        }))
                        .on_key_down(cx.listener(move |this, event: &gpui::KeyDownEvent, _, cx| {
                            if matches!(event.keystroke.key.as_str(), "enter" | "space") {
                                this.select_item(&keyboard_item, cx);
                                if event.keystroke.key.as_str() == "enter" {
                                    cx.emit(WorkIndexSurfaceEvent::Open(keyboard_item.clone()));
                                } else {
                                    cx.emit(WorkIndexSurfaceEvent::Inspect(keyboard_item.clone()));
                                }
                                cx.stop_propagation();
                            }
                        }))
                        .child(attention_icon(item.attention))
                        .child(
                            v_flex()
                                .min_w_0()
                                .flex_1()
                                .child(div().truncate().child(item.summary.title.0.clone()))
                                .child(
                                    Label::new(source)
                                        .size(LabelSize::XSmall)
                                        .color(Color::Muted),
                                ),
                        )
                        .child(
                            Label::new(item.attention.label())
                                .size(LabelSize::XSmall)
                                .color(Color::Muted),
                        )
                        .child(
                            Button::new(
                                ("inspect-work-index-item", range_start + offset),
                                "Details",
                            )
                            .style(ButtonStyle::Subtle)
                            .size(ButtonSize::Compact)
                            .on_click(cx.listener(
                                move |this, _, _, cx| {
                                    this.select_item(&inspect_item, cx);
                                    cx.emit(WorkIndexSurfaceEvent::Inspect(inspect_item.clone()));
                                    cx.stop_propagation();
                                },
                            )),
                        )
                        .into_any_element()
                }
            })
            .collect()
    }

    fn render_status(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let projection = self.index.projection();
        let (message, warning) = match projection.health {
            WorkIndexHealth::Loading => ("Loading source-backed work…".to_string(), false),
            WorkIndexHealth::Offline => (
                "Offline · showing the last qualified snapshot while sources reconnect."
                    .to_string(),
                true,
            ),
            WorkIndexHealth::Partial => {
                let detail = if projection.gap_refs.is_empty() {
                    "One source is incomplete; other lanes remain available.".to_string()
                } else {
                    format!(
                        "Incomplete projection · {} reported gap{}.",
                        projection.gap_refs.len(),
                        if projection.gap_refs.len() == 1 {
                            ""
                        } else {
                            "s"
                        }
                    )
                };
                (detail, true)
            }
            WorkIndexHealth::Error => (
                "Work sources are unavailable. Missing data is not shown as empty success."
                    .to_string(),
                true,
            ),
            WorkIndexHealth::Conflict => (
                "Conflicting Work identities were quarantined. Unaffected lanes remain visible."
                    .to_string(),
                true,
            ),
            WorkIndexHealth::Ready | WorkIndexHealth::Empty => return None,
        };
        Some(
            h_flex()
                .id("omega-work-index-status")
                .w_full()
                .px_3()
                .py_2()
                .gap_2()
                .rounded_md()
                .bg(cx.theme().colors().elevated_surface_background)
                .role(gpui::Role::Status)
                .aria_label(message.clone())
                .when(warning, |row| {
                    row.child(Icon::new(IconName::Warning).color(Color::Warning))
                })
                .when(!warning, |row| {
                    row.child(Icon::new(IconName::ArrowCircle).color(Color::Muted))
                })
                .child(Label::new(message).size(LabelSize::Small))
                .into_any_element(),
        )
    }
}

impl Render for WorkIndexSurface {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let mut key_context = KeyContext::new_with_defaults();
        key_context.add("OmegaWorkIndex");
        let projection = self.index.projection();
        let empty = self.rows.is_empty()
            && matches!(
                projection.health,
                WorkIndexHealth::Ready | WorkIndexHealth::Empty
            );
        let view = self.view;

        v_flex()
            .id("omega-work-index-surface")
            .debug_selector(move || {
                format!(
                    "omega.omega.work-index.{}",
                    match view {
                        WorkIndexView::Inbox => "inbox",
                        WorkIndexView::MyWork => "my-work",
                    }
                )
            })
            .key_context(key_context)
            .track_focus(&self.focus_handle)
            .size_full()
            .overflow_hidden()
            .on_key_down(cx.listener(|this, event: &gpui::KeyDownEvent, _, cx| {
                if event.keystroke.modifiers.modified() {
                    return;
                }
                match event.keystroke.key.as_str() {
                    "down" | "j" => this.select_relative(1, cx),
                    "up" | "k" => this.select_relative(-1, cx),
                    "enter" => this.open_selected(cx),
                    "space" | "i" => this.inspect_selected(cx),
                    _ => return,
                }
                cx.stop_propagation();
            }))
            .child(
                v_flex()
                    .p_4()
                    .gap_3()
                    .border_b_1()
                    .border_color(cx.theme().colors().border_variant)
                    .child(
                        h_flex()
                            .w_full()
                            .justify_between()
                            .child(
                                v_flex()
                                    .child(Label::new(self.view.title()).size(LabelSize::Large))
                                    .child(
                                        Label::new(match self.view {
                                            WorkIndexView::Inbox => {
                                                "Questions, blockers, failures, stale work, and recoverable work"
                                            }
                                            WorkIndexView::MyWork => {
                                                "Your accountability, participation, and bounded agent delegation"
                                            }
                                        })
                                        .size(LabelSize::Small)
                                        .color(Color::Muted),
                                    ),
                            )
                            .child(
                                Button::new("refresh-work-index", "Refresh")
                                    .style(ButtonStyle::Outlined)
                                    .size(ButtonSize::Compact)
                                    .on_click(cx.listener(|_, _, _, cx| {
                                        cx.emit(WorkIndexSurfaceEvent::Refresh)
                                    })),
                            ),
                    )
                    .child(
                        h_flex()
                            .h_8()
                            .w_full()
                            .pl_2()
                            .pr_3()
                            .gap_2()
                            .border_1()
                            .border_color(cx.theme().colors().border)
                            .rounded_md()
                            .child(Icon::new(IconName::MagnifyingGlass).color(Color::Muted))
                            .child(self.render_text_input(cx)),
                    )
                    .child(
                        ToggleButtonGroup::single_row(
                            "omega-work-index-filter",
                            [
                                ToggleButtonSimple::new(
                                    "All",
                                    cx.listener(|this, _, _, cx| {
                                        this.filter = SurfaceFilter::All;
                                        this.rebuild_rows(cx);
                                    }),
                                ),
                                ToggleButtonSimple::new(
                                    "Needs attention",
                                    cx.listener(|this, _, _, cx| {
                                        this.filter = SurfaceFilter::NeedsAttention;
                                        this.rebuild_rows(cx);
                                    }),
                                ),
                                ToggleButtonSimple::new(
                                    "Active",
                                    cx.listener(|this, _, _, cx| {
                                        this.filter = SurfaceFilter::Active;
                                        this.rebuild_rows(cx);
                                    }),
                                ),
                                ToggleButtonSimple::new(
                                    "Completed",
                                    cx.listener(|this, _, _, cx| {
                                        this.filter = SurfaceFilter::Completed;
                                        this.rebuild_rows(cx);
                                    }),
                                ),
                            ],
                        )
                        .style(ToggleButtonGroupStyle::Outlined)
                        .size(ToggleButtonGroupSize::Custom(rems_from_px(30.)))
                        .label_size(LabelSize::Small)
                        .auto_width()
                        .selected_index(match self.filter {
                            SurfaceFilter::All => 0,
                            SurfaceFilter::NeedsAttention => 1,
                            SurfaceFilter::Active => 2,
                            SurfaceFilter::Completed => 3,
                        }),
                    )
                    .when_some(self.render_status(cx), |header, status| {
                        header.child(status)
                    }),
            )
            .child(
                v_flex()
                    .flex_1()
                    .min_h_0()
                    .px_4()
                    .pb_4()
                    .when(empty, |body| {
                        body.child(
                            v_flex()
                                .id("omega-work-index-empty")
                                .flex_1()
                                .items_center()
                                .justify_center()
                                .gap_2()
                                .role(gpui::Role::Status)
                                .aria_label(format!("{} is empty", self.view.title()))
                                .child(
                                    Icon::new(IconName::Check)
                                        .size(IconSize::XLarge)
                                        .color(Color::Muted),
                                )
                                .child(Label::new(match self.view {
                                    WorkIndexView::Inbox => "Nothing needs your attention.",
                                    WorkIndexView::MyWork => "No source-backed work matches.",
                                })),
                        )
                    })
                    .when(!empty, |body| {
                        let count = self.rows.len();
                        let handle = self.list.clone();
                        body.child(
                            uniform_list(
                                "omega-work-index-rows",
                                count,
                                cx.processor(Self::render_rows),
                            )
                            .flex_grow_1()
                            .track_scroll(&handle),
                        )
                    }),
            )
    }
}

impl Focusable for WorkIndexSurface {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl EventEmitter<WorkIndexSurfaceEvent> for WorkIndexSurface {}

fn source_label(item: &WorkIndexItem) -> &'static str {
    match &item.source_entity {
        omega_work_index::WorkSourceEntity::Thread { .. } => "Thread",
        omega_work_index::WorkSourceEntity::ForensicsCase { .. }
        | omega_work_index::WorkSourceEntity::ForensicsRun { .. } => "Forensics",
        omega_work_index::WorkSourceEntity::EffectWork { .. } => "OpenAgents",
    }
}

fn attention_icon(attention: AttentionGroup) -> Icon {
    let (icon, color) = match attention {
        AttentionGroup::Question => (IconName::CircleHelp, Color::Accent),
        AttentionGroup::Recoverable | AttentionGroup::Blocked | AttentionGroup::Failed => {
            (IconName::Warning, Color::Warning)
        }
        AttentionGroup::Stale => (IconName::ArrowCircle, Color::Muted),
        AttentionGroup::Completed => (IconName::Check, Color::Success),
        AttentionGroup::Archived | AttentionGroup::Canceled => (IconName::Archive, Color::Muted),
        AttentionGroup::Active
        | AttentionGroup::Waiting
        | AttentionGroup::Triage
        | AttentionGroup::Planned => (IconName::Circle, Color::Muted),
    };
    Icon::new(icon).size(IconSize::Small).color(color)
}
