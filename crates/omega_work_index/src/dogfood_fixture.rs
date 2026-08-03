//! Development-only v0.2.0 release-planning fixture projection.
//!
//! This projection is deliberately outside [`crate::WorkIndex`]. It cannot
//! qualify an adapter lane, persist a production route, or admit a command.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};
use thiserror::Error;

pub const DOGFOOD_FIXTURE_SCHEMA_V1: &str = "openagents.omega.dogfood-fixture.v1";
pub const DOGFOOD_FIXTURE_ENV: &str = "OMEGA_UI_MOCKS";
pub const DOGFOOD_PROJECT_ID: &str = "project:omega-v0.2.0-dogfood";
pub const SECURITY_PROJECT_ID: &str = "project:forensics-security-work";
pub const DOGFOOD_OPEN_ISSUE_COUNT: usize = 10;
pub const SECURITY_OPEN_ISSUE_COUNT: usize = 12;
pub const FOUNDATION_ISSUE_COUNT: usize = 6;

const FIXTURE_BYTES: &[u8] = include_bytes!("../fixtures/v0.2.0-all-work.v1.json");

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DogfoodFixtureGate {
    debug_assertions: bool,
    mocks_requested: bool,
}

impl DogfoodFixtureGate {
    pub fn from_process_environment() -> Self {
        Self::from_runtime_state(
            cfg!(debug_assertions),
            std::env::var(DOGFOOD_FIXTURE_ENV).as_deref() == Ok("1"),
        )
    }

    const fn from_runtime_state(debug_assertions: bool, mocks_requested: bool) -> Self {
        Self {
            debug_assertions,
            mocks_requested,
        }
    }

    pub const fn enabled(self) -> bool {
        self.debug_assertions && self.mocks_requested
    }
}

#[derive(Clone, Debug, Error, Eq, PartialEq)]
pub enum DogfoodFixtureError {
    #[error("fixture JSON is invalid: {0}")]
    InvalidJson(String),
    #[error("unsupported fixture schema {0:?}")]
    UnsupportedSchema(String),
    #[error("fixture provenance is incomplete or not explicitly simulated")]
    InvalidProvenance,
    #[error("fixture digest does not match its normalized graph")]
    DigestMismatch,
    #[error("fixture contains a duplicate logical id {0:?}")]
    DuplicateId(String),
    #[error("fixture contains duplicate issue identity {0:?}")]
    DuplicateIssue(String),
    #[error("fixture contains an invalid source coordinate for {0:?}")]
    InvalidSourceCoordinate(String),
    #[error("fixture relation references missing issue {0:?}")]
    MissingRelationTarget(String),
    #[error("fixture contains an unexpected project inventory")]
    InventoryMismatch,
    #[error("fixture invented live accountability, execution, or evidence for {0:?}")]
    InventedAuthority(String),
    #[error("fixture extension identity does not match issue {0:?}")]
    ExtensionMismatch(String),
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DogfoodFixtureProjection {
    pub fixture_version: String,
    pub fixture_sha256: String,
    pub generated_at: String,
    pub source_snapshot_at: String,
    pub simulation: bool,
    pub graph: DogfoodPlanningGraph,
}

impl DogfoodFixtureProjection {
    pub fn normalized_graph_sha256(&self) -> Result<String, DogfoodFixtureError> {
        let bytes = serde_json::to_vec(&self.graph)
            .map_err(|error| DogfoodFixtureError::InvalidJson(error.to_string()))?;
        Ok(format!("{:x}", Sha256::digest(bytes)))
    }

    pub fn validate(&self) -> Result<(), DogfoodFixtureError> {
        if self.fixture_version != DOGFOOD_FIXTURE_SCHEMA_V1 {
            return Err(DogfoodFixtureError::UnsupportedSchema(
                self.fixture_version.clone(),
            ));
        }
        if !self.simulation
            || self.generated_at != "2026-08-03T05:00:00Z"
            || self.source_snapshot_at != "2026-08-03T05:00:00Z"
            || self.fixture_sha256.len() != 64
            || !self
                .fixture_sha256
                .bytes()
                .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
        {
            return Err(DogfoodFixtureError::InvalidProvenance);
        }
        if self.normalized_graph_sha256()? != self.fixture_sha256 {
            return Err(DogfoodFixtureError::DigestMismatch);
        }
        self.graph.validate()
    }
}

pub struct DogfoodFixtureAdapter;

impl DogfoodFixtureAdapter {
    pub fn load(
        gate: DogfoodFixtureGate,
    ) -> Result<Option<DogfoodFixtureProjection>, DogfoodFixtureError> {
        if !gate.enabled() {
            return Ok(None);
        }
        let projection: DogfoodFixtureProjection = serde_json::from_slice(FIXTURE_BYTES)
            .map_err(|error| DogfoodFixtureError::InvalidJson(error.to_string()))?;
        projection.validate()?;
        Ok(Some(projection))
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn load_for_tests() -> Result<DogfoodFixtureProjection, DogfoodFixtureError> {
        let projection: DogfoodFixtureProjection = serde_json::from_slice(FIXTURE_BYTES)
            .map_err(|error| DogfoodFixtureError::InvalidJson(error.to_string()))?;
        projection.validate()?;
        Ok(projection)
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DogfoodPlanningGraph {
    pub source_repositories: Vec<FixtureRepository>,
    pub organization: FixtureOrganization,
    pub teams: Vec<FixtureTeam>,
    pub initiatives: Vec<FixtureInitiative>,
    pub projects: Vec<FixtureProject>,
    pub project_statuses: Vec<NamedFixtureRecord>,
    pub project_milestones: Vec<FixtureProjectMilestone>,
    pub cycles: Vec<FixtureCycle>,
    pub release_pipelines: Vec<FixtureReleasePipeline>,
    pub release_stages: Vec<FixtureReleaseStage>,
    pub releases: Vec<FixtureReleasePlanningRecord>,
    pub workflow_states: Vec<FixtureWorkflowState>,
    pub labels: Vec<NamedFixtureRecord>,
    pub issues: Vec<FixtureIssue>,
    pub issue_relations: Vec<FixtureIssueRelation>,
    pub documents: Vec<FixtureDocument>,
    pub project_updates: Vec<FixtureProjectUpdate>,
    pub custom_views: Vec<FixtureCustomView>,
    pub users: Vec<FixtureUserProjection>,
    pub agent_sessions: Vec<FixtureAgentSession>,
    pub agent_activities: Vec<FixtureAgentActivity>,
    pub pull_requests: Vec<FixturePullRequest>,
    pub diffs: Vec<FixtureDiff>,
    pub open_agents_extensions: FixtureOpenAgentsEnvelope,
    pub projection_issues: Vec<FixtureProjectionIssue>,
}

impl DogfoodPlanningGraph {
    fn validate(&self) -> Result<(), DogfoodFixtureError> {
        let repositories = unique_map(
            self.source_repositories
                .iter()
                .map(|repository| (repository.id.as_str(), repository)),
        )?;
        let teams = unique_map(self.teams.iter().map(|team| (team.id.as_str(), team)))?;
        let initiatives = unique_map(
            self.initiatives
                .iter()
                .map(|initiative| (initiative.id.as_str(), initiative)),
        )?;
        let projects = unique_map(
            self.projects
                .iter()
                .map(|project| (project.id.as_str(), project)),
        )?;
        let project_statuses = unique_map(
            self.project_statuses
                .iter()
                .map(|status| (status.id.as_str(), status)),
        )?;
        let milestones = unique_map(
            self.project_milestones
                .iter()
                .map(|milestone| (milestone.id.as_str(), milestone)),
        )?;
        let states = unique_map(
            self.workflow_states
                .iter()
                .map(|state| (state.id.as_str(), state)),
        )?;
        let cycles = unique_map(self.cycles.iter().map(|cycle| (cycle.id.as_str(), cycle)))?;
        let pipelines = unique_map(
            self.release_pipelines
                .iter()
                .map(|pipeline| (pipeline.id.as_str(), pipeline)),
        )?;
        let stages = unique_map(
            self.release_stages
                .iter()
                .map(|stage| (stage.id.as_str(), stage)),
        )?;
        let releases = unique_map(
            self.releases
                .iter()
                .map(|release| (release.id.as_str(), release)),
        )?;
        let labels = unique_map(self.labels.iter().map(|label| (label.id.as_str(), label)))?;
        let issues = unique_map(self.issues.iter().map(|issue| (issue.id.as_str(), issue)))?;
        unique_map(
            self.issue_relations
                .iter()
                .map(|relation| (relation.id.as_str(), relation)),
        )?;
        unique_map(
            self.documents
                .iter()
                .map(|document| (document.id.as_str(), document)),
        )?;
        unique_map(
            self.project_updates
                .iter()
                .map(|update| (update.id.as_str(), update)),
        )?;
        unique_map(
            self.custom_views
                .iter()
                .map(|view| (view.id.as_str(), view)),
        )?;
        if self.organization.id != "organization:openagentsinc"
            || teams
                .values()
                .any(|team| team.organization_id != self.organization.id || team.key != "OMEGA")
            || initiatives.values().any(|initiative| {
                initiative.organization_id != self.organization.id
                    || initiative
                        .project_ids
                        .iter()
                        .any(|project_id| !projects.contains_key(project_id.as_str()))
            })
            || projects.values().any(|project| {
                !teams.contains_key(project.team_id.as_str())
                    || !initiatives.contains_key(project.initiative_id.as_str())
                    || !project_statuses.contains_key(project.status_id.as_str())
            })
            || milestones
                .values()
                .any(|milestone| !projects.contains_key(milestone.project_id.as_str()))
            || cycles.values().any(|cycle| {
                !teams.contains_key(cycle.team_id.as_str())
                    || !projects.contains_key(cycle.project_id.as_str())
            })
            || stages
                .values()
                .any(|stage| !pipelines.contains_key(stage.pipeline_id.as_str()))
            || self.releases.iter().any(|release| {
                !pipelines.contains_key(release.pipeline_id.as_str())
                    || !stages.contains_key(release.stage_id.as_str())
                    || !projects.contains_key(release.project_id.as_str())
                    || release.canonical_release_authority
            })
        {
            return Err(DogfoodFixtureError::InventoryMismatch);
        }
        let mut identifiers = BTreeSet::new();
        for issue in &self.issues {
            if !identifiers.insert((issue.repository_id.clone(), issue.number)) {
                return Err(DogfoodFixtureError::DuplicateIssue(
                    issue.identifier.clone(),
                ));
            }
            let Some(repository) = repositories.get(issue.repository_id.as_str()) else {
                return Err(DogfoodFixtureError::InvalidSourceCoordinate(
                    issue.identifier.clone(),
                ));
            };
            let expected_identifier = format!("{}#{}", repository.name, issue.number);
            let expected_url = format!("{}/issues/{}", repository.url, issue.number);
            if issue.identifier != expected_identifier
                || issue.source_url != expected_url
                || !projects.contains_key(issue.project_id.as_str())
                || !states.contains_key(issue.workflow_state_id.as_str())
                || issue
                    .project_milestone_id
                    .as_ref()
                    .is_some_and(|id| !milestones.contains_key(id.as_str()))
                || issue
                    .cycle_id
                    .as_ref()
                    .is_some_and(|id| !cycles.contains_key(id.as_str()))
                || issue
                    .release_planning_record_id
                    .as_ref()
                    .is_some_and(|id| !releases.contains_key(id.as_str()))
                || issue
                    .label_ids
                    .iter()
                    .any(|id| !labels.contains_key(id.as_str()))
            {
                return Err(DogfoodFixtureError::InvalidSourceCoordinate(
                    issue.identifier.clone(),
                ));
            }
            if issue.assignee_user_id.is_some()
                || issue.delegate_user_id.is_some()
                || issue.execution_refs.has_any()
                || !issue.receipt_refs.is_empty()
                || !issue.evidence_refs.is_empty()
                || !issue.verification_refs.is_empty()
                || !issue.owner_disposition_refs.is_empty()
            {
                return Err(DogfoodFixtureError::InventedAuthority(
                    issue.identifier.clone(),
                ));
            }
        }
        for relation in &self.issue_relations {
            if !issues.contains_key(relation.issue_id.as_str())
                || !issues.contains_key(relation.related_issue_id.as_str())
                || relation.issue_id == relation.related_issue_id
            {
                return Err(DogfoodFixtureError::MissingRelationTarget(
                    relation.id.clone(),
                ));
            }
        }
        if self.releases.len() != 1
            || self.releases[0].scope_issue_ids != ["issue:omega:160"]
            || self.documents.iter().any(|document| {
                !projects.contains_key(document.project_id.as_str())
                    || !document
                        .url
                        .starts_with("https://github.com/OpenAgentsInc/")
            })
            || self
                .project_updates
                .iter()
                .any(|update| !projects.contains_key(update.project_id.as_str()))
            || self
                .custom_views
                .iter()
                .any(|view| !projects.contains_key(view.project_id.as_str()))
        {
            return Err(DogfoodFixtureError::InventoryMismatch);
        }
        let dogfood_open = self
            .issues
            .iter()
            .filter(|issue| issue.project_id == DOGFOOD_PROJECT_ID && !issue.completed)
            .count();
        let security_open = self
            .issues
            .iter()
            .filter(|issue| issue.project_id == SECURITY_PROJECT_ID && !issue.completed)
            .count();
        let foundation = self.issues.iter().filter(|issue| issue.completed).count();
        if dogfood_open != DOGFOOD_OPEN_ISSUE_COUNT
            || security_open != SECURITY_OPEN_ISSUE_COUNT
            || foundation != FOUNDATION_ISSUE_COUNT
            || self.issues.len()
                != DOGFOOD_OPEN_ISSUE_COUNT + SECURITY_OPEN_ISSUE_COUNT + FOUNDATION_ISSUE_COUNT
            || self.agent_sessions.len()
                + self.agent_activities.len()
                + self.pull_requests.len()
                + self.diffs.len()
                != 0
            || repositories.len() != 2
            || projects.len() != 2
            || milestones.len() != 5
            || self.release_stages.len() != 4
            || !self.users.is_empty()
        {
            return Err(DogfoodFixtureError::InventoryMismatch);
        }
        let extension = &self.open_agents_extensions;
        if extension.work_identity_scheme != "work:fixture:{issueId}"
            || extension.issue_projection_identity_scheme != "{issueId}"
            || extension.source_snapshot_at != "2026-08-03T05:00:00Z"
            || !extension.simulation
            || extension.canonical_authority
            || !extension.allowed_commands.is_empty()
            || extension.authority_loss_facts.is_empty()
        {
            return Err(DogfoodFixtureError::ExtensionMismatch(
                "openAgentsExtensions".into(),
            ));
        }
        Ok(())
    }
}

fn unique_map<'a, T>(
    values: impl IntoIterator<Item = (&'a str, &'a T)>,
) -> Result<BTreeMap<&'a str, &'a T>, DogfoodFixtureError> {
    let mut result = BTreeMap::new();
    for (id, value) in values {
        if result.insert(id, value).is_some() {
            return Err(DogfoodFixtureError::DuplicateId(id.to_string()));
        }
    }
    Ok(result)
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct FixtureRepository {
    pub id: String,
    pub name: String,
    pub owner: String,
    pub url: String,
    pub revision: String,
    pub default_branch: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct FixtureOrganization {
    pub id: String,
    pub name: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct FixtureTeam {
    pub id: String,
    pub name: String,
    pub key: String,
    pub organization_id: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct FixtureInitiative {
    pub id: String,
    pub name: String,
    pub organization_id: String,
    pub project_ids: Vec<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct FixtureProject {
    pub id: String,
    pub name: String,
    pub team_id: String,
    pub initiative_id: String,
    pub status_id: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct NamedFixtureRecord {
    pub id: String,
    pub name: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct FixtureProjectMilestone {
    pub id: String,
    pub name: String,
    pub project_id: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct FixtureCycle {
    pub id: String,
    pub name: String,
    pub team_id: String,
    pub project_id: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct FixtureReleasePipeline {
    pub id: String,
    pub name: String,
    pub organization_id: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct FixtureReleaseStage {
    pub id: String,
    pub name: String,
    pub pipeline_id: String,
    pub lifecycle_type: FixtureLifecycleType,
    pub position: u8,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct FixtureReleasePlanningRecord {
    pub id: String,
    pub name: String,
    pub pipeline_id: String,
    pub stage_id: String,
    pub project_id: String,
    pub target_version: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target_date: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target_commit: Option<String>,
    pub scope_issue_ids: Vec<String>,
    pub canonical_release_authority: bool,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct FixtureWorkflowState {
    pub id: String,
    pub name: String,
    pub lifecycle_type: FixtureLifecycleType,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FixtureLifecycleType {
    Backlog,
    Unstarted,
    Started,
    Completed,
    Canceled,
    Planned,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct FixtureIssue {
    pub id: String,
    pub identifier: String,
    pub title: String,
    pub repository_id: String,
    pub number: u64,
    pub source_url: String,
    pub project_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub project_milestone_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cycle_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub release_planning_record_id: Option<String>,
    pub workflow_state_id: String,
    pub priority: FixturePriority,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub label_ids: Vec<String>,
    pub completed: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub assignee_user_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub delegate_user_id: Option<String>,
    #[serde(default, skip_serializing_if = "FixtureExecutionRefs::is_empty")]
    pub execution_refs: FixtureExecutionRefs,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub receipt_refs: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub evidence_refs: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub verification_refs: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub owner_disposition_refs: Vec<String>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FixturePriority {
    NoPriority,
    Urgent,
    High,
    Normal,
    Low,
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct FixtureExecutionRefs {
    pub thread_ref: Option<String>,
    pub session_ref: Option<String>,
    pub agent_session_ref: Option<String>,
    pub run_ref: Option<String>,
    pub repository_work_claim_ref: Option<String>,
    pub lease_ref: Option<String>,
}

impl FixtureExecutionRefs {
    fn is_empty(&self) -> bool {
        !self.has_any()
    }

    fn has_any(&self) -> bool {
        self.thread_ref.is_some()
            || self.session_ref.is_some()
            || self.agent_session_ref.is_some()
            || self.run_ref.is_some()
            || self.repository_work_claim_ref.is_some()
            || self.lease_ref.is_some()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FixtureIssueRelationKind {
    Blocks,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct FixtureIssueRelation {
    pub id: String,
    pub issue_id: String,
    pub related_issue_id: String,
    pub kind: FixtureIssueRelationKind,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct FixtureDocument {
    pub id: String,
    pub title: String,
    pub url: String,
    pub project_id: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct FixtureProjectUpdate {
    pub id: String,
    pub project_id: String,
    pub health: String,
    pub summary: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct FixtureCustomView {
    pub id: String,
    pub name: String,
    pub project_id: String,
    pub filter: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct FixtureUserProjection {
    pub id: String,
    pub display_name: String,
    pub agent_member_ref: Option<String>,
    pub supports_agent_sessions: bool,
    pub canonical_principal: bool,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct FixtureAgentSession {
    pub id: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct FixtureAgentActivity {
    pub id: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct FixturePullRequest {
    pub id: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct FixtureDiff {
    pub id: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct FixtureOpenAgentsEnvelope {
    pub work_identity_scheme: String,
    pub issue_projection_identity_scheme: String,
    pub source_snapshot_at: String,
    pub simulation: bool,
    pub canonical_authority: bool,
    pub allowed_commands: Vec<String>,
    pub authority_loss_facts: Vec<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct FixtureProjectionIssue {
    pub id: String,
    pub field: String,
    pub reason: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn release_and_disabled_debug_gates_omit_the_fixture() {
        assert_eq!(
            DogfoodFixtureAdapter::load(DogfoodFixtureGate::from_runtime_state(false, true)),
            Ok(None)
        );
        assert_eq!(
            DogfoodFixtureAdapter::load(DogfoodFixtureGate::from_runtime_state(true, false)),
            Ok(None)
        );
    }

    #[test]
    fn admitted_fixture_has_exact_inventory_and_digest() {
        let fixture =
            DogfoodFixtureAdapter::load(DogfoodFixtureGate::from_runtime_state(true, true))
                .expect("valid fixture")
                .expect("enabled fixture");
        assert_eq!(fixture.graph.issues.len(), 28);
        assert_eq!(
            fixture.normalized_graph_sha256().expect("digest"),
            fixture.fixture_sha256
        );
        assert!(fixture.graph.issues.iter().all(|issue| {
            issue.assignee_user_id.is_none()
                && issue.delegate_user_id.is_none()
                && !issue.execution_refs.has_any()
        }));
    }

    #[test]
    fn every_relation_resolves_and_the_extension_denies_authority() {
        let fixture =
            DogfoodFixtureAdapter::load(DogfoodFixtureGate::from_runtime_state(true, true))
                .expect("valid fixture")
                .expect("enabled fixture");
        let ids = fixture
            .graph
            .issues
            .iter()
            .map(|issue| issue.id.as_str())
            .collect::<BTreeSet<_>>();
        assert!(fixture.graph.issue_relations.iter().all(|relation| {
            ids.contains(relation.issue_id.as_str())
                && ids.contains(relation.related_issue_id.as_str())
        }));
        assert!(fixture.graph.open_agents_extensions.simulation);
    }

    #[test]
    fn tampering_with_normalized_graph_fails_digest_validation() {
        let mut fixture =
            DogfoodFixtureAdapter::load(DogfoodFixtureGate::from_runtime_state(true, true))
                .expect("valid fixture")
                .expect("enabled fixture");
        fixture.graph.issues[0].title.push_str(" changed");
        assert_eq!(fixture.validate(), Err(DogfoodFixtureError::DigestMismatch));
    }
}
