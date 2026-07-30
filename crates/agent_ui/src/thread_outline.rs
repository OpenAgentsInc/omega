//! Retained, typed event and artifact outline for the active agent thread.

#[cfg(any(test, feature = "test-support"))]
use std::path::PathBuf;
use std::{collections::BTreeMap, ops::Range, rc::Rc, sync::Arc};

use acp_thread::{
    AcpThread, AcpThreadEvent, ThreadActionTarget, ThreadArtifact, ThreadArtifactId, ThreadEntryId,
    ThreadEventKind, ThreadEventProjection, ThreadEventStatus, ThreadProjectionBinding,
    ThreadProjectionSnapshot, ThreadStatus,
};
use gpui::{
    App, Context, Entity, EntityId, FocusHandle, Focusable, Render, ScrollStrategy, SharedString,
    Subscription, UniformListScrollHandle, Window, actions, uniform_list,
};
use omega_workbench_state::RepositoryBinding;
use ui::{
    Button, ButtonSize, ButtonStyle, Color, Icon, IconButton, IconName, IconSize, Label, LabelSize,
    ListItem, ListItemSpacing, prelude::*,
};

actions!(
    omega_thread_outline,
    [
        ToggleOutline,
        SelectEvents,
        SelectArtifacts,
        CycleOutlineFilter,
        SelectNextOutlineItem,
        SelectPreviousOutlineItem,
        ActivateOutlineItem,
        NavigatePreviousArtifactSource,
        NavigateNextArtifactSource,
    ]
);

const OUTLINE_WIDTH: gpui::Pixels = gpui::px(280.);
const COLLAPSED_WIDTH: gpui::Pixels = gpui::px(36.);
pub const OUTLINE_EXPANDED_MIN_VIEWPORT_WIDTH: gpui::Pixels = gpui::px(1120.);

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ThreadOutlineBinding {
    pub thread_id: String,
    pub repository: Option<RepositoryBinding>,
    pub generation: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ThreadOutlineLifecycle {
    Loading,
    Ready,
    Streaming,
    PartiallyAvailable(SharedString),
    Error(SharedString),
    Stale,
    Reconnecting,
}

impl ThreadOutlineLifecycle {
    fn label(&self) -> Option<SharedString> {
        match self {
            Self::Loading => Some("Loading thread outline…".into()),
            Self::Ready => None,
            Self::Streaming => Some("Updating…".into()),
            Self::PartiallyAvailable(message) => Some(message.clone()),
            Self::Error(message) => Some(message.clone()),
            Self::Stale => Some("Showing the last verified outline".into()),
            Self::Reconnecting => Some("Reconnecting; outline updates are paused".into()),
        }
    }

    fn selector_suffix(&self) -> &'static str {
        match self {
            Self::Loading => "loading",
            Self::Ready => "ready",
            Self::Streaming => "streaming",
            Self::PartiallyAvailable(_) => "partial",
            Self::Error(_) => "error",
            Self::Stale => "stale",
            Self::Reconnecting => "reconnecting",
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum OutlineView {
    #[default]
    Events,
    Artifacts,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum OutlineFilter {
    #[default]
    All,
    Active,
    Problems,
}

impl OutlineFilter {
    fn next(self) -> Self {
        match self {
            Self::All => Self::Active,
            Self::Active => Self::Problems,
            Self::Problems => Self::All,
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::All => "All",
            Self::Active => "Active",
            Self::Problems => "Problems",
        }
    }

    fn includes(self, status: ThreadEventStatus) -> bool {
        match self {
            Self::All => true,
            Self::Active => matches!(
                status,
                ThreadEventStatus::Pending
                    | ThreadEventStatus::WaitingForConfirmation
                    | ThreadEventStatus::InProgress
            ),
            Self::Problems => matches!(
                status,
                ThreadEventStatus::Failed
                    | ThreadEventStatus::Rejected
                    | ThreadEventStatus::Canceled
                    | ThreadEventStatus::Unknown
            ),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum OutlineItemId {
    Event(ThreadEntryId),
    Artifact(ThreadArtifactId),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OutlineItem {
    pub outline_binding: ThreadOutlineBinding,
    pub projection_binding: ThreadProjectionBinding,
    pub id: OutlineItemId,
    pub label: SharedString,
    pub entry_id: ThreadEntryId,
    pub parent_id: Option<ThreadEntryId>,
    pub entry_revision: u64,
    pub entry_index: Option<usize>,
    pub artifact_source_events: Vec<ThreadEntryId>,
    pub artifact_revision: Option<u64>,
    pub artifact_is_current: bool,
    pub artifact_history_len: usize,
    pub status: ThreadEventStatus,
    pub action: Option<ThreadActionTarget>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct RetainedOutlineState {
    collapsed: bool,
    view: OutlineView,
    filter: OutlineFilter,
    selected: Option<OutlineItemId>,
    artifact_source_cursors: BTreeMap<ThreadArtifactId, usize>,
}

type NavigateHandler = Rc<dyn Fn(OutlineItem, &mut Window, &mut App) -> bool>;
type ArtifactActionHandler = Rc<dyn Fn(OutlineItem, &mut Window, &mut App) -> OutlineActionOutcome>;
type ArtifactSourceNavigationHandler =
    Rc<dyn Fn(OutlineItem, ThreadEntryId, usize, &mut Window, &mut App) -> bool>;

pub struct ThreadOutline {
    focus_handle: FocusHandle,
    binding: Option<ThreadOutlineBinding>,
    thread: Option<Entity<AcpThread>>,
    bound_entity_id: Option<EntityId>,
    _thread_subscription: Option<Subscription>,
    snapshot: ThreadProjectionSnapshot,
    lifecycle: ThreadOutlineLifecycle,
    state: RetainedOutlineState,
    retained_state: BTreeMap<String, RetainedOutlineState>,
    list_scroll_handle: UniformListScrollHandle,
    rendered_range: Range<usize>,
    responsive_collapsed: bool,
    navigate_handler: Option<NavigateHandler>,
    artifact_action_handler: Option<ArtifactActionHandler>,
    artifact_source_navigation_handler: Option<ArtifactSourceNavigationHandler>,
    stale_update_count: u64,
    replay_update_count: u64,
    foreign_update_count: u64,
    frozen_update_count: u64,
    // Read only by the test-support snapshot; production paths only write it.
    #[cfg_attr(not(any(test, feature = "test-support")), allow(dead_code))]
    conflicting_update_count: u64,
    last_action_succeeded: Option<bool>,
    last_action_message: Option<SharedString>,
    /// When true, production `bind_thread` / projection refresh is ignored so
    /// visual/harness scenes can keep a deterministic synthetic seed.
    synthetic_seed: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum OutlineActionOutcome {
    Completed,
    Pending,
    SourceFallback,
    Unavailable(SharedString),
}

impl OutlineActionOutcome {
    pub(crate) fn succeeded(&self) -> bool {
        !matches!(self, Self::Unavailable(_))
    }

    fn message(&self) -> SharedString {
        match self {
            Self::Completed => "Outline action completed".into(),
            Self::Pending => "Outline action is in progress".into(),
            Self::SourceFallback => {
                "Native action unavailable; navigated to its source event".into()
            }
            Self::Unavailable(message) => message.clone(),
        }
    }
}

impl ThreadOutline {
    pub fn new(cx: &mut Context<Self>) -> Self {
        Self {
            focus_handle: cx.focus_handle(),
            binding: None,
            thread: None,
            bound_entity_id: None,
            _thread_subscription: None,
            snapshot: ThreadProjectionSnapshot {
                binding: ThreadProjectionBinding {
                    thread_id: "".into(),
                    work_dirs: Arc::from([]),
                },
                thread_id: "".into(),
                work_dirs: Arc::from([]),
                revision: 0,
                entries: Vec::new(),
                artifacts: Vec::new(),
            },
            lifecycle: ThreadOutlineLifecycle::Loading,
            state: RetainedOutlineState::default(),
            retained_state: BTreeMap::new(),
            list_scroll_handle: UniformListScrollHandle::new(),
            rendered_range: 0..0,
            responsive_collapsed: false,
            navigate_handler: None,
            artifact_action_handler: None,
            artifact_source_navigation_handler: None,
            stale_update_count: 0,
            replay_update_count: 0,
            foreign_update_count: 0,
            frozen_update_count: 0,
            conflicting_update_count: 0,
            last_action_succeeded: None,
            last_action_message: None,
            synthetic_seed: false,
        }
    }

    pub fn set_navigation_handler(&mut self, handler: NavigateHandler) {
        self.navigate_handler = Some(handler);
    }

    pub fn set_artifact_action_handler(&mut self, handler: ArtifactActionHandler) {
        self.artifact_action_handler = Some(handler);
    }

    pub fn set_artifact_source_navigation_handler(
        &mut self,
        handler: ArtifactSourceNavigationHandler,
    ) {
        self.artifact_source_navigation_handler = Some(handler);
    }

    pub fn binding(&self) -> Option<&ThreadOutlineBinding> {
        self.binding.as_ref()
    }

    pub fn bound_entity_id(&self) -> Option<EntityId> {
        self.bound_entity_id
    }

    pub fn bind_thread(
        &mut self,
        binding: ThreadOutlineBinding,
        thread: Entity<AcpThread>,
        lifecycle: ThreadOutlineLifecycle,
        cx: &mut Context<Self>,
    ) {
        if self.synthetic_seed {
            // Visual/harness seeds own the outline until unbind/force-clear.
            return;
        }
        let entity_id = thread.entity_id();
        let was_frozen = matches!(
            self.lifecycle,
            ThreadOutlineLifecycle::Stale | ThreadOutlineLifecycle::Reconnecting
        );
        let should_freeze = matches!(
            lifecycle,
            ThreadOutlineLifecycle::Stale | ThreadOutlineLifecycle::Reconnecting
        );
        let binding_generation = binding.generation;
        let same_exact_binding =
            self.binding.as_ref() == Some(&binding) && self.bound_entity_id == Some(entity_id);
        if !same_exact_binding {
            if let Some(previous) = self.binding.as_ref() {
                self.retained_state
                    .insert(previous.thread_id.clone(), self.state.clone());
            }
            self.state = self
                .retained_state
                .get(&binding.thread_id)
                .cloned()
                .unwrap_or_default();
            self.binding = Some(binding);
            self.bound_entity_id = Some(entity_id);
            self.thread = Some(thread.clone());
            self.snapshot = ThreadProjectionSnapshot {
                binding: ThreadProjectionBinding {
                    thread_id: "".into(),
                    work_dirs: Arc::from([]),
                },
                thread_id: "".into(),
                work_dirs: Arc::from([]),
                revision: 0,
                entries: Vec::new(),
                artifacts: Vec::new(),
            };
            self.rendered_range = 0..0;
            self.last_action_succeeded = None;
            self.last_action_message = None;
            let expected_entity_id = entity_id;
            let expected_generation = binding_generation;
            self._thread_subscription = Some(cx.subscribe(
                &thread,
                move |this, thread, event: &AcpThreadEvent, cx| {
                    if !matches!(
                        event,
                        AcpThreadEvent::ProjectionUpdated(_)
                            | AcpThreadEvent::StatusChanged
                            | AcpThreadEvent::Stopped(_)
                            | AcpThreadEvent::Error
                            | AcpThreadEvent::LoadError(_)
                            | AcpThreadEvent::Refusal
                    ) {
                        return;
                    }
                    if this.bound_entity_id != Some(expected_entity_id)
                        || thread.entity_id() != expected_entity_id
                        || this.binding.as_ref().map(|binding| binding.generation)
                            != Some(expected_generation)
                    {
                        this.foreign_update_count = this.foreign_update_count.saturating_add(1);
                        return;
                    }
                    if matches!(
                        this.lifecycle,
                        ThreadOutlineLifecycle::Stale | ThreadOutlineLifecycle::Reconnecting
                    ) {
                        if thread.read(cx).projection(cx).revision > this.snapshot.revision {
                            this.frozen_update_count = this.frozen_update_count.saturating_add(1);
                        }
                        cx.notify();
                        return;
                    }
                    this.lifecycle = match event {
                        AcpThreadEvent::Error | AcpThreadEvent::LoadError(_) => {
                            ThreadOutlineLifecycle::Error(
                                "The thread stopped before the outline fully updated".into(),
                            )
                        }
                        AcpThreadEvent::Refusal => ThreadOutlineLifecycle::PartiallyAvailable(
                            "Showing the outline available before the refusal".into(),
                        ),
                        _ if matches!(
                            this.lifecycle,
                            ThreadOutlineLifecycle::Ready | ThreadOutlineLifecycle::Streaming
                        ) =>
                        {
                            match thread.read(cx).status() {
                                ThreadStatus::Idle => ThreadOutlineLifecycle::Ready,
                                ThreadStatus::Generating => ThreadOutlineLifecycle::Streaming,
                            }
                        }
                        _ => this.lifecycle.clone(),
                    };
                    this.refresh_projection(&thread, cx);
                },
            ));
        }
        self.lifecycle = lifecycle;
        if !same_exact_binding {
            self.snapshot = thread.read(cx).projection(cx);
            self.reconcile_selection();
            cx.notify();
        } else if should_freeze {
            if thread.read(cx).projection(cx).revision > self.snapshot.revision {
                self.frozen_update_count = self.frozen_update_count.saturating_add(1);
            }
            cx.notify();
        } else if was_frozen {
            self.snapshot = thread.read(cx).projection(cx);
            self.reconcile_selection();
            cx.notify();
        } else if thread.read(cx).projection(cx).revision > self.snapshot.revision {
            self.refresh_projection(&thread, cx);
        } else {
            cx.notify();
        }
    }

    pub fn unbind(&mut self, cx: &mut Context<Self>) {
        if self.binding.is_none() && self.thread.is_none() && !self.synthetic_seed {
            return;
        }
        if let Some(previous) = self.binding.take() {
            self.retained_state
                .insert(previous.thread_id, self.state.clone());
        }
        self.thread = None;
        self.bound_entity_id = None;
        self._thread_subscription = None;
        self.synthetic_seed = false;
        self.snapshot = ThreadProjectionSnapshot {
            binding: ThreadProjectionBinding {
                thread_id: "".into(),
                work_dirs: Arc::from([]),
            },
            thread_id: "".into(),
            work_dirs: Arc::from([]),
            revision: 0,
            entries: Vec::new(),
            artifacts: Vec::new(),
        };
        self.rendered_range = 0..0;
        self.lifecycle = ThreadOutlineLifecycle::Loading;
        self.state.selected = None;
        self.last_action_succeeded = None;
        self.last_action_message = None;
        cx.notify();
    }

    pub fn report_artifact_action_result(
        &mut self,
        expected_binding: &ThreadOutlineBinding,
        artifact_id: ThreadArtifactId,
        artifact_revision: u64,
        result: Result<(), SharedString>,
        cx: &mut Context<Self>,
    ) {
        let item_is_current = self.binding.as_ref() == Some(expected_binding)
            && self.snapshot.artifacts.iter().any(|artifact| {
                artifact.binding == self.snapshot.binding
                    && artifact.id == artifact_id
                    && artifact.revision == artifact_revision
            });
        if !item_is_current {
            self.foreign_update_count = self.foreign_update_count.saturating_add(1);
            return;
        }
        match result {
            Ok(()) => {
                self.last_action_succeeded = Some(true);
                self.last_action_message = Some("Artifact opened".into());
            }
            Err(message) => {
                self.last_action_succeeded = Some(false);
                self.last_action_message = Some(message);
            }
        }
        cx.notify();
    }

    pub fn set_lifecycle(
        &mut self,
        generation: u64,
        lifecycle: ThreadOutlineLifecycle,
        cx: &mut Context<Self>,
    ) {
        if self
            .binding
            .as_ref()
            .is_none_or(|binding| binding.generation != generation)
        {
            self.foreign_update_count = self.foreign_update_count.saturating_add(1);
            return;
        }
        self.lifecycle = lifecycle;
        cx.notify();
    }

    fn refresh_projection(&mut self, thread: &Entity<AcpThread>, cx: &mut Context<Self>) {
        if self.bound_entity_id != Some(thread.entity_id()) {
            self.foreign_update_count = self.foreign_update_count.saturating_add(1);
            return;
        }
        let snapshot = thread.read(cx).projection(cx);
        if snapshot.revision < self.snapshot.revision {
            self.stale_update_count = self.stale_update_count.saturating_add(1);
            return;
        }
        if snapshot.revision == self.snapshot.revision {
            self.replay_update_count = self.replay_update_count.saturating_add(1);
            return;
        }
        self.snapshot = snapshot;
        let items = self.visible_items();
        if self
            .state
            .selected
            .as_ref()
            .is_some_and(|selected| !items.iter().any(|item| &item.id == selected))
        {
            self.state.selected = None;
        }
        cx.notify();
    }

    pub fn set_collapsed(&mut self, collapsed: bool, cx: &mut Context<Self>) {
        self.state.collapsed = collapsed;
        self.retain_current_state();
        cx.notify();
    }

    pub fn select_view(&mut self, view: OutlineView, cx: &mut Context<Self>) {
        self.state.view = view;
        self.rendered_range = 0..0;
        self.reconcile_selection();
        self.retain_current_state();
        cx.notify();
    }

    pub fn set_filter(&mut self, filter: OutlineFilter, cx: &mut Context<Self>) {
        self.state.filter = filter;
        self.rendered_range = 0..0;
        self.reconcile_selection();
        self.retain_current_state();
        cx.notify();
    }

    fn retain_current_state(&mut self) {
        if let Some(binding) = self.binding.as_ref() {
            self.retained_state
                .insert(binding.thread_id.clone(), self.state.clone());
        }
    }

    fn reconcile_selection(&mut self) {
        let items = self.visible_items();
        if self
            .state
            .selected
            .as_ref()
            .is_some_and(|selected| !items.iter().any(|item| &item.id == selected))
        {
            self.state.selected = None;
        }
    }

    pub fn counts(&self) -> (usize, usize) {
        let events = self.event_items(OutlineFilter::All).len();
        let artifacts = self.artifact_items(OutlineFilter::All).len();
        (events, artifacts)
    }

    pub fn is_responsively_collapsed(viewport_width: gpui::Pixels) -> bool {
        viewport_width < OUTLINE_EXPANDED_MIN_VIEWPORT_WIDTH
    }

    pub fn visible_items(&self) -> Vec<OutlineItem> {
        match self.state.view {
            OutlineView::Events => self.event_items(self.state.filter),
            OutlineView::Artifacts => self.artifact_items(self.state.filter),
        }
    }

    fn event_items(&self, filter: OutlineFilter) -> Vec<OutlineItem> {
        let Some(outline_binding) = self.binding.as_ref() else {
            return Vec::new();
        };
        self.snapshot
            .entries
            .iter()
            .filter(|entry| entry.binding == self.snapshot.binding)
            .filter(|entry| filter.includes(entry.status))
            .map(|entry| OutlineItem {
                outline_binding: outline_binding.clone(),
                projection_binding: entry.binding.clone(),
                id: OutlineItemId::Event(entry.id),
                label: event_label(entry),
                entry_id: entry.id,
                parent_id: entry.parent_id,
                entry_revision: entry.revision,
                entry_index: entry.entry_index,
                artifact_source_events: vec![entry.id],
                artifact_revision: None,
                artifact_is_current: true,
                artifact_history_len: 0,
                status: entry.status,
                action: Some(ThreadActionTarget::Entry(entry.id)),
            })
            .collect()
    }

    fn artifact_items(&self, filter: OutlineFilter) -> Vec<OutlineItem> {
        let Some(outline_binding) = self.binding.as_ref() else {
            return Vec::new();
        };
        let mut items = Vec::new();
        for artifact in &self.snapshot.artifacts {
            if artifact.binding != self.snapshot.binding || !filter.includes(artifact.status) {
                continue;
            }
            let Some(entry) = artifact
                .source_events
                .iter()
                .rev()
                .find_map(|source_event| {
                    self.snapshot.entries.iter().find(|entry| {
                        entry.id == *source_event && entry.binding == self.snapshot.binding
                    })
                })
            else {
                continue;
            };
            let mut label = artifact_label(&artifact.artifact).to_string();
            if artifact.source_events.len() > 1 || !artifact.history.is_empty() {
                label.push_str(&format!(
                    " · {} sources · {} revisions",
                    artifact.source_events.len(),
                    artifact.history.len().saturating_add(1)
                ));
            }
            items.push(OutlineItem {
                outline_binding: outline_binding.clone(),
                projection_binding: artifact.binding.clone(),
                id: OutlineItemId::Artifact(artifact.id),
                label: if artifact.is_current {
                    label.into()
                } else {
                    format!("{label} · historical").into()
                },
                entry_id: entry.id,
                parent_id: None,
                entry_revision: entry.revision,
                entry_index: entry.entry_index,
                artifact_source_events: artifact.source_events.clone(),
                artifact_revision: Some(artifact.revision),
                artifact_is_current: artifact.is_current,
                artifact_history_len: artifact.history.len(),
                status: artifact.status,
                action: artifact.action_target.clone(),
            });
        }
        items
    }

    fn select_index(&mut self, index: usize, cx: &mut Context<Self>) {
        let Some(item) = self.visible_items().get(index).cloned() else {
            return;
        };
        self.state.selected = Some(item.id);
        self.list_scroll_handle
            .scroll_to_item(index, ScrollStrategy::Center);
        self.retain_current_state();
        cx.notify();
    }

    fn move_selection(&mut self, delta: isize, cx: &mut Context<Self>) {
        let items = self.visible_items();
        if items.is_empty() {
            self.state.selected = None;
            cx.notify();
            return;
        }
        let current = self
            .state
            .selected
            .as_ref()
            .and_then(|selected| items.iter().position(|item| &item.id == selected));
        let next = if delta < 0 {
            current.unwrap_or(0).saturating_sub(1)
        } else {
            current.map_or(0, |index| (index + 1).min(items.len() - 1))
        };
        self.select_index(next, cx);
    }

    fn activate_selected(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        let Some(selected) = self.state.selected.clone() else {
            self.last_action_succeeded = Some(false);
            self.last_action_message = Some("Select an outline item first".into());
            cx.notify();
            return false;
        };
        let Some(item) = self
            .visible_items()
            .into_iter()
            .find(|item| item.id == selected)
        else {
            self.last_action_succeeded = Some(false);
            self.last_action_message =
                Some("The selected outline item is no longer available".into());
            cx.notify();
            return false;
        };
        let succeeded = match item.id {
            OutlineItemId::Event(_) => {
                if self
                    .navigate_handler
                    .as_ref()
                    .is_some_and(|handler| handler(item.clone(), window, cx))
                {
                    OutlineActionOutcome::Completed
                } else {
                    OutlineActionOutcome::Unavailable(
                        "This event has no navigable transcript entry".into(),
                    )
                }
            }
            OutlineItemId::Artifact(_) => self.artifact_action_handler.as_ref().map_or_else(
                || OutlineActionOutcome::Unavailable("No artifact action is available".into()),
                |handler| handler(item, window, cx),
            ),
        };
        self.last_action_succeeded = Some(succeeded.succeeded());
        self.last_action_message = Some(succeeded.message());
        cx.notify();
        succeeded.succeeded()
    }

    fn navigate_artifact_source(
        &mut self,
        delta: isize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some(OutlineItemId::Artifact(artifact_id)) = self.state.selected.clone() else {
            return false;
        };
        let Some(item) = self
            .artifact_items(OutlineFilter::All)
            .into_iter()
            .find(|item| item.id == OutlineItemId::Artifact(artifact_id))
        else {
            return false;
        };
        if item.artifact_source_events.is_empty() {
            return false;
        }
        let current = self
            .state
            .artifact_source_cursors
            .get(&artifact_id)
            .copied()
            .unwrap_or(item.artifact_source_events.len() - 1);
        let next = if delta < 0 {
            current.saturating_sub(1)
        } else {
            (current + 1).min(item.artifact_source_events.len() - 1)
        };
        let Some(source_event) = item.artifact_source_events.get(next).copied() else {
            return false;
        };
        let Some((entry, entry_index)) = self.snapshot.entries.iter().find_map(|entry| {
            (entry.id == source_event)
                .then_some(entry.entry_index)
                .flatten()
                .map(|entry_index| (entry, entry_index))
        }) else {
            return false;
        };
        let succeeded = self
            .artifact_source_navigation_handler
            .as_ref()
            .is_some_and(|handler| handler(item.clone(), entry.id, entry_index, window, cx));
        if succeeded {
            self.state.artifact_source_cursors.insert(artifact_id, next);
            self.retain_current_state();
        }
        self.last_action_succeeded = Some(succeeded);
        cx.notify();
        succeeded
    }

    fn select_and_activate(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        self.select_index(index, cx);
        self.activate_selected(window, cx);
    }

    fn render_items(
        &mut self,
        range: Range<usize>,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Vec<ListItem> {
        let items = self.visible_items();
        self.rendered_range = range.start.min(items.len())..range.end.min(items.len());
        range
            .filter_map(|index| {
                let item = items.get(index)?.clone();
                let selected = self.state.selected.as_ref() == Some(&item.id);
                let initially_reachable = self.state.selected.is_none() && index == 0;
                let selector = match &item.id {
                    OutlineItemId::Event(ThreadEntryId(id)) => {
                        format!("omega.thread-outline.item.event.{id}")
                    }
                    OutlineItemId::Artifact(ThreadArtifactId(id)) => {
                        format!("omega.thread-outline.item.artifact.{id}")
                    }
                };
                let aria_label = format!("{}; {}", item.label, status_label(item.status));
                Some(
                    ListItem::new(selector.clone())
                        .debug_selector(selector)
                        .aria_role(gpui::Role::ListItem)
                        .aria_label(aria_label)
                        .spacing(ListItemSpacing::Dense)
                        .toggle_state(selected)
                        .child(
                            h_flex()
                                .tab_index(if selected || initially_reachable {
                                    0
                                } else {
                                    -1
                                })
                                .w_full()
                                .h_6()
                                .min_w_0()
                                .gap_2()
                                .when(item.parent_id.is_some(), |this| this.pl_4())
                                .child(
                                    Icon::new(status_icon(item.status))
                                        .size(IconSize::Small)
                                        .color(status_color(item.status)),
                                )
                                .child(
                                    Label::new(item.label)
                                        .size(LabelSize::Small)
                                        .color(if selected {
                                            Color::Default
                                        } else {
                                            Color::Muted
                                        })
                                        .truncate(),
                                ),
                        )
                        .on_click(cx.listener(move |this, _, window, cx| {
                            this.select_and_activate(index, window, cx);
                        })),
                )
            })
            .collect()
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn bind_projection_for_tests(
        &mut self,
        binding: ThreadOutlineBinding,
        entity_id: EntityId,
        snapshot: ThreadProjectionSnapshot,
        lifecycle: ThreadOutlineLifecycle,
        cx: &mut Context<Self>,
    ) {
        let was_frozen = matches!(
            self.lifecycle,
            ThreadOutlineLifecycle::Stale | ThreadOutlineLifecycle::Reconnecting
        );
        let should_freeze = matches!(
            lifecycle,
            ThreadOutlineLifecycle::Stale | ThreadOutlineLifecycle::Reconnecting
        );
        let same_exact_binding =
            self.binding.as_ref() == Some(&binding) && self.bound_entity_id == Some(entity_id);
        if !same_exact_binding {
            if let Some(previous) = self.binding.as_ref() {
                self.retained_state
                    .insert(previous.thread_id.clone(), self.state.clone());
            }
            self.state = self
                .retained_state
                .get(&binding.thread_id)
                .cloned()
                .unwrap_or_default();
            self.binding = Some(binding);
            self.bound_entity_id = Some(entity_id);
            self.snapshot = ThreadProjectionSnapshot {
                binding: ThreadProjectionBinding {
                    thread_id: "".into(),
                    work_dirs: Arc::from([]),
                },
                thread_id: "".into(),
                work_dirs: Arc::from([]),
                revision: 0,
                entries: Vec::new(),
                artifacts: Vec::new(),
            };
            self.rendered_range = 0..0;
            self.last_action_succeeded = None;
            self.last_action_message = None;
        }
        self.lifecycle = lifecycle;
        if same_exact_binding && should_freeze {
            match snapshot.revision.cmp(&self.snapshot.revision) {
                std::cmp::Ordering::Less => {
                    self.stale_update_count = self.stale_update_count.saturating_add(1);
                }
                std::cmp::Ordering::Greater => {
                    self.frozen_update_count = self.frozen_update_count.saturating_add(1);
                }
                std::cmp::Ordering::Equal => {}
            }
            cx.notify();
            return;
        }
        if same_exact_binding && snapshot.revision < self.snapshot.revision {
            self.stale_update_count = self.stale_update_count.saturating_add(1);
            cx.notify();
            return;
        }
        if same_exact_binding && snapshot.revision == self.snapshot.revision {
            if !was_frozen {
                self.replay_update_count = self.replay_update_count.saturating_add(1);
            }
            cx.notify();
            return;
        }
        self.snapshot = snapshot;
        self.reconcile_selection();
        cx.notify();
    }

    /// Force-seed a synthetic projection for visual/harness scenes.
    ///
    /// Unlike [`Self::bind_projection_for_tests`], this always replaces the
    /// current projection and drops any live AcpThread subscription so later
    /// production synchronizes cannot silently overwrite the seed via a
    /// different binding generation.
    #[cfg(any(test, feature = "test-support"))]
    pub fn force_seed_projection_for_tests(
        &mut self,
        binding: ThreadOutlineBinding,
        entity_id: EntityId,
        snapshot: ThreadProjectionSnapshot,
        lifecycle: ThreadOutlineLifecycle,
        cx: &mut Context<Self>,
    ) {
        self.thread = None;
        self._thread_subscription = None;
        self.synthetic_seed = true;
        self.bound_entity_id = Some(entity_id);
        self.binding = Some(binding);
        self.lifecycle = lifecycle;
        self.snapshot = snapshot;
        self.rendered_range = 0..0;
        self.last_action_succeeded = None;
        self.last_action_message = None;
        self.stale_update_count = 0;
        self.replay_update_count = 0;
        self.foreign_update_count = 0;
        self.frozen_update_count = 0;
        self.conflicting_update_count = 0;
        self.reconcile_selection();
        cx.notify();
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn select_index_for_tests(&mut self, index: usize, cx: &mut Context<Self>) {
        self.select_index(index, cx);
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn apply_projection_update_for_tests(
        &mut self,
        entity_id: EntityId,
        snapshot: ThreadProjectionSnapshot,
        cx: &mut Context<Self>,
    ) {
        if matches!(
            self.lifecycle,
            ThreadOutlineLifecycle::Stale | ThreadOutlineLifecycle::Reconnecting
        ) {
            if snapshot.revision > self.snapshot.revision {
                self.frozen_update_count = self.frozen_update_count.saturating_add(1);
            }
            return;
        }
        if self.bound_entity_id != Some(entity_id) {
            self.foreign_update_count = self.foreign_update_count.saturating_add(1);
            return;
        }
        if snapshot.revision < self.snapshot.revision {
            self.stale_update_count = self.stale_update_count.saturating_add(1);
            return;
        }
        if snapshot.revision == self.snapshot.revision {
            let same_content = snapshot.entries == self.snapshot.entries
                && snapshot.artifacts == self.snapshot.artifacts;
            if same_content {
                self.replay_update_count = self.replay_update_count.saturating_add(1);
            } else {
                self.conflicting_update_count = self.conflicting_update_count.saturating_add(1);
            }
            return;
        }
        self.snapshot = snapshot;
        self.reconcile_selection();
        cx.notify();
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn set_virtual_window_for_tests(
        &mut self,
        start: usize,
        len: usize,
        cx: &mut Context<Self>,
    ) {
        let item_count = self.visible_items().len();
        let start = start.min(item_count);
        self.rendered_range = start..start.saturating_add(len).min(item_count);
        cx.notify();
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn focus_handle_for_tests(&self) -> FocusHandle {
        self.focus_handle.clone()
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn snapshot_for_tests(&self) -> ThreadOutlineTestSnapshot {
        let (event_count, artifact_count) = self.counts();
        let selected_artifact_source_index = match self.state.selected.clone() {
            Some(OutlineItemId::Artifact(artifact_id)) => self
                .state
                .artifact_source_cursors
                .get(&artifact_id)
                .copied(),
            _ => None,
        };
        ThreadOutlineTestSnapshot {
            binding: self.binding.clone(),
            bound_entity_id: self.bound_entity_id,
            projection_binding: self.snapshot.binding.clone(),
            projection_thread_id: self.snapshot.thread_id.clone(),
            projection_work_dirs: self.snapshot.work_dirs.clone(),
            revision: self.snapshot.revision,
            lifecycle: self.lifecycle.clone(),
            collapsed: self.state.collapsed,
            view: self.state.view,
            filter: self.state.filter,
            selected: self.state.selected.clone(),
            anchor: self.state.selected.clone(),
            visible_items: self.visible_items(),
            event_count,
            artifact_count,
            rejected_update_count: self
                .stale_update_count
                .saturating_add(self.replay_update_count)
                .saturating_add(self.foreign_update_count)
                .saturating_add(self.frozen_update_count)
                .saturating_add(self.conflicting_update_count),
            stale_update_count: self.stale_update_count,
            replay_update_count: self.replay_update_count,
            foreign_update_count: self.foreign_update_count,
            frozen_update_count: self.frozen_update_count,
            conflicting_update_count: self.conflicting_update_count,
            last_action_succeeded: self.last_action_succeeded,
            last_action_message: self.last_action_message.clone(),
            virtual_start: self.rendered_range.start,
            virtual_len: self.rendered_range.len(),
            responsive_collapsed: self.responsive_collapsed,
            effective_collapsed: self.state.collapsed || self.responsive_collapsed,
            selected_artifact_source_index,
        }
    }
}

impl Focusable for ThreadOutline {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for ThreadOutline {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let (event_count, artifact_count) = self.counts();
        let items = self.visible_items();
        self.responsive_collapsed = Self::is_responsively_collapsed(window.viewport_size().width);
        let effective_collapsed = self.state.collapsed || self.responsive_collapsed;
        let lifecycle_label = self.lifecycle.label();
        let lifecycle_selector = format!(
            "omega.thread-outline.lifecycle.{}",
            self.lifecycle.selector_suffix()
        );
        let selected_artifact = self
            .state
            .selected
            .as_ref()
            .and_then(|selected| items.iter().find(|item| &item.id == selected))
            .and_then(|item| match &item.id {
                OutlineItemId::Artifact(artifact_id) => Some((*artifact_id, item)),
                OutlineItemId::Event(_) => None,
            });
        let selected_artifact_sources = selected_artifact
            .map(|(_, item)| item.artifact_source_events.len())
            .unwrap_or_default();
        let selected_artifact_source_index = selected_artifact
            .map(|(artifact_id, _)| {
                self.state
                    .artifact_source_cursors
                    .get(&artifact_id)
                    .copied()
                    .unwrap_or(selected_artifact_sources.saturating_sub(1))
            })
            .unwrap_or_default();

        v_flex()
            .id("omega.thread-outline")
            .debug_selector(|| "omega.thread-outline".into())
            .key_context("ThreadOutline")
            .role(gpui::Role::Complementary)
            .aria_label("Thread events and artifacts")
            .track_focus(&self.focus_handle)
            .h_full()
            .w(if effective_collapsed {
                COLLAPSED_WIDTH
            } else {
                OUTLINE_WIDTH
            })
            .flex_none()
            .border_l_1()
            .border_color(cx.theme().colors().border)
            .bg(cx.theme().colors().panel_background)
            .on_action(cx.listener(|this, _: &ToggleOutline, _window, cx| {
                if !this.responsive_collapsed || this.state.collapsed {
                    this.set_collapsed(!this.state.collapsed, cx);
                }
            }))
            .on_action(cx.listener(|this, _: &SelectEvents, _window, cx| {
                this.select_view(OutlineView::Events, cx);
            }))
            .on_action(cx.listener(|this, _: &SelectArtifacts, _window, cx| {
                this.select_view(OutlineView::Artifacts, cx);
            }))
            .on_action(cx.listener(|this, _: &CycleOutlineFilter, _window, cx| {
                this.set_filter(this.state.filter.next(), cx);
            }))
            .on_action(cx.listener(|this, _: &SelectNextOutlineItem, _window, cx| {
                this.move_selection(1, cx);
            }))
            .on_action(
                cx.listener(|this, _: &SelectPreviousOutlineItem, _window, cx| {
                    this.move_selection(-1, cx);
                }),
            )
            .on_action(cx.listener(|this, _: &ActivateOutlineItem, window, cx| {
                this.activate_selected(window, cx);
            }))
            .on_action(
                cx.listener(|this, _: &NavigatePreviousArtifactSource, window, cx| {
                    this.navigate_artifact_source(-1, window, cx);
                }),
            )
            .on_action(
                cx.listener(|this, _: &NavigateNextArtifactSource, window, cx| {
                    this.navigate_artifact_source(1, window, cx);
                }),
            )
            .child(
                h_flex()
                    .w_full()
                    .h_8()
                    .px_1()
                    .justify_between()
                    .border_b_1()
                    .border_color(cx.theme().colors().border)
                    .when(!effective_collapsed, |this| {
                        this.child(Label::new("Outline").size(LabelSize::Small))
                    })
                    .child(
                        IconButton::new(
                            "omega-thread-outline-collapse",
                            if effective_collapsed {
                                IconName::ChevronLeft
                            } else {
                                IconName::ChevronRight
                            },
                        )
                        .tab_index(0isize)
                        .aria_label(if self.responsive_collapsed && !self.state.collapsed {
                            "Thread outline collapsed for the narrow window"
                        } else if self.state.collapsed {
                            "Expand thread outline"
                        } else {
                            "Collapse thread outline"
                        })
                        .on_click(cx.listener(|this, _, _window, cx| {
                            if !this.responsive_collapsed || this.state.collapsed {
                                this.set_collapsed(!this.state.collapsed, cx);
                            }
                        })),
                    ),
            )
            .when(!effective_collapsed, |this| {
                this.child(
                    h_flex()
                        .id("omega.thread-outline.tabs")
                        .debug_selector(|| "omega.thread-outline.tabs".into())
                        .role(gpui::Role::Group)
                        .aria_label("Outline views")
                        .w_full()
                        .p_1()
                        .gap_1()
                        .child(
                            Button::new(
                                "omega-thread-outline-events",
                                format!("Events {event_count}"),
                            )
                            .size(ButtonSize::Compact)
                            .style(if self.state.view == OutlineView::Events {
                                ButtonStyle::Filled
                            } else {
                                ButtonStyle::Subtle
                            })
                            .toggle_state(self.state.view == OutlineView::Events)
                            .tab_index(0isize)
                            .aria_label(format!("Events, {event_count} items"))
                            .on_click(cx.listener(
                                |this, _, _window, cx| {
                                    this.select_view(OutlineView::Events, cx);
                                },
                            )),
                        )
                        .child(
                            Button::new(
                                "omega-thread-outline-artifacts",
                                format!("Artifacts {artifact_count}"),
                            )
                            .size(ButtonSize::Compact)
                            .style(if self.state.view == OutlineView::Artifacts {
                                ButtonStyle::Filled
                            } else {
                                ButtonStyle::Subtle
                            })
                            .toggle_state(self.state.view == OutlineView::Artifacts)
                            .tab_index(0isize)
                            .aria_label(format!("Artifacts, {artifact_count} items"))
                            .on_click(cx.listener(
                                |this, _, _window, cx| {
                                    this.select_view(OutlineView::Artifacts, cx);
                                },
                            )),
                        ),
                )
                .child(
                    h_flex()
                        .w_full()
                        .px_2()
                        .pb_1()
                        .justify_between()
                        .child(
                            Button::new(
                                "omega-thread-outline-filter",
                                format!("Filter: {}", self.state.filter.label()),
                            )
                            .size(ButtonSize::Compact)
                            .style(ButtonStyle::Subtle)
                            .tab_index(0isize)
                            .aria_label(format!(
                                "Outline filter: {}. Activate to change.",
                                self.state.filter.label()
                            ))
                            .on_click(cx.listener(
                                |this, _, _window, cx| {
                                    this.set_filter(this.state.filter.next(), cx);
                                },
                            )),
                        )
                        .child(
                            h_flex()
                                .gap_1()
                                .when(selected_artifact_sources > 1, |this| {
                                    this.child(
                                        IconButton::new(
                                            "omega-thread-outline-previous-source",
                                            IconName::ChevronLeft,
                                        )
                                        .tab_index(0isize)
                                        .disabled(selected_artifact_source_index == 0)
                                        .aria_label("Navigate to the previous artifact source")
                                        .on_click(
                                            cx.listener(|this, _, window, cx| {
                                                this.navigate_artifact_source(-1, window, cx);
                                            }),
                                        ),
                                    )
                                    .child(
                                        Label::new(format!(
                                            "{}/{}",
                                            selected_artifact_source_index.saturating_add(1),
                                            selected_artifact_sources
                                        ))
                                        .size(LabelSize::XSmall)
                                        .color(Color::Muted),
                                    )
                                    .child(
                                        IconButton::new(
                                            "omega-thread-outline-next-source",
                                            IconName::ChevronRight,
                                        )
                                        .tab_index(0isize)
                                        .disabled(
                                            selected_artifact_source_index.saturating_add(1)
                                                >= selected_artifact_sources,
                                        )
                                        .aria_label("Navigate to the next artifact source")
                                        .on_click(
                                            cx.listener(|this, _, window, cx| {
                                                this.navigate_artifact_source(1, window, cx);
                                            }),
                                        ),
                                    )
                                })
                                .child(
                                    Label::new(format!("r{}", self.snapshot.revision))
                                        .size(LabelSize::XSmall)
                                        .color(Color::Muted),
                                ),
                        ),
                )
                .when_some(lifecycle_label, |this, label| {
                    this.child(
                        h_flex()
                            .id(SharedString::from(lifecycle_selector.clone()))
                            .debug_selector(move || lifecycle_selector)
                            .role(gpui::Role::Status)
                            .aria_label(label.clone())
                            .w_full()
                            .px_2()
                            .py_1()
                            .gap_1()
                            .bg(cx.theme().colors().element_background)
                            .child(
                                Icon::new(
                                    if matches!(
                                        self.lifecycle,
                                        ThreadOutlineLifecycle::Error(_)
                                            | ThreadOutlineLifecycle::PartiallyAvailable(_)
                                    ) {
                                        IconName::Warning
                                    } else {
                                        IconName::Circle
                                    },
                                )
                                .size(IconSize::XSmall)
                                .color(
                                    if matches!(self.lifecycle, ThreadOutlineLifecycle::Error(_)) {
                                        Color::Error
                                    } else {
                                        Color::Muted
                                    },
                                ),
                            )
                            .child(
                                Label::new(label)
                                    .size(LabelSize::XSmall)
                                    .color(Color::Muted)
                                    .truncate(),
                            ),
                    )
                })
                .child(
                    v_flex()
                        .id("omega.thread-outline.body")
                        .flex_1()
                        .min_h_0()
                        .w_full()
                        .overflow_hidden()
                        .when(items.is_empty(), |this| {
                            this.child(
                                v_flex()
                                    .id("omega.thread-outline.empty")
                                    .debug_selector(|| "omega.thread-outline.empty".into())
                                    .role(gpui::Role::Status)
                                    .aria_label("No matching outline items")
                                    .p_3()
                                    .gap_1()
                                    .child(
                                        Label::new(match self.state.view {
                                            OutlineView::Events => "No matching events",
                                            OutlineView::Artifacts => "No matching artifacts",
                                        })
                                        .size(LabelSize::Small),
                                    )
                                    .child(
                                        Label::new("Change the filter or continue the thread.")
                                            .size(LabelSize::XSmall)
                                            .color(Color::Muted),
                                    ),
                            )
                        })
                        .when(!items.is_empty(), |this| {
                            this.child(
                                div()
                                    .id("omega.thread-outline.list")
                                    .debug_selector(|| "omega.thread-outline.list".into())
                                    .role(gpui::Role::List)
                                    .aria_label(match self.state.view {
                                        OutlineView::Events => "Thread events",
                                        OutlineView::Artifacts => "Thread artifacts",
                                    })
                                    .flex_1()
                                    .size_full()
                                    .min_h_0()
                                    .overflow_hidden()
                                    .child(
                                        uniform_list(
                                            "omega-thread-outline-list-rows",
                                            items.len(),
                                            cx.processor(Self::render_items),
                                        )
                                        .track_scroll(&self.list_scroll_handle)
                                        .size_full(),
                                    ),
                            )
                        }),
                )
                .when_some(self.last_action_message.clone(), |this, message| {
                    this.child(
                        div()
                            .id("omega.thread-outline.action-status")
                            .debug_selector(|| "omega.thread-outline.action-status".into())
                            .role(gpui::Role::Status)
                            .aria_label(message),
                    )
                })
            })
    }
}

fn event_label(entry: &ThreadEventProjection) -> SharedString {
    match entry.kind {
        ThreadEventKind::UserMessage => "User message".into(),
        ThreadEventKind::AssistantMessage => "Assistant message".into(),
        ThreadEventKind::ToolCall => "Tool call".into(),
        ThreadEventKind::Elicitation => "Approval request".into(),
        ThreadEventKind::CompletedPlan => "Completed plan".into(),
        ThreadEventKind::ContextCompaction => "Context compacted".into(),
        ThreadEventKind::SystemNote => "System note".into(),
        ThreadEventKind::Reasoning => "Reasoning".into(),
        ThreadEventKind::ToolResult => "Tool result".into(),
        ThreadEventKind::Approval => "Approval".into(),
        ThreadEventKind::PlanUpdate => "Plan updated".into(),
        ThreadEventKind::Checkpoint => "Checkpoint".into(),
        ThreadEventKind::Error => "Error".into(),
        ThreadEventKind::Retry => "Retry".into(),
        ThreadEventKind::ReplayBoundary => "Reconnect replay".into(),
        ThreadEventKind::Completion => "Completed".into(),
        ThreadEventKind::Cancellation => "Canceled".into(),
        ThreadEventKind::Refusal => "Refused".into(),
        ThreadEventKind::Unknown => "Unknown event".into(),
    }
}

fn artifact_label(artifact: &ThreadArtifact) -> SharedString {
    match artifact {
        ThreadArtifact::File { path, .. } => path.display().to_string().into(),
        ThreadArtifact::Diff { path, .. } => path
            .as_ref()
            .map(|path| format!("Diff: {}", path.display()))
            .unwrap_or_else(|| "Thread diff".into())
            .into(),
        ThreadArtifact::Resource { uri, .. } => uri.to_string().into(),
        ThreadArtifact::Link { name, .. } => name.to_string().into(),
        ThreadArtifact::Image { width, height, .. } => {
            let dimensions = match (width, height) {
                (Some(width), Some(height)) => format!("{width} × {height}"),
                _ => "unknown size".into(),
            };
            format!("Image · {dimensions}").into()
        }
        ThreadArtifact::TerminalResult { terminal_id, .. } => {
            format!("Terminal result · {terminal_id}").into()
        }
    }
}

fn status_icon(status: ThreadEventStatus) -> IconName {
    match status {
        ThreadEventStatus::Completed => IconName::Check,
        ThreadEventStatus::Failed | ThreadEventStatus::Rejected => IconName::Close,
        ThreadEventStatus::Pending
        | ThreadEventStatus::WaitingForConfirmation
        | ThreadEventStatus::InProgress => IconName::Circle,
        ThreadEventStatus::Canceled | ThreadEventStatus::Unknown => IconName::Warning,
    }
}

fn status_label(status: ThreadEventStatus) -> &'static str {
    match status {
        ThreadEventStatus::Pending => "pending",
        ThreadEventStatus::WaitingForConfirmation => "waiting for confirmation",
        ThreadEventStatus::InProgress => "in progress",
        ThreadEventStatus::Completed => "completed",
        ThreadEventStatus::Failed => "failed",
        ThreadEventStatus::Rejected => "rejected",
        ThreadEventStatus::Canceled => "canceled",
        ThreadEventStatus::Unknown => "unknown status",
    }
}

fn status_color(status: ThreadEventStatus) -> Color {
    match status {
        ThreadEventStatus::Completed => Color::Success,
        ThreadEventStatus::Failed | ThreadEventStatus::Rejected => Color::Error,
        ThreadEventStatus::WaitingForConfirmation => Color::Warning,
        ThreadEventStatus::InProgress => Color::Accent,
        ThreadEventStatus::Pending | ThreadEventStatus::Canceled | ThreadEventStatus::Unknown => {
            Color::Muted
        }
    }
}

#[cfg(any(test, feature = "test-support"))]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ThreadOutlineTestSnapshot {
    pub binding: Option<ThreadOutlineBinding>,
    pub bound_entity_id: Option<EntityId>,
    pub projection_binding: ThreadProjectionBinding,
    pub projection_thread_id: Arc<str>,
    pub projection_work_dirs: Arc<[PathBuf]>,
    pub revision: u64,
    pub lifecycle: ThreadOutlineLifecycle,
    pub collapsed: bool,
    pub view: OutlineView,
    pub filter: OutlineFilter,
    pub selected: Option<OutlineItemId>,
    pub anchor: Option<OutlineItemId>,
    pub visible_items: Vec<OutlineItem>,
    pub event_count: usize,
    pub artifact_count: usize,
    pub rejected_update_count: u64,
    pub stale_update_count: u64,
    pub replay_update_count: u64,
    pub foreign_update_count: u64,
    pub frozen_update_count: u64,
    pub conflicting_update_count: u64,
    pub last_action_succeeded: Option<bool>,
    pub last_action_message: Option<SharedString>,
    pub virtual_start: usize,
    pub virtual_len: usize,
    pub responsive_collapsed: bool,
    pub effective_collapsed: bool,
    pub selected_artifact_source_index: Option<usize>,
}

#[cfg(test)]
mod tests {
    use std::{cell::Cell, path::PathBuf, sync::Arc};

    use acp_thread::{ThreadArtifactProjection, ThreadEventOwner, ThreadEventSource};
    use gpui::TestAppContext;

    use super::*;

    fn binding(thread_id: &str, generation: u64) -> ThreadOutlineBinding {
        ThreadOutlineBinding {
            thread_id: thread_id.into(),
            repository: Some(
                RepositoryBinding::new("repository", format!("worktree-{thread_id}"))
                    .expect("valid fixture binding"),
            ),
            generation,
        }
    }

    fn projection_binding() -> ThreadProjectionBinding {
        ThreadProjectionBinding {
            thread_id: "test-thread".into(),
            work_dirs: Arc::from([]),
        }
    }

    fn event(
        id: u64,
        entry_index: usize,
        status: ThreadEventStatus,
        mut artifacts: Vec<ThreadArtifactProjection>,
    ) -> ThreadEventProjection {
        for artifact in &mut artifacts {
            if !artifact.source_events.contains(&ThreadEntryId(id)) {
                artifact.source_events.push(ThreadEntryId(id));
            }
            artifact.status = status;
        }
        ThreadEventProjection {
            binding: projection_binding(),
            id: ThreadEntryId(id),
            parent_id: None,
            revision: 1,
            entry_index: Some(entry_index),
            kind: ThreadEventKind::ToolCall,
            owner: ThreadEventOwner::Tool,
            source: ThreadEventSource::ToolCall(Arc::from(format!("tool-{id}"))),
            status,
            related_kinds: Vec::new(),
            artifacts,
            action_targets: vec![ThreadActionTarget::Entry(ThreadEntryId(id))],
        }
    }

    fn snapshot(revision: u64, entries: Vec<ThreadEventProjection>) -> ThreadProjectionSnapshot {
        let mut artifacts = BTreeMap::new();
        for entry in &entries {
            for artifact in &entry.artifacts {
                artifacts.insert(artifact.id, artifact.clone());
            }
        }
        ThreadProjectionSnapshot {
            binding: projection_binding(),
            thread_id: "test-thread".into(),
            work_dirs: Arc::from([]),
            revision,
            entries,
            artifacts: artifacts.into_values().collect(),
        }
    }

    fn file(id: u64, path: &str) -> ThreadArtifactProjection {
        let artifact = ThreadArtifact::File {
            path: PathBuf::from(path),
            line: None,
        };
        ThreadArtifactProjection {
            binding: projection_binding(),
            id: ThreadArtifactId(id),
            revision: 1,
            source_events: Vec::new(),
            owner: ThreadEventOwner::Tool,
            status: ThreadEventStatus::Completed,
            action_target: Some(ThreadActionTarget::File {
                path: PathBuf::from(path),
                line: None,
            }),
            artifact,
            is_current: true,
            history: Vec::new(),
        }
    }

    fn file_with_history(id: u64, path: &str, source_event: u64) -> ThreadArtifactProjection {
        let mut projection = file(id, path);
        projection.source_events.push(ThreadEntryId(source_event));
        projection.history.push(acp_thread::ThreadArtifactRevision {
            revision: 1,
            source_events: vec![ThreadEntryId(source_event)],
            owner: ThreadEventOwner::Tool,
            status: ThreadEventStatus::Completed,
            artifact: projection.artifact.clone(),
            action_target: projection.action_target.clone(),
            is_current: true,
        });
        projection.revision = 2;
        projection
    }

    #[test]
    fn responsive_collapse_has_an_exact_non_destructive_boundary() {
        assert!(ThreadOutline::is_responsively_collapsed(
            OUTLINE_EXPANDED_MIN_VIEWPORT_WIDTH - gpui::px(1.)
        ));
        assert!(!ThreadOutline::is_responsively_collapsed(
            OUTLINE_EXPANDED_MIN_VIEWPORT_WIDTH
        ));
        assert!(!ThreadOutline::is_responsively_collapsed(
            OUTLINE_EXPANDED_MIN_VIEWPORT_WIDTH + gpui::px(1.)
        ));
    }

    #[gpui::test(iterations = 8)]
    fn outline_projection_is_revisioned_deduplicated_and_anchor_stable(cx: &mut TestAppContext) {
        cx.update(|cx| {
            let owner = cx.new(|_| ());
            let outline = cx.new(ThreadOutline::new);
            outline.update(cx, |outline, cx| {
                outline.bind_projection_for_tests(
                    binding("a", 1),
                    owner.entity_id(),
                    snapshot(
                        2,
                        vec![
                            event(
                                11,
                                0,
                                ThreadEventStatus::Completed,
                                vec![file(1, "src/main.rs")],
                            ),
                            event(
                                12,
                                1,
                                ThreadEventStatus::InProgress,
                                vec![file(2, "src/lib.rs")],
                            ),
                        ],
                    ),
                    ThreadOutlineLifecycle::Streaming,
                    cx,
                );
                outline.select_view(OutlineView::Artifacts, cx);
                outline.select_index_for_tests(1, cx);
                outline.set_virtual_window_for_tests(0, 1, cx);
            });
            let selected = outline.read(cx).snapshot_for_tests().selected;

            outline.update(cx, |outline, cx| {
                outline.bind_projection_for_tests(
                    binding("a", 1),
                    owner.entity_id(),
                    snapshot(
                        3,
                        vec![
                            event(11, 0, ThreadEventStatus::Completed, Vec::new()),
                            event(
                                12,
                                1,
                                ThreadEventStatus::Completed,
                                vec![file(2, "src/lib.rs")],
                            ),
                            event(
                                13,
                                2,
                                ThreadEventStatus::InProgress,
                                vec![file_with_history(1, "src/main.rs", 11)],
                            ),
                        ],
                    ),
                    ThreadOutlineLifecycle::Streaming,
                    cx,
                );
            });

            let outline_snapshot = outline.read(cx).snapshot_for_tests();
            assert_eq!(outline_snapshot.revision, 3);
            assert_eq!(
                outline_snapshot.artifact_count, 2,
                "replayed artifact paths deduplicate"
            );
            assert_eq!(
                outline_snapshot.selected, selected,
                "streaming append preserves anchor"
            );
            assert_eq!(outline_snapshot.anchor, selected);
            assert_eq!(
                (outline_snapshot.virtual_start, outline_snapshot.virtual_len),
                (0, 1)
            );
            let main_artifact = outline_snapshot
                .visible_items
                .iter()
                .find(|item| item.id == OutlineItemId::Artifact(ThreadArtifactId(1)))
                .expect("authority-owned material history remains visible");
            assert_eq!(main_artifact.artifact_source_events.len(), 2);
            assert_eq!(main_artifact.artifact_history_len, 1);
            assert!(main_artifact.label.contains("2 sources · 2 revisions"));

            outline.update(cx, |outline, cx| {
                outline.bind_projection_for_tests(
                    binding("a", 1),
                    owner.entity_id(),
                    snapshot(2, Vec::new()),
                    ThreadOutlineLifecycle::Stale,
                    cx,
                );
                outline.set_filter(OutlineFilter::Problems, cx);
            });
            let snapshot = outline.read(cx).snapshot_for_tests();
            assert_eq!(
                snapshot.revision, 3,
                "stale completion cannot replace revision 3"
            );
            assert_eq!(snapshot.rejected_update_count, 1);
            assert!(snapshot.visible_items.is_empty());
            assert_eq!(snapshot.selected, None, "filtered target is reconciled");
        });
    }

    #[gpui::test(iterations = 8)]
    fn outline_state_is_thread_scoped_and_exact_entity_replacement_is_safe(
        cx: &mut TestAppContext,
    ) {
        cx.update(|cx| {
            let entity_a = cx.new(|_| ());
            let replacement_a = cx.new(|_| ());
            let entity_b = cx.new(|_| ());
            let outline = cx.new(ThreadOutline::new);

            outline.update(cx, |outline, cx| {
                outline.bind_projection_for_tests(
                    binding("a", 4),
                    entity_a.entity_id(),
                    snapshot(
                        1,
                        vec![event(
                            21,
                            0,
                            ThreadEventStatus::Completed,
                            vec![file(7, "a.rs")],
                        )],
                    ),
                    ThreadOutlineLifecycle::Ready,
                    cx,
                );
                outline.select_view(OutlineView::Artifacts, cx);
                outline.select_index_for_tests(0, cx);
                outline.set_collapsed(true, cx);
                outline.bind_projection_for_tests(
                    binding("b", 1),
                    entity_b.entity_id(),
                    snapshot(8, vec![event(30, 0, ThreadEventStatus::Failed, Vec::new())]),
                    ThreadOutlineLifecycle::Reconnecting,
                    cx,
                );
            });
            let thread_b = outline.read(cx).snapshot_for_tests();
            assert!(
                !thread_b.collapsed,
                "another thread gets independent view state"
            );
            assert_eq!(thread_b.view, OutlineView::Events);
            assert_eq!(thread_b.bound_entity_id, Some(entity_b.entity_id()));

            outline.update(cx, |outline, cx| {
                outline.bind_projection_for_tests(
                    binding("a", 5),
                    replacement_a.entity_id(),
                    snapshot(
                        1,
                        vec![event(
                            21,
                            0,
                            ThreadEventStatus::Completed,
                            vec![file(7, "a.rs")],
                        )],
                    ),
                    ThreadOutlineLifecycle::Ready,
                    cx,
                );
            });
            let restored = outline.read(cx).snapshot_for_tests();
            assert!(restored.collapsed);
            assert_eq!(restored.view, OutlineView::Artifacts);
            assert_eq!(restored.bound_entity_id, Some(replacement_a.entity_id()));
            assert_eq!(
                restored.selected,
                Some(OutlineItemId::Artifact(ThreadArtifactId(7)))
            );
            assert_eq!(
                restored.revision, 1,
                "replacement entity resets revision authority"
            );
        });
    }

    #[gpui::test(iterations = 8)]
    fn stable_event_selection_survives_reorder(cx: &mut TestAppContext) {
        cx.update(|cx| {
            let owner = cx.new(|_| ());
            let outline = cx.new(ThreadOutline::new);
            outline.update(cx, |outline, cx| {
                outline.bind_projection_for_tests(
                    binding("reorder", 1),
                    owner.entity_id(),
                    snapshot(
                        1,
                        vec![
                            event(41, 0, ThreadEventStatus::Completed, Vec::new()),
                            event(42, 1, ThreadEventStatus::InProgress, Vec::new()),
                        ],
                    ),
                    ThreadOutlineLifecycle::Streaming,
                    cx,
                );
                outline.select_index_for_tests(0, cx);
                outline.apply_projection_update_for_tests(
                    owner.entity_id(),
                    snapshot(
                        2,
                        vec![
                            event(42, 0, ThreadEventStatus::Completed, Vec::new()),
                            event(41, 1, ThreadEventStatus::Completed, Vec::new()),
                        ],
                    ),
                    cx,
                );
            });
            let snapshot = outline.read(cx).snapshot_for_tests();
            assert_eq!(
                snapshot.selected,
                Some(OutlineItemId::Event(ThreadEntryId(41)))
            );
            assert_eq!(
                snapshot.visible_items.get(1).map(|item| &item.id),
                Some(&OutlineItemId::Event(ThreadEntryId(41)))
            );
        });
    }

    #[gpui::test(iterations = 8)]
    fn reconnect_freezes_last_verified_projection_until_ready(cx: &mut TestAppContext) {
        cx.update(|cx| {
            let owner = cx.new(|_| ());
            let outline = cx.new(ThreadOutline::new);
            outline.update(cx, |outline, cx| {
                outline.bind_projection_for_tests(
                    binding("frozen", 2),
                    owner.entity_id(),
                    snapshot(
                        4,
                        vec![event(51, 0, ThreadEventStatus::Completed, Vec::new())],
                    ),
                    ThreadOutlineLifecycle::Ready,
                    cx,
                );
                outline.bind_projection_for_tests(
                    binding("frozen", 2),
                    owner.entity_id(),
                    snapshot(
                        5,
                        vec![event(52, 0, ThreadEventStatus::InProgress, Vec::new())],
                    ),
                    ThreadOutlineLifecycle::Reconnecting,
                    cx,
                );
                outline.apply_projection_update_for_tests(
                    owner.entity_id(),
                    snapshot(
                        6,
                        vec![event(53, 0, ThreadEventStatus::Completed, Vec::new())],
                    ),
                    cx,
                );
            });
            let frozen = outline.read(cx).snapshot_for_tests();
            assert_eq!(frozen.revision, 4);
            assert_eq!(frozen.lifecycle, ThreadOutlineLifecycle::Reconnecting);
            assert_eq!(
                frozen.visible_items.first().map(|item| item.entry_id),
                Some(ThreadEntryId(51))
            );

            outline.update(cx, |outline, cx| {
                outline.bind_projection_for_tests(
                    binding("frozen", 2),
                    owner.entity_id(),
                    snapshot(
                        6,
                        vec![event(53, 0, ThreadEventStatus::Completed, Vec::new())],
                    ),
                    ThreadOutlineLifecycle::Ready,
                    cx,
                );
            });
            let resumed = outline.read(cx).snapshot_for_tests();
            assert_eq!(resumed.revision, 6);
            assert_eq!(resumed.lifecycle, ThreadOutlineLifecycle::Ready);
            assert_eq!(
                resumed.visible_items.first().map(|item| item.entry_id),
                Some(ThreadEntryId(53))
            );
        });
    }

    #[gpui::test(iterations = 8)]
    fn ownership_filter_keeps_child_rows_and_rejects_foreign_items(cx: &mut TestAppContext) {
        cx.update(|cx| {
            let owner = cx.new(|_| ());
            let parent = event(
                61,
                0,
                ThreadEventStatus::Completed,
                vec![file(17, "owned.rs")],
            );
            let mut child = event(62, 0, ThreadEventStatus::Completed, Vec::new());
            child.parent_id = Some(parent.id);
            child.kind = ThreadEventKind::Reasoning;
            let mut foreign = event(63, 1, ThreadEventStatus::Failed, Vec::new());
            foreign.binding = ThreadProjectionBinding {
                thread_id: "foreign-thread".into(),
                work_dirs: Arc::from([PathBuf::from("/foreign")]),
            };
            let mut projection = snapshot(1, vec![parent.clone(), child, foreign]);
            let mut foreign_artifact = file(18, "foreign.rs");
            foreign_artifact.binding = ThreadProjectionBinding {
                thread_id: "foreign-thread".into(),
                work_dirs: Arc::from([PathBuf::from("/foreign")]),
            };
            foreign_artifact.source_events = vec![parent.id];
            projection.artifacts.push(foreign_artifact);

            let outline = cx.new(ThreadOutline::new);
            outline.update(cx, |outline, cx| {
                outline.bind_projection_for_tests(
                    binding("owned", 1),
                    owner.entity_id(),
                    projection,
                    ThreadOutlineLifecycle::Ready,
                    cx,
                );
            });
            let events = outline.read(cx).snapshot_for_tests();
            assert_eq!(events.event_count, 2);
            assert_eq!(events.visible_items.len(), 2);
            assert_eq!(events.visible_items[1].parent_id, Some(parent.id));

            outline.update(cx, |outline, cx| {
                outline.select_view(OutlineView::Artifacts, cx);
            });
            let artifacts = outline.read(cx).snapshot_for_tests();
            assert_eq!(artifacts.artifact_count, 1);
            assert_eq!(artifacts.visible_items.len(), 1);
            assert_eq!(
                artifacts.visible_items[0].id,
                OutlineItemId::Artifact(ThreadArtifactId(17))
            );
        });
    }

    #[gpui::test(iterations = 8)]
    async fn actual_action_dispatch_uses_captured_item_authority(cx: &mut TestAppContext) {
        cx.update(|cx| {
            let mut settings_store = settings::SettingsStore::test(cx);
            settings_store.register_setting::<feature_flags::FeatureFlagsSettings>();
            cx.set_global(settings_store);
            theme_settings::init(theme::LoadThemes::JustBase, cx);
            release_channel::init(semver::Version::new(0, 0, 0), cx);
        });
        let owner = cx.new(|_| ());
        let action_count = Rc::new(Cell::new(0_u64));
        let observed_binding = Rc::new(std::cell::RefCell::new(None));
        let (outline, cx) = cx.add_window_view({
            let action_count = action_count.clone();
            let observed_binding = observed_binding.clone();
            move |_window, cx| {
                let mut outline = ThreadOutline::new(cx);
                outline.set_navigation_handler(Rc::new(move |item, _window, _cx| {
                    action_count.set(action_count.get().saturating_add(1));
                    *observed_binding.borrow_mut() =
                        Some((item.outline_binding, item.projection_binding));
                    true
                }));
                outline
            }
        });
        outline.update(cx, |outline, cx| {
            outline.bind_projection_for_tests(
                binding("dispatch", 7),
                owner.entity_id(),
                snapshot(
                    1,
                    vec![event(71, 0, ThreadEventStatus::Completed, Vec::new())],
                ),
                ThreadOutlineLifecycle::Ready,
                cx,
            );
            outline.select_index_for_tests(0, cx);
        });
        let focus_handle = outline.read_with(cx, |outline, cx| outline.focus_handle(cx));
        cx.update(|window, cx| focus_handle.focus(window, cx));
        cx.dispatch_action(ActivateOutlineItem);

        assert_eq!(action_count.get(), 1);
        let (outline_binding, captured_projection_binding) = observed_binding
            .borrow()
            .clone()
            .expect("actual action should deliver captured authority");
        assert_eq!(outline_binding, binding("dispatch", 7));
        assert_eq!(captured_projection_binding, projection_binding());
    }
}
