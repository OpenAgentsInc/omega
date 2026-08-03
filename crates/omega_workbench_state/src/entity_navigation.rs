use serde::{Deserialize, Serialize};
use thiserror::Error;

pub const ENTITY_NAVIGATION_SCHEMA_V1: &str = "openagents.omega.entity-navigation.v1";
pub const MAX_ROUTE_HISTORY_ENTRIES: usize = 128;
const MAX_ENTITY_REF_BYTES: usize = 256;

#[derive(Clone, Debug, Error, Eq, PartialEq)]
pub enum EntityNavigationError {
    #[error("entity reference is empty")]
    EmptyReference,
    #[error("entity reference exceeds {MAX_ENTITY_REF_BYTES} bytes")]
    ReferenceTooLong,
    #[error("entity reference contains unsupported characters")]
    InvalidReference,
    #[error("unsupported entity navigation schema {0:?}")]
    UnsupportedSchema(String),
    #[error("route history index {index} is outside {entry_count} entries")]
    InvalidHistoryIndex { index: usize, entry_count: usize },
    #[error("route history exceeds {MAX_ROUTE_HISTORY_ENTRIES} entries")]
    HistoryTooLong,
}

#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct EntityRef(String);

impl EntityRef {
    pub fn new(value: impl Into<String>) -> Result<Self, EntityNavigationError> {
        Self::try_from(value.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl TryFrom<String> for EntityRef {
    type Error = EntityNavigationError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        if value.is_empty() {
            return Err(EntityNavigationError::EmptyReference);
        }
        if value.len() > MAX_ENTITY_REF_BYTES {
            return Err(EntityNavigationError::ReferenceTooLong);
        }
        let mut characters = value.chars();
        if !characters
            .next()
            .is_some_and(|character| character.is_ascii_alphanumeric())
            || !characters.all(|character| {
                character.is_ascii_alphanumeric()
                    || matches!(character, '.' | '_' | ':' | '/' | '-')
            })
        {
            return Err(EntityNavigationError::InvalidReference);
        }
        Ok(Self(value))
    }
}

impl From<EntityRef> for String {
    fn from(value: EntityRef) -> Self {
        value.0
    }
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EntityRouteKind {
    Thread,
    Work,
    IssueProjection,
    Project,
    Document,
    Decision,
    AgentSession,
    Settings,
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EntityRouteIcon {
    Thread,
    Work,
    Issue,
    Project,
    Document,
    Decision,
    AgentSession,
    Settings,
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EntityRouteFocus {
    ThreadTranscript,
    WorkBlock,
    IssueProjection,
    ProjectOverview,
    DocumentBody,
    DecisionRecord,
    AgentSessionTranscript,
    Settings,
}

#[derive(Clone, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DomainBlockRoute {
    pub block_ref: EntityRef,
    pub domain: EntityRef,
}

impl DomainBlockRoute {
    pub fn new(
        block_ref: impl Into<String>,
        domain: impl Into<String>,
    ) -> Result<Self, EntityNavigationError> {
        Ok(Self {
            block_ref: EntityRef::new(block_ref)?,
            domain: EntityRef::new(domain)?,
        })
    }
}

#[derive(Clone, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkRoute {
    pub work_ref: EntityRef,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub block: Option<DomainBlockRoute>,
}

impl WorkRoute {
    pub fn new(
        work_ref: impl Into<String>,
        block: Option<DomainBlockRoute>,
    ) -> Result<Self, EntityNavigationError> {
        Ok(Self {
            work_ref: EntityRef::new(work_ref)?,
            block,
        })
    }
}

#[derive(Clone, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "target", rename_all = "snake_case")]
pub enum EntityRoute {
    Thread(EntityRef),
    Work(WorkRoute),
    IssueProjection { work_ref: EntityRef },
    Project(EntityRef),
    Document(EntityRef),
    Decision(EntityRef),
    AgentSession(EntityRef),
    Settings,
}

impl EntityRoute {
    pub fn thread(thread_ref: impl Into<String>) -> Result<Self, EntityNavigationError> {
        Ok(Self::Thread(EntityRef::new(thread_ref)?))
    }

    pub fn work(
        work_ref: impl Into<String>,
        block: Option<DomainBlockRoute>,
    ) -> Result<Self, EntityNavigationError> {
        Ok(Self::Work(WorkRoute::new(work_ref, block)?))
    }

    pub fn issue_projection(work_ref: impl Into<String>) -> Result<Self, EntityNavigationError> {
        Ok(Self::IssueProjection {
            work_ref: EntityRef::new(work_ref)?,
        })
    }

    pub fn project(project_ref: impl Into<String>) -> Result<Self, EntityNavigationError> {
        Ok(Self::Project(EntityRef::new(project_ref)?))
    }

    pub fn document(document_ref: impl Into<String>) -> Result<Self, EntityNavigationError> {
        Ok(Self::Document(EntityRef::new(document_ref)?))
    }

    pub fn decision(decision_ref: impl Into<String>) -> Result<Self, EntityNavigationError> {
        Ok(Self::Decision(EntityRef::new(decision_ref)?))
    }

    pub fn agent_session(
        agent_session_ref: impl Into<String>,
    ) -> Result<Self, EntityNavigationError> {
        Ok(Self::AgentSession(EntityRef::new(agent_session_ref)?))
    }

    pub const fn kind(&self) -> EntityRouteKind {
        match self {
            Self::Thread(_) => EntityRouteKind::Thread,
            Self::Work(_) => EntityRouteKind::Work,
            Self::IssueProjection { .. } => EntityRouteKind::IssueProjection,
            Self::Project(_) => EntityRouteKind::Project,
            Self::Document(_) => EntityRouteKind::Document,
            Self::Decision(_) => EntityRouteKind::Decision,
            Self::AgentSession(_) => EntityRouteKind::AgentSession,
            Self::Settings => EntityRouteKind::Settings,
        }
    }

    pub const fn icon(&self) -> EntityRouteIcon {
        match self {
            Self::Thread(_) => EntityRouteIcon::Thread,
            Self::Work(_) => EntityRouteIcon::Work,
            Self::IssueProjection { .. } => EntityRouteIcon::Issue,
            Self::Project(_) => EntityRouteIcon::Project,
            Self::Document(_) => EntityRouteIcon::Document,
            Self::Decision(_) => EntityRouteIcon::Decision,
            Self::AgentSession(_) => EntityRouteIcon::AgentSession,
            Self::Settings => EntityRouteIcon::Settings,
        }
    }

    pub const fn focus(&self) -> EntityRouteFocus {
        match self {
            Self::Thread(_) => EntityRouteFocus::ThreadTranscript,
            Self::Work(_) => EntityRouteFocus::WorkBlock,
            Self::IssueProjection { .. } => EntityRouteFocus::IssueProjection,
            Self::Project(_) => EntityRouteFocus::ProjectOverview,
            Self::Document(_) => EntityRouteFocus::DocumentBody,
            Self::Decision(_) => EntityRouteFocus::DecisionRecord,
            Self::AgentSession(_) => EntityRouteFocus::AgentSessionTranscript,
            Self::Settings => EntityRouteFocus::Settings,
        }
    }

    pub const fn default_title(&self) -> &'static str {
        match self {
            Self::Thread(_) => "Thread",
            Self::Work(_) => "Work",
            Self::IssueProjection { .. } => "Issue",
            Self::Project(_) => "Project",
            Self::Document(_) => "Document",
            Self::Decision(_) => "Decision",
            Self::AgentSession(_) => "Agent session",
            Self::Settings => "Settings",
        }
    }

    pub fn stable_key(&self) -> String {
        match self {
            Self::Thread(reference) => format!("thread:{}", reference.as_str()),
            Self::Work(route) => match &route.block {
                Some(block) => format!(
                    "work:{}|block:{}|domain:{}",
                    route.work_ref.as_str(),
                    block.block_ref.as_str(),
                    block.domain.as_str()
                ),
                None => format!("work:{}", route.work_ref.as_str()),
            },
            Self::IssueProjection { work_ref } => {
                format!("issue_projection:{}", work_ref.as_str())
            }
            Self::Project(reference) => format!("project:{}", reference.as_str()),
            Self::Document(reference) => format!("document:{}", reference.as_str()),
            Self::Decision(reference) => format!("decision:{}", reference.as_str()),
            Self::AgentSession(reference) => {
                format!("agent_session:{}", reference.as_str())
            }
            Self::Settings => "settings".to_string(),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RouteUnavailableReason {
    Unknown,
    Stale,
    Deleted,
    Unauthorized,
    NotImplemented,
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
#[serde(tag = "state", content = "reason", rename_all = "snake_case")]
pub enum RouteAvailability {
    Available,
    Unavailable(RouteUnavailableReason),
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EntityRouteState {
    pub route: EntityRoute,
    pub availability: RouteAvailability,
}

impl EntityRouteState {
    pub fn focus(&self) -> Option<EntityRouteFocus> {
        matches!(self.availability, RouteAvailability::Available).then_some(self.route.focus())
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PersistedEntityNavigation {
    pub schema: String,
    pub entries: Vec<EntityRoute>,
    pub index: usize,
}

impl PersistedEntityNavigation {
    pub fn validate(&self) -> Result<(), EntityNavigationError> {
        if self.schema != ENTITY_NAVIGATION_SCHEMA_V1 {
            return Err(EntityNavigationError::UnsupportedSchema(
                self.schema.clone(),
            ));
        }
        if self.entries.len() > MAX_ROUTE_HISTORY_ENTRIES {
            return Err(EntityNavigationError::HistoryTooLong);
        }
        if !self.entries.is_empty() && self.index >= self.entries.len() {
            return Err(EntityNavigationError::InvalidHistoryIndex {
                index: self.index,
                entry_count: self.entries.len(),
            });
        }
        if self.entries.is_empty() && self.index != 0 {
            return Err(EntityNavigationError::InvalidHistoryIndex {
                index: self.index,
                entry_count: 0,
            });
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct EntityNavigationHistory {
    entries: Vec<EntityRoute>,
    index: usize,
}

impl EntityNavigationHistory {
    pub fn from_persisted(
        persisted: PersistedEntityNavigation,
    ) -> Result<Self, EntityNavigationError> {
        persisted.validate()?;
        Ok(Self {
            entries: persisted.entries,
            index: persisted.index,
        })
    }

    pub fn migrate_legacy_thread(thread_ref: Option<&str>) -> Result<Self, EntityNavigationError> {
        let Some(thread_ref) = thread_ref else {
            return Ok(Self::default());
        };
        Ok(Self {
            entries: vec![EntityRoute::thread(thread_ref)?],
            index: 0,
        })
    }

    pub fn persisted(&self) -> PersistedEntityNavigation {
        PersistedEntityNavigation {
            schema: ENTITY_NAVIGATION_SCHEMA_V1.to_string(),
            entries: self.entries.clone(),
            index: self.index,
        }
    }

    pub fn current(&self) -> Option<&EntityRoute> {
        self.entries.get(self.index)
    }

    pub fn push(&mut self, route: EntityRoute) -> bool {
        if self.current() == Some(&route) {
            return false;
        }
        if self.entries.is_empty() {
            self.entries.push(route);
            self.index = 0;
            return true;
        }
        self.entries.truncate(self.index + 1);
        self.entries.push(route);
        if self.entries.len() > MAX_ROUTE_HISTORY_ENTRIES {
            let remove_count = self.entries.len() - MAX_ROUTE_HISTORY_ENTRIES;
            self.entries.drain(..remove_count);
        }
        self.index = self.entries.len().saturating_sub(1);
        true
    }

    pub fn can_back(&self) -> bool {
        self.index > 0
    }

    pub fn can_forward(&self) -> bool {
        self.index.saturating_add(1) < self.entries.len()
    }

    pub fn back(&mut self) -> Option<EntityRoute> {
        if !self.can_back() {
            return None;
        }
        self.index -= 1;
        self.current().cloned()
    }

    pub fn forward(&mut self) -> Option<EntityRoute> {
        if !self.can_forward() {
            return None;
        }
        self.index += 1;
        self.current().cloned()
    }

    pub fn restore_index(&mut self, index: usize) {
        self.index = index.min(self.entries.len().saturating_sub(1));
    }

    pub fn index(&self) -> usize {
        self.index
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn reference(value: &str) -> EntityRef {
        EntityRef::new(value).expect("valid test reference")
    }

    #[test]
    fn every_entity_route_has_stable_identity_icon_and_focus() {
        let block =
            DomainBlockRoute::new("block:forensics", "forensics").expect("valid domain block");
        let routes = [
            EntityRoute::thread("thread:1").expect("thread route"),
            EntityRoute::work("work:1", Some(block)).expect("work route"),
            EntityRoute::issue_projection("work:issue:1").expect("issue route"),
            EntityRoute::project("project:1").expect("project route"),
            EntityRoute::document("document:1").expect("document route"),
            EntityRoute::decision("decision:1").expect("decision route"),
            EntityRoute::agent_session("agent-session:1").expect("agent session route"),
            EntityRoute::Settings,
        ];

        let stable_keys = routes
            .iter()
            .map(EntityRoute::stable_key)
            .collect::<std::collections::BTreeSet<_>>();
        let icons = routes
            .iter()
            .map(EntityRoute::icon)
            .collect::<std::collections::HashSet<_>>();
        assert_eq!(stable_keys.len(), routes.len());
        assert_eq!(icons.len(), routes.len());
        for route in routes {
            assert!(!route.default_title().is_empty());
            assert_eq!(
                EntityRouteState {
                    route: route.clone(),
                    availability: RouteAvailability::Available,
                }
                .focus(),
                Some(route.focus())
            );
        }
    }

    #[test]
    fn a_domain_block_remains_attached_to_its_owning_work_route() {
        let route = EntityRoute::work(
            "work:1",
            Some(DomainBlockRoute::new("block:1", "forensics").expect("block")),
        )
        .expect("work route");
        let EntityRoute::Work(work) = route else {
            panic!("expected work route");
        };
        assert_eq!(work.work_ref, reference("work:1"));
        assert_eq!(
            work.block.as_ref().map(|block| &block.block_ref),
            Some(&reference("block:1"))
        );
    }

    #[test]
    fn route_history_walks_branches_persists_and_bounds_growth() {
        let mut history = EntityNavigationHistory::default();
        history.push(EntityRoute::thread("thread:1").expect("thread"));
        history.push(EntityRoute::Settings);
        history.push(EntityRoute::work("work:1", None).expect("work"));
        assert_eq!(history.back(), Some(EntityRoute::Settings));
        history.push(EntityRoute::document("document:1").expect("document"));
        assert!(!history.can_forward());

        for index in 0..(MAX_ROUTE_HISTORY_ENTRIES + 20) {
            history.push(
                EntityRoute::document(format!("document:{index}")).expect("bounded document route"),
            );
        }
        let persisted = history.persisted();
        assert_eq!(persisted.entries.len(), MAX_ROUTE_HISTORY_ENTRIES);
        assert_eq!(
            EntityNavigationHistory::from_persisted(persisted.clone())
                .expect("persisted history")
                .persisted(),
            persisted
        );
    }

    #[test]
    fn persistence_rejects_unknown_schema_bad_index_and_bad_refs() {
        let unknown = PersistedEntityNavigation {
            schema: "openagents.omega.entity-navigation.v99".to_string(),
            entries: Vec::new(),
            index: 0,
        };
        assert!(matches!(
            unknown.validate(),
            Err(EntityNavigationError::UnsupportedSchema(_))
        ));
        let bad_index = PersistedEntityNavigation {
            schema: ENTITY_NAVIGATION_SCHEMA_V1.to_string(),
            entries: vec![EntityRoute::Settings],
            index: 1,
        };
        assert!(matches!(
            bad_index.validate(),
            Err(EntityNavigationError::InvalidHistoryIndex { .. })
        ));
        assert!(EntityRef::new("bad reference").is_err());
    }

    #[test]
    fn legacy_thread_migration_preserves_the_restorable_thread() {
        let migrated = EntityNavigationHistory::migrate_legacy_thread(Some("thread:legacy"))
            .expect("legacy migration");
        assert_eq!(
            migrated.current(),
            Some(&EntityRoute::thread("thread:legacy").expect("thread route"))
        );
    }
}
