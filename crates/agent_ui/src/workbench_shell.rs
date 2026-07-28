use std::{cell::Cell, collections::BTreeMap};

use anyhow::{Result, anyhow, bail};
use gpui::{
    Action, App, Context, Entity, FocusHandle, Focusable, Pixels, Render, SharedString, Window,
    actions, px,
};
use omega_workbench_state::{
    ConnectionPhase, ProjectionTransition, RepositoryBinding, WorkSurface, WorkbenchProjection,
};
use ui::{Color, Icon, IconName, IconSize, Label, LabelSize, prelude::*, v_flex};

use crate::omega_sidebar;

pub const ACTIVITY_RAIL_WIDTH: Pixels = px(40.);
pub const DEFAULT_DOCK_WIDTH: Pixels = px(320.);
pub const MIN_DOCK_WIDTH: Pixels = px(240.);
pub const MAX_DOCK_WIDTH: Pixels = px(480.);
pub const RESIZE_HANDLE_WIDTH: Pixels = px(6.);

actions!(
    omega_workbench,
    [
        /// Focus the work-surface activity rail.
        FocusActivityRail,
        /// Select the Files work surface.
        SelectFiles,
        /// Select the Search work surface.
        SelectSearch,
        /// Select the Review work surface.
        SelectReview,
        /// Select the Git work surface.
        SelectGit,
        /// Select the Terminal work surface.
        SelectTerminal,
        /// Select the Plan work surface.
        SelectPlan,
        /// Move focus to the next activity-rail item.
        FocusNextSurface,
        /// Move focus to the previous activity-rail item.
        FocusPreviousSurface,
        /// Move focus to the first activity-rail item.
        FocusFirstSurface,
        /// Move focus to the last activity-rail item.
        FocusLastSurface,
        /// Activate the focused activity-rail item.
        ActivateFocusedSurface,
        /// Collapse the work-surface dock.
        CollapseWorkSurfaceDock,
        /// Return focus to the active thread transcript.
        FocusThreadTranscript,
    ]
);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WorkbenchFocusTarget {
    Transcript,
    Rail(WorkSurface),
    Surface(WorkSurface),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BadgeTone {
    Neutral,
    Accent,
    Warning,
    Error,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SurfaceBadge {
    Count {
        count: usize,
        tone: BadgeTone,
        label: SharedString,
    },
    Attention {
        tone: BadgeTone,
        label: SharedString,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SurfaceAvailability {
    Available,
    Unavailable { reason: SharedString },
}

impl SurfaceAvailability {
    pub fn is_available(&self) -> bool {
        matches!(self, Self::Available)
    }

    pub fn reason(&self) -> Option<&SharedString> {
        match self {
            Self::Available => None,
            Self::Unavailable { reason } => Some(reason),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SurfaceCapability {
    pub availability: SurfaceAvailability,
    pub badge: Option<SurfaceBadge>,
}

impl SurfaceCapability {
    fn available() -> Self {
        Self {
            availability: SurfaceAvailability::Available,
            badge: None,
        }
    }

    fn unavailable(reason: impl Into<SharedString>) -> Self {
        Self {
            availability: SurfaceAvailability::Unavailable {
                reason: reason.into(),
            },
            badge: None,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct SurfaceHostKey {
    pub thread_id: String,
    pub binding: Option<RepositoryBinding>,
    pub surface: WorkSurface,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SurfaceContentState {
    Ready,
    Loading,
    Error(SharedString),
    Offline,
}

pub struct WorkSurfaceHost {
    key: SurfaceHostKey,
    focus_handle: FocusHandle,
    content_state: SurfaceContentState,
}

impl WorkSurfaceHost {
    fn new(key: SurfaceHostKey, cx: &mut Context<Self>) -> Self {
        Self {
            key,
            focus_handle: cx.focus_handle(),
            content_state: SurfaceContentState::Ready,
        }
    }

    pub fn key(&self) -> &SurfaceHostKey {
        &self.key
    }

    pub fn content_state(&self) -> &SurfaceContentState {
        &self.content_state
    }

    fn set_content_state(&mut self, content_state: SurfaceContentState, cx: &mut Context<Self>) {
        self.content_state = content_state;
        cx.notify();
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SurfaceLoadContext {
    request_id: String,
    thread_id: String,
    surface: WorkSurface,
    generation: u64,
    binding: Option<RepositoryBinding>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SurfaceLoadOutcome {
    Ready,
    Error(SharedString),
}

impl Focusable for WorkSurfaceHost {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for WorkSurfaceHost {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        let surface = self.key.surface;
        let label = surface.label();
        let content = match &self.content_state {
            SurfaceContentState::Ready => Label::new(format!("{label} is ready"))
                .size(LabelSize::Small)
                .color(Color::Muted)
                .into_any_element(),
            SurfaceContentState::Loading => Label::new(format!("Loading {label}…"))
                .size(LabelSize::Small)
                .color(Color::Muted)
                .into_any_element(),
            SurfaceContentState::Error(error) => Label::new(error.clone())
                .size(LabelSize::Small)
                .color(Color::Error)
                .into_any_element(),
            SurfaceContentState::Offline => Label::new(format!("{label} is unavailable offline"))
                .size(LabelSize::Small)
                .color(Color::Warning)
                .into_any_element(),
        };

        v_flex()
            .id(surface.surface_element_id())
            .debug_selector(move || surface.surface_selector())
            .track_focus(&self.focus_handle)
            .tab_index(0)
            .role(gpui::Role::Group)
            .aria_label(format!("{label} work surface"))
            .size_full()
            .items_center()
            .justify_center()
            .gap_2()
            .child(
                Icon::new(surface.icon())
                    .size(IconSize::Medium)
                    .color(Color::Muted),
            )
            .child(content)
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct WorkbenchLayout {
    pub sidebar: omega_sidebar::Layout,
    pub dock_visible: bool,
    pub dock_width: Pixels,
}

impl WorkbenchLayout {
    pub fn clamp_dock_width(available: Pixels, requested_dock_width: Pixels) -> Option<Pixels> {
        let maximum_dock_width = (available
            - omega_sidebar::RAIL_WIDTH
            - ACTIVITY_RAIL_WIDTH
            - omega_sidebar::MIN_CONTENT_WIDTH)
            .min(MAX_DOCK_WIDTH);
        if maximum_dock_width < MIN_DOCK_WIDTH {
            return None;
        }

        Some(requested_dock_width.clamp(MIN_DOCK_WIDTH, maximum_dock_width))
    }

    pub fn allocate(
        available: Pixels,
        sidebar_requested_open: bool,
        dock_requested_open: bool,
        requested_dock_width: Pixels,
    ) -> Self {
        let dock_width = if dock_requested_open {
            Self::clamp_dock_width(available, requested_dock_width)
        } else {
            None
        };
        let dock_visible = dock_width.is_some();
        let dock_width = dock_width.unwrap_or(Pixels::ZERO);

        let sidebar = if sidebar_requested_open
            && available - ACTIVITY_RAIL_WIDTH - dock_width - omega_sidebar::SIDEBAR_WIDTH
                >= omega_sidebar::MIN_CONTENT_WIDTH
        {
            omega_sidebar::Layout::Expanded
        } else {
            omega_sidebar::Layout::Rail
        };

        Self {
            sidebar,
            dock_visible,
            dock_width,
        }
    }
}

#[derive(Debug)]
pub(crate) struct WorkbenchDockResizeDrag {
    width_before: Pixels,
    pointer_x_before: Cell<Pixels>,
}

impl WorkbenchDockResizeDrag {
    pub(crate) fn new(width_before: Pixels) -> Self {
        Self {
            width_before,
            pointer_x_before: Cell::new(Pixels::ZERO),
        }
    }

    pub(crate) fn begin(&self, pointer_x: Pixels) {
        self.pointer_x_before.set(pointer_x);
    }

    pub(crate) fn requested_width(&self, pointer_x: Pixels) -> Pixels {
        self.width_before + pointer_x - self.pointer_x_before.get()
    }
}

pub struct WorkbenchShell {
    projection: WorkbenchProjection,
    capabilities: BTreeMap<WorkSurface, SurfaceCapability>,
    hosts: BTreeMap<SurfaceHostKey, Entity<WorkSurfaceHost>>,
    rail_focus_handles: BTreeMap<WorkSurface, FocusHandle>,
    focused_rail_surface: WorkSurface,
    focus_target: WorkbenchFocusTarget,
    dock_width: Pixels,
    last_error: Option<SharedString>,
    #[cfg(any(test, feature = "test-support"))]
    fail_next_host_creation: Option<WorkSurface>,
}

impl WorkbenchShell {
    pub fn new(cx: &mut Context<crate::AgentPanel>) -> Self {
        let rail_focus_handles = WorkSurface::FALLBACK_ORDER
            .into_iter()
            .map(|surface| (surface, cx.focus_handle()))
            .collect();
        Self {
            projection: WorkbenchProjection::new(),
            capabilities: WorkSurface::FALLBACK_ORDER
                .into_iter()
                .map(|surface| {
                    (
                        surface,
                        SurfaceCapability::unavailable("Open a thread to use this surface"),
                    )
                })
                .collect(),
            hosts: BTreeMap::new(),
            rail_focus_handles,
            focused_rail_surface: WorkSurface::Files,
            focus_target: WorkbenchFocusTarget::Transcript,
            dock_width: DEFAULT_DOCK_WIDTH,
            last_error: None,
            #[cfg(any(test, feature = "test-support"))]
            fail_next_host_creation: None,
        }
    }

    pub fn projection(&self) -> &WorkbenchProjection {
        &self.projection
    }

    pub fn capabilities(&self) -> &BTreeMap<WorkSurface, SurfaceCapability> {
        &self.capabilities
    }

    pub fn capability(&self, surface: WorkSurface) -> Option<&SurfaceCapability> {
        self.capabilities.get(&surface)
    }

    pub fn focus_target(&self) -> WorkbenchFocusTarget {
        self.focus_target
    }

    pub fn focused_rail_surface(&self) -> WorkSurface {
        self.focused_rail_surface
    }

    pub fn rail_focus_handle(&self, surface: WorkSurface) -> Option<&FocusHandle> {
        self.rail_focus_handles.get(&surface)
    }

    pub fn last_error(&self) -> Option<&SharedString> {
        self.last_error.as_ref()
    }

    pub fn clear_error(&mut self) {
        self.last_error = None;
    }

    pub fn record_error(&mut self, error: impl Into<SharedString>) {
        self.last_error = Some(error.into());
    }

    pub fn dock_width(&self) -> Pixels {
        self.dock_width
    }

    pub fn resize_dock(&mut self, width: Pixels, available: Pixels) -> bool {
        let Some(width) = WorkbenchLayout::clamp_dock_width(available, width) else {
            return false;
        };
        if width == self.dock_width {
            return false;
        }

        self.dock_width = width;
        true
    }

    pub fn sync_active_thread(
        &mut self,
        thread_id: Option<String>,
        binding: Option<RepositoryBinding>,
    ) -> Result<()> {
        let Some(thread_id) = thread_id else {
            self.capabilities = unavailable_capabilities("Open a thread to use this surface");
            self.focus_target = WorkbenchFocusTarget::Transcript;
            return Ok(());
        };

        let available_surfaces = available_surfaces(binding.is_some());
        if !self.projection.threads.contains_key(&thread_id) {
            self.projection
                .apply(ProjectionTransition::OpenThread {
                    thread_id: thread_id.clone(),
                    binding,
                    available_surfaces,
                })
                .map_err(anyhow::Error::new)?;
        } else {
            self.reconcile_binding(&thread_id, binding, available_surfaces)?;
        }
        if self.projection.active_thread_id.as_deref() != Some(thread_id.as_str()) {
            self.projection
                .apply(ProjectionTransition::SwitchThread {
                    thread_id: thread_id.clone(),
                })
                .map_err(anyhow::Error::new)?;
        }

        let thread =
            self.projection.threads.get(&thread_id).ok_or_else(|| {
                anyhow!("thread {thread_id:?} disappeared during capability sync")
            })?;
        let available_surfaces = thread.available_surfaces.clone();
        let mut capabilities = capabilities_for_binding(thread.binding.is_some());
        for (surface, capability) in &mut capabilities {
            if !available_surfaces.contains(surface) {
                capability.availability = SurfaceAvailability::Unavailable {
                    reason: "This surface is no longer available".into(),
                };
            }
            capability.badge = self
                .capabilities
                .get(surface)
                .and_then(|previous| previous.badge.clone());
        }
        self.capabilities = capabilities;
        if let Some(visible) = self.projection.visible_projection() {
            self.focus_target = if visible.dock_open {
                visible
                    .effective_surface
                    .map(WorkbenchFocusTarget::Surface)
                    .unwrap_or(WorkbenchFocusTarget::Transcript)
            } else {
                WorkbenchFocusTarget::Transcript
            };
        }
        Ok(())
    }

    pub fn close_thread(&mut self, thread_id: &str) -> Result<()> {
        if self.projection.threads.contains_key(thread_id) {
            self.projection
                .apply(ProjectionTransition::CloseThread {
                    thread_id: thread_id.into(),
                })
                .map_err(anyhow::Error::new)?;
        }
        self.hosts.retain(|key, _| key.thread_id != thread_id);
        if self.projection.active_thread_id.is_none() {
            self.focus_target = WorkbenchFocusTarget::Transcript;
        }
        Ok(())
    }

    fn reconcile_binding(
        &mut self,
        thread_id: &str,
        binding: Option<RepositoryBinding>,
        available_surfaces: Vec<WorkSurface>,
    ) -> Result<()> {
        let thread = self
            .projection
            .threads
            .get(thread_id)
            .ok_or_else(|| anyhow!("thread {thread_id:?} disappeared during reconciliation"))?
            .clone();
        if thread.binding == binding {
            return Ok(());
        }

        let previous_effective = thread.effective_surface;
        match (thread.binding, binding) {
            (None, Some(binding)) => {
                self.projection
                    .apply(ProjectionTransition::BindRepository {
                        thread_id: thread_id.into(),
                        generation: thread.generation,
                        binding,
                        available_surfaces,
                    })
                    .map_err(anyhow::Error::new)?;
            }
            (Some(previous), Some(binding)) if previous.repository_id == binding.repository_id => {
                self.projection
                    .apply(ProjectionTransition::ChangeWorktree {
                        thread_id: thread_id.into(),
                        generation: thread.generation,
                        worktree_id: binding.worktree_id,
                        available_surfaces,
                    })
                    .map_err(anyhow::Error::new)?;
            }
            (Some(_), Some(binding)) => {
                self.projection
                    .apply(ProjectionTransition::RemoveBinding {
                        thread_id: thread_id.into(),
                        generation: thread.generation,
                        available_surfaces: vec![WorkSurface::Plan],
                    })
                    .map_err(anyhow::Error::new)?;
                let generation = self
                    .projection
                    .threads
                    .get(thread_id)
                    .map(|thread| thread.generation)
                    .ok_or_else(|| anyhow!("thread {thread_id:?} disappeared after unbinding"))?;
                self.projection
                    .apply(ProjectionTransition::BindRepository {
                        thread_id: thread_id.into(),
                        generation,
                        binding,
                        available_surfaces,
                    })
                    .map_err(anyhow::Error::new)?;
            }
            (Some(_), None) => {
                self.projection
                    .apply(ProjectionTransition::RemoveBinding {
                        thread_id: thread_id.into(),
                        generation: thread.generation,
                        available_surfaces,
                    })
                    .map_err(anyhow::Error::new)?;
            }
            (None, None) => {}
        }

        let current = self
            .projection
            .threads
            .get(thread_id)
            .cloned()
            .ok_or_else(|| anyhow!("thread {thread_id:?} disappeared after reconciliation"))?;
        if previous_effective != current.effective_surface && current.dock_open {
            self.projection
                .apply(ProjectionTransition::CollapseDock {
                    thread_id: thread_id.into(),
                })
                .map_err(anyhow::Error::new)?;
            self.focus_target = WorkbenchFocusTarget::Transcript;
        }
        let current_binding = current.binding;
        self.hosts
            .retain(|key, _| key.thread_id != thread_id || key.binding == current_binding);
        Ok(())
    }

    pub fn set_connection(&mut self, phase: ConnectionPhase) -> Result<()> {
        match (self.projection.connection, phase) {
            (current, requested) if current == requested => {}
            (
                ConnectionPhase::Online | ConnectionPhase::StaleProjection,
                ConnectionPhase::Offline,
            ) => {
                self.projection
                    .apply(ProjectionTransition::Disconnect)
                    .map_err(anyhow::Error::new)?;
            }
            (ConnectionPhase::Offline, ConnectionPhase::Reconnecting) => {
                self.projection
                    .apply(ProjectionTransition::Reconnect)
                    .map_err(anyhow::Error::new)?;
            }
            (current, requested) => {
                bail!("unsupported shell connection transition {current:?} -> {requested:?}");
            }
        }
        if phase != ConnectionPhase::Online {
            self.focus_target = WorkbenchFocusTarget::Transcript;
        }
        Ok(())
    }

    pub fn select_surface(
        &mut self,
        surface: WorkSurface,
        cx: &mut Context<crate::AgentPanel>,
    ) -> Result<SurfaceSelection> {
        let unavailable_reason = self
            .capability(surface)
            .ok_or_else(|| anyhow!("the {} surface is not registered", surface.label()))?
            .availability
            .reason()
            .cloned();
        if let Some(reason) = unavailable_reason {
            self.last_error = Some(reason.clone());
            bail!("{reason}");
        }
        let visible = self
            .projection
            .visible_projection()
            .ok_or_else(|| anyhow!("open a thread before selecting a work surface"))?;
        let thread_id = visible.thread_id.clone();

        if visible.dock_open && visible.effective_surface == Some(surface) {
            self.projection
                .apply(ProjectionTransition::CollapseDock { thread_id })
                .map_err(anyhow::Error::new)?;
            self.focus_target = WorkbenchFocusTarget::Transcript;
            self.last_error = None;
            return Ok(SurfaceSelection::Collapsed);
        }

        let host = self.ensure_host(&visible.thread_id, visible.binding.clone(), surface, cx)?;
        self.projection
            .apply(ProjectionTransition::RequestSurface { thread_id, surface })
            .map_err(anyhow::Error::new)?;
        self.focused_rail_surface = surface;
        self.focus_target = WorkbenchFocusTarget::Surface(surface);
        self.last_error = None;
        Ok(SurfaceSelection::Opened(host))
    }

    fn ensure_host(
        &mut self,
        thread_id: &str,
        binding: Option<RepositoryBinding>,
        surface: WorkSurface,
        cx: &mut Context<crate::AgentPanel>,
    ) -> Result<Entity<WorkSurfaceHost>> {
        let key = SurfaceHostKey {
            thread_id: thread_id.into(),
            binding,
            surface,
        };
        if let Some(host) = self.hosts.get(&key) {
            return Ok(host.clone());
        }
        #[cfg(any(test, feature = "test-support"))]
        if self.fail_next_host_creation == Some(surface) {
            self.fail_next_host_creation = None;
            let message: SharedString =
                format!("Could not create the {} surface", surface.label()).into();
            self.last_error = Some(message.clone());
            bail!("{message}");
        }
        let host = cx.new(|cx| WorkSurfaceHost::new(key.clone(), cx));
        self.hosts.insert(key, host.clone());
        Ok(host)
    }

    pub fn begin_surface_load(
        &mut self,
        request_id: impl Into<String>,
        surface: WorkSurface,
        cx: &mut Context<crate::AgentPanel>,
    ) -> Result<SurfaceLoadContext> {
        let request_id = request_id.into();
        let visible = self
            .projection
            .visible_projection()
            .ok_or_else(|| anyhow!("open a thread before loading a work surface"))?;
        if !visible.dock_open || visible.effective_surface != Some(surface) {
            bail!("the {} surface is not visible", surface.label());
        }
        let key = SurfaceHostKey {
            thread_id: visible.thread_id.clone(),
            binding: visible.binding.clone(),
            surface,
        };
        let host = self
            .hosts
            .get(&key)
            .cloned()
            .ok_or_else(|| anyhow!("the {} host is not mounted", surface.label()))?;
        let context = SurfaceLoadContext {
            request_id,
            thread_id: visible.thread_id,
            surface,
            generation: visible.generation,
            binding: visible.binding,
        };
        self.projection
            .apply(ProjectionTransition::BeginSurfaceLoad {
                request_id: context.request_id.clone(),
                thread_id: context.thread_id.clone(),
                surface: context.surface,
                generation: context.generation,
                binding: context.binding.clone(),
            })
            .map_err(anyhow::Error::new)?;
        host.update(cx, |host, cx| {
            host.set_content_state(SurfaceContentState::Loading, cx);
        });
        Ok(context)
    }

    pub fn complete_surface_load(
        &mut self,
        context: SurfaceLoadContext,
        outcome: SurfaceLoadOutcome,
        cx: &mut Context<crate::AgentPanel>,
    ) -> Result<omega_workbench_state::TransitionEffect> {
        let transition = match &outcome {
            SurfaceLoadOutcome::Ready => ProjectionTransition::CompleteSurfaceLoad {
                request_id: context.request_id.clone(),
                thread_id: context.thread_id.clone(),
                surface: context.surface,
                generation: context.generation,
                binding: context.binding.clone(),
            },
            SurfaceLoadOutcome::Error(_) => ProjectionTransition::FailSurfaceLoad {
                request_id: context.request_id.clone(),
                thread_id: context.thread_id.clone(),
                surface: context.surface,
                generation: context.generation,
                binding: context.binding.clone(),
            },
        };
        let effect = self
            .projection
            .apply(transition)
            .map_err(anyhow::Error::new)?;
        if effect == omega_workbench_state::TransitionEffect::Applied {
            let key = SurfaceHostKey {
                thread_id: context.thread_id,
                binding: context.binding,
                surface: context.surface,
            };
            if let Some(host) = self.hosts.get(&key) {
                let content_state = match outcome {
                    SurfaceLoadOutcome::Ready => SurfaceContentState::Ready,
                    SurfaceLoadOutcome::Error(error) => SurfaceContentState::Error(error),
                };
                host.update(cx, |host, cx| {
                    host.set_content_state(content_state, cx);
                });
            }
        }
        Ok(effect)
    }

    pub fn visible_host(&self) -> Option<&Entity<WorkSurfaceHost>> {
        let visible = self.projection.visible_projection()?;
        if !visible.dock_open {
            return None;
        }
        let key = SurfaceHostKey {
            thread_id: visible.thread_id,
            binding: visible.binding,
            surface: visible.effective_surface?,
        };
        self.hosts.get(&key)
    }

    pub fn collapse_dock(&mut self) -> Result<bool> {
        let Some(visible) = self.projection.visible_projection() else {
            return Ok(false);
        };
        if !visible.dock_open {
            return Ok(false);
        }
        self.projection
            .apply(ProjectionTransition::CollapseDock {
                thread_id: visible.thread_id,
            })
            .map_err(anyhow::Error::new)?;
        self.focus_target = WorkbenchFocusTarget::Transcript;
        Ok(true)
    }

    pub fn collapse_for_layout(&mut self, layout: WorkbenchLayout) -> Result<bool> {
        if layout.dock_visible {
            return Ok(false);
        }
        self.collapse_dock()
    }

    pub fn focus_rail(&mut self) -> WorkSurface {
        let surface = self
            .projection
            .visible_projection()
            .and_then(|visible| visible.effective_surface)
            .unwrap_or(self.focused_rail_surface);
        self.focused_rail_surface = surface;
        self.focus_target = WorkbenchFocusTarget::Rail(surface);
        surface
    }

    pub fn move_rail_focus(&mut self, movement: RailFocusMovement) -> WorkSurface {
        let current_index = WorkSurface::FALLBACK_ORDER
            .iter()
            .position(|surface| *surface == self.focused_rail_surface)
            .unwrap_or(0);
        let last_index = WorkSurface::FALLBACK_ORDER.len() - 1;
        let next_index = match movement {
            RailFocusMovement::Next => (current_index + 1).min(last_index),
            RailFocusMovement::Previous => current_index.saturating_sub(1),
            RailFocusMovement::First => 0,
            RailFocusMovement::Last => last_index,
        };
        let surface = WorkSurface::FALLBACK_ORDER[next_index];
        self.focused_rail_surface = surface;
        self.focus_target = WorkbenchFocusTarget::Rail(surface);
        surface
    }

    pub fn return_to_transcript(&mut self) {
        self.focus_target = WorkbenchFocusTarget::Transcript;
    }

    pub fn host_count(&self) -> usize {
        self.hosts.len()
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn fail_next_host_creation(&mut self, surface: WorkSurface) {
        self.fail_next_host_creation = Some(surface);
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn set_badge(&mut self, surface: WorkSurface, badge: Option<SurfaceBadge>) {
        if let Some(capability) = self.capabilities.get_mut(&surface) {
            capability.badge = badge;
        }
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn invalidate_surface(
        &mut self,
        surface: WorkSurface,
    ) -> Result<omega_workbench_state::TransitionEffect> {
        let visible = self
            .projection
            .visible_projection()
            .ok_or_else(|| anyhow!("no active thread"))?;
        let effect = self
            .projection
            .apply(ProjectionTransition::InvalidateCapability {
                thread_id: visible.thread_id.clone(),
                generation: visible.generation,
                surface,
            })
            .map_err(anyhow::Error::new)?;
        self.capabilities.insert(
            surface,
            SurfaceCapability::unavailable("This surface is no longer available"),
        );
        self.hosts.remove(&SurfaceHostKey {
            thread_id: visible.thread_id.clone(),
            binding: visible.binding.clone(),
            surface,
        });
        if visible.effective_surface == Some(surface) {
            self.collapse_dock()?;
            self.focus_target = WorkbenchFocusTarget::Transcript;
        }
        Ok(effect)
    }
}

#[derive(Clone)]
pub enum SurfaceSelection {
    Collapsed,
    Opened(Entity<WorkSurfaceHost>),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RailFocusMovement {
    Next,
    Previous,
    First,
    Last,
}

pub trait WorkSurfaceExt {
    fn label(self) -> &'static str;
    fn icon(self) -> IconName;
    fn rail_element_id(self) -> &'static str;
    fn surface_element_id(self) -> &'static str;
    fn surface_selector(self) -> String;
}

impl WorkSurfaceExt for WorkSurface {
    fn label(self) -> &'static str {
        match self {
            Self::Files => "Files",
            Self::Search => "Search",
            Self::Review => "Review",
            Self::Git => "Git",
            Self::Terminal => "Terminal",
            Self::Plan => "Plan",
        }
    }

    fn icon(self) -> IconName {
        match self {
            Self::Files => IconName::FileTree,
            Self::Search => IconName::MagnifyingGlass,
            Self::Review => IconName::ListTodo,
            Self::Git => IconName::GitBranch,
            Self::Terminal => IconName::TerminalAlt,
            Self::Plan => IconName::TodoProgress,
        }
    }

    fn rail_element_id(self) -> &'static str {
        match self {
            Self::Files => "omega.workbench.control.rail.files",
            Self::Search => "omega.workbench.control.rail.search",
            Self::Review => "omega.workbench.control.rail.review",
            Self::Git => "omega.workbench.control.rail.git",
            Self::Terminal => "omega.workbench.control.rail.terminal",
            Self::Plan => "omega.workbench.control.rail.plan",
        }
    }

    fn surface_element_id(self) -> &'static str {
        match self {
            Self::Files => "omega.workbench.surface.files",
            Self::Search => "omega.workbench.surface.search",
            Self::Review => "omega.workbench.surface.review",
            Self::Git => "omega.workbench.surface.git",
            Self::Terminal => "omega.workbench.surface.terminal",
            Self::Plan => "omega.workbench.surface.plan",
        }
    }

    fn surface_selector(self) -> String {
        self.surface_element_id().into()
    }
}

pub fn select_action(surface: WorkSurface) -> Box<dyn Action> {
    match surface {
        WorkSurface::Files => SelectFiles.boxed_clone(),
        WorkSurface::Search => SelectSearch.boxed_clone(),
        WorkSurface::Review => SelectReview.boxed_clone(),
        WorkSurface::Git => SelectGit.boxed_clone(),
        WorkSurface::Terminal => SelectTerminal.boxed_clone(),
        WorkSurface::Plan => SelectPlan.boxed_clone(),
    }
}

fn available_surfaces(has_binding: bool) -> Vec<WorkSurface> {
    if has_binding {
        WorkSurface::FALLBACK_ORDER.into()
    } else {
        vec![WorkSurface::Plan]
    }
}

fn capabilities_for_binding(has_binding: bool) -> BTreeMap<WorkSurface, SurfaceCapability> {
    WorkSurface::FALLBACK_ORDER
        .into_iter()
        .map(|surface| {
            let capability = if has_binding || !surface.requires_binding() {
                SurfaceCapability::available()
            } else {
                SurfaceCapability::unavailable("Open a project to use this surface")
            };
            (surface, capability)
        })
        .collect()
}

fn unavailable_capabilities(reason: &'static str) -> BTreeMap<WorkSurface, SurfaceCapability> {
    WorkSurface::FALLBACK_ORDER
        .into_iter()
        .map(|surface| (surface, SurfaceCapability::unavailable(reason)))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn layout_has_one_shared_allocation_boundary() {
        let boundary = omega_sidebar::RAIL_WIDTH
            + ACTIVITY_RAIL_WIDTH
            + MIN_DOCK_WIDTH
            + omega_sidebar::MIN_CONTENT_WIDTH;
        let below = WorkbenchLayout::allocate(boundary - px(1.), true, true, DEFAULT_DOCK_WIDTH);
        assert!(!below.dock_visible);

        let exact = WorkbenchLayout::allocate(boundary, true, true, DEFAULT_DOCK_WIDTH);
        assert!(exact.dock_visible);
        assert_eq!(exact.dock_width, MIN_DOCK_WIDTH);
        assert_eq!(exact.sidebar, omega_sidebar::Layout::Rail);

        let above = WorkbenchLayout::allocate(boundary + px(1.), true, true, DEFAULT_DOCK_WIDTH);
        assert!(above.dock_visible);
        assert_eq!(above.dock_width, MIN_DOCK_WIDTH + px(1.));
        assert_eq!(above.sidebar, omega_sidebar::Layout::Rail);

        let sidebar_boundary = omega_sidebar::SIDEBAR_WIDTH
            + ACTIVITY_RAIL_WIDTH
            + DEFAULT_DOCK_WIDTH
            + omega_sidebar::MIN_CONTENT_WIDTH;
        let both = WorkbenchLayout::allocate(sidebar_boundary, true, true, DEFAULT_DOCK_WIDTH);
        assert!(both.dock_visible);
        assert_eq!(both.sidebar, omega_sidebar::Layout::Expanded);
    }

    #[test]
    fn dock_width_is_clamped_without_stealing_transcript_floor() {
        let available = px(1050.);
        let layout = WorkbenchLayout::allocate(available, false, true, MAX_DOCK_WIDTH);
        assert!(layout.dock_visible);
        assert_eq!(
            layout.dock_width,
            available
                - omega_sidebar::RAIL_WIDTH
                - ACTIVITY_RAIL_WIDTH
                - omega_sidebar::MIN_CONTENT_WIDTH
        );
    }

    #[test]
    fn dock_resize_clamp_preserves_limits_and_transcript_floor() {
        let transcript_limited_width = px(300.);
        let transcript_limited_available = omega_sidebar::RAIL_WIDTH
            + ACTIVITY_RAIL_WIDTH
            + omega_sidebar::MIN_CONTENT_WIDTH
            + transcript_limited_width;
        assert_eq!(
            WorkbenchLayout::clamp_dock_width(transcript_limited_available, MAX_DOCK_WIDTH),
            Some(transcript_limited_width)
        );

        let roomy_available = omega_sidebar::RAIL_WIDTH
            + ACTIVITY_RAIL_WIDTH
            + omega_sidebar::MIN_CONTENT_WIDTH
            + MAX_DOCK_WIDTH
            + px(100.);
        assert_eq!(
            WorkbenchLayout::clamp_dock_width(roomy_available, px(100.)),
            Some(MIN_DOCK_WIDTH)
        );
        assert_eq!(
            WorkbenchLayout::clamp_dock_width(roomy_available, px(600.)),
            Some(MAX_DOCK_WIDTH)
        );

        let too_narrow = omega_sidebar::RAIL_WIDTH
            + ACTIVITY_RAIL_WIDTH
            + omega_sidebar::MIN_CONTENT_WIDTH
            + MIN_DOCK_WIDTH
            - px(1.);
        assert_eq!(
            WorkbenchLayout::clamp_dock_width(too_narrow, DEFAULT_DOCK_WIDTH),
            None
        );
    }

    #[test]
    fn dock_resize_drag_uses_the_width_and_pointer_at_drag_start() {
        let drag = WorkbenchDockResizeDrag::new(px(320.));
        drag.begin(px(800.));

        assert_eq!(drag.requested_width(px(800.)), px(320.));
        assert_eq!(drag.requested_width(px(920.)), px(440.));
        assert_eq!(drag.requested_width(px(700.)), px(220.));
    }
}
