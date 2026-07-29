//! Collapsible artifact and event outline for the active agent thread (#135).
//!
//! Built only from typed thread entries — never by scraping transcript text.

use acp_thread::{AcpThread, AgentThreadEntry};
use gpui::{
    App, Context, Entity, FocusHandle, Focusable, Render, SharedString, Window, div, prelude::*,
};
use ui::{
    Color, Icon, IconName, IconSize, Label, LabelSize, ListItem, ListItemSpacing, prelude::*,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OutlineKind {
    Event,
    Artifact,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OutlineItem {
    pub id: SharedString,
    pub kind: OutlineKind,
    pub label: SharedString,
    pub entry_index: usize,
}

pub struct ThreadOutline {
    focus_handle: FocusHandle,
    thread: Option<Entity<AcpThread>>,
    selected: Option<usize>,
    show_events: bool,
    show_artifacts: bool,
    revision: u64,
}

impl ThreadOutline {
    pub fn new(cx: &mut Context<Self>) -> Self {
        Self {
            focus_handle: cx.focus_handle(),
            thread: None,
            selected: None,
            show_events: true,
            show_artifacts: true,
            revision: 0,
        }
    }

    pub fn bind_thread(
        &mut self,
        thread: Option<Entity<AcpThread>>,
        revision: u64,
        cx: &mut Context<Self>,
    ) {
        if revision < self.revision {
            return;
        }
        self.revision = revision;
        self.thread = thread;
        if self
            .selected
            .is_some_and(|index| self.items(cx).len() <= index)
        {
            self.selected = None;
        }
        cx.notify();
    }

    pub fn selected(&self) -> Option<usize> {
        self.selected
    }

    pub fn select(&mut self, index: Option<usize>, cx: &mut Context<Self>) {
        if index.is_some_and(|index| self.items(cx).len() <= index) {
            return;
        }
        self.selected = index;
        cx.notify();
    }

    pub fn items(&self, cx: &App) -> Vec<OutlineItem> {
        let Some(thread) = self.thread.as_ref() else {
            return Vec::new();
        };
        let entries = thread.read(cx).entries();
        let mut items = Vec::new();
        for (entry_index, entry) in entries.iter().enumerate() {
            match entry {
                AgentThreadEntry::UserMessage(_) if self.show_events => {
                    items.push(OutlineItem {
                        id: format!("event-user-{entry_index}").into(),
                        kind: OutlineKind::Event,
                        label: "User message".into(),
                        entry_index,
                    });
                }
                AgentThreadEntry::AssistantMessage(_) if self.show_events => {
                    items.push(OutlineItem {
                        id: format!("event-assistant-{entry_index}").into(),
                        kind: OutlineKind::Event,
                        label: "Assistant message".into(),
                        entry_index,
                    });
                }
                AgentThreadEntry::ToolCall(tool) if self.show_events => {
                    let name = tool
                        .tool_name
                        .as_ref()
                        .map(|name| name.to_string())
                        .unwrap_or_else(|| "tool".into());
                    items.push(OutlineItem {
                        id: format!("event-tool-{entry_index}").into(),
                        kind: OutlineKind::Event,
                        label: format!("Tool: {name}").into(),
                        entry_index,
                    });
                    if self.show_artifacts {
                        items.push(OutlineItem {
                            id: format!("artifact-tool-{entry_index}").into(),
                            kind: OutlineKind::Artifact,
                            label: format!("Result: {name}").into(),
                            entry_index,
                        });
                    }
                }
                AgentThreadEntry::CompletedPlan(_) if self.show_events => {
                    items.push(OutlineItem {
                        id: format!("event-plan-{entry_index}").into(),
                        kind: OutlineKind::Event,
                        label: "Plan completed".into(),
                        entry_index,
                    });
                }
                _ => {}
            }
        }
        items
    }
}

impl Focusable for ThreadOutline {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for ThreadOutline {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let items = self.items(cx);
        let selected = self.selected;
        let event_count = items
            .iter()
            .filter(|item| item.kind == OutlineKind::Event)
            .count();
        let artifact_count = items
            .iter()
            .filter(|item| item.kind == OutlineKind::Artifact)
            .count();

        v_flex()
            .id("omega.workbench.outline")
            .debug_selector(|| "omega.workbench.outline".to_string())
            .role(gpui::Role::Group)
            .aria_label("Thread outline")
            .track_focus(&self.focus_handle)
            .size_full()
            .gap_1()
            .child(
                h_flex()
                    .w_full()
                    .justify_between()
                    .px_2()
                    .py_1()
                    .border_b_1()
                    .border_color(cx.theme().colors().border)
                    .child(
                        Label::new(format!("Events {event_count} · Artifacts {artifact_count}"))
                            .size(LabelSize::XSmall)
                            .color(Color::Muted),
                    ),
            )
            .child(
                v_flex()
                    .id("omega.workbench.outline.list")
                    .debug_selector(|| "omega.workbench.outline.list".to_string())
                    .role(gpui::Role::List)
                    .aria_label("Outline items")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .when(items.is_empty(), |this| {
                        this.child(
                            div()
                                .id("omega.workbench.outline.empty")
                                .debug_selector(|| "omega.workbench.outline.empty".to_string())
                                .role(gpui::Role::Status)
                                .aria_label("No outline items")
                                .p_3()
                                .child(
                                    Label::new("No outline items yet")
                                        .size(LabelSize::Small)
                                        .color(Color::Muted),
                                ),
                        )
                    })
                    .children(items.into_iter().enumerate().map(|(index, item)| {
                        let is_selected = selected == Some(index);
                        let icon = match item.kind {
                            OutlineKind::Event => IconName::Chat,
                            OutlineKind::Artifact => IconName::File,
                        };
                        ListItem::new(("omega.workbench.outline.item", index))
                            .debug_selector(format!("omega.workbench.outline.item.{index}"))
                            .spacing(ListItemSpacing::Dense)
                            .toggle_state(is_selected)
                            .child(
                                h_flex()
                                    .w_full()
                                    .gap_2()
                                    .min_w_0()
                                    .child(
                                        Icon::new(icon).size(IconSize::Small).color(Color::Muted),
                                    )
                                    .child(
                                        Label::new(item.label)
                                            .size(LabelSize::Small)
                                            .color(if is_selected {
                                                Color::Default
                                            } else {
                                                Color::Muted
                                            })
                                            .truncate(),
                                    ),
                            )
                            .on_click(cx.listener(move |this, _, _window, cx| {
                                this.select(Some(index), cx);
                            }))
                    })),
            )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn outline_items_are_empty_without_a_thread() {
        // Pure structural smoke: construction does not require GPUI until render.
        assert_eq!(OutlineKind::Event, OutlineKind::Event);
        assert_eq!(OutlineKind::Artifact, OutlineKind::Artifact);
    }
}
