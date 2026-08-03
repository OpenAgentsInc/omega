//! Domain-neutral read projections over one canonical planning graph.
//!
//! Renderers consume this projection. They do not copy Work identity or own a
//! mutation path. The current v0.2.0 surface is development-gated, but the
//! input can be either the checked fixture or the generated owned read.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use crate::{DogfoodPlanningViewModel, FixtureIssue, FixtureLifecycleType, FixturePriority};

/// Match the GitHub bootstrap adapter's canonical repository slug exactly.
pub fn github_work_ref(owner: &str, repository: &str, number: u64) -> String {
    let mut slug = String::new();
    for character in format!("{owner}/{repository}").chars() {
        if character.is_ascii_alphanumeric() {
            slug.push(character.to_ascii_lowercase());
        } else if !slug.is_empty() && !slug.ends_with('-') {
            slug.push('-');
        }
    }
    let slug = slug.trim_end_matches('-');
    format!("work:github:{slug}:{number}")
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PlanningViewKind {
    #[default]
    List,
    Board,
    Table,
    Timeline,
    Roadmap,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PlanningFilter {
    #[default]
    All,
    Open,
    Blocked,
    Completed,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PlanningGroup {
    #[default]
    Lifecycle,
    Milestone,
    Project,
    Priority,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PlanningSort {
    #[default]
    SourceOrder,
    Priority,
    Title,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PlanningSavedView {
    #[default]
    All,
    CriticalPath,
    Unassigned,
    Blocked,
    AgentActive,
    NeedsOwner,
    InReview,
    Verification,
}

impl PlanningSavedView {
    pub const ALL: [Self; 8] = [
        Self::All,
        Self::CriticalPath,
        Self::Unassigned,
        Self::Blocked,
        Self::AgentActive,
        Self::NeedsOwner,
        Self::InReview,
        Self::Verification,
    ];

    pub const fn label(self) -> &'static str {
        match self {
            Self::All => "All Work",
            Self::CriticalPath => "Critical path",
            Self::Unassigned => "Unassigned",
            Self::Blocked => "Blocked",
            Self::AgentActive => "Agent active",
            Self::NeedsOwner => "Needs owner",
            Self::InReview => "In review",
            Self::Verification => "Verification",
        }
    }

    pub const fn key(self) -> &'static str {
        match self {
            Self::All => "all",
            Self::CriticalPath => "critical-path",
            Self::Unassigned => "unassigned",
            Self::Blocked => "blocked",
            Self::AgentActive => "agent-active",
            Self::NeedsOwner => "needs-owner",
            Self::InReview => "in-review",
            Self::Verification => "verification",
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PlanningViewQuery {
    pub organization_id: String,
    pub project_id: String,
    pub saved_view: PlanningSavedView,
    pub filter: PlanningFilter,
    pub group: PlanningGroup,
    pub sort: PlanningSort,
    pub search: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PlanningViewRow {
    pub work_ref: String,
    pub issue_id: String,
    pub identifier: String,
    pub title: String,
    pub repository_id: String,
    pub source_url: String,
    pub project_id: String,
    pub milestone_id: Option<String>,
    pub cycle_id: Option<String>,
    pub release_id: Option<String>,
    pub lifecycle: FixtureLifecycleType,
    pub priority: FixturePriority,
    pub completed: bool,
    pub blocked_by_count: usize,
    pub source_revision: String,
    pub attention: BTreeSet<PlanningAttentionKind>,
}

impl PlanningViewRow {
    pub fn group_label(&self, group: PlanningGroup, model: &DogfoodPlanningViewModel) -> String {
        match group {
            PlanningGroup::Lifecycle => lifecycle_label(self.lifecycle).into(),
            PlanningGroup::Milestone => self
                .milestone_id
                .as_ref()
                .and_then(|id| {
                    model
                        .graph
                        .project_milestones
                        .iter()
                        .find(|value| &value.id == id)
                })
                .map_or_else(|| "No milestone".into(), |value| value.name.clone()),
            PlanningGroup::Project => model
                .graph
                .projects
                .iter()
                .find(|value| value.id == self.project_id)
                .map_or_else(|| "Unknown project".into(), |value| value.name.clone()),
            PlanningGroup::Priority => priority_label(self.priority).into(),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PlanningViewProjection {
    pub kind: PlanningViewKind,
    pub rows: Vec<PlanningViewRow>,
    pub groups: BTreeMap<String, Vec<String>>,
    pub source_revision: u64,
    pub event_cursor: String,
    pub attention_complete: bool,
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum PlanningAttentionKind {
    AgentActive,
    NeedsOwner,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct PlanningAttentionProjection {
    pub by_work_ref: BTreeMap<String, BTreeSet<PlanningAttentionKind>>,
    pub complete: bool,
}

pub fn project_planning_view(
    model: &DogfoodPlanningViewModel,
    kind: PlanningViewKind,
    query: &PlanningViewQuery,
) -> PlanningViewProjection {
    project_planning_view_with_attention(
        model,
        kind,
        query,
        &PlanningAttentionProjection::default(),
    )
}

pub fn project_planning_view_with_attention(
    model: &DogfoodPlanningViewModel,
    kind: PlanningViewKind,
    query: &PlanningViewQuery,
    attention: &PlanningAttentionProjection,
) -> PlanningViewProjection {
    if query.organization_id != model.graph.organization.id {
        return PlanningViewProjection {
            kind,
            rows: Vec::new(),
            groups: BTreeMap::new(),
            source_revision: model.revision,
            event_cursor: model.event_cursor.clone(),
            attention_complete: attention.complete,
        };
    }
    let search = query.search.trim().to_ascii_lowercase();
    let mut rows = model
        .graph
        .issues
        .iter()
        .enumerate()
        .filter(|(_, issue)| issue.project_id == query.project_id)
        .filter(|(_, issue)| saved_view_matches(model, issue, query.saved_view, attention))
        .filter(|(_, issue)| filter_matches(model, issue, query.filter))
        .filter(|(_, issue)| {
            search.is_empty()
                || issue.identifier.to_ascii_lowercase().contains(&search)
                || issue.title.to_ascii_lowercase().contains(&search)
        })
        .filter_map(|(source_order, issue)| {
            planning_row(model, issue, attention).map(|row| (source_order, row))
        })
        .collect::<Vec<_>>();
    rows.sort_by(
        |(left_order, left), (right_order, right)| match query.sort {
            PlanningSort::SourceOrder => left_order.cmp(right_order),
            PlanningSort::Priority => priority_rank(left.priority)
                .cmp(&priority_rank(right.priority))
                .then_with(|| left.identifier.cmp(&right.identifier)),
            PlanningSort::Title => left
                .title
                .to_ascii_lowercase()
                .cmp(&right.title.to_ascii_lowercase())
                .then_with(|| left.identifier.cmp(&right.identifier)),
        },
    );
    let rows = rows.into_iter().map(|(_, row)| row).collect::<Vec<_>>();
    let mut groups = BTreeMap::<String, Vec<String>>::new();
    for row in &rows {
        groups
            .entry(row.group_label(query.group, model))
            .or_default()
            .push(row.work_ref.clone());
    }
    PlanningViewProjection {
        kind,
        rows,
        groups,
        source_revision: model.revision,
        event_cursor: model.event_cursor.clone(),
        attention_complete: attention.complete,
    }
}

fn saved_view_matches(
    model: &DogfoodPlanningViewModel,
    issue: &FixtureIssue,
    saved_view: PlanningSavedView,
    attention: &PlanningAttentionProjection,
) -> bool {
    match saved_view {
        PlanningSavedView::All => true,
        PlanningSavedView::CriticalPath => {
            issue.release_planning_record_id.is_some()
                || model.graph.issue_relations.iter().any(|relation| {
                    relation.issue_id == issue.id
                        && relation.kind == crate::FixtureIssueRelationKind::Blocks
                })
        }
        PlanningSavedView::Unassigned => issue.assignee_user_id.is_none(),
        PlanningSavedView::Blocked => model.graph.issue_relations.iter().any(|relation| {
            relation.related_issue_id == issue.id
                && relation.kind == crate::FixtureIssueRelationKind::Blocks
        }),
        PlanningSavedView::AgentActive => attention
            .by_work_ref
            .get(&work_ref(model, issue))
            .is_some_and(|facts| facts.contains(&PlanningAttentionKind::AgentActive)),
        PlanningSavedView::NeedsOwner => attention
            .by_work_ref
            .get(&work_ref(model, issue))
            .is_some_and(|facts| facts.contains(&PlanningAttentionKind::NeedsOwner)),
        PlanningSavedView::InReview => issue.workflow_state_id == "workflow:in-review",
        PlanningSavedView::Verification => issue.workflow_state_id == "workflow:verification",
    }
}

fn work_ref(model: &DogfoodPlanningViewModel, issue: &FixtureIssue) -> String {
    model
        .graph
        .source_repositories
        .iter()
        .find(|value| value.id == issue.repository_id)
        .map_or_else(String::new, |repository| {
            github_work_ref(&repository.owner, &repository.name, issue.number)
        })
}

fn planning_row(
    model: &DogfoodPlanningViewModel,
    issue: &FixtureIssue,
    attention: &PlanningAttentionProjection,
) -> Option<PlanningViewRow> {
    let repository = model
        .graph
        .source_repositories
        .iter()
        .find(|value| value.id == issue.repository_id)?;
    let workflow = model
        .graph
        .workflow_states
        .iter()
        .find(|value| value.id == issue.workflow_state_id)?;
    let blocked_by_count = model
        .graph
        .issue_relations
        .iter()
        .filter(|relation| relation.related_issue_id == issue.id)
        .count();
    let work_ref = github_work_ref(&repository.owner, &repository.name, issue.number);
    Some(PlanningViewRow {
        attention: attention
            .by_work_ref
            .get(&work_ref)
            .cloned()
            .unwrap_or_default(),
        work_ref,
        issue_id: issue.id.clone(),
        identifier: issue.identifier.clone(),
        title: issue.title.clone(),
        repository_id: issue.repository_id.clone(),
        source_url: issue.source_url.clone(),
        project_id: issue.project_id.clone(),
        milestone_id: issue.project_milestone_id.clone(),
        cycle_id: issue.cycle_id.clone(),
        release_id: issue.release_planning_record_id.clone(),
        lifecycle: workflow.lifecycle_type,
        priority: issue.priority,
        completed: issue.completed,
        blocked_by_count,
        source_revision: repository.revision.clone(),
    })
}

fn filter_matches(
    model: &DogfoodPlanningViewModel,
    issue: &FixtureIssue,
    filter: PlanningFilter,
) -> bool {
    match filter {
        PlanningFilter::All => true,
        PlanningFilter::Open => !issue.completed,
        PlanningFilter::Completed => issue.completed,
        PlanningFilter::Blocked => model
            .graph
            .issue_relations
            .iter()
            .any(|relation| relation.related_issue_id == issue.id),
    }
}

const fn priority_rank(priority: FixturePriority) -> u8 {
    match priority {
        FixturePriority::Urgent => 0,
        FixturePriority::High => 1,
        FixturePriority::Normal => 2,
        FixturePriority::Low => 3,
        FixturePriority::NoPriority => 4,
    }
}

const fn priority_label(priority: FixturePriority) -> &'static str {
    match priority {
        FixturePriority::Urgent => "Urgent",
        FixturePriority::High => "High",
        FixturePriority::Normal => "Normal",
        FixturePriority::Low => "Low",
        FixturePriority::NoPriority => "No priority",
    }
}

const fn lifecycle_label(lifecycle: FixtureLifecycleType) -> &'static str {
    match lifecycle {
        FixtureLifecycleType::Backlog => "Backlog",
        FixtureLifecycleType::Unstarted => "Ready",
        FixtureLifecycleType::Started => "Active",
        FixtureLifecycleType::Completed => "Done",
        FixtureLifecycleType::Canceled => "Canceled",
        FixtureLifecycleType::Planned => "Planned",
    }
}

#[cfg(all(test, feature = "test-support"))]
mod tests {
    use std::collections::BTreeSet;

    use super::*;
    use crate::{DOGFOOD_PROJECT_ID, DogfoodFixtureAdapter};

    fn model() -> DogfoodPlanningViewModel {
        DogfoodPlanningViewModel::from_fixture(
            DogfoodFixtureAdapter::load_for_tests().expect("valid planning fixture"),
        )
    }

    fn query(model: &DogfoodPlanningViewModel) -> PlanningViewQuery {
        PlanningViewQuery {
            organization_id: model.graph.organization.id.clone(),
            project_id: DOGFOOD_PROJECT_ID.into(),
            saved_view: PlanningSavedView::All,
            filter: PlanningFilter::Open,
            group: PlanningGroup::Lifecycle,
            sort: PlanningSort::SourceOrder,
            search: String::new(),
        }
    }

    #[test]
    fn github_work_identity_matches_the_bootstrap_adapter_slug() {
        assert_eq!(
            github_work_ref("OpenAgentsInc", "Omega", 214),
            "work:github:openagentsinc-omega:214"
        );
        assert_eq!(
            github_work_ref(" OpenAgentsInc ", "omega.desktop", 214),
            "work:github:openagentsinc-omega-desktop:214"
        );
    }

    #[test]
    fn every_renderer_preserves_one_work_identity_and_revision() {
        let model = model();
        let query = query(&model);
        let expected = project_planning_view(&model, PlanningViewKind::List, &query);
        let expected_refs = expected
            .rows
            .iter()
            .map(|row| row.work_ref.clone())
            .collect::<BTreeSet<_>>();
        for kind in [
            PlanningViewKind::Board,
            PlanningViewKind::Table,
            PlanningViewKind::Timeline,
            PlanningViewKind::Roadmap,
        ] {
            let projection = project_planning_view(&model, kind, &query);
            assert_eq!(projection.source_revision, expected.source_revision);
            assert_eq!(projection.event_cursor, expected.event_cursor);
            assert_eq!(
                projection
                    .rows
                    .iter()
                    .map(|row| row.work_ref.clone())
                    .collect::<BTreeSet<_>>(),
                expected_refs
            );
        }
    }

    #[test]
    fn organization_scope_fails_closed_without_leaking_rows_or_groups() {
        let model = model();
        let mut query = query(&model);
        query.organization_id = "organization:other".into();
        let projection = project_planning_view(&model, PlanningViewKind::Table, &query);
        assert!(projection.rows.is_empty());
        assert!(projection.groups.is_empty());
    }

    #[test]
    fn filters_sort_and_search_reduce_the_same_canonical_rows() {
        let model = model();
        let mut query = query(&model);
        query.filter = PlanningFilter::Blocked;
        query.sort = PlanningSort::Priority;
        let blocked = project_planning_view(&model, PlanningViewKind::Board, &query);
        assert!(!blocked.rows.is_empty());
        assert!(blocked.rows.iter().all(|row| row.blocked_by_count > 0));
        query.filter = PlanningFilter::All;
        query.search = "portfolio".into();
        let searched = project_planning_view(&model, PlanningViewKind::List, &query);
        assert_eq!(searched.rows.len(), 1);
        assert_eq!(searched.rows[0].identifier, "omega#215");
    }

    #[test]
    fn saved_views_use_only_exact_attention_facts_on_the_shared_rows() {
        let model = model();
        let mut query = query(&model);
        query.filter = PlanningFilter::All;

        query.saved_view = PlanningSavedView::CriticalPath;
        let critical = project_planning_view(&model, PlanningViewKind::Roadmap, &query);
        assert!(
            critical
                .rows
                .iter()
                .any(|row| row.identifier == "omega#160")
        );
        assert!(
            critical
                .rows
                .iter()
                .any(|row| row.identifier == "omega#214")
        );

        query.saved_view = PlanningSavedView::Blocked;
        let blocked = project_planning_view(&model, PlanningViewKind::Board, &query);
        assert!(blocked.rows.iter().all(|row| row.blocked_by_count > 0));

        query.saved_view = PlanningSavedView::Unassigned;
        let unassigned = project_planning_view(&model, PlanningViewKind::Table, &query);
        assert_eq!(
            unassigned.rows.len(),
            model
                .graph
                .issues
                .iter()
                .filter(|issue| issue.project_id == DOGFOOD_PROJECT_ID)
                .count()
        );

        for saved_view in [
            PlanningSavedView::AgentActive,
            PlanningSavedView::NeedsOwner,
            PlanningSavedView::InReview,
            PlanningSavedView::Verification,
        ] {
            query.saved_view = saved_view;
            assert!(
                project_planning_view(&model, PlanningViewKind::List, &query)
                    .rows
                    .is_empty(),
                "{saved_view:?} must not infer unavailable attention state"
            );
        }
    }

    #[test]
    fn saved_view_selection_preserves_identity_across_every_renderer() {
        let model = model();
        let mut query = query(&model);
        query.filter = PlanningFilter::All;
        query.saved_view = PlanningSavedView::CriticalPath;
        let expected = project_planning_view(&model, PlanningViewKind::List, &query)
            .rows
            .into_iter()
            .map(|row| (row.work_ref, row.source_revision))
            .collect::<BTreeSet<_>>();
        for kind in [
            PlanningViewKind::Board,
            PlanningViewKind::Table,
            PlanningViewKind::Timeline,
            PlanningViewKind::Roadmap,
        ] {
            let actual = project_planning_view(&model, kind, &query)
                .rows
                .into_iter()
                .map(|row| (row.work_ref, row.source_revision))
                .collect::<BTreeSet<_>>();
            assert_eq!(actual, expected);
        }
    }

    #[test]
    fn exact_partial_attention_filters_every_renderer_without_claiming_completeness() {
        let model = model();
        let mut query = query(&model);
        query.filter = PlanningFilter::All;
        query.saved_view = PlanningSavedView::AgentActive;
        let attention = PlanningAttentionProjection {
            by_work_ref: BTreeMap::from([
                (
                    github_work_ref("OpenAgentsInc", "omega", 214),
                    BTreeSet::from([PlanningAttentionKind::AgentActive]),
                ),
                (
                    github_work_ref("OpenAgentsInc", "omega", 215),
                    BTreeSet::from([PlanningAttentionKind::NeedsOwner]),
                ),
            ]),
            complete: false,
        };

        for kind in [
            PlanningViewKind::List,
            PlanningViewKind::Board,
            PlanningViewKind::Table,
            PlanningViewKind::Timeline,
            PlanningViewKind::Roadmap,
        ] {
            let active = project_planning_view_with_attention(&model, kind, &query, &attention);
            assert_eq!(active.rows.len(), 1);
            assert_eq!(active.rows[0].identifier, "omega#214");
            assert!(
                active.rows[0]
                    .attention
                    .contains(&PlanningAttentionKind::AgentActive)
            );
            assert!(!active.attention_complete);

            query.saved_view = PlanningSavedView::NeedsOwner;
            let needs_owner =
                project_planning_view_with_attention(&model, kind, &query, &attention);
            assert_eq!(needs_owner.rows.len(), 1);
            assert_eq!(needs_owner.rows[0].identifier, "omega#215");
            assert!(!needs_owner.attention_complete);
            query.saved_view = PlanningSavedView::AgentActive;
        }
    }
}
