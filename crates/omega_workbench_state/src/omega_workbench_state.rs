use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

mod entity_navigation;

pub use entity_navigation::{
    DomainBlockRoute, EntityNavigationError, EntityNavigationHistory, EntityRef, EntityRoute,
    EntityRouteFocus, EntityRouteIcon, EntityRouteKind, EntityRouteState,
    PersistedEntityNavigation, RouteAvailability, RouteUnavailableReason, WorkRoute,
};

pub const PROJECTION_STATE_SCHEMA_V1: &str = "openagents.omega.workbench-state.v1";
pub const MAX_IDENTIFIER_BYTES: usize = 256;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkSurface {
    Files,
    Search,
    Review,
    Forensics,
    Git,
    Terminal,
    Plan,
}

impl WorkSurface {
    pub const FALLBACK_ORDER: [Self; 7] = [
        Self::Files,
        Self::Search,
        Self::Review,
        Self::Forensics,
        Self::Git,
        Self::Terminal,
        Self::Plan,
    ];

    pub const fn requires_binding(self) -> bool {
        matches!(
            self,
            Self::Files
                | Self::Search
                | Self::Review
                | Self::Forensics
                | Self::Git
                | Self::Terminal
        )
    }
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct RepositoryBinding {
    pub repository_id: String,
    pub worktree_id: String,
}

impl RepositoryBinding {
    pub fn new(
        repository_id: impl Into<String>,
        worktree_id: impl Into<String>,
    ) -> Result<Self, ProjectionError> {
        let binding = Self {
            repository_id: repository_id.into(),
            worktree_id: worktree_id.into(),
        };
        binding.validate()?;
        Ok(binding)
    }

    fn validate(&self) -> Result<(), ProjectionError> {
        validate_id("repository", &self.repository_id)?;
        validate_id("worktree", &self.worktree_id)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConnectionPhase {
    Online,
    Offline,
    Reconnecting,
    StaleProjection,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ThreadProjection {
    pub binding: Option<RepositoryBinding>,
    pub generation: u64,
    pub available_surfaces: BTreeSet<WorkSurface>,
    pub requested_surface: Option<WorkSurface>,
    pub effective_surface: Option<WorkSurface>,
    pub dock_open: bool,
    pub focus_owner: Option<WorkSurface>,
    pub artifact_revision: u64,
    pub event_revision: u64,
}

impl ThreadProjection {
    pub fn new(
        binding: Option<RepositoryBinding>,
        available_surfaces: impl IntoIterator<Item = WorkSurface>,
    ) -> Result<Self, ProjectionError> {
        if let Some(binding) = &binding {
            binding.validate()?;
        }
        let mut projection = Self {
            binding,
            generation: 0,
            available_surfaces: available_surfaces.into_iter().collect(),
            requested_surface: None,
            effective_surface: None,
            dock_open: false,
            focus_owner: None,
            artifact_revision: 0,
            event_revision: 0,
        };
        projection.normalize();
        Ok(projection)
    }

    fn normalize(&mut self) {
        self.effective_surface =
            deterministic_surface(self.requested_surface, &self.available_surfaces);
        if self.dock_open && self.effective_surface.is_none() {
            self.dock_open = false;
        }
    }

    fn validate(&self, thread_id: &str) -> Result<(), ProjectionError> {
        if let Some(binding) = &self.binding {
            binding.validate()?;
        }
        let expected = deterministic_surface(self.requested_surface, &self.available_surfaces);
        if self.effective_surface != expected {
            return Err(ProjectionError::InvalidState(format!(
                "thread {thread_id:?} effective surface {:?} does not match deterministic projection {expected:?}",
                self.effective_surface
            )));
        }
        if self.dock_open && self.effective_surface.is_none() {
            return Err(ProjectionError::InvalidState(format!(
                "thread {thread_id:?} has an open dock without an effective surface"
            )));
        }
        if self.binding.is_none()
            && self
                .available_surfaces
                .iter()
                .any(|surface| surface.requires_binding())
        {
            return Err(ProjectionError::InvalidState(format!(
                "unbound thread {thread_id:?} advertises a repository-bound surface"
            )));
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PersistedSelection {
    pub thread_id: String,
    pub generation: u64,
    pub binding: Option<RepositoryBinding>,
    pub requested_surface: Option<WorkSurface>,
    pub dock_open: bool,
    pub revision: u64,
}

impl PersistedSelection {
    pub fn validate(&self) -> Result<(), ProjectionError> {
        validate_id("persisted thread", &self.thread_id)?;
        if let Some(binding) = &self.binding {
            binding.validate()?;
        }
        if self.dock_open && self.requested_surface.is_none() {
            return Err(ProjectionError::InvalidState(
                "persisted open dock has no requested surface".into(),
            ));
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PendingSurfaceLoad {
    pub request_id: String,
    pub thread_id: String,
    pub surface: WorkSurface,
    pub binding: Option<RepositoryBinding>,
    pub generation: u64,
}

impl PendingSurfaceLoad {
    fn validate(&self) -> Result<(), ProjectionError> {
        validate_id("request", &self.request_id)?;
        validate_id("load thread", &self.thread_id)?;
        if let Some(binding) = &self.binding {
            binding.validate()?;
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProjectionSnapshot {
    pub revision: u64,
    pub persistence_revision: u64,
    pub active_thread_id: Option<String>,
    pub threads: BTreeMap<String, ThreadProjection>,
    pub persisted_selection: Option<PersistedSelection>,
}

impl ProjectionSnapshot {
    fn validate(&self) -> Result<(), ProjectionError> {
        if let Some(active_thread_id) = &self.active_thread_id {
            validate_id("snapshot active thread", active_thread_id)?;
            if !self.threads.contains_key(active_thread_id) {
                return Err(ProjectionError::UnknownThread(active_thread_id.clone()));
            }
        }
        for (thread_id, thread) in &self.threads {
            validate_id("snapshot thread", thread_id)?;
            thread.validate(thread_id)?;
        }
        if let Some(selection) = &self.persisted_selection {
            selection.validate()?;
            if selection.revision != self.persistence_revision {
                return Err(ProjectionError::InvalidState(format!(
                    "snapshot selection revision {} does not match snapshot persistence revision {}",
                    selection.revision, self.persistence_revision
                )));
            }
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct VisibleProjection {
    pub thread_id: String,
    pub binding: Option<RepositoryBinding>,
    pub generation: u64,
    pub requested_surface: Option<WorkSurface>,
    pub effective_surface: Option<WorkSurface>,
    pub dock_open: bool,
    pub focus_owner: Option<WorkSurface>,
    pub artifact_revision: u64,
    pub event_revision: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkbenchProjection {
    pub schema: String,
    pub projection_revision: u64,
    pub persistence_revision: u64,
    pub connection: ConnectionPhase,
    pub active_thread_id: Option<String>,
    pub threads: BTreeMap<String, ThreadProjection>,
    pub pending_loads: BTreeMap<String, PendingSurfaceLoad>,
    pub persisted_selection: Option<PersistedSelection>,
    pub restore_pending: bool,
}

impl Default for WorkbenchProjection {
    fn default() -> Self {
        Self::new()
    }
}

impl WorkbenchProjection {
    pub fn new() -> Self {
        Self {
            schema: PROJECTION_STATE_SCHEMA_V1.into(),
            projection_revision: 0,
            persistence_revision: 0,
            connection: ConnectionPhase::Online,
            active_thread_id: None,
            threads: BTreeMap::new(),
            pending_loads: BTreeMap::new(),
            persisted_selection: None,
            restore_pending: false,
        }
    }

    pub fn visible_projection(&self) -> Option<VisibleProjection> {
        let thread_id = self.active_thread_id.as_ref()?;
        let thread = self.threads.get(thread_id)?;
        Some(VisibleProjection {
            thread_id: thread_id.clone(),
            binding: thread.binding.clone(),
            generation: thread.generation,
            requested_surface: thread.requested_surface,
            effective_surface: thread.effective_surface,
            dock_open: thread.dock_open,
            focus_owner: thread.focus_owner,
            artifact_revision: thread.artifact_revision,
            event_revision: thread.event_revision,
        })
    }

    pub fn apply(
        &mut self,
        transition: ProjectionTransition,
    ) -> Result<TransitionEffect, ProjectionError> {
        let mut next = self.clone();
        let effect = next.apply_unchecked(transition)?;
        next.refresh_focus_owner();
        next.validate()?;
        *self = next;
        Ok(effect)
    }

    pub fn validate(&self) -> Result<(), ProjectionError> {
        if self.schema != PROJECTION_STATE_SCHEMA_V1 {
            return Err(ProjectionError::InvalidState(format!(
                "unsupported projection schema {:?}",
                self.schema
            )));
        }
        if let Some(active_thread_id) = &self.active_thread_id {
            validate_id("active thread", active_thread_id)?;
            if !self.threads.contains_key(active_thread_id) {
                return Err(ProjectionError::UnknownThread(active_thread_id.clone()));
            }
        }
        for (thread_id, thread) in &self.threads {
            validate_id("thread", thread_id)?;
            thread.validate(thread_id)?;
        }
        validate_focus_owners(
            self.active_thread_id.as_deref(),
            &self.threads,
            self.connection == ConnectionPhase::Online,
        )?;
        for (request_id, load) in &self.pending_loads {
            if request_id != &load.request_id {
                return Err(ProjectionError::InvalidState(format!(
                    "pending load key {request_id:?} does not match request {:?}",
                    load.request_id
                )));
            }
            load.validate()?;
            if !self.threads.contains_key(&load.thread_id) {
                return Err(ProjectionError::UnknownThread(load.thread_id.clone()));
            }
        }
        if let Some(selection) = &self.persisted_selection {
            selection.validate()?;
            if selection.revision != self.persistence_revision {
                return Err(ProjectionError::InvalidState(format!(
                    "persisted revision {} does not match projection revision {}",
                    selection.revision, self.persistence_revision
                )));
            }
        }
        if self.restore_pending && self.persisted_selection.is_none() {
            return Err(ProjectionError::InvalidState(
                "restore is pending without persisted state".into(),
            ));
        }
        Ok(())
    }

    fn apply_unchecked(
        &mut self,
        transition: ProjectionTransition,
    ) -> Result<TransitionEffect, ProjectionError> {
        match transition {
            ProjectionTransition::OpenThread {
                thread_id,
                binding,
                available_surfaces,
            } => {
                validate_id("thread", &thread_id)?;
                if self.threads.contains_key(&thread_id) {
                    return Err(ProjectionError::DuplicateThread(thread_id));
                }
                let thread = ThreadProjection::new(binding, available_surfaces)?;
                self.threads.insert(thread_id.clone(), thread);
                if self.active_thread_id.is_none() {
                    self.active_thread_id = Some(thread_id);
                }
                Ok(TransitionEffect::Applied)
            }
            ProjectionTransition::CloseThread { thread_id } => {
                self.thread(&thread_id)?;
                self.threads.remove(&thread_id);
                self.pending_loads
                    .retain(|_, pending| pending.thread_id != thread_id);
                let closed_active_thread =
                    self.active_thread_id.as_deref() == Some(thread_id.as_str());
                if closed_active_thread {
                    self.active_thread_id = self.threads.keys().next().cloned();
                }
                if self
                    .persisted_selection
                    .as_ref()
                    .is_some_and(|selection| selection.thread_id == thread_id)
                {
                    self.restore_pending = false;
                }
                Ok(if closed_active_thread {
                    TransitionEffect::DeterministicFallback
                } else {
                    TransitionEffect::Applied
                })
            }
            ProjectionTransition::SwitchThread { thread_id } => {
                self.thread(&thread_id)?;
                self.active_thread_id = Some(thread_id);
                Ok(TransitionEffect::Applied)
            }
            ProjectionTransition::RequestSurface { thread_id, surface } => {
                self.require_active_thread(&thread_id)?;
                if surface != WorkSurface::Plan {
                    self.require_online()?;
                }
                if !self
                    .thread(&thread_id)?
                    .available_surfaces
                    .contains(&surface)
                {
                    return Err(ProjectionError::UnavailableSurface { thread_id, surface });
                }
                let thread = self.thread_mut(&thread_id)?;
                thread.requested_surface = Some(surface);
                thread.dock_open = true;
                thread.normalize();
                Ok(TransitionEffect::Applied)
            }
            ProjectionTransition::CloseSurface { thread_id } => {
                self.require_active_thread(&thread_id)?;
                let thread = self.thread_mut(&thread_id)?;
                thread.requested_surface = None;
                thread.dock_open = false;
                thread.normalize();
                Ok(TransitionEffect::Applied)
            }
            ProjectionTransition::CollapseDock { thread_id } => {
                self.require_active_thread(&thread_id)?;
                let thread = self.thread_mut(&thread_id)?;
                thread.dock_open = false;
                thread.normalize();
                Ok(TransitionEffect::Applied)
            }
            ProjectionTransition::ExpandDock { thread_id } => {
                self.require_active_thread(&thread_id)?;
                if self.thread(&thread_id)?.effective_surface != Some(WorkSurface::Plan) {
                    self.require_online()?;
                }
                let thread = self.thread_mut(&thread_id)?;
                thread.dock_open = true;
                thread.normalize();
                Ok(if thread.effective_surface.is_some() {
                    TransitionEffect::Applied
                } else {
                    TransitionEffect::DeterministicFallback
                })
            }
            ProjectionTransition::BindRepository {
                thread_id,
                generation,
                binding,
                available_surfaces,
            } => {
                binding.validate()?;
                let thread = self.thread_mut(&thread_id)?;
                require_generation(thread, generation)?;
                if thread.binding.is_some() {
                    return Err(ProjectionError::AlreadyBound(thread_id));
                }
                let previous_surface = thread.effective_surface;
                thread.binding = Some(binding);
                thread.generation = next_revision("thread generation", generation)?;
                thread.available_surfaces = available_surfaces.into_iter().collect();
                thread.normalize();
                Ok(fallback_effect(previous_surface, thread.effective_surface))
            }
            ProjectionTransition::ChangeWorktree {
                thread_id,
                generation,
                worktree_id,
                available_surfaces,
            } => {
                validate_id("worktree", &worktree_id)?;
                let thread = self.thread_mut(&thread_id)?;
                require_generation(thread, generation)?;
                let previous_surface = thread.effective_surface;
                let binding = thread
                    .binding
                    .as_mut()
                    .ok_or_else(|| ProjectionError::AlreadyUnbound(thread_id.clone()))?;
                binding.worktree_id = worktree_id;
                thread.generation = next_revision("thread generation", generation)?;
                thread.available_surfaces = available_surfaces.into_iter().collect();
                thread.normalize();
                Ok(fallback_effect(previous_surface, thread.effective_surface))
            }
            ProjectionTransition::RemoveBinding {
                thread_id,
                generation,
                available_surfaces,
            } => {
                let thread = self.thread_mut(&thread_id)?;
                require_generation(thread, generation)?;
                if thread.binding.is_none() {
                    return Err(ProjectionError::AlreadyUnbound(thread_id));
                }
                let previous_surface = thread.effective_surface;
                thread.binding = None;
                thread.generation = next_revision("thread generation", generation)?;
                thread.available_surfaces = available_surfaces.into_iter().collect();
                thread.normalize();
                Ok(fallback_effect(previous_surface, thread.effective_surface))
            }
            ProjectionTransition::ChangeBinding {
                thread_id,
                generation,
                binding,
                available_surfaces,
            } => {
                if let Some(binding) = &binding {
                    binding.validate()?;
                }
                let thread = self.thread_mut(&thread_id)?;
                require_generation(thread, generation)?;
                let previous_surface = thread.effective_surface;
                thread.binding = binding;
                thread.generation = next_revision("thread generation", generation)?;
                thread.available_surfaces = available_surfaces.into_iter().collect();
                thread.normalize();
                Ok(fallback_effect(previous_surface, thread.effective_surface))
            }
            ProjectionTransition::BeginSurfaceLoad {
                request_id,
                thread_id,
                surface,
                generation,
                binding,
            } => {
                validate_id("request", &request_id)?;
                if self.pending_loads.contains_key(&request_id) {
                    return Err(ProjectionError::DuplicateRequest(request_id));
                }
                let thread = self.thread(&thread_id)?;
                require_generation(thread, generation)?;
                if thread.binding != binding {
                    return Err(ProjectionError::InvalidBinding(thread_id));
                }
                if !thread.available_surfaces.contains(&surface) {
                    return Err(ProjectionError::UnavailableSurface { thread_id, surface });
                }
                self.pending_loads.insert(
                    request_id.clone(),
                    PendingSurfaceLoad {
                        request_id,
                        thread_id,
                        surface,
                        binding,
                        generation,
                    },
                );
                Ok(TransitionEffect::Applied)
            }
            ProjectionTransition::CompleteSurfaceLoad {
                request_id,
                thread_id,
                surface,
                generation,
                binding,
            } => {
                validate_id("request", &request_id)?;
                validate_id("thread", &thread_id)?;
                if binding
                    .as_ref()
                    .is_some_and(|binding| binding.validate().is_err())
                {
                    return Err(ProjectionError::InvalidBinding(thread_id));
                }
                let load = self
                    .pending_loads
                    .get(&request_id)
                    .cloned()
                    .ok_or_else(|| ProjectionError::UnknownRequest(request_id.clone()))?;
                if load.thread_id != thread_id
                    || load.surface != surface
                    || load.generation != generation
                    || load.binding != binding
                {
                    return Err(ProjectionError::RequestContextMismatch(request_id));
                }
                self.pending_loads.remove(&request_id);
                let thread = self.thread_mut(&load.thread_id)?;
                if thread.generation != load.generation
                    || thread.binding != load.binding
                    || !thread.available_surfaces.contains(&load.surface)
                {
                    return Ok(TransitionEffect::StaleCompletionIgnored);
                }
                Ok(TransitionEffect::Applied)
            }
            ProjectionTransition::FailSurfaceLoad {
                request_id,
                thread_id,
                surface,
                generation,
                binding,
            } => {
                validate_id("request", &request_id)?;
                validate_id("thread", &thread_id)?;
                if binding
                    .as_ref()
                    .is_some_and(|binding| binding.validate().is_err())
                {
                    return Err(ProjectionError::InvalidBinding(thread_id));
                }
                let load = self
                    .pending_loads
                    .get(&request_id)
                    .cloned()
                    .ok_or_else(|| ProjectionError::UnknownRequest(request_id.clone()))?;
                if load.thread_id != thread_id
                    || load.surface != surface
                    || load.generation != generation
                    || load.binding != binding
                {
                    return Err(ProjectionError::RequestContextMismatch(request_id));
                }
                self.pending_loads.remove(&request_id);
                let thread = self.thread(&load.thread_id)?;
                if thread.generation != load.generation
                    || thread.binding != load.binding
                    || !thread.available_surfaces.contains(&load.surface)
                {
                    Ok(TransitionEffect::StaleCompletionIgnored)
                } else {
                    Ok(TransitionEffect::Applied)
                }
            }
            ProjectionTransition::Disconnect => {
                if !matches!(
                    self.connection,
                    ConnectionPhase::Online | ConnectionPhase::StaleProjection
                ) {
                    return Err(ProjectionError::InvalidConnectionTransition {
                        from: self.connection,
                        action: "disconnect",
                    });
                }
                self.connection = ConnectionPhase::Offline;
                Ok(TransitionEffect::Applied)
            }
            ProjectionTransition::Reconnect => {
                if self.connection != ConnectionPhase::Offline {
                    return Err(ProjectionError::InvalidConnectionTransition {
                        from: self.connection,
                        action: "reconnect",
                    });
                }
                self.connection = ConnectionPhase::Reconnecting;
                Ok(TransitionEffect::Applied)
            }
            ProjectionTransition::ReceiveProjectionSnapshot { snapshot } => {
                if !matches!(
                    self.connection,
                    ConnectionPhase::Reconnecting | ConnectionPhase::StaleProjection
                ) {
                    return Err(ProjectionError::InvalidConnectionTransition {
                        from: self.connection,
                        action: "receive_projection_snapshot",
                    });
                }
                snapshot.validate()?;
                if snapshot.revision <= self.projection_revision
                    || snapshot.persistence_revision < self.persistence_revision
                {
                    self.connection = ConnectionPhase::StaleProjection;
                    return Ok(TransitionEffect::OlderRevisionIgnored);
                }
                let used_fallback = snapshot.threads.values().any(|thread| {
                    thread.requested_surface.is_some()
                        && thread.requested_surface != thread.effective_surface
                });
                self.projection_revision = snapshot.revision;
                self.active_thread_id = snapshot.active_thread_id;
                self.threads = snapshot.threads;
                self.pending_loads.clear();
                self.persistence_revision = snapshot.persistence_revision;
                self.persisted_selection = snapshot.persisted_selection;
                self.connection = ConnectionPhase::Online;
                self.restore_pending = false;
                Ok(if used_fallback {
                    TransitionEffect::DeterministicFallback
                } else {
                    TransitionEffect::Applied
                })
            }
            ProjectionTransition::PersistSelection { revision } => {
                if revision <= self.persistence_revision {
                    return Ok(TransitionEffect::OlderRevisionIgnored);
                }
                let thread_id = self
                    .active_thread_id
                    .clone()
                    .ok_or(ProjectionError::NoActiveThread)?;
                let (generation, binding, requested_surface, dock_open) = {
                    let thread = self.thread(&thread_id)?;
                    (
                        thread.generation,
                        thread.binding.clone(),
                        thread.requested_surface,
                        thread.dock_open,
                    )
                };
                self.persistence_revision = revision;
                self.persisted_selection = Some(PersistedSelection {
                    thread_id,
                    generation,
                    binding,
                    requested_surface,
                    dock_open,
                    revision,
                });
                Ok(TransitionEffect::Applied)
            }
            ProjectionTransition::AdoptPersistedSelection { selection } => {
                selection.validate()?;
                // Durable records come from an earlier projection instance, so
                // adoption must re-stamp the persistence revision: keep the
                // record's revision when it is ahead, and never regress a
                // projection that already persisted further in this session.
                let revision = selection.revision.max(1).max(self.persistence_revision);
                if self
                    .persisted_selection
                    .as_ref()
                    .is_some_and(|current| current.revision >= revision)
                {
                    return Ok(TransitionEffect::OlderRevisionIgnored);
                }
                self.persistence_revision = revision;
                self.persisted_selection = Some(PersistedSelection {
                    revision,
                    ..selection
                });
                Ok(TransitionEffect::Applied)
            }
            ProjectionTransition::ColdStart => {
                for thread in self.threads.values_mut() {
                    thread.dock_open = false;
                    thread.normalize();
                }
                self.active_thread_id = None;
                self.pending_loads.clear();
                self.restore_pending = self.persisted_selection.is_some();
                Ok(TransitionEffect::Applied)
            }
            ProjectionTransition::RestoreSelection => {
                if !self.restore_pending {
                    return Err(ProjectionError::RestoreNotPending);
                }
                self.require_online()?;
                let selection = self
                    .persisted_selection
                    .clone()
                    .ok_or(ProjectionError::NoPersistedSelection)?;
                let effect = if let Some(thread) = self.threads.get_mut(&selection.thread_id) {
                    self.active_thread_id = Some(selection.thread_id);
                    if thread.generation == selection.generation
                        && thread.binding == selection.binding
                        && selection
                            .requested_surface
                            .is_none_or(|surface| thread.available_surfaces.contains(&surface))
                    {
                        thread.requested_surface = selection.requested_surface;
                        thread.dock_open = selection.dock_open;
                        thread.normalize();
                        if thread.requested_surface == thread.effective_surface {
                            TransitionEffect::Applied
                        } else {
                            TransitionEffect::DeterministicFallback
                        }
                    } else {
                        thread.requested_surface =
                            deterministic_fallback(&thread.available_surfaces);
                        thread.dock_open =
                            selection.dock_open && thread.requested_surface.is_some();
                        thread.normalize();
                        TransitionEffect::DeterministicFallback
                    }
                } else {
                    self.active_thread_id = self.threads.keys().next().cloned();
                    if let Some(thread_id) = &self.active_thread_id
                        && let Some(thread) = self.threads.get_mut(thread_id)
                    {
                        thread.requested_surface =
                            deterministic_fallback(&thread.available_surfaces);
                        thread.dock_open =
                            selection.dock_open && thread.requested_surface.is_some();
                        thread.normalize();
                    }
                    TransitionEffect::DeterministicFallback
                };
                self.restore_pending = false;
                self.persisted_selection = self.active_thread_id.as_ref().and_then(|thread_id| {
                    self.threads
                        .get(thread_id)
                        .map(|thread| PersistedSelection {
                            thread_id: thread_id.clone(),
                            generation: thread.generation,
                            binding: thread.binding.clone(),
                            requested_surface: thread.requested_surface,
                            dock_open: thread.dock_open,
                            revision: self.persistence_revision,
                        })
                });
                Ok(effect)
            }
            ProjectionTransition::InvalidateCapability {
                thread_id,
                generation,
                surface,
            } => {
                let thread = self.thread_mut(&thread_id)?;
                require_generation(thread, generation)?;
                if !thread.available_surfaces.remove(&surface) {
                    return Err(ProjectionError::CapabilityAlreadyUnavailable {
                        thread_id,
                        surface,
                    });
                }
                thread.generation = next_revision("thread generation", generation)?;
                thread.normalize();
                Ok(if thread.requested_surface == thread.effective_surface {
                    TransitionEffect::Applied
                } else {
                    TransitionEffect::DeterministicFallback
                })
            }
            ProjectionTransition::DispatchSurfaceCommand {
                thread_id,
                surface,
                binding,
                generation,
            } => {
                self.require_online()?;
                self.require_active_thread(&thread_id)?;
                let thread = self.thread(&thread_id)?;
                require_generation(thread, generation)?;
                if thread.binding != binding {
                    return Err(ProjectionError::CommandBindingMismatch(format!(
                        "command binding does not match thread {thread_id:?}"
                    )));
                }
                if !thread.dock_open
                    || thread.focus_owner != Some(surface)
                    || thread.effective_surface != Some(surface)
                {
                    return Err(ProjectionError::UnavailableSurface { thread_id, surface });
                }
                Ok(TransitionEffect::Applied)
            }
        }
    }

    fn thread(&self, thread_id: &str) -> Result<&ThreadProjection, ProjectionError> {
        validate_id("thread", thread_id)?;
        self.threads
            .get(thread_id)
            .ok_or_else(|| ProjectionError::UnknownThread(thread_id.into()))
    }

    fn thread_mut(&mut self, thread_id: &str) -> Result<&mut ThreadProjection, ProjectionError> {
        validate_id("thread", thread_id)?;
        self.threads
            .get_mut(thread_id)
            .ok_or_else(|| ProjectionError::UnknownThread(thread_id.into()))
    }

    fn require_active_thread(&self, thread_id: &str) -> Result<(), ProjectionError> {
        validate_id("thread", thread_id)?;
        if self.active_thread_id.as_deref() != Some(thread_id) {
            return Err(ProjectionError::InactiveThread {
                requested: thread_id.into(),
                active: self.active_thread_id.clone(),
            });
        }
        Ok(())
    }

    fn require_online(&self) -> Result<(), ProjectionError> {
        if self.connection != ConnectionPhase::Online {
            return Err(ProjectionError::InvalidConnectionTransition {
                from: self.connection,
                action: "dispatch_online_action",
            });
        }
        Ok(())
    }

    fn refresh_focus_owner(&mut self) {
        for thread in self.threads.values_mut() {
            thread.focus_owner = None;
        }
        if self.connection != ConnectionPhase::Online {
            return;
        }
        let Some(active_thread_id) = self.active_thread_id.as_ref() else {
            return;
        };
        let Some(thread) = self.threads.get_mut(active_thread_id) else {
            return;
        };
        thread.focus_owner = thread
            .dock_open
            .then_some(thread.effective_surface)
            .flatten();
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ProjectionTransition {
    OpenThread {
        thread_id: String,
        binding: Option<RepositoryBinding>,
        available_surfaces: Vec<WorkSurface>,
    },
    CloseThread {
        thread_id: String,
    },
    SwitchThread {
        thread_id: String,
    },
    RequestSurface {
        thread_id: String,
        surface: WorkSurface,
    },
    CloseSurface {
        thread_id: String,
    },
    CollapseDock {
        thread_id: String,
    },
    ExpandDock {
        thread_id: String,
    },
    BindRepository {
        thread_id: String,
        generation: u64,
        binding: RepositoryBinding,
        available_surfaces: Vec<WorkSurface>,
    },
    ChangeWorktree {
        thread_id: String,
        generation: u64,
        worktree_id: String,
        available_surfaces: Vec<WorkSurface>,
    },
    RemoveBinding {
        thread_id: String,
        generation: u64,
        available_surfaces: Vec<WorkSurface>,
    },
    ChangeBinding {
        thread_id: String,
        generation: u64,
        binding: Option<RepositoryBinding>,
        available_surfaces: Vec<WorkSurface>,
    },
    BeginSurfaceLoad {
        request_id: String,
        thread_id: String,
        surface: WorkSurface,
        generation: u64,
        binding: Option<RepositoryBinding>,
    },
    CompleteSurfaceLoad {
        request_id: String,
        thread_id: String,
        surface: WorkSurface,
        generation: u64,
        binding: Option<RepositoryBinding>,
    },
    FailSurfaceLoad {
        request_id: String,
        thread_id: String,
        surface: WorkSurface,
        generation: u64,
        binding: Option<RepositoryBinding>,
    },
    Disconnect,
    Reconnect,
    ReceiveProjectionSnapshot {
        snapshot: ProjectionSnapshot,
    },
    PersistSelection {
        revision: u64,
    },
    AdoptPersistedSelection {
        selection: PersistedSelection,
    },
    ColdStart,
    RestoreSelection,
    InvalidateCapability {
        thread_id: String,
        generation: u64,
        surface: WorkSurface,
    },
    DispatchSurfaceCommand {
        thread_id: String,
        surface: WorkSurface,
        binding: Option<RepositoryBinding>,
        generation: u64,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TransitionEffect {
    Applied,
    StaleCompletionIgnored,
    OlderRevisionIgnored,
    DeterministicFallback,
}

#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum ProjectionError {
    #[error("{0}")]
    InvalidState(String),
    #[error("invalid {kind} ID {value:?}")]
    InvalidId { kind: &'static str, value: String },
    #[error("thread {0:?} already exists")]
    DuplicateThread(String),
    #[error("thread {0:?} does not exist")]
    UnknownThread(String),
    #[error("request {0:?} already exists")]
    DuplicateRequest(String),
    #[error("request {0:?} does not exist")]
    UnknownRequest(String),
    #[error("surface load completion does not match request {0:?}")]
    RequestContextMismatch(String),
    #[error("surface {surface:?} is unavailable for thread {thread_id:?}")]
    UnavailableSurface {
        thread_id: String,
        surface: WorkSurface,
    },
    #[error("the workbench has no active thread")]
    NoActiveThread,
    #[error("there is no persisted selection to restore")]
    NoPersistedSelection,
    #[error("restore was requested without a pending cold-start restoration")]
    RestoreNotPending,
    #[error("thread {0:?} already has a repository binding")]
    AlreadyBound(String),
    #[error("thread {0:?} has no repository binding")]
    AlreadyUnbound(String),
    #[error("thread {0:?} has a mismatched repository binding")]
    InvalidBinding(String),
    #[error("surface {surface:?} is already unavailable for thread {thread_id:?}")]
    CapabilityAlreadyUnavailable {
        thread_id: String,
        surface: WorkSurface,
    },
    #[error("thread generation is {actual}, not requested generation {requested}")]
    StaleGeneration { requested: u64, actual: u64 },
    #[error("surface command binding mismatch: {0}")]
    CommandBindingMismatch(String),
    #[error("thread {requested:?} is inactive; active thread is {active:?}")]
    InactiveThread {
        requested: String,
        active: Option<String>,
    },
    #[error("cannot {action} while connection phase is {from:?}")]
    InvalidConnectionTransition {
        from: ConnectionPhase,
        action: &'static str,
    },
    #[error("{field} cannot advance past u64::MAX")]
    RevisionOverflow { field: &'static str },
}

fn deterministic_surface(
    requested_surface: Option<WorkSurface>,
    available_surfaces: &BTreeSet<WorkSurface>,
) -> Option<WorkSurface> {
    match requested_surface {
        None => None,
        Some(requested_surface) if available_surfaces.contains(&requested_surface) => {
            Some(requested_surface)
        }
        Some(_) => WorkSurface::FALLBACK_ORDER
            .into_iter()
            .find(|surface| available_surfaces.contains(surface)),
    }
}

fn deterministic_fallback(available_surfaces: &BTreeSet<WorkSurface>) -> Option<WorkSurface> {
    WorkSurface::FALLBACK_ORDER
        .into_iter()
        .find(|surface| available_surfaces.contains(surface))
}

fn fallback_effect(
    previous_surface: Option<WorkSurface>,
    effective_surface: Option<WorkSurface>,
) -> TransitionEffect {
    if previous_surface == effective_surface {
        TransitionEffect::Applied
    } else {
        TransitionEffect::DeterministicFallback
    }
}

fn validate_id(kind: &'static str, value: &str) -> Result<(), ProjectionError> {
    if value.is_empty()
        || value.len() > MAX_IDENTIFIER_BYTES
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
    {
        return Err(ProjectionError::InvalidId {
            kind,
            value: value.into(),
        });
    }
    Ok(())
}

fn require_generation(thread: &ThreadProjection, generation: u64) -> Result<(), ProjectionError> {
    if thread.generation == generation {
        Ok(())
    } else {
        Err(ProjectionError::StaleGeneration {
            requested: generation,
            actual: thread.generation,
        })
    }
}

fn next_revision(field: &'static str, current: u64) -> Result<u64, ProjectionError> {
    current
        .checked_add(1)
        .ok_or(ProjectionError::RevisionOverflow { field })
}

fn validate_focus_owners(
    active_thread_id: Option<&str>,
    threads: &BTreeMap<String, ThreadProjection>,
    connection_is_online: bool,
) -> Result<(), ProjectionError> {
    for (thread_id, thread) in threads {
        let expected = if connection_is_online && active_thread_id == Some(thread_id.as_str()) {
            thread
                .dock_open
                .then_some(thread.effective_surface)
                .flatten()
        } else {
            None
        };
        if thread.focus_owner != expected {
            return Err(ProjectionError::InvalidState(format!(
                "thread {thread_id:?} focus owner {:?} does not match {expected:?}",
                thread.focus_owner
            )));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    fn binding(name: &str) -> RepositoryBinding {
        RepositoryBinding::new("repo", name).expect("valid test binding")
    }

    fn repository_surfaces() -> Vec<WorkSurface> {
        WorkSurface::FALLBACK_ORDER.into()
    }

    fn open_thread(projection: &mut WorkbenchProjection, thread_id: &str, worktree_id: &str) {
        projection
            .apply(ProjectionTransition::OpenThread {
                thread_id: thread_id.into(),
                binding: Some(binding(worktree_id)),
                available_surfaces: repository_surfaces(),
            })
            .expect("open thread");
    }

    proptest! {
        #[test]
        fn generated_surface_sequences_preserve_projection_invariants(
            actions in proptest::collection::vec(0u8..=15, 0..128)
        ) {
            let mut projection = WorkbenchProjection::new();
            open_thread(&mut projection, "thread-a", "worktree-a");
            open_thread(&mut projection, "thread-b", "worktree-b");

            for action in actions {
                let active_thread_id = if action & 1 == 0 {
                    "thread-a"
                } else {
                    "thread-b"
                };
                let transition = match action % 8 {
                    0 => ProjectionTransition::SwitchThread {
                        thread_id: active_thread_id.into(),
                    },
                    1..=6 => {
                        let surface = WorkSurface::FALLBACK_ORDER
                            [(action as usize - 1) % WorkSurface::FALLBACK_ORDER.len()];
                        ProjectionTransition::RequestSurface {
                            thread_id: active_thread_id.into(),
                            surface,
                        }
                    }
                    _ => ProjectionTransition::CollapseDock {
                        thread_id: active_thread_id.into(),
                    },
                };
                let before = projection.clone();
                let result = projection.apply(transition);
                if result.is_err() {
                    prop_assert_eq!(&projection, &before);
                }
                prop_assert!(projection.validate().is_ok());
                let focused_threads = projection
                    .threads
                    .values()
                    .filter(|thread| thread.focus_owner.is_some())
                    .count();
                prop_assert!(focused_threads <= 1);
            }
        }
    }

    #[test]
    fn switching_threads_projects_only_the_active_thread() {
        let mut projection = WorkbenchProjection::new();
        open_thread(&mut projection, "thread-a", "worktree-a");
        open_thread(&mut projection, "thread-b", "worktree-b");
        projection
            .apply(ProjectionTransition::RequestSurface {
                thread_id: "thread-a".into(),
                surface: WorkSurface::Git,
            })
            .expect("select git");
        projection
            .apply(ProjectionTransition::SwitchThread {
                thread_id: "thread-b".into(),
            })
            .expect("switch to thread b");
        projection
            .apply(ProjectionTransition::RequestSurface {
                thread_id: "thread-b".into(),
                surface: WorkSurface::Terminal,
            })
            .expect("select terminal");
        projection
            .apply(ProjectionTransition::SwitchThread {
                thread_id: "thread-b".into(),
            })
            .expect("switch thread");

        let visible = projection.visible_projection().expect("visible projection");
        assert_eq!(visible.thread_id, "thread-b");
        assert_eq!(visible.binding, Some(binding("worktree-b")));
        assert_eq!(visible.effective_surface, Some(WorkSurface::Terminal));
    }

    #[test]
    fn stale_completion_cannot_overwrite_a_new_binding() {
        let mut projection = WorkbenchProjection::new();
        open_thread(&mut projection, "thread-a", "worktree-a");
        projection
            .apply(ProjectionTransition::BeginSurfaceLoad {
                request_id: "request-a".into(),
                thread_id: "thread-a".into(),
                surface: WorkSurface::Git,
                generation: 0,
                binding: Some(binding("worktree-a")),
            })
            .expect("begin load");
        projection
            .apply(ProjectionTransition::ChangeWorktree {
                thread_id: "thread-a".into(),
                generation: 0,
                worktree_id: "worktree-b".into(),
                available_surfaces: repository_surfaces(),
            })
            .expect("change binding");

        assert_eq!(
            projection
                .apply(ProjectionTransition::CompleteSurfaceLoad {
                    request_id: "request-a".into(),
                    thread_id: "thread-a".into(),
                    surface: WorkSurface::Git,
                    generation: 0,
                    binding: Some(binding("worktree-a")),
                })
                .expect("ignore stale completion"),
            TransitionEffect::StaleCompletionIgnored
        );
        let thread = projection.thread("thread-a").expect("thread exists");
        assert_eq!(thread.binding, Some(binding("worktree-b")));
        assert_eq!(thread.artifact_revision, 0);
        assert_eq!(thread.event_revision, 0);
    }

    #[test]
    fn repository_and_worktree_change_is_one_atomic_generation() {
        let mut projection = WorkbenchProjection::new();
        open_thread(&mut projection, "thread-a", "worktree-a");
        projection
            .apply(ProjectionTransition::RequestSurface {
                thread_id: "thread-a".into(),
                surface: WorkSurface::Git,
            })
            .expect("select git");
        projection
            .apply(ProjectionTransition::BeginSurfaceLoad {
                request_id: "request-a".into(),
                thread_id: "thread-a".into(),
                surface: WorkSurface::Git,
                generation: 0,
                binding: Some(binding("worktree-a")),
            })
            .expect("begin load");
        let replacement =
            RepositoryBinding::new("repo-b", "worktree-b").expect("replacement binding");

        projection
            .apply(ProjectionTransition::ChangeBinding {
                thread_id: "thread-a".into(),
                generation: 0,
                binding: Some(replacement.clone()),
                available_surfaces: vec![WorkSurface::Files, WorkSurface::Plan],
            })
            .expect("atomically replace binding");

        let thread = projection.thread("thread-a").expect("thread exists");
        assert_eq!(thread.binding, Some(replacement));
        assert_eq!(thread.generation, 1);
        assert_eq!(thread.effective_surface, Some(WorkSurface::Files));
        assert_eq!(
            projection
                .apply(ProjectionTransition::CompleteSurfaceLoad {
                    request_id: "request-a".into(),
                    thread_id: "thread-a".into(),
                    surface: WorkSurface::Git,
                    generation: 0,
                    binding: Some(binding("worktree-a")),
                })
                .expect("ignore stale completion"),
            TransitionEffect::StaleCompletionIgnored
        );
    }

    #[test]
    fn same_binding_change_refreshes_the_content_generation() {
        let mut projection = WorkbenchProjection::new();
        open_thread(&mut projection, "thread-a", "worktree-a");
        projection
            .apply(ProjectionTransition::BeginSurfaceLoad {
                request_id: "request-a".into(),
                thread_id: "thread-a".into(),
                surface: WorkSurface::Files,
                generation: 0,
                binding: Some(binding("worktree-a")),
            })
            .expect("begin load before the content epoch changes");

        projection
            .apply(ProjectionTransition::ChangeBinding {
                thread_id: "thread-a".into(),
                generation: 0,
                binding: Some(binding("worktree-a")),
                available_surfaces: WorkSurface::FALLBACK_ORDER.into(),
            })
            .expect("refresh the same binding after a branch change");

        assert_eq!(
            projection
                .thread("thread-a")
                .expect("thread remains projected")
                .generation,
            1
        );
        assert_eq!(
            projection
                .apply(ProjectionTransition::CompleteSurfaceLoad {
                    request_id: "request-a".into(),
                    thread_id: "thread-a".into(),
                    surface: WorkSurface::Files,
                    generation: 0,
                    binding: Some(binding("worktree-a")),
                })
                .expect("old-epoch load completion should be handled"),
            TransitionEffect::StaleCompletionIgnored
        );
    }

    #[test]
    fn invalid_surface_uses_documented_fallback() {
        let mut projection = WorkbenchProjection::new();
        open_thread(&mut projection, "thread-a", "worktree-a");
        projection
            .apply(ProjectionTransition::RequestSurface {
                thread_id: "thread-a".into(),
                surface: WorkSurface::Git,
            })
            .expect("select git");

        assert_eq!(
            projection
                .apply(ProjectionTransition::InvalidateCapability {
                    thread_id: "thread-a".into(),
                    generation: 0,
                    surface: WorkSurface::Git,
                })
                .expect("invalidate git"),
            TransitionEffect::DeterministicFallback
        );
        assert_eq!(
            projection
                .visible_projection()
                .expect("visible projection")
                .effective_surface,
            Some(WorkSurface::Files)
        );
    }

    #[test]
    fn unavailable_surface_request_is_rejected_atomically() {
        let mut projection = WorkbenchProjection::new();
        projection
            .apply(ProjectionTransition::OpenThread {
                thread_id: "thread-a".into(),
                binding: None,
                available_surfaces: vec![WorkSurface::Plan],
            })
            .expect("open unbound thread");
        let before = projection.clone();

        let error = projection
            .apply(ProjectionTransition::RequestSurface {
                thread_id: "thread-a".into(),
                surface: WorkSurface::Files,
            })
            .expect_err("files should be unavailable without a binding");
        assert!(matches!(
            error,
            ProjectionError::UnavailableSurface {
                surface: WorkSurface::Files,
                ..
            }
        ));
        assert_eq!(projection, before);
    }

    #[test]
    fn hidden_or_cross_binding_surface_cannot_receive_commands() {
        let mut projection = WorkbenchProjection::new();
        open_thread(&mut projection, "thread-a", "worktree-a");
        open_thread(&mut projection, "thread-b", "worktree-b");
        projection
            .apply(ProjectionTransition::RequestSurface {
                thread_id: "thread-a".into(),
                surface: WorkSurface::Terminal,
            })
            .expect("select terminal");

        let error = projection
            .apply(ProjectionTransition::DispatchSurfaceCommand {
                thread_id: "thread-b".into(),
                surface: WorkSurface::Terminal,
                binding: Some(binding("worktree-b")),
                generation: 0,
            })
            .expect_err("hidden thread cannot receive a command");
        assert!(matches!(error, ProjectionError::InactiveThread { .. }));

        projection
            .apply(ProjectionTransition::CollapseDock {
                thread_id: "thread-a".into(),
            })
            .expect("collapse dock");
        let error = projection
            .apply(ProjectionTransition::DispatchSurfaceCommand {
                thread_id: "thread-a".into(),
                surface: WorkSurface::Terminal,
                binding: Some(binding("worktree-a")),
                generation: 0,
            })
            .expect_err("collapsed surface cannot receive a command");
        assert!(matches!(error, ProjectionError::UnavailableSurface { .. }));
    }

    #[test]
    fn offline_plan_can_open_and_expand() {
        let mut projection = WorkbenchProjection::new();
        open_thread(&mut projection, "thread-a", "worktree-a");
        projection
            .apply(ProjectionTransition::Disconnect)
            .expect("disconnect");
        projection
            .apply(ProjectionTransition::RequestSurface {
                thread_id: "thread-a".into(),
                surface: WorkSurface::Plan,
            })
            .expect("offline Plan is thread-local");
        projection
            .apply(ProjectionTransition::CollapseDock {
                thread_id: "thread-a".into(),
            })
            .expect("collapse Plan");
        projection
            .apply(ProjectionTransition::ExpandDock {
                thread_id: "thread-a".into(),
            })
            .expect("expand offline Plan");

        let before = projection.clone();
        let error = projection
            .apply(ProjectionTransition::RequestSurface {
                thread_id: "thread-a".into(),
                surface: WorkSurface::Git,
            })
            .expect_err("repository-bound surfaces require a connection");
        assert!(matches!(
            error,
            ProjectionError::InvalidConnectionTransition {
                from: ConnectionPhase::Offline,
                ..
            }
        ));
        assert_eq!(projection, before);
    }

    #[test]
    fn persistence_is_monotonic_and_restore_is_thread_local() {
        let mut projection = WorkbenchProjection::new();
        open_thread(&mut projection, "thread-a", "worktree-a");
        open_thread(&mut projection, "thread-b", "worktree-b");
        projection
            .apply(ProjectionTransition::RequestSurface {
                thread_id: "thread-a".into(),
                surface: WorkSurface::Review,
            })
            .expect("select review");
        projection
            .apply(ProjectionTransition::PersistSelection { revision: 2 })
            .expect("persist");
        assert_eq!(
            projection
                .apply(ProjectionTransition::PersistSelection { revision: 1 })
                .expect("ignore older persist"),
            TransitionEffect::OlderRevisionIgnored
        );

        projection
            .apply(ProjectionTransition::ColdStart)
            .expect("cold start");
        projection
            .apply(ProjectionTransition::RestoreSelection)
            .expect("restore selection");
        let visible = projection.visible_projection().expect("visible projection");
        assert_eq!(visible.thread_id, "thread-a");
        assert_eq!(visible.effective_surface, Some(WorkSurface::Review));
        assert_eq!(projection.persistence_revision, 2);
    }

    #[test]
    fn adopting_a_disk_selection_into_a_fresh_projection_reconciles_revisions() {
        // Regression: a previous session persisted revision 2 to disk, while a
        // fresh projection starts at persistence revision 0. Directly poking
        // `persisted_selection` used to install that mismatch permanently and
        // every later transition failed validation with
        // "persisted revision 2 does not match projection revision 0".
        let disk_selection = PersistedSelection {
            thread_id: "thread-a".into(),
            generation: 0,
            binding: Some(binding("worktree-a")),
            requested_surface: Some(WorkSurface::Git),
            dock_open: true,
            revision: 2,
        };

        let mut projection = WorkbenchProjection::new();
        assert_eq!(projection.persistence_revision, 0);
        open_thread(&mut projection, "thread-a", "worktree-a");
        assert_eq!(
            projection
                .apply(ProjectionTransition::AdoptPersistedSelection {
                    selection: disk_selection,
                })
                .expect("adopt the disk selection"),
            TransitionEffect::Applied
        );
        assert_eq!(projection.persistence_revision, 2);
        assert!(projection.validate().is_ok());

        projection
            .apply(ProjectionTransition::ColdStart)
            .expect("cold start after adoption");
        projection
            .apply(ProjectionTransition::RestoreSelection)
            .expect("restore the adopted selection");
        let visible = projection.visible_projection().expect("visible projection");
        assert_eq!(visible.thread_id, "thread-a");
        assert_eq!(visible.effective_surface, Some(WorkSurface::Git));
        assert!(visible.dock_open);

        // Later transitions keep working: the projection is not poisoned.
        projection
            .apply(ProjectionTransition::RequestSurface {
                thread_id: "thread-a".into(),
                surface: WorkSurface::Files,
            })
            .expect("the projection accepts transitions after adoption");
    }

    #[test]
    fn adoption_never_regresses_persistence_and_ignores_older_records() {
        let mut projection = WorkbenchProjection::new();
        open_thread(&mut projection, "thread-a", "worktree-a");
        projection
            .apply(ProjectionTransition::RequestSurface {
                thread_id: "thread-a".into(),
                surface: WorkSurface::Review,
            })
            .expect("select review");
        projection
            .apply(ProjectionTransition::PersistSelection { revision: 5 })
            .expect("persist in this session");

        // An in-memory selection at revision 5 outranks a disk record at 2.
        assert_eq!(
            projection
                .apply(ProjectionTransition::AdoptPersistedSelection {
                    selection: PersistedSelection {
                        thread_id: "thread-a".into(),
                        generation: 0,
                        binding: Some(binding("worktree-a")),
                        requested_surface: Some(WorkSurface::Git),
                        dock_open: true,
                        revision: 2,
                    },
                })
                .expect("older record is handled"),
            TransitionEffect::OlderRevisionIgnored
        );
        assert_eq!(projection.persistence_revision, 5);
        assert_eq!(
            projection
                .persisted_selection
                .as_ref()
                .and_then(|selection| selection.requested_surface),
            Some(WorkSurface::Review)
        );

        // A revision-0 record (never validly written) is clamped, not adopted
        // at a reserved revision.
        let mut fresh = WorkbenchProjection::new();
        fresh
            .apply(ProjectionTransition::AdoptPersistedSelection {
                selection: PersistedSelection {
                    thread_id: "thread-a".into(),
                    generation: 0,
                    binding: None,
                    requested_surface: None,
                    dock_open: false,
                    revision: 0,
                },
            })
            .expect("adopt a zero-revision record");
        assert_eq!(fresh.persistence_revision, 1);
        assert!(fresh.validate().is_ok());
    }

    #[test]
    fn removed_binding_cannot_keep_git_or_terminal_effective() {
        let mut projection = WorkbenchProjection::new();
        open_thread(&mut projection, "thread-a", "worktree-a");
        projection
            .apply(ProjectionTransition::RequestSurface {
                thread_id: "thread-a".into(),
                surface: WorkSurface::Terminal,
            })
            .expect("select terminal");
        projection
            .apply(ProjectionTransition::RemoveBinding {
                thread_id: "thread-a".into(),
                generation: 0,
                available_surfaces: vec![WorkSurface::Plan],
            })
            .expect("remove binding");
        let visible = projection.visible_projection().expect("visible projection");
        assert_eq!(visible.binding, None);
        assert_eq!(visible.effective_surface, Some(WorkSurface::Plan));
        assert_eq!(visible.focus_owner, Some(WorkSurface::Plan));
    }

    #[test]
    fn restore_repairs_a_persisted_surface_that_lost_its_capability() {
        let mut projection = WorkbenchProjection::new();
        open_thread(&mut projection, "thread-a", "worktree-a");
        projection
            .apply(ProjectionTransition::RequestSurface {
                thread_id: "thread-a".into(),
                surface: WorkSurface::Git,
            })
            .expect("select git");
        projection
            .apply(ProjectionTransition::PersistSelection { revision: 1 })
            .expect("persist git");
        projection
            .apply(ProjectionTransition::ColdStart)
            .expect("cold start");
        projection
            .apply(ProjectionTransition::InvalidateCapability {
                thread_id: "thread-a".into(),
                generation: 0,
                surface: WorkSurface::Git,
            })
            .expect("invalidate git");

        assert_eq!(
            projection
                .apply(ProjectionTransition::RestoreSelection)
                .expect("restore fallback"),
            TransitionEffect::DeterministicFallback
        );
        let visible = projection.visible_projection().expect("visible fallback");
        assert_eq!(visible.requested_surface, Some(WorkSurface::Files));
        assert_eq!(visible.effective_surface, Some(WorkSurface::Files));
        assert_eq!(
            projection
                .persisted_selection
                .as_ref()
                .and_then(|selection| selection.requested_surface),
            Some(WorkSurface::Files)
        );
    }

    #[test]
    fn older_reconnect_snapshot_cannot_roll_state_back() {
        let mut projection = WorkbenchProjection::new();
        open_thread(&mut projection, "thread-a", "worktree-a");
        projection.projection_revision = 4;
        projection
            .apply(ProjectionTransition::Disconnect)
            .expect("disconnect");
        projection
            .apply(ProjectionTransition::Reconnect)
            .expect("reconnect");

        let snapshot = ProjectionSnapshot {
            revision: 3,
            persistence_revision: 0,
            active_thread_id: projection.active_thread_id.clone(),
            threads: projection.threads.clone(),
            persisted_selection: None,
        };
        assert_eq!(
            projection
                .apply(ProjectionTransition::ReceiveProjectionSnapshot { snapshot })
                .expect("ignore old snapshot"),
            TransitionEffect::OlderRevisionIgnored
        );
        assert_eq!(projection.projection_revision, 4);
        assert_eq!(projection.connection, ConnectionPhase::StaleProjection);
    }

    #[test]
    fn failed_transition_is_atomic() {
        let mut projection = WorkbenchProjection::new();
        open_thread(&mut projection, "thread-a", "worktree-a");
        let before = projection.clone();
        projection
            .apply(ProjectionTransition::DispatchSurfaceCommand {
                thread_id: "thread-a".into(),
                surface: WorkSurface::Git,
                binding: Some(binding("worktree-a")),
                generation: 0,
            })
            .expect_err("unfocused surface command must fail");
        assert_eq!(projection, before);
    }
}
