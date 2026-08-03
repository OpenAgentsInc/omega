//! Atomic canonical-planning refresh into the dogfood screen view model.
//!
//! The fixture and live boundary both decode into [`DogfoodPlanningViewModel`].
//! A partial or failed revision can update freshness/loss metadata, but it can
//! never replace the last complete graph or grant a command capability.

use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    io::Write as _,
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};

use omega_effectd::all_work_contract::{
    CompletenessState, ContractValidate, FreshnessState, Nullable, PlanningGraph, PlanningResource,
    PlanningResourceKind, WorkPriority, WorkRelationKind, WorkState,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};
use thiserror::Error;

use crate::{
    DOGFOOD_FIXTURE_SCHEMA_V1, DogfoodFixtureProjection, DogfoodPlanningGraph,
    FixtureAgentActivity, FixtureAgentSession, FixtureCustomView, FixtureDiff, FixtureDocument,
    FixtureExecutionRefs, FixtureInitiative, FixtureIssue, FixtureIssueRelation,
    FixtureIssueRelationKind, FixtureLifecycleType, FixtureOpenAgentsEnvelope, FixturePriority,
    FixtureProject, FixtureProjectMilestone, FixtureProjectUpdate, FixtureProjectionIssue,
    FixturePullRequest, FixtureReleasePipeline, FixtureReleasePlanningRecord, FixtureReleaseStage,
    FixtureRepository, FixtureTeam, FixtureUserProjection, NamedFixtureRecord,
};

pub const DOGFOOD_PLANNING_VIEW_SCHEMA_V1: &str = "openagents.omega.dogfood-planning-view.v1";
pub const DOGFOOD_PLANNING_ADAPTER_VERSION: &str = "openagents.planning-graph.v1";
const STORE_DIR: &str = "dogfood-planning-v1";
const STORE_FILE: &str = "last-known-good.json";
static NEXT_TEMP_FILE: AtomicU64 = AtomicU64::new(1);

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DogfoodPlanningOrigin {
    Fixture,
    Live,
    OfflineCache,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DogfoodPlanningSourceState {
    Fixture,
    Fresh,
    Stale,
    Partial,
    Gap,
    Offline,
    Error,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DogfoodPlanningTextRecord {
    pub record_ref: String,
    pub kind: String,
    pub work_ref: Option<String>,
    pub resource_ref: Option<String>,
    pub body: String,
    pub author_ref: String,
    pub created_at: String,
}

impl DogfoodPlanningSourceState {
    pub const fn label(self) -> &'static str {
        match self {
            Self::Fixture => "Development mock data",
            Self::Fresh => "Live planning data",
            Self::Stale => "Live data is stale",
            Self::Partial => "Partial refresh; showing last known good",
            Self::Gap => "Cursor gap; showing last known good",
            Self::Offline => "Offline; showing last known good",
            Self::Error => "Refresh failed; showing last known good",
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DogfoodPlanningViewModel {
    pub schema: String,
    pub fixture_sha256: String,
    pub source_snapshot_at: String,
    pub graph: DogfoodPlanningGraph,
    pub origin: DogfoodPlanningOrigin,
    pub source_state: DogfoodPlanningSourceState,
    pub revision: u64,
    pub event_cursor: String,
    pub adapter_generation: u64,
    pub adapter_version: String,
    pub projection_version: String,
    pub text_records: Vec<DogfoodPlanningTextRecord>,
    pub refresh_gap_refs: Vec<String>,
    /// Loss facts from the latest attempted refresh. These can advance without
    /// replacing `graph`, which remains the last complete projection.
    pub refresh_projection_issues: Vec<FixtureProjectionIssue>,
    pub last_error: Option<String>,
}

impl DogfoodPlanningViewModel {
    pub fn from_fixture(fixture: DogfoodFixtureProjection) -> Self {
        Self {
            schema: DOGFOOD_PLANNING_VIEW_SCHEMA_V1.into(),
            fixture_sha256: fixture.fixture_sha256,
            source_snapshot_at: fixture.source_snapshot_at,
            graph: fixture.graph,
            origin: DogfoodPlanningOrigin::Fixture,
            source_state: DogfoodPlanningSourceState::Fixture,
            revision: 0,
            event_cursor: "cursor:fixture:0".into(),
            adapter_generation: 0,
            adapter_version: DOGFOOD_FIXTURE_SCHEMA_V1.into(),
            projection_version: DOGFOOD_FIXTURE_SCHEMA_V1.into(),
            text_records: Vec::new(),
            refresh_gap_refs: Vec::new(),
            refresh_projection_issues: Vec::new(),
            last_error: None,
        }
    }

    pub fn from_live(
        graph: PlanningGraph,
        adapter_generation: u64,
    ) -> Result<Self, DogfoodPlanningRefreshError> {
        graph.validate()?;
        if graph.completeness.state != CompletenessState::Complete {
            return Err(DogfoodPlanningRefreshError::IncompleteRevision {
                state: format!("{:?}", graph.completeness.state),
            });
        }
        let projection = project_live_graph(&graph)?;
        let refresh_projection_issues = projection.graph_projection_issues().to_vec();
        let mut text_records = graph
            .text_records
            .iter()
            .map(|record| DogfoodPlanningTextRecord {
                record_ref: record.record_ref.0.clone(),
                kind: format!("{:?}", record.kind),
                work_ref: record
                    .work_ref
                    .as_ref()
                    .and_then(Option::as_ref)
                    .map(|value| value.0.clone()),
                resource_ref: record
                    .resource_ref
                    .as_ref()
                    .and_then(Option::as_ref)
                    .map(|value| value.0.clone()),
                body: record.body.0.clone(),
                author_ref: record.author_ref.0.clone(),
                created_at: record.created_at.0.clone(),
            })
            .collect::<Vec<_>>();
        text_records.sort_by(|left, right| left.record_ref.cmp(&right.record_ref));
        Ok(Self {
            schema: DOGFOOD_PLANNING_VIEW_SCHEMA_V1.into(),
            fixture_sha256: graph.reconciliation_digest.0.clone(),
            source_snapshot_at: graph.generated_at.0.clone(),
            graph: projection,
            origin: DogfoodPlanningOrigin::Live,
            source_state: source_state(&graph),
            revision: graph.revision.0,
            projection_version: graph.contract_version_label(),
            event_cursor: graph.event_cursor.0,
            adapter_generation,
            adapter_version: DOGFOOD_PLANNING_ADAPTER_VERSION.into(),
            text_records,
            refresh_gap_refs: graph
                .completeness
                .gap_refs
                .iter()
                .map(|value| value.0.clone())
                .collect(),
            refresh_projection_issues,
            last_error: None,
        })
    }

    pub fn retain_after_failure(
        &self,
        state: DogfoodPlanningSourceState,
        error: impl Into<String>,
    ) -> Self {
        self.retain_after_incomplete(state, Vec::new(), error)
    }

    pub fn retain_after_incomplete(
        &self,
        state: DogfoodPlanningSourceState,
        projection_issues: Vec<FixtureProjectionIssue>,
        error: impl Into<String>,
    ) -> Self {
        let mut retained = self.clone();
        retained.source_state = state;
        retained.refresh_projection_issues = projection_issues;
        retained.last_error = Some(error.into());
        if retained.origin == DogfoodPlanningOrigin::Live {
            retained.origin = DogfoodPlanningOrigin::OfflineCache;
        }
        retained
    }

    pub fn retain_rejected_graph(&self, graph: &PlanningGraph, error: impl Into<String>) -> Self {
        let state = if graph.completeness.state == CompletenessState::Complete {
            DogfoodPlanningSourceState::Error
        } else {
            source_state(graph)
        };
        let issues = projection_issues(graph);
        let mut retained = self.retain_after_incomplete(state, issues, error);
        retained.refresh_gap_refs = graph
            .completeness
            .gap_refs
            .iter()
            .map(|value| value.0.clone())
            .collect();
        retained
    }

    pub fn restored_offline(mut self) -> Self {
        self.origin = DogfoodPlanningOrigin::OfflineCache;
        self.source_state = DogfoodPlanningSourceState::Offline;
        self.last_error =
            Some("The canonical planning service has not refreshed this cache.".into());
        self
    }

    pub fn stage_live(
        &self,
        graph: PlanningGraph,
        adapter_generation: u64,
    ) -> Result<Self, DogfoodPlanningRefreshError> {
        if graph.completeness.state != CompletenessState::Complete {
            return Err(DogfoodPlanningRefreshError::IncompleteRevision {
                state: format!("{:?}", graph.completeness.state),
            });
        }
        if self.origin != DogfoodPlanningOrigin::Fixture && graph.revision.0 < self.revision {
            return Err(DogfoodPlanningRefreshError::RevisionRegression {
                previous: self.revision,
                incoming: graph.revision.0,
            });
        }
        if self.origin != DogfoodPlanningOrigin::Fixture
            && adapter_generation < self.adapter_generation
        {
            return Err(DogfoodPlanningRefreshError::AdapterGenerationRegression {
                previous: self.adapter_generation,
                incoming: adapter_generation,
            });
        }
        if self.origin != DogfoodPlanningOrigin::Fixture
            && graph.revision.0 == self.revision
            && graph.event_cursor.0 != self.event_cursor
        {
            return Err(DogfoodPlanningRefreshError::CursorChangedWithoutRevision);
        }
        Self::from_live(graph, adapter_generation)
    }

    pub fn provenance_label(&self) -> &'static str {
        self.source_state.label()
    }

    pub fn is_fresh_live(&self) -> bool {
        self.origin == DogfoodPlanningOrigin::Live
            && self.source_state == DogfoodPlanningSourceState::Fresh
    }
}

trait DogfoodPlanningGraphExt {
    fn graph_projection_issues(&self) -> &[FixtureProjectionIssue];
}

impl DogfoodPlanningGraphExt for DogfoodPlanningGraph {
    fn graph_projection_issues(&self) -> &[FixtureProjectionIssue] {
        &self.projection_issues
    }
}

trait ContractVersionLabel {
    fn contract_version_label(&self) -> String;
}

impl ContractVersionLabel for PlanningGraph {
    fn contract_version_label(&self) -> String {
        serde_json::to_value(&self.contract_version)
            .ok()
            .and_then(|value| value.as_str().map(str::to_owned))
            .unwrap_or_else(|| "openagents.all_work_boundary.v1".into())
    }
}

#[derive(Clone, Debug, Error, Eq, PartialEq)]
pub enum DogfoodPlanningRefreshError {
    #[error("invalid generated PlanningGraph: {0}")]
    InvalidContract(String),
    #[error("planning graph revision is incomplete ({state})")]
    IncompleteRevision { state: String },
    #[error("planning graph revision regressed from {previous} to {incoming}")]
    RevisionRegression { previous: u64, incoming: u64 },
    #[error("planning graph cursor changed without a revision advance")]
    CursorChangedWithoutRevision,
    #[error("planning adapter generation regressed from {previous} to {incoming}")]
    AdapterGenerationRegression { previous: u64, incoming: u64 },
    #[error("planning graph is missing {0}")]
    MissingRequired(String),
    #[error("planning graph contains duplicate identity {0}")]
    DuplicateIdentity(String),
    #[error("planning graph contains unsupported relation {0}")]
    UnsupportedRelation(String),
    #[error("planning graph persistence failed: {0}")]
    Persistence(String),
}

impl From<omega_effectd::all_work_contract::ContractValidationError>
    for DogfoodPlanningRefreshError
{
    fn from(error: omega_effectd::all_work_contract::ContractValidationError) -> Self {
        Self::InvalidContract(error.to_string())
    }
}

fn source_state(graph: &PlanningGraph) -> DogfoodPlanningSourceState {
    match (&graph.completeness.state, &graph.freshness.state) {
        (CompletenessState::Gap | CompletenessState::Truncated, _) => {
            DogfoodPlanningSourceState::Gap
        }
        (CompletenessState::Partial | CompletenessState::Unknown, _) => {
            DogfoodPlanningSourceState::Partial
        }
        (_, FreshnessState::OfflineCached) => DogfoodPlanningSourceState::Offline,
        (_, FreshnessState::Stale | FreshnessState::Unknown) => DogfoodPlanningSourceState::Stale,
        _ => DogfoodPlanningSourceState::Fresh,
    }
}

fn parent(resource: &PlanningResource) -> Option<&str> {
    resource
        .parent_ref
        .as_ref()
        .and_then(Option::as_ref)
        .map(|reference| reference.0.as_str())
}

fn state(resource: &PlanningResource) -> Option<&str> {
    resource.state.as_ref().map(|value| value.0.as_str())
}

fn resources<'a>(
    graph: &'a PlanningGraph,
    kind: PlanningResourceKind,
) -> impl Iterator<Item = &'a PlanningResource> {
    graph
        .resources
        .iter()
        .filter(move |resource| resource.kind == kind)
}

fn lifecycle(value: Option<&str>) -> FixtureLifecycleType {
    match value {
        Some("backlog") => FixtureLifecycleType::Backlog,
        Some("unstarted") => FixtureLifecycleType::Unstarted,
        Some("completed") => FixtureLifecycleType::Completed,
        Some("canceled") => FixtureLifecycleType::Canceled,
        Some("planned") => FixtureLifecycleType::Planned,
        _ => FixtureLifecycleType::Started,
    }
}

fn nullable_string<T>(value: &Nullable<T>, extract: impl FnOnce(&T) -> &str) -> Option<String> {
    value.0.as_ref().map(|value| extract(value).to_string())
}

fn project_live_graph(
    graph: &PlanningGraph,
) -> Result<DogfoodPlanningGraph, DogfoodPlanningRefreshError> {
    ensure_unique(
        graph
            .resources
            .iter()
            .map(|value| value.resource_ref.0.as_str()),
        "planning resource",
    )?;
    ensure_unique(
        graph
            .work
            .iter()
            .map(|value| value.summary.work_ref.0.as_str()),
        "Work",
    )?;
    ensure_unique(
        graph
            .planning_links
            .iter()
            .map(|value| value.work_ref.0.as_str()),
        "planning link",
    )?;
    ensure_unique(
        graph
            .source_coordinates
            .iter()
            .map(|value| value.work_ref.0.as_str()),
        "source coordinate",
    )?;
    ensure_unique(
        graph
            .text_records
            .iter()
            .map(|value| value.record_ref.0.as_str()),
        "planning text record",
    )?;
    let organization = resources(graph, PlanningResourceKind::Organization)
        .next()
        .ok_or_else(|| DogfoodPlanningRefreshError::MissingRequired("Organization".into()))?;
    let teams = resources(graph, PlanningResourceKind::Team)
        .map(|resource| FixtureTeam {
            id: resource.resource_ref.0.clone(),
            name: resource.name.0.clone(),
            key: state(resource).unwrap_or("OMEGA").to_string(),
            organization_id: parent(resource)
                .unwrap_or(&organization.resource_ref.0)
                .to_string(),
        })
        .collect::<Vec<_>>();
    let default_team = teams
        .first()
        .map(|team| team.id.clone())
        .ok_or_else(|| DogfoodPlanningRefreshError::MissingRequired("Team".into()))?;
    let projects = resources(graph, PlanningResourceKind::Project)
        .map(|resource| FixtureProject {
            id: resource.resource_ref.0.clone(),
            name: resource.name.0.clone(),
            team_id: default_team.clone(),
            initiative_id: parent(resource).unwrap_or("initiative:unknown").to_string(),
            status_id: state(resource)
                .unwrap_or("project-status:started")
                .to_string(),
        })
        .collect::<Vec<_>>();
    let initiatives = resources(graph, PlanningResourceKind::Initiative)
        .map(|resource| FixtureInitiative {
            id: resource.resource_ref.0.clone(),
            name: resource.name.0.clone(),
            organization_id: parent(resource)
                .unwrap_or(&organization.resource_ref.0)
                .to_string(),
            project_ids: projects
                .iter()
                .filter(|project| project.initiative_id == resource.resource_ref.0)
                .map(|project| project.id.clone())
                .collect(),
        })
        .collect::<Vec<_>>();
    let project_statuses = resources(graph, PlanningResourceKind::ProjectStatus)
        .map(named_resource)
        .collect();
    let project_milestones = resources(graph, PlanningResourceKind::ProjectMilestone)
        .map(|resource| FixtureProjectMilestone {
            id: resource.resource_ref.0.clone(),
            name: resource.name.0.clone(),
            project_id: parent(resource).unwrap_or("project:unknown").to_string(),
        })
        .collect();
    let cycles = resources(graph, PlanningResourceKind::Cycle)
        .map(|resource| crate::FixtureCycle {
            id: resource.resource_ref.0.clone(),
            name: resource.name.0.clone(),
            team_id: default_team.clone(),
            project_id: parent(resource).unwrap_or("project:unknown").to_string(),
        })
        .collect();
    let release_pipelines = resources(graph, PlanningResourceKind::ReleasePipeline)
        .map(|resource| FixtureReleasePipeline {
            id: resource.resource_ref.0.clone(),
            name: resource.name.0.clone(),
            organization_id: parent(resource)
                .unwrap_or(&organization.resource_ref.0)
                .to_string(),
        })
        .collect::<Vec<_>>();
    let mut release_stages = resources(graph, PlanningResourceKind::ReleaseStage)
        .enumerate()
        .map(|(position, resource)| FixtureReleaseStage {
            id: resource.resource_ref.0.clone(),
            name: resource.name.0.clone(),
            pipeline_id: parent(resource)
                .unwrap_or("release-pipeline:unknown")
                .to_string(),
            lifecycle_type: lifecycle(state(resource)),
            position: u8::try_from(position).unwrap_or(u8::MAX),
        })
        .collect::<Vec<_>>();
    release_stages.sort_by_key(|stage| stage.position);
    let work_to_issue = graph
        .work
        .iter()
        .map(|work| {
            let identity = graph
                .source_coordinates
                .iter()
                .find(|coordinate| coordinate.work_ref == work.summary.work_ref)
                .and_then(|coordinate| issue_identity(coordinate).ok())
                .map(|identity| identity.0)
                .unwrap_or_else(|| native_issue_identity(&work.summary.work_ref.0).0);
            (work.summary.work_ref.0.clone(), identity)
        })
        .collect::<BTreeMap<_, _>>();
    let releases = resources(graph, PlanningResourceKind::ReleasePlanningRecord)
        .map(|resource| FixtureReleasePlanningRecord {
            id: resource.resource_ref.0.clone(),
            name: resource.name.0.clone(),
            pipeline_id: release_stages
                .iter()
                .find(|stage| Some(stage.id.as_str()) == state(resource))
                .map_or("release-pipeline:unknown", |stage| {
                    stage.pipeline_id.as_str()
                })
                .to_string(),
            stage_id: state(resource)
                .unwrap_or("release-stage:unknown")
                .to_string(),
            project_id: parent(resource).unwrap_or("project:unknown").to_string(),
            target_version: resource
                .name
                .0
                .split_whitespace()
                .last()
                .unwrap_or("Not available")
                .to_string(),
            target_date: None,
            target_commit: None,
            scope_issue_ids: graph
                .release_scope_links
                .iter()
                .filter(|link| link.release_planning_record_ref == resource.resource_ref)
                .filter_map(|link| work_to_issue.get(&link.work_ref.0).cloned())
                .collect(),
            canonical_release_authority: false,
        })
        .collect();
    let workflow_states = resources(graph, PlanningResourceKind::WorkflowState)
        .map(|resource| crate::FixtureWorkflowState {
            id: resource.resource_ref.0.clone(),
            name: resource.name.0.clone(),
            lifecycle_type: lifecycle(state(resource)),
        })
        .collect();
    let labels = resources(graph, PlanningResourceKind::Label)
        .map(named_resource)
        .collect();
    let planning = graph
        .planning_links
        .iter()
        .map(|link| (link.work_ref.0.as_str(), link))
        .collect::<BTreeMap<_, _>>();
    let coordinates = graph
        .source_coordinates
        .iter()
        .map(|coordinate| (coordinate.work_ref.0.as_str(), coordinate))
        .collect::<BTreeMap<_, _>>();
    let label_links =
        graph
            .label_links
            .iter()
            .fold(BTreeMap::<&str, Vec<String>>::new(), |mut grouped, link| {
                grouped
                    .entry(&link.work_ref.0)
                    .or_default()
                    .push(link.label_ref.0.clone());
                grouped
            });
    let mut issues = Vec::with_capacity(graph.work.len());
    for work in &graph.work {
        let coordinate = coordinates.get(work.summary.work_ref.0.as_str()).copied();
        let (id, repository_id, number) = coordinate
            .map(issue_identity)
            .transpose()?
            .unwrap_or_else(|| native_issue_identity(&work.summary.work_ref.0));
        let placement = planning
            .get(work.summary.work_ref.0.as_str())
            .ok_or_else(|| {
                DogfoodPlanningRefreshError::MissingRequired(format!(
                    "planning link for {}",
                    work.summary.work_ref.0
                ))
            })?;
        issues.push(FixtureIssue {
            id,
            identifier: coordinate.map_or_else(
                || work.summary.work_ref.0.clone(),
                |coordinate| {
                    coordinate.repository.0.split('/').next_back().map_or_else(
                        || coordinate.identifier.0.clone(),
                        |name| format!("{name}#{number}"),
                    )
                },
            ),
            title: work.summary.title.0.clone(),
            repository_id,
            number,
            source_url: coordinate.map_or_else(
                || "Not available from this source".into(),
                |coordinate| coordinate.url.0.clone(),
            ),
            project_id: nullable_string(&placement.project_ref, |value| &value.0)
                .unwrap_or_else(|| "project:unassigned".into()),
            project_milestone_id: nullable_string(&placement.project_milestone_ref, |value| {
                &value.0
            }),
            cycle_id: nullable_string(&placement.cycle_ref, |value| &value.0),
            release_planning_record_id: nullable_string(
                &placement.release_planning_record_ref,
                |value| &value.0,
            ),
            workflow_state_id: nullable_string(&placement.workflow_state_ref, |value| &value.0)
                .unwrap_or_else(|| "workflow:backlog".into()),
            priority: fixture_priority(&work.summary.priority),
            label_ids: label_links
                .get(work.summary.work_ref.0.as_str())
                .cloned()
                .unwrap_or_default(),
            completed: work.summary.state == WorkState::Completed,
            work_revision: Some(work.summary.revision.0),
            assignee_user_id: None,
            delegate_user_id: None,
            execution_refs: FixtureExecutionRefs {
                thread_ref: work.thread_refs.first().map(|value| value.0.clone()),
                session_ref: work.session_refs.first().map(|value| value.0.clone()),
                agent_session_ref: work.agent_session_refs.first().map(|value| value.0.clone()),
                run_ref: work.run_refs.first().map(|value| value.0.clone()),
                repository_work_claim_ref: None,
                lease_ref: None,
            },
            receipt_refs: work
                .receipt_refs
                .iter()
                .map(|value| value.0.clone())
                .collect(),
            evidence_refs: work
                .evidence_refs
                .iter()
                .map(|value| value.0.clone())
                .collect(),
            verification_refs: work
                .verification_refs
                .iter()
                .map(|value| value.0.clone())
                .collect(),
            owner_disposition_refs: work
                .owner_disposition_refs
                .iter()
                .map(|value| value.0.clone())
                .collect(),
        });
    }
    let known_issue_ids = issues
        .iter()
        .map(|issue| issue.id.clone())
        .collect::<BTreeSet<_>>();
    let mut issue_relations = Vec::new();
    for work in &graph.work {
        let Some(issue_id) = work_to_issue.get(&work.summary.work_ref.0) else {
            continue;
        };
        for relation in &work.relations {
            if relation.kind != WorkRelationKind::Blocks {
                return Err(DogfoodPlanningRefreshError::UnsupportedRelation(format!(
                    "{:?}",
                    relation.kind
                )));
            }
            let related_issue_id = work_to_issue
                .get(&relation.target_work_ref.0)
                .ok_or_else(|| {
                    DogfoodPlanningRefreshError::MissingRequired(format!(
                        "relation target {}",
                        relation.target_work_ref.0
                    ))
                })?
                .clone();
            if known_issue_ids.contains(&related_issue_id) {
                issue_relations.push(FixtureIssueRelation {
                    id: format!("relation:{}-blocks-{}", issue_id, related_issue_id),
                    issue_id: issue_id.clone(),
                    related_issue_id,
                    kind: FixtureIssueRelationKind::Blocks,
                });
            }
        }
    }
    let source_repositories = repositories_from_coordinates(graph);
    let documents = resources(graph, PlanningResourceKind::Document)
        .map(|resource| FixtureDocument {
            id: resource.resource_ref.0.clone(),
            title: resource.name.0.clone(),
            url: resource
                .description
                .as_ref()
                .map_or("Not available from this source", |value| value.0.as_str())
                .to_string(),
            project_id: parent(resource).unwrap_or("project:unknown").to_string(),
        })
        .collect();
    let project_updates = resources(graph, PlanningResourceKind::ProjectUpdate)
        .map(|resource| FixtureProjectUpdate {
            id: resource.resource_ref.0.clone(),
            project_id: parent(resource).unwrap_or("project:unknown").to_string(),
            health: state(resource).unwrap_or("unknown").to_string(),
            summary: resource.name.0.clone(),
        })
        .collect();
    let custom_views = resources(graph, PlanningResourceKind::CustomView)
        .map(|resource| FixtureCustomView {
            id: resource.resource_ref.0.clone(),
            name: resource.name.0.clone(),
            project_id: parent(resource).unwrap_or("project:unknown").to_string(),
            filter: resource
                .description
                .as_ref()
                .map_or("Not available from this source", |value| value.0.as_str())
                .to_string(),
        })
        .collect();
    let projection_issues = projection_issues(graph);

    let mut projected = DogfoodPlanningGraph {
        source_repositories,
        organization: crate::FixtureOrganization {
            id: organization.resource_ref.0.clone(),
            name: organization.name.0.clone(),
        },
        teams,
        initiatives,
        projects,
        project_statuses,
        project_milestones,
        cycles,
        release_pipelines,
        release_stages,
        releases,
        workflow_states,
        labels,
        issues,
        issue_relations,
        documents,
        project_updates,
        custom_views,
        users: Vec::<FixtureUserProjection>::new(),
        agent_sessions: Vec::<FixtureAgentSession>::new(),
        agent_activities: Vec::<FixtureAgentActivity>::new(),
        pull_requests: Vec::<FixturePullRequest>::new(),
        diffs: Vec::<FixtureDiff>::new(),
        open_agents_extensions: FixtureOpenAgentsEnvelope {
            work_identity_scheme: "canonical PlanningGraph WorkRef".into(),
            issue_projection_identity_scheme: "same WorkRef".into(),
            source_snapshot_at: graph.generated_at.0.clone(),
            simulation: false,
            canonical_authority: true,
            allowed_commands: Vec::new(),
            authority_loss_facts: vec![
                "read_grants_no_command_authority".into(),
                "release_planning_record_is_not_release".into(),
            ],
        },
        projection_issues,
    };
    normalize_projected_graph(&mut projected);
    Ok(projected)
}

fn ensure_unique<'a>(
    values: impl Iterator<Item = &'a str>,
    kind: &str,
) -> Result<(), DogfoodPlanningRefreshError> {
    let mut seen = BTreeSet::new();
    for value in values {
        if !seen.insert(value) {
            return Err(DogfoodPlanningRefreshError::DuplicateIdentity(format!(
                "{kind} {value}"
            )));
        }
    }
    Ok(())
}

fn normalize_projected_graph(graph: &mut DogfoodPlanningGraph) {
    graph
        .source_repositories
        .sort_by(|left, right| left.id.cmp(&right.id));
    graph.teams.sort_by(|left, right| left.id.cmp(&right.id));
    graph
        .initiatives
        .sort_by(|left, right| left.id.cmp(&right.id));
    graph.projects.sort_by(|left, right| left.id.cmp(&right.id));
    graph
        .project_statuses
        .sort_by(|left, right| left.id.cmp(&right.id));
    graph
        .project_milestones
        .sort_by(|left, right| left.id.cmp(&right.id));
    graph.cycles.sort_by(|left, right| left.id.cmp(&right.id));
    graph
        .release_pipelines
        .sort_by(|left, right| left.id.cmp(&right.id));
    graph
        .release_stages
        .sort_by(|left, right| left.id.cmp(&right.id));
    graph.releases.sort_by(|left, right| left.id.cmp(&right.id));
    graph
        .workflow_states
        .sort_by(|left, right| left.id.cmp(&right.id));
    graph.labels.sort_by(|left, right| left.id.cmp(&right.id));
    for issue in &mut graph.issues {
        issue.label_ids.sort();
        issue.label_ids.dedup();
    }
    graph.issues.sort_by(|left, right| left.id.cmp(&right.id));
    graph
        .issue_relations
        .sort_by(|left, right| left.id.cmp(&right.id));
    graph
        .issue_relations
        .dedup_by(|left, right| left.id == right.id);
    graph
        .documents
        .sort_by(|left, right| left.id.cmp(&right.id));
    graph
        .project_updates
        .sort_by(|left, right| left.id.cmp(&right.id));
    graph
        .custom_views
        .sort_by(|left, right| left.id.cmp(&right.id));
    graph
        .projection_issues
        .sort_by(|left, right| left.id.cmp(&right.id));
}

fn projection_issues(graph: &PlanningGraph) -> Vec<FixtureProjectionIssue> {
    graph
        .projection_issues
        .iter()
        .map(|issue| FixtureProjectionIssue {
            id: issue.issue_ref.0.clone(),
            field: issue
                .source_ref
                .0
                .as_ref()
                .map_or("planningGraph", |value| value.0.as_str())
                .to_string(),
            reason: format!("{} ({:?})", issue.detail.0, issue.kind),
        })
        .chain(
            graph
                .source_coordinates
                .iter()
                .filter(|coordinate| !coordinate.available)
                .map(|coordinate| FixtureProjectionIssue {
                    id: format!("projection-issue:unavailable:{}", coordinate.work_ref.0),
                    field: coordinate.source_ref.0.clone(),
                    reason: "Source unavailable; showing last-known-good Work.".into(),
                }),
        )
        .collect()
}

fn named_resource(resource: &PlanningResource) -> NamedFixtureRecord {
    NamedFixtureRecord {
        id: resource.resource_ref.0.clone(),
        name: resource.name.0.clone(),
    }
}

fn issue_identity(
    coordinate: &omega_effectd::all_work_contract::SourceCoordinate,
) -> Result<(String, String, u64), DogfoodPlanningRefreshError> {
    let repository = coordinate
        .repository
        .0
        .split('/')
        .next_back()
        .ok_or_else(|| DogfoodPlanningRefreshError::MissingRequired("repository name".into()))?
        .to_ascii_lowercase();
    let number = coordinate
        .url
        .0
        .split('/')
        .next_back()
        .and_then(|value| value.parse::<u64>().ok())
        .ok_or_else(|| {
            DogfoodPlanningRefreshError::MissingRequired(format!(
                "issue number for {}",
                coordinate.work_ref.0
            ))
        })?;
    Ok((
        format!("issue:{repository}:{number}"),
        format!("repository:{repository}"),
        number,
    ))
}

fn native_issue_identity(work_ref: &str) -> (String, String, u64) {
    (
        format!("issue:native:{}", work_ref.replace(':', "-")),
        "repository:openagents-native".into(),
        0,
    )
}

fn fixture_priority(priority: &WorkPriority) -> FixturePriority {
    match priority {
        WorkPriority::None => FixturePriority::NoPriority,
        WorkPriority::Urgent => FixturePriority::Urgent,
        WorkPriority::High => FixturePriority::High,
        WorkPriority::Normal => FixturePriority::Normal,
        WorkPriority::Low => FixturePriority::Low,
    }
}

fn repositories_from_coordinates(graph: &PlanningGraph) -> Vec<FixtureRepository> {
    let mut repositories = BTreeMap::<String, FixtureRepository>::new();
    for coordinate in &graph.source_coordinates {
        let mut segments = coordinate.repository.0.split('/');
        let owner = segments.next().unwrap_or("OpenAgentsInc");
        let name = segments.next().unwrap_or(&coordinate.repository.0);
        let url = coordinate
            .url
            .0
            .split("/issues/")
            .next()
            .unwrap_or(&coordinate.url.0);
        repositories
            .entry(name.to_ascii_lowercase())
            .or_insert_with(|| FixtureRepository {
                id: format!("repository:{}", name.to_ascii_lowercase()),
                name: name.to_string(),
                owner: owner.to_string(),
                url: url.to_string(),
                revision: coordinate.source_revision.0.clone(),
                default_branch: "main".into(),
            });
    }
    if graph.work.iter().any(|work| {
        !graph
            .source_coordinates
            .iter()
            .any(|coordinate| coordinate.work_ref == work.summary.work_ref)
    }) {
        repositories.insert(
            "openagents-native".into(),
            FixtureRepository {
                id: "repository:openagents-native".into(),
                name: "OpenAgents native Work".into(),
                owner: "OpenAgentsInc".into(),
                url: "Not available from this source".into(),
                revision: graph.revision.0.to_string(),
                default_branch: "Not applicable".into(),
            },
        );
    }
    repositories.into_values().collect()
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct PersistedDogfoodPlanningView {
    schema: String,
    view: DogfoodPlanningViewModel,
}

fn store_path(data_root: &Path) -> PathBuf {
    data_root.join(STORE_DIR).join(STORE_FILE)
}

pub fn read_dogfood_planning_snapshot(
    data_root: &Path,
) -> Result<Option<DogfoodPlanningViewModel>, DogfoodPlanningRefreshError> {
    let path = store_path(data_root);
    let bytes = match fs::read(&path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(DogfoodPlanningRefreshError::Persistence(error.to_string())),
    };
    let stored: PersistedDogfoodPlanningView = serde_json::from_slice(&bytes)
        .map_err(|error| DogfoodPlanningRefreshError::Persistence(error.to_string()))?;
    if stored.schema != DOGFOOD_PLANNING_VIEW_SCHEMA_V1
        || stored.view.schema != DOGFOOD_PLANNING_VIEW_SCHEMA_V1
    {
        return Err(DogfoodPlanningRefreshError::Persistence(
            "unsupported planning cache schema".into(),
        ));
    }
    Ok(Some(stored.view.restored_offline()))
}

pub fn write_dogfood_planning_snapshot(
    data_root: &Path,
    view: &DogfoodPlanningViewModel,
) -> Result<(), DogfoodPlanningRefreshError> {
    if view.origin == DogfoodPlanningOrigin::Fixture {
        return Ok(());
    }
    let path = store_path(data_root);
    let directory = path
        .parent()
        .ok_or_else(|| DogfoodPlanningRefreshError::Persistence("cache has no parent".into()))?;
    fs::create_dir_all(directory)
        .map_err(|error| DogfoodPlanningRefreshError::Persistence(error.to_string()))?;
    let bytes = serde_json::to_vec(&PersistedDogfoodPlanningView {
        schema: DOGFOOD_PLANNING_VIEW_SCHEMA_V1.into(),
        view: view.clone(),
    })
    .map_err(|error| DogfoodPlanningRefreshError::Persistence(error.to_string()))?;
    let digest = format!("{:x}", Sha256::digest(&bytes));
    let sequence = NEXT_TEMP_FILE.fetch_add(1, Ordering::Relaxed);
    let temporary = directory.join(format!(".{STORE_FILE}.{digest}.{sequence}.tmp"));
    let mut file = fs::OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(&temporary)
        .map_err(|error| DogfoodPlanningRefreshError::Persistence(error.to_string()))?;
    file.write_all(&bytes)
        .and_then(|()| file.sync_all())
        .map_err(|error| DogfoodPlanningRefreshError::Persistence(error.to_string()))?;
    fs::rename(&temporary, &path)
        .map_err(|error| DogfoodPlanningRefreshError::Persistence(error.to_string()))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::DogfoodFixtureAdapter;

    fn incomplete_graph(state: &str, revision: u64, cursor: &str) -> PlanningGraph {
        serde_json::from_value(serde_json::json!({
            "contractVersion": "openagents.all_work_boundary.v1",
            "graphRef": "planning-graph:all-work",
            "revision": revision,
            "eventCursor": cursor,
            "reconciliationDigest": "0".repeat(64),
            "generatedAt": "2026-08-03T07:00:00Z",
            "resources": [],
            "work": [],
            "planningLinks": [],
            "labelLinks": [],
            "textRecords": [],
            "releaseScopeLinks": [],
            "sourceCoordinates": [],
            "projectionIssues": [],
            "completeness": {
                "state": state,
                "cursor": cursor,
                "gapRefs": if state == "gap" {
                    vec!["source:github:page:missing"]
                } else {
                    Vec::<&str>::new()
                },
            },
            "freshness": {
                "state": "fresh",
                "observedAt": "2026-08-03T07:00:00Z",
                "sourceUpdatedAt": null,
            },
        }))
        .expect("typed incomplete graph")
    }

    fn representative_live_graph(issue: &FixtureIssue) -> PlanningGraph {
        let work_ref = crate::github_work_ref("OpenAgentsInc", "omega", issue.number);
        let resource = |resource_ref: &str,
                        kind: &str,
                        name: &str,
                        parent_ref: Option<&str>,
                        state: Option<&str>| {
            serde_json::json!({
                "resourceRef": resource_ref,
                "kind": kind,
                "name": name,
                "parentRef": parent_ref,
                "state": state,
                "revision": 1,
                "updatedAt": "2026-08-03T07:00:00Z",
            })
        };
        serde_json::from_value(serde_json::json!({
            "contractVersion": "openagents.all_work_boundary.v1",
            "graphRef": "planning-graph:all-work",
            "revision": 1,
            "eventCursor": "cursor:planning:1",
            "reconciliationDigest": "1".repeat(64),
            "generatedAt": "2026-08-03T07:00:00Z",
            "resources": [
                resource("organization:openagentsinc", "organization", "OpenAgents Inc.", None, None),
                resource("team:omega", "team", "Omega", Some("organization:openagentsinc"), Some("OMEGA")),
                resource("initiative:omega-all-work-client", "initiative", "Omega as the first-class All Work client", Some("organization:openagentsinc"), None),
                resource(&issue.project_id, "project", "Omega v0.2.0 dogfood", Some("initiative:omega-all-work-client"), Some("project-status:started")),
            ],
            "work": [{
                "summary": {
                    "contractVersion": "openagents.all_work_boundary.v1",
                    "workRef": work_ref.clone(),
                    "title": issue.title.clone(),
                    "domain": "development",
                    "workClass": "task",
                    "state": "active",
                    "priority": "high",
                    "ownerRef": "principal:github:openagentsinc",
                    "assignee": null,
                    "sourceAuthority": {
                        "kind": "imported_read_only",
                        "sourceRef": format!("source:github:omega:{}", issue.number),
                        "adapterVersion": "github-bootstrap.v1",
                        "writable": false
                    },
                    "revision": 1,
                    "updatedAt": "2026-08-03T07:00:00Z",
                    "freshness": { "state": "fresh", "observedAt": "2026-08-03T07:00:00Z", "sourceUpdatedAt": null },
                    "completeness": { "state": "complete", "cursor": "cursor:planning:1", "gapRefs": [] },
                    "redaction": { "privacyClass": "public", "redactedFieldCount": 0, "policyRef": "policy:public" }
                },
                "relations": [],
                "threadRefs": [], "sessionRefs": [], "agentSessionRefs": [],
                "agentActivityRefs": [], "runRefs": [], "intentRefs": [], "eventRefs": [],
                "receiptRefs": [], "evidenceRefs": [], "verificationRefs": [], "ownerDispositionRefs": []
            }],
            "planningLinks": [{
                "workRef": work_ref.clone(),
                "projectRef": issue.project_id.clone(),
                "projectMilestoneRef": null,
                "cycleRef": null,
                "workflowStateRef": null,
                "releasePlanningRecordRef": null
            }],
            "labelLinks": [], "textRecords": [], "releaseScopeLinks": [],
            "sourceCoordinates": [{
                "workRef": work_ref,
                "sourceRef": format!("source:github:omega:{}", issue.number),
                "repository": "OpenAgentsInc/omega",
                "identifier": issue.identifier.clone(),
                "url": issue.source_url.clone(),
                "sourceRevision": "github-revision-1",
                "fetchedAt": "2026-08-03T07:00:00Z",
                "available": true
            }],
            "projectionIssues": [],
            "completeness": { "state": "complete", "cursor": "cursor:planning:1", "gapRefs": [] },
            "freshness": { "state": "fresh", "observedAt": "2026-08-03T07:00:00Z", "sourceUpdatedAt": null }
        }))
        .expect("typed representative graph")
    }

    #[test]
    fn fixture_and_live_identity_decode_through_the_same_view_model() {
        let fixture = DogfoodFixtureAdapter::load_for_tests().expect("valid fixture");
        let expected = fixture.graph.issues.first().expect("fixture issue").clone();
        let live = DogfoodPlanningViewModel::from_live(representative_live_graph(&expected), 4)
            .expect("complete live graph");
        let actual = live.graph.issues.first().expect("live issue");
        assert_eq!(actual.id, expected.id);
        assert_eq!(actual.identifier, expected.identifier);
        assert_eq!(actual.title, expected.title);
        assert_eq!(actual.project_id, expected.project_id);
        assert_eq!(actual.source_url, expected.source_url);
    }

    #[test]
    fn incomplete_and_gap_revisions_never_replace_the_complete_projection() {
        let current = DogfoodPlanningViewModel::from_fixture(
            DogfoodFixtureAdapter::load_for_tests().expect("valid fixture"),
        );
        let digest = current.fixture_sha256.clone();
        for state in ["partial", "gap", "truncated"] {
            let candidate = incomplete_graph(state, 2, "cursor:planning:2");
            let error = current
                .stage_live(candidate.clone(), 1)
                .expect_err("incomplete revision must be rejected");
            let retained = current.retain_rejected_graph(&candidate, error.to_string());
            assert_eq!(retained.fixture_sha256, digest);
            assert_eq!(retained.graph, current.graph);
            assert!(matches!(
                retained.source_state,
                DogfoodPlanningSourceState::Partial | DogfoodPlanningSourceState::Gap
            ));
        }
    }

    // omega#223 close criterion: "the reference-process reconnect journey
    // preserves identity and last-known-good state". The staged-refresh tests
    // above prove an incomplete revision is refused; none of them walk the
    // journey a service outage actually produces — fresh live, then failure,
    // then a reconnect at a NEW adapter generation. Each step is checked for
    // the two things that could quietly go wrong: losing the last complete
    // projection, and labelling retained data as current.
    #[test]
    fn an_outage_retains_last_known_good_and_a_reconnect_restores_the_same_identity() {
        let fixture = DogfoodFixtureAdapter::load_for_tests().expect("valid fixture");
        let issue = fixture.graph.issues.first().expect("fixture issue").clone();
        let live = DogfoodPlanningViewModel::from_live(representative_live_graph(&issue), 4)
            .expect("complete live graph");
        assert!(live.is_fresh_live());
        let identity = live.graph.issues.first().expect("live issue").id.clone();

        let offline = live.retain_after_failure(
            DogfoodPlanningSourceState::Offline,
            "planning refresh failed after three bounded attempts",
        );
        assert_eq!(
            offline.graph, live.graph,
            "the outage dropped the last complete projection"
        );
        assert_eq!(offline.revision, live.revision);
        assert_eq!(offline.event_cursor, live.event_cursor);
        assert_eq!(offline.adapter_generation, live.adapter_generation);
        assert_eq!(offline.source_state, DogfoodPlanningSourceState::Offline);
        assert_eq!(
            offline.origin,
            DogfoodPlanningOrigin::OfflineCache,
            "a retained live projection must stop calling itself a live origin"
        );
        assert!(
            !offline.is_fresh_live(),
            "retained data must never present itself as a current live read"
        );
        assert!(offline.last_error.is_some());

        // The reference process came back as a new generation. A reconnect must
        // land the same Work identity and clear the offline label, not fork the
        // graph or keep serving the cache.
        let reconnected = offline
            .stage_live(representative_live_graph(&issue), 5)
            .expect("reconnected complete revision");
        assert_eq!(reconnected.adapter_generation, 5);
        assert!(reconnected.is_fresh_live());
        assert_eq!(reconnected.last_error, None);
        assert_eq!(
            reconnected
                .graph
                .issues
                .iter()
                .filter(|candidate| candidate.id == identity)
                .count(),
            1,
            "the reconnect must not fork the retained Work identity"
        );
    }

    #[test]
    fn persisted_live_projection_restores_as_visible_offline_cache() {
        let directory = std::env::temp_dir().join(format!(
            "omega-dogfood-planning-test-{}",
            NEXT_TEMP_FILE.fetch_add(1, Ordering::Relaxed)
        ));
        let mut view = DogfoodPlanningViewModel::from_fixture(
            DogfoodFixtureAdapter::load_for_tests().expect("valid fixture"),
        );
        view.origin = DogfoodPlanningOrigin::Live;
        view.source_state = DogfoodPlanningSourceState::Fresh;
        view.revision = 7;
        write_dogfood_planning_snapshot(&directory, &view).expect("persist snapshot");
        let restored = read_dogfood_planning_snapshot(&directory)
            .expect("read snapshot")
            .expect("stored snapshot");
        assert_eq!(restored.graph, view.graph);
        assert_eq!(restored.origin, DogfoodPlanningOrigin::OfflineCache);
        assert_eq!(restored.source_state, DogfoodPlanningSourceState::Offline);
        fs::remove_dir_all(directory).expect("remove isolated test directory");
    }
}
