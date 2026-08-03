//! The production destination for canonical All Work planning state.
//!
//! omega#239. Omega had exactly one consumer of a planning graph — the
//! development `DogfoodSurface` — and its only entrance returned `false`
//! unless the omega#209 mock gate had loaded a bundled fixture. A release
//! build therefore had no destination for planning Work at all, which is why
//! omega#223's refresh had nowhere to land, omega#214's Assignee / Agent
//! Delegate / Thread / Session / Run distinctions had nothing to be visible
//! on, and omega#216's Workroom activity had nothing to attach to.
//!
//! This surface reads [`omega_work_index::PlanningDestinationState`], which is
//! decided by the boundary's own answer. It holds no fixture, names no
//! fixture, and has no path that could produce Work without the service
//! producing it first: a state that did not read a graph owns no value rows
//! could be drawn from.

use gpui::{
    AnyElement, App, Context, EventEmitter, FocusHandle, Focusable, IntoElement, KeyContext,
    Render, Styled, UniformListScrollHandle, Window, uniform_list,
};
use omega_work_index::{
    PLANNING_DESTINATION_METHOD, PlanningDestinationRow, PlanningDestinationState,
    PlanningDestinationStateKind,
};
use ui::{Button, ButtonSize, ButtonStyle, Color, Icon, IconName, Label, LabelSize, prelude::*};

/// The debug selector the destination itself is published under, so a UI proof
/// can prove it is on screen without knowing which state it is in.
pub const OMEGA_PLANNING_SURFACE_SELECTOR: &str = "omega.omega.planning";

#[derive(Clone, Debug)]
pub enum PlanningSurfaceEvent {
    /// Read the planning graph again. Emitted only from a state where a retry
    /// could change the answer.
    Refresh,
    SelectionChanged(Option<String>),
}

pub struct OmegaPlanningSurface {
    focus_handle: FocusHandle,
    list: UniformListScrollHandle,
    state: PlanningDestinationState,
    selected_work_ref: Option<String>,
}

impl OmegaPlanningSurface {
    pub fn new(state: PlanningDestinationState, cx: &mut Context<Self>) -> Self {
        let mut surface = Self {
            focus_handle: cx.focus_handle(),
            list: UniformListScrollHandle::new(),
            state,
            selected_work_ref: None,
        };
        surface.reconcile_selection();
        surface
    }

    pub fn set_state(&mut self, state: PlanningDestinationState, cx: &mut Context<Self>) {
        if self.state == state {
            return;
        }
        self.state = state;
        self.reconcile_selection();
        cx.notify();
    }

    pub fn state(&self) -> &PlanningDestinationState {
        &self.state
    }

    pub fn state_kind(&self) -> PlanningDestinationStateKind {
        self.state.kind()
    }

    pub fn selected_work_ref(&self) -> Option<&str> {
        self.selected_work_ref.as_deref()
    }

    /// A selection that no longer names a visible row is dropped rather than
    /// left pointing at nothing. A state with no rows has no selection at all,
    /// which is what keeps the detail pane from surviving a refusal.
    fn reconcile_selection(&mut self) {
        let rows = self.state.rows();
        if rows.is_empty() {
            self.selected_work_ref = None;
            return;
        }
        let still_present = self
            .selected_work_ref
            .as_deref()
            .is_some_and(|work_ref| rows.iter().any(|row| row.work_ref == work_ref));
        if !still_present {
            self.selected_work_ref = rows.first().map(|row| row.work_ref.clone());
        }
    }

    fn selected_row(&self) -> Option<&PlanningDestinationRow> {
        let work_ref = self.selected_work_ref.as_deref()?;
        self.state
            .rows()
            .iter()
            .find(|row| row.work_ref == work_ref)
    }

    fn select_relative(&mut self, delta: isize, cx: &mut Context<Self>) {
        let rows = self.state.rows();
        if rows.is_empty() {
            return;
        }
        let current = self
            .selected_work_ref
            .as_deref()
            .and_then(|work_ref| rows.iter().position(|row| row.work_ref == work_ref))
            .unwrap_or(0);
        let last = rows.len().saturating_sub(1);
        let next = if delta < 0 {
            current.saturating_sub(delta.unsigned_abs())
        } else {
            current.saturating_add(delta as usize).min(last)
        };
        let work_ref = rows
            .get(next)
            .map(|row| row.work_ref.clone())
            .unwrap_or_default();
        if self.selected_work_ref.as_deref() == Some(work_ref.as_str()) {
            return;
        }
        self.selected_work_ref = Some(work_ref.clone());
        self.list.scroll_to_item(next, gpui::ScrollStrategy::Top);
        cx.emit(PlanningSurfaceEvent::SelectionChanged(Some(work_ref)));
        cx.notify();
    }

    fn select_work_ref(&mut self, work_ref: String, cx: &mut Context<Self>) {
        if self.selected_work_ref.as_deref() == Some(work_ref.as_str()) {
            return;
        }
        self.selected_work_ref = Some(work_ref.clone());
        cx.emit(PlanningSurfaceEvent::SelectionChanged(Some(work_ref)));
        cx.notify();
    }

    fn request_refresh(&mut self, cx: &mut Context<Self>) {
        if !self.state.is_retryable() {
            return;
        }
        cx.emit(PlanningSurfaceEvent::Refresh);
    }

    /// The state banner.
    ///
    /// Always drawn, in every state, with the state's own kind in its debug
    /// selector. A proof that finds `omega.omega.planning.state.empty` on
    /// screen has proven that this build distinguished an answered-but-empty
    /// graph from an unreachable boundary, because the two selectors differ.
    fn render_state_banner(&self, cx: &App) -> AnyElement {
        let kind = self.state.kind();
        let selector = self.state.debug_selector();
        let announcement = self.state.announcement();
        let (icon, color) = match kind {
            PlanningDestinationStateKind::Loading => (IconName::ArrowCircle, Color::Muted),
            PlanningDestinationStateKind::Populated => (IconName::ListTodo, Color::Default),
            PlanningDestinationStateKind::Empty => (IconName::Check, Color::Muted),
            PlanningDestinationStateKind::Unavailable => (IconName::Warning, Color::Warning),
            PlanningDestinationStateKind::Refused => (IconName::XCircle, Color::Error),
        };
        v_flex()
            .id("omega-planning-state")
            .debug_selector(move || selector)
            .w_full()
            .px_3()
            .py_2()
            .gap_1()
            .rounded_md()
            .bg(cx.theme().colors().elevated_surface_background)
            // omega#217/omega#209. The state is the value, so macOS speaks the
            // whole sentence on arrival and on every state change. Without an
            // exact value here a refusal and an empty list sound the same.
            .role(gpui::Role::Status)
            .aria_label(announcement.clone())
            .aria_live(gpui::Live::Polite)
            .aria_value(announcement)
            .child(
                h_flex()
                    .gap_2()
                    .child(Icon::new(icon).size(IconSize::Small).color(color))
                    .child(Label::new(self.state.headline()).size(LabelSize::Default)),
            )
            .child(
                Label::new(self.state.detail())
                    .size(LabelSize::Small)
                    .color(Color::Muted),
            )
            .into_any_element()
    }

    fn render_rows(
        &mut self,
        range: std::ops::Range<usize>,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Vec<AnyElement> {
        let colors = cx.theme().colors();
        let selected_background = colors.element_selected;
        let hover_background = colors.ghost_element_hover;
        let border_selected = colors.border_selected;
        let rows = self.state.rows().to_vec();
        let selected_work_ref = self.selected_work_ref.clone();
        range
            .filter_map(|index| rows.get(index).cloned())
            .map(|row| {
                let selected = selected_work_ref.as_deref() == Some(row.work_ref.as_str());
                let work_ref = row.work_ref.clone();
                h_flex()
                    .id(gpui::SharedString::from(row.work_ref.clone()))
                    .w_full()
                    .px_2()
                    .py_1p5()
                    .gap_2()
                    .rounded_md()
                    .cursor_pointer()
                    .role(gpui::Role::Button)
                    .tab_index(0isize)
                    .aria_label(row.accessibility_label())
                    .aria_selected(selected)
                    .when(selected, |element| {
                        element
                            .bg(selected_background)
                            .border_1()
                            .border_color(border_selected)
                    })
                    .when(!selected, |element| {
                        element.hover(move |style| style.bg(hover_background))
                    })
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.select_work_ref(work_ref.clone(), cx);
                    }))
                    .child(
                        div()
                            .min_w_0()
                            .flex_1()
                            .truncate()
                            .child(Label::new(row.title.clone()).size(LabelSize::Small)),
                    )
                    .child(
                        Label::new(row.state_label())
                            .size(LabelSize::XSmall)
                            .color(Color::Muted),
                    )
                    .child(
                        Label::new(row.priority_label())
                            .size(LabelSize::XSmall)
                            .color(Color::Muted),
                    )
                    .into_any_element()
            })
            .collect()
    }

    /// The selected Work item, with the five accountability and execution
    /// kinds drawn as five separate lines.
    ///
    /// omega#214 exists because they are five different things. Folding an
    /// absent one into another one is exactly the defect, so an absent value
    /// is drawn as its own words rather than left blank or borrowed.
    fn render_selected_detail(&self, _cx: &App) -> Option<AnyElement> {
        let row = self.selected_row()?;
        let fields: Vec<(&'static str, String)> = vec![
            ("Work", row.work_ref.clone()),
            ("Owner", row.owner_ref.clone()),
            (
                "Assignee",
                row.assignee_ref
                    .clone()
                    .unwrap_or_else(|| "No assignee".to_string()),
            ),
            (
                "Agent delegate",
                row.agent_delegate_ref
                    .clone()
                    .unwrap_or_else(|| "No agent delegate".to_string()),
            ),
            ("Threads", reference_summary(&row.thread_refs, "No thread")),
            (
                "Sessions",
                reference_summary(&row.session_refs, "No session"),
            ),
            (
                "Agent sessions",
                reference_summary(&row.agent_session_refs, "No agent session"),
            ),
            ("Runs", reference_summary(&row.run_refs, "No run")),
            (
                "Source authority",
                format!(
                    "{} · {}",
                    row.source_authority_kind,
                    if row.writable {
                        "writable"
                    } else {
                        "read only"
                    }
                ),
            ),
        ];
        Some(
            v_flex()
                .id("omega-planning-detail")
                .debug_selector(|| "omega.omega.planning.detail".into())
                .w_full()
                .gap_1()
                .role(gpui::Role::Group)
                .aria_label(format!("Planning Work detail for {}", row.title))
                .children(fields.into_iter().map(|(label, value)| {
                    // Each field is published with its own name and value.
                    // Drawing the pair and publishing neither is how a
                    // correctly computed distinction reaches nobody: the
                    // labels here are plain text nodes, which the platform
                    // tree does not carry.
                    h_flex()
                        .id(gpui::SharedString::from(format!(
                            "omega-planning-field-{label}"
                        )))
                        .w_full()
                        .gap_2()
                        .role(gpui::Role::Label)
                        .aria_label(format!("{label}: {value}"))
                        .aria_value(value.clone())
                        .child(
                            div().w_32().flex_none().child(
                                Label::new(label)
                                    .size(LabelSize::XSmall)
                                    .color(Color::Muted),
                            ),
                        )
                        .child(
                            div()
                                .min_w_0()
                                .flex_1()
                                .truncate()
                                .child(Label::new(value).size(LabelSize::XSmall)),
                        )
                        .into_any_element()
                }))
                .into_any_element(),
        )
    }
}

fn reference_summary(references: &[String], absent: &'static str) -> String {
    match references.len() {
        0 => absent.to_string(),
        1 => references[0].clone(),
        count => format!("{} · {count} references", references[0]),
    }
}

impl Render for OmegaPlanningSurface {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let mut key_context = KeyContext::new_with_defaults();
        key_context.add("OmegaPlanning");
        let row_count = self.state.rows().len();
        let retryable = self.state.is_retryable();
        let handle = self.list.clone();

        v_flex()
            .id("omega-planning-surface")
            .debug_selector(|| OMEGA_PLANNING_SURFACE_SELECTOR.into())
            .key_context(key_context)
            .track_focus(&self.focus_handle)
            // omega#217/omega#234. The destination is one landmark region so a
            // screen reader can jump to it and know when it has left it.
            .role(gpui::Role::Main)
            .aria_label("Planning")
            .size_full()
            .overflow_hidden()
            .on_key_down(cx.listener(|this, event: &gpui::KeyDownEvent, _, cx| {
                if event.keystroke.modifiers.modified() {
                    return;
                }
                match event.keystroke.key.as_str() {
                    "down" | "j" => this.select_relative(1, cx),
                    "up" | "k" => this.select_relative(-1, cx),
                    "r" => this.request_refresh(cx),
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
                                    .child(Label::new("Planning").size(LabelSize::Large))
                                    .child(
                                        Label::new(format!(
                                            "Canonical All Work planning state, read from \
                                             {PLANNING_DESTINATION_METHOD}"
                                        ))
                                        .size(LabelSize::Small)
                                        .color(Color::Muted),
                                    ),
                            )
                            // A refusal offers no retry: no number of retries
                            // adds a capability to an installed component, and
                            // a button that cannot work is a worse answer than
                            // no button.
                            .when(retryable, |header| {
                                header.child(
                                    Button::new("omega-planning-refresh", "Refresh")
                                        .style(ButtonStyle::Outlined)
                                        .size(ButtonSize::Compact)
                                        .on_click(cx.listener(|this, _, _, cx| {
                                            this.request_refresh(cx);
                                        })),
                                )
                            }),
                    )
                    .child(self.render_state_banner(cx)),
            )
            .child(
                v_flex()
                    .flex_1()
                    .min_h_0()
                    .px_4()
                    .pb_4()
                    .pt_2()
                    .gap_3()
                    .when(row_count > 0, |body| {
                        body.child(
                            uniform_list(
                                "omega-planning-rows",
                                row_count,
                                cx.processor(Self::render_rows),
                            )
                            .flex_grow_1()
                            .track_scroll(&handle),
                        )
                    })
                    .when_some(self.render_selected_detail(cx), |body, detail| {
                        body.child(detail)
                    }),
            )
    }
}

impl Focusable for OmegaPlanningSurface {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl EventEmitter<PlanningSurfaceEvent> for OmegaPlanningSurface {}
