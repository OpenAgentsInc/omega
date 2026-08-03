//! The state a production planning destination can be in, and the exactly one
//! rule that decides which.
//!
//! omega#239. Before this module the only consumer of a canonical planning
//! graph in Omega was the development `DogfoodSurface`, reachable only behind
//! the omega#209 mock gate, so a release build had no destination for planning
//! Work at all. A production destination needs a state set, and the part this
//! codebase keeps collapsing is the difference between *the service answered
//! and has no Work*, *the boundary could not be reached*, and *this build's
//! component cannot serve the boundary at all*. Those three read identically
//! through an `Option`, and a reader who cannot tell them apart cannot act on
//! any of them: the first is normal, the second is worth retrying, and the
//! third is a packaging defect that no retry will ever fix.
//!
//! So the classification is one total function over a `Result`, the three
//! failing shapes are separate variants rather than one string, and every
//! variant carries the sentence a person is shown. Nothing here renders; this
//! is deliberately free of GPUI so the state set can be proven without a
//! window.

use omega_effectd::SupervisorError;
use omega_effectd::all_work_contract::{
    AgentDelegate, CompletenessState, FreshnessState, PlanningGraph, WorkPriority, WorkSnapshot,
    WorkState,
};
use serde::{Deserialize, Serialize};

/// The All Work method a planning destination reads.
pub const PLANNING_DESTINATION_METHOD: &str = "planning.graph.read";

/// Why this build cannot serve planning Work.
///
/// Kept as two variants rather than one message because they have different
/// remedies: an absent boundary means the packaged component predates All Work
/// entirely, a withheld capability means it negotiated All Work and declined
/// this one method.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum PlanningBoundaryRefusal {
    /// The running component answered `initialize` with no All Work
    /// negotiation at all.
    BoundaryAbsent { method: String },
    /// The running component negotiated All Work but withheld this capability.
    CapabilityWithheld { method: String, capability: String },
}

impl PlanningBoundaryRefusal {
    #[must_use]
    pub fn method(&self) -> &str {
        match self {
            Self::BoundaryAbsent { method } | Self::CapabilityWithheld { method, .. } => method,
        }
    }

    /// The name of the thing that is absent, so the refusal can never read as
    /// a generic error.
    #[must_use]
    pub fn absent_boundary(&self) -> String {
        match self {
            Self::BoundaryAbsent { .. } => "the All Work boundary".to_string(),
            Self::CapabilityWithheld { capability, .. } => capability.clone(),
        }
    }

    #[must_use]
    pub fn detail(&self) -> String {
        match self {
            Self::BoundaryAbsent { method } => format!(
                "This build's omega-effectd component implements no All Work boundary, \
                 so {method} cannot be served. Reinstall Omega with a component that \
                 carries the All Work boundary."
            ),
            Self::CapabilityWithheld { method, capability } => format!(
                "This build's omega-effectd component did not negotiate {capability}, \
                 so {method} cannot be served. Reinstall Omega with a component that \
                 grants {capability}."
            ),
        }
    }
}

/// Every way a planning read can fail, with the refusal kept apart from the
/// rest.
///
/// The whole point of this type is that it is not `Option` and not `String`:
/// `SupervisorError` already distinguishes a component that cannot serve the
/// boundary from one that could not be reached, and that distinction must
/// survive the trip to the screen.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PlanningReadFailure {
    Refused(PlanningBoundaryRefusal),
    Unavailable { detail: String },
}

impl PlanningReadFailure {
    /// Carry the supervisor's own typed refusal through instead of flattening
    /// it into a message.
    #[must_use]
    pub fn from_supervisor_error(error: &SupervisorError) -> Self {
        match error {
            SupervisorError::AllWorkBoundaryAbsent { method } => {
                Self::Refused(PlanningBoundaryRefusal::BoundaryAbsent {
                    method: (*method).to_string(),
                })
            }
            SupervisorError::AllWorkCapabilityWithheld { method, capability } => {
                Self::Refused(PlanningBoundaryRefusal::CapabilityWithheld {
                    method: (*method).to_string(),
                    capability: capability.clone(),
                })
            }
            other => Self::Unavailable {
                detail: other.to_string(),
            },
        }
    }

    #[must_use]
    pub fn unavailable(detail: impl Into<String>) -> Self {
        Self::Unavailable {
            detail: detail.into(),
        }
    }
}

/// The label a state is known by, so a test can assert which state a build is
/// in without matching on payloads.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PlanningDestinationStateKind {
    Loading,
    Populated,
    Empty,
    Unavailable,
    Refused,
}

impl PlanningDestinationStateKind {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Loading => "loading",
            Self::Populated => "populated",
            Self::Empty => "empty",
            Self::Unavailable => "unavailable",
            Self::Refused => "refused",
        }
    }

    /// Every state this destination can be in. Used by the proofs so a new
    /// variant cannot be added without being checked for distinguishability.
    #[must_use]
    pub const fn all() -> [Self; 5] {
        [
            Self::Loading,
            Self::Populated,
            Self::Empty,
            Self::Unavailable,
            Self::Refused,
        ]
    }
}

/// One canonical planning Work item, projected for display.
///
/// The delegate, thread, session, agent-session and run fields are kept apart
/// rather than merged into one "who is on this" line: omega#214 exists because
/// an Assignee, an Agent Delegate, a Thread, a Session and a Run are five
/// different things, and a destination that folds them cannot be the surface
/// on which they become visible.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PlanningDestinationRow {
    pub work_ref: String,
    pub title: String,
    pub state: WorkState,
    pub priority: WorkPriority,
    pub owner_ref: String,
    pub assignee_ref: Option<String>,
    pub agent_delegate_ref: Option<String>,
    pub thread_refs: Vec<String>,
    pub session_refs: Vec<String>,
    pub agent_session_refs: Vec<String>,
    pub run_refs: Vec<String>,
    pub source_authority_kind: String,
    pub writable: bool,
}

impl PlanningDestinationRow {
    fn from_snapshot(snapshot: &WorkSnapshot) -> Self {
        let summary = &snapshot.summary;
        Self {
            work_ref: summary.work_ref.0.clone(),
            title: summary.title.0.clone(),
            state: summary.state.clone(),
            priority: summary.priority.clone(),
            owner_ref: summary.owner_ref.0.clone(),
            assignee_ref: summary
                .assignee
                .0
                .as_ref()
                .map(|assignee| assignee.principal_ref.0.clone()),
            agent_delegate_ref: summary
                .agent_delegate
                .as_ref()
                .and_then(Option::as_ref)
                .map(|delegate: &AgentDelegate| delegate.agent_ref.0.clone()),
            thread_refs: snapshot
                .thread_refs
                .iter()
                .map(|value| value.0.clone())
                .collect(),
            session_refs: snapshot
                .session_refs
                .iter()
                .map(|value| value.0.clone())
                .collect(),
            agent_session_refs: snapshot
                .agent_session_refs
                .iter()
                .map(|value| value.0.clone())
                .collect(),
            run_refs: snapshot
                .run_refs
                .iter()
                .map(|value| value.0.clone())
                .collect(),
            source_authority_kind: serde_json::to_value(&snapshot.summary.source_authority.kind)
                .ok()
                .and_then(|value| value.as_str().map(str::to_owned))
                .unwrap_or_else(|| "unknown".to_string()),
            writable: snapshot.summary.source_authority.writable,
        }
    }

    #[must_use]
    pub fn state_label(&self) -> &'static str {
        match &self.state {
            WorkState::Triage => "Triage",
            WorkState::Planned => "Planned",
            WorkState::Active => "Active",
            WorkState::Waiting => "Waiting",
            WorkState::Blocked => "Blocked",
            WorkState::Failed => "Failed",
            WorkState::Completed => "Completed",
            WorkState::Canceled => "Canceled",
            WorkState::Archived => "Archived",
        }
    }

    #[must_use]
    pub fn priority_label(&self) -> &'static str {
        match &self.priority {
            WorkPriority::None => "No priority",
            WorkPriority::Urgent => "Urgent",
            WorkPriority::High => "High",
            WorkPriority::Normal => "Normal",
            WorkPriority::Low => "Low",
        }
    }

    /// The single line a screen reader is given for this row.
    #[must_use]
    pub fn accessibility_label(&self) -> String {
        let assignee = self
            .assignee_ref
            .clone()
            .unwrap_or_else(|| "no assignee".to_string());
        let delegate = self
            .agent_delegate_ref
            .clone()
            .unwrap_or_else(|| "no agent delegate".to_string());
        format!(
            "{}, {}, {}, {}, {}",
            self.title,
            self.state_label(),
            self.priority_label(),
            assignee,
            delegate
        )
    }
}

/// Which revision of the planning graph a person is reading.
///
/// Deliberately carries no rows. `Empty` holds one of these rather than a
/// whole snapshot so that "the service answered and has no Work" is
/// *structurally* unable to draw Work: there is nothing to draw from. The same
/// reasoning keeps `Unavailable` and `Refused` payload-free.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PlanningGraphIdentity {
    pub graph_ref: String,
    pub revision: u64,
    pub generated_at: String,
    pub freshness: FreshnessState,
    pub completeness: CompletenessState,
    pub projection_issue_count: usize,
}

impl PlanningGraphIdentity {
    #[must_use]
    pub fn from_graph(graph: &PlanningGraph) -> Self {
        Self {
            graph_ref: graph.graph_ref.0.clone(),
            revision: graph.revision.0,
            generated_at: graph.generated_at.0.clone(),
            freshness: graph.freshness.state.clone(),
            completeness: graph.completeness.state.clone(),
            projection_issue_count: graph.projection_issues.len(),
        }
    }

    #[must_use]
    pub fn provenance(&self) -> String {
        let freshness = match self.freshness {
            FreshnessState::Fresh => "fresh",
            FreshnessState::Stale => "stale",
            FreshnessState::OfflineCached => "offline cached",
            FreshnessState::Unknown => "unknown freshness",
        };
        let completeness = match self.completeness {
            CompletenessState::Complete => "complete",
            CompletenessState::Partial => "partial",
            CompletenessState::Gap => "has a cursor gap",
            CompletenessState::Truncated => "truncated",
            CompletenessState::Unknown => "unknown completeness",
        };
        format!(
            "{} · revision {} · {freshness} · {completeness}",
            self.graph_ref, self.revision
        )
    }
}

/// A planning graph that carries at least one Work item.
///
/// The invariant that `rows` is never empty is enforced by the only
/// constructor: [`PlanningDestinationState::classify`] routes an empty answer
/// to `Empty` before this type is ever built.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PlanningDestinationSnapshot {
    pub identity: PlanningGraphIdentity,
    pub rows: Vec<PlanningDestinationRow>,
}

impl PlanningDestinationSnapshot {
    #[must_use]
    pub fn provenance(&self) -> String {
        self.identity.provenance()
    }
}

/// The complete state set of the production planning destination.
///
/// `Empty` carries a snapshot and the two failing states do not, which is the
/// structural reason a failure can never be drawn as "no Work": there is no
/// snapshot to draw rows from.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub enum PlanningDestinationState {
    /// A read is in flight and nothing has answered yet.
    #[default]
    Loading,
    /// The boundary answered with at least one canonical planning Work item.
    Populated(PlanningDestinationSnapshot),
    /// The boundary answered, and its planning graph carries no Work.
    Empty(PlanningGraphIdentity),
    /// The boundary exists in this build but did not answer.
    Unavailable { detail: String },
    /// This build's component cannot serve the boundary at all.
    Refused(PlanningBoundaryRefusal),
}

impl PlanningDestinationState {
    /// The one rule that decides which state the destination is in.
    #[must_use]
    pub fn classify(answer: Result<&PlanningGraph, &PlanningReadFailure>) -> Self {
        match answer {
            Ok(graph) => {
                let identity = PlanningGraphIdentity::from_graph(graph);
                let rows: Vec<PlanningDestinationRow> = graph
                    .work
                    .iter()
                    .map(PlanningDestinationRow::from_snapshot)
                    .collect();
                if rows.is_empty() {
                    Self::Empty(identity)
                } else {
                    Self::Populated(PlanningDestinationSnapshot { identity, rows })
                }
            }
            Err(PlanningReadFailure::Refused(refusal)) => Self::Refused(refusal.clone()),
            Err(PlanningReadFailure::Unavailable { detail }) => Self::Unavailable {
                detail: detail.clone(),
            },
        }
    }

    /// Classify straight from the supervisor's own result, so no call site has
    /// to remember to preserve the refusal.
    #[must_use]
    pub fn from_supervisor_result(answer: Result<&PlanningGraph, &SupervisorError>) -> Self {
        match answer {
            Ok(graph) => Self::classify(Ok(graph)),
            Err(error) => Self::classify(Err(&PlanningReadFailure::from_supervisor_error(error))),
        }
    }

    #[must_use]
    pub const fn kind(&self) -> PlanningDestinationStateKind {
        match self {
            Self::Loading => PlanningDestinationStateKind::Loading,
            Self::Populated(_) => PlanningDestinationStateKind::Populated,
            Self::Empty(_) => PlanningDestinationStateKind::Empty,
            Self::Unavailable { .. } => PlanningDestinationStateKind::Unavailable,
            Self::Refused(_) => PlanningDestinationStateKind::Refused,
        }
    }

    #[must_use]
    pub const fn snapshot(&self) -> Option<&PlanningDestinationSnapshot> {
        match self {
            Self::Populated(snapshot) => Some(snapshot),
            Self::Empty(_) | Self::Loading | Self::Unavailable { .. } | Self::Refused(_) => None,
        }
    }

    /// Which revision was read, for the two states that actually read one.
    #[must_use]
    pub const fn identity(&self) -> Option<&PlanningGraphIdentity> {
        match self {
            Self::Populated(snapshot) => Some(&snapshot.identity),
            Self::Empty(identity) => Some(identity),
            Self::Loading | Self::Unavailable { .. } | Self::Refused(_) => None,
        }
    }

    /// Only a populated state has rows, and only a populated state owns a
    /// value they could come from.
    #[must_use]
    pub fn rows(&self) -> &[PlanningDestinationRow] {
        match self {
            Self::Populated(snapshot) => &snapshot.rows,
            Self::Empty(_) | Self::Loading | Self::Unavailable { .. } | Self::Refused(_) => &[],
        }
    }

    /// The heading a person reads. Distinct per state by construction: the
    /// proofs below assert all five differ.
    #[must_use]
    pub fn headline(&self) -> String {
        match self {
            Self::Loading => "Reading planning Work…".to_string(),
            Self::Populated(snapshot) => {
                let count = snapshot.rows.len();
                if count == 1 {
                    "1 planning Work item".to_string()
                } else {
                    format!("{count} planning Work items")
                }
            }
            Self::Empty(_) => "No planning Work".to_string(),
            Self::Unavailable { .. } => "Planning Work is unavailable".to_string(),
            Self::Refused(_) => "Planning Work is not served by this build".to_string(),
        }
    }

    /// The sentence under the heading. Says which of the three failing shapes
    /// this is, in words, not only in a variant name.
    #[must_use]
    pub fn detail(&self) -> String {
        match self {
            Self::Loading => {
                format!("Omega is reading {PLANNING_DESTINATION_METHOD} from omega-effectd.")
            }
            Self::Populated(snapshot) => snapshot.provenance(),
            Self::Empty(identity) => format!(
                "The All Work boundary answered and its planning graph has no Work. {}",
                identity.provenance()
            ),
            Self::Unavailable { detail } => {
                format!("The All Work boundary is part of this build but did not answer: {detail}")
            }
            Self::Refused(refusal) => refusal.detail(),
        }
    }

    /// What macOS speaks when the destination changes state.
    #[must_use]
    pub fn announcement(&self) -> String {
        format!("Planning. {}. {}", self.headline(), self.detail())
    }

    /// The debug selector suffix, so a UI proof can name the state it found.
    #[must_use]
    pub fn debug_selector(&self) -> String {
        format!("omega.omega.planning.state.{}", self.kind().as_str())
    }

    /// Whether a person should be offered a retry. A refusal is not
    /// retryable: no number of retries adds a capability to an installed
    /// component.
    #[must_use]
    pub const fn is_retryable(&self) -> bool {
        matches!(
            self,
            Self::Unavailable { .. } | Self::Empty(_) | Self::Populated(_)
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use omega_effectd::all_work_contract::{
        AgentRef, AssigneeKind, Completeness, ContractDigest, ContractVersion, DelegationGrantRef,
        Freshness, HumanAssignee, IsoTimestamp, Nullable, PlanningResourceRef, PrincipalRef,
        PrivacyClass, RedactionMetadata, RunRef, SafeInteger, ShortText, SourceAuthority,
        SourceAuthorityKind, SourceRef, ThreadRef, WorkClass, WorkCursor, WorkDomain, WorkRef,
        WorkSummary,
    };

    fn graph(work: Vec<WorkSnapshot>) -> PlanningGraph {
        PlanningGraph {
            contract_version: ContractVersion::OpenagentsAllWorkBoundaryV1,
            graph_ref: PlanningResourceRef("planning-graph:all-work".into()),
            revision: SafeInteger(7),
            event_cursor: WorkCursor("cursor:test:1".into()),
            reconciliation_digest: ContractDigest(
                "0000000000000000000000000000000000000000000000000000000000000000".into(),
            ),
            generated_at: IsoTimestamp("2026-08-03T09:15:00Z".into()),
            resources: Vec::new(),
            work,
            planning_links: Vec::new(),
            label_links: Vec::new(),
            text_records: Vec::new(),
            release_scope_links: Vec::new(),
            source_coordinates: Vec::new(),
            projection_issues: Vec::new(),
            completeness: Completeness {
                state: CompletenessState::Complete,
                cursor: Some(Some(WorkCursor("cursor:test:1".into()))),
                gap_refs: Vec::new(),
            },
            freshness: Freshness {
                state: FreshnessState::Fresh,
                observed_at: IsoTimestamp("2026-08-03T09:15:00Z".into()),
                source_updated_at: None,
            },
        }
    }

    fn work_snapshot(work_ref: &str, assigned: bool, delegated: bool) -> WorkSnapshot {
        WorkSnapshot {
            summary: WorkSummary {
                contract_version: ContractVersion::OpenagentsAllWorkBoundaryV1,
                work_ref: WorkRef(work_ref.into()),
                title: ShortText("Cut and verify the candidate".into()),
                description: None,
                domain: WorkDomain::Development,
                work_class: WorkClass::Task,
                state: WorkState::Planned,
                priority: WorkPriority::High,
                owner_ref: PrincipalRef("principal:organization:openagents".into()),
                assignee: Nullable(assigned.then(|| HumanAssignee {
                    kind: AssigneeKind::Human,
                    principal_ref: PrincipalRef("principal:human:owner".into()),
                })),
                agent_delegate: delegated.then(|| {
                    Some(AgentDelegate {
                        agent_ref: AgentRef("agent:omega:coder".into()),
                        delegation_grant_ref: DelegationGrantRef("grant:omega:1".into()),
                        generation: SafeInteger(1),
                    })
                }),
                portfolio: None,
                source_authority: SourceAuthority {
                    kind: SourceAuthorityKind::ImportedReadOnly,
                    source_ref: SourceRef("github:openagentsinc-omega:160".into()),
                    adapter_version: ShortText("github-bootstrap-v1".into()),
                    writable: false,
                },
                revision: SafeInteger(1),
                updated_at: IsoTimestamp("2026-08-03T05:00:00Z".into()),
                freshness: Freshness {
                    state: FreshnessState::Fresh,
                    observed_at: IsoTimestamp("2026-08-03T09:15:00Z".into()),
                    source_updated_at: None,
                },
                completeness: Completeness {
                    state: CompletenessState::Complete,
                    cursor: Some(Some(WorkCursor("cursor:test:1".into()))),
                    gap_refs: Vec::new(),
                },
                redaction: RedactionMetadata {
                    privacy_class: PrivacyClass::Organization,
                    redacted_field_count: SafeInteger(0),
                    policy_ref: SourceRef("policy:all-work:public".into()),
                },
            },
            issue: None,
            relations: Vec::new(),
            thread_refs: vec![ThreadRef("thread:omega:1".into())],
            session_refs: Vec::new(),
            agent_session_refs: Vec::new(),
            agent_activity_refs: Vec::new(),
            run_refs: vec![RunRef("run:omega:1".into())],
            session_projections: None,
            agent_activity_projections: None,
            intent_refs: Vec::new(),
            event_refs: Vec::new(),
            receipt_refs: Vec::new(),
            evidence_refs: Vec::new(),
            verification_refs: Vec::new(),
            owner_disposition_refs: Vec::new(),
        }
    }

    fn every_state() -> Vec<PlanningDestinationState> {
        vec![
            PlanningDestinationState::Loading,
            PlanningDestinationState::classify(Ok(&graph(vec![work_snapshot(
                "work:github:openagentsinc-omega:160",
                true,
                true,
            )]))),
            PlanningDestinationState::classify(Ok(&graph(Vec::new()))),
            PlanningDestinationState::classify(Err(&PlanningReadFailure::unavailable(
                "omega-effectd closed stdout",
            ))),
            PlanningDestinationState::classify(Err(&PlanningReadFailure::Refused(
                PlanningBoundaryRefusal::CapabilityWithheld {
                    method: PLANNING_DESTINATION_METHOD.into(),
                    capability: PLANNING_DESTINATION_METHOD.into(),
                },
            ))),
        ]
    }

    #[test]
    fn an_answered_graph_with_no_work_is_empty_and_not_unavailable() {
        let state = PlanningDestinationState::classify(Ok(&graph(Vec::new())));
        assert_eq!(state.kind(), PlanningDestinationStateKind::Empty);
        assert_ne!(state.kind(), PlanningDestinationStateKind::Unavailable);
        assert_ne!(state.kind(), PlanningDestinationStateKind::Refused);
        assert!(
            state.identity().is_some(),
            "empty still knows which revision it read"
        );
        assert!(
            state.snapshot().is_none(),
            "empty owns no value rows could be drawn from"
        );
    }

    #[test]
    fn an_answered_graph_with_work_is_populated() {
        let state = PlanningDestinationState::classify(Ok(&graph(vec![work_snapshot(
            "work:github:openagentsinc-omega:160",
            true,
            true,
        )])));
        assert_eq!(state.kind(), PlanningDestinationStateKind::Populated);
        assert_eq!(state.rows().len(), 1);
    }

    #[test]
    fn an_absent_boundary_is_refused_and_names_the_boundary() {
        let state = PlanningDestinationState::from_supervisor_result(Err(
            &SupervisorError::AllWorkBoundaryAbsent {
                method: "planning.graph.read",
            },
        ));
        assert_eq!(state.kind(), PlanningDestinationStateKind::Refused);
        assert!(
            state.detail().contains("All Work boundary"),
            "a refusal must name the absent boundary: {}",
            state.detail()
        );
        assert!(state.detail().contains("planning.graph.read"));
        assert!(state.rows().is_empty(), "a refusal draws no rows");
    }

    #[test]
    fn a_withheld_capability_is_refused_and_names_the_capability() {
        let state = PlanningDestinationState::from_supervisor_result(Err(
            &SupervisorError::AllWorkCapabilityWithheld {
                method: "planning.graph.read",
                capability: "planning.graph.read".into(),
            },
        ));
        assert_eq!(state.kind(), PlanningDestinationStateKind::Refused);
        match &state {
            PlanningDestinationState::Refused(refusal) => {
                assert_eq!(refusal.absent_boundary(), "planning.graph.read");
            }
            other => panic!("expected a refusal, got {other:?}"),
        }
    }

    #[test]
    fn a_transport_failure_is_unavailable_and_never_refused() {
        let state =
            PlanningDestinationState::from_supervisor_result(Err(&SupervisorError::Protocol {
                code: omega_effectd::ProtocolErrorCode::Unavailable,
                message: "omega-effectd closed stdout".into(),
            }));
        assert_eq!(state.kind(), PlanningDestinationStateKind::Unavailable);
        assert!(state.detail().contains("closed stdout"));
        assert!(
            !state.detail().contains("Reinstall"),
            "a transport failure must not read as a packaging defect"
        );
    }

    #[test]
    fn every_state_is_distinguishable_from_every_other() {
        let states = every_state();
        assert_eq!(
            states.len(),
            PlanningDestinationStateKind::all().len(),
            "every declared state kind must be constructible"
        );
        for (index, left) in states.iter().enumerate() {
            for (other_index, right) in states.iter().enumerate() {
                if index == other_index {
                    continue;
                }
                assert_ne!(left.kind(), right.kind(), "two states share a kind");
                assert_ne!(
                    left.headline(),
                    right.headline(),
                    "two states share a headline: {:?} and {:?}",
                    left.kind(),
                    right.kind()
                );
                assert_ne!(
                    left.detail(),
                    right.detail(),
                    "two states share a detail sentence: {:?} and {:?}",
                    left.kind(),
                    right.kind()
                );
                assert_ne!(
                    left.debug_selector(),
                    right.debug_selector(),
                    "two states share a debug selector"
                );
                assert_ne!(
                    left.announcement(),
                    right.announcement(),
                    "two states are spoken identically"
                );
            }
        }
    }

    #[test]
    fn only_a_populated_state_yields_rows() {
        for state in every_state() {
            let expected_rows = matches!(state.kind(), PlanningDestinationStateKind::Populated);
            assert_eq!(
                !state.rows().is_empty(),
                expected_rows,
                "{:?} drew the wrong number of rows",
                state.kind()
            );
        }
    }

    #[test]
    fn a_refusal_is_never_offered_as_retryable() {
        for state in every_state() {
            let retryable = state.is_retryable();
            match state.kind() {
                PlanningDestinationStateKind::Refused | PlanningDestinationStateKind::Loading => {
                    assert!(!retryable, "{:?} must not offer a retry", state.kind());
                }
                _ => assert!(retryable, "{:?} must offer a retry", state.kind()),
            }
        }
    }

    #[test]
    fn a_row_keeps_assignee_delegate_thread_and_run_apart() {
        let state = PlanningDestinationState::classify(Ok(&graph(vec![work_snapshot(
            "work:github:openagentsinc-omega:160",
            true,
            true,
        )])));
        let row = state.rows().first().expect("one row");
        assert_eq!(row.assignee_ref.as_deref(), Some("principal:human:owner"));
        assert_eq!(row.agent_delegate_ref.as_deref(), Some("agent:omega:coder"));
        assert_eq!(row.thread_refs, vec!["thread:omega:1".to_string()]);
        assert_eq!(row.run_refs, vec!["run:omega:1".to_string()]);
        assert!(row.session_refs.is_empty());
        assert!(row.agent_session_refs.is_empty());
        assert_ne!(
            row.assignee_ref, row.agent_delegate_ref,
            "an assignee and an agent delegate must not collapse"
        );
    }

    #[test]
    fn an_unassigned_undelegated_row_says_so_rather_than_borrowing_the_other() {
        let state = PlanningDestinationState::classify(Ok(&graph(vec![work_snapshot(
            "work:github:openagentsinc-omega:161",
            false,
            false,
        )])));
        let row = state.rows().first().expect("one row");
        assert_eq!(row.assignee_ref, None);
        assert_eq!(row.agent_delegate_ref, None);
        let label = row.accessibility_label();
        assert!(label.contains("no assignee"), "{label}");
        assert!(label.contains("no agent delegate"), "{label}");
    }

    #[test]
    fn an_assigned_but_undelegated_row_never_borrows_the_assignee_as_its_delegate() {
        let state = PlanningDestinationState::classify(Ok(&graph(vec![work_snapshot(
            "work:github:openagentsinc-omega:162",
            true,
            false,
        )])));
        let row = state.rows().first().expect("one row");
        assert_eq!(row.assignee_ref.as_deref(), Some("principal:human:owner"));
        assert_eq!(
            row.agent_delegate_ref, None,
            "an assigned Work item with no delegate must not report its assignee as one"
        );
        let label = row.accessibility_label();
        assert!(label.contains("no agent delegate"), "{label}");
    }

    #[test]
    fn a_delegated_but_unassigned_row_never_borrows_the_delegate_as_its_assignee() {
        let state = PlanningDestinationState::classify(Ok(&graph(vec![work_snapshot(
            "work:github:openagentsinc-omega:163",
            false,
            true,
        )])));
        let row = state.rows().first().expect("one row");
        assert_eq!(row.assignee_ref, None);
        assert_eq!(row.agent_delegate_ref.as_deref(), Some("agent:omega:coder"));
        let label = row.accessibility_label();
        assert!(label.contains("no assignee"), "{label}");
    }

    #[test]
    fn no_state_but_populated_owns_a_snapshot_rows_could_come_from() {
        for state in every_state() {
            let populated = matches!(state.kind(), PlanningDestinationStateKind::Populated);
            assert_eq!(
                state.snapshot().is_some(),
                populated,
                "{:?} owns the wrong payload",
                state.kind()
            );
        }
    }

    #[test]
    fn the_projected_row_carries_the_source_authority_rather_than_inventing_writability() {
        let state = PlanningDestinationState::classify(Ok(&graph(vec![work_snapshot(
            "work:github:openagentsinc-omega:160",
            true,
            true,
        )])));
        let row = state.rows().first().expect("one row");
        assert_eq!(row.source_authority_kind, "imported_read_only");
        assert!(!row.writable);
    }
}
