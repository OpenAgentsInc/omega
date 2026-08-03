//! Declarative Work Domain profiles.
//!
//! A Work Domain is a PROFILE over one canonical Work shape. It is not a
//! renderer branch, an adapter special case, or a second identity. A profile
//! declares, as data, which Work classes and states its source can emit, which
//! optional fields it admits, how urgency is decided, and the vocabulary a
//! surface uses to name a canonical state inside that domain.
//!
//! Two consequences are deliberate:
//!
//! * Adding a domain must not require new code anywhere else. If it does, the
//!   abstraction is not neutral and the abstraction is the defect.
//! * A profile can only NARROW the canonical contract. It never widens it and
//!   never invents a field, a state, or an authority the contract does not
//!   already carry.
//!
//! A domain the product has not specified yet is marked `specified: false` and
//! admits the full canonical vocabulary, so an unspecified domain fails open
//! against this table and closed against the contract, rather than silently
//! being rejected by a table nobody has filled in.

use omega_effectd::all_work_contract::{
    WorkClass, WorkDomain, WorkPriority, WorkState, WorkSummary,
};

use crate::WorkIndexError;

/// How a Work Domain decides what is urgent.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum WorkUrgency {
    /// A person or the source system declares a priority, and the surface
    /// ranks by that declaration.
    Declared,
    /// Urgency is derived only from observed state. A declared priority is
    /// refused, because there is nobody in the loop to have declared it.
    Observed,
}

/// An optional Work field a domain may admit.
///
/// Absence is not "unset". A domain that does not admit a field refuses Work
/// that carries it, so a surface can render field availability from the
/// profile instead of guessing from a null.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum WorkDomainField {
    /// A person can be accountable for this Work.
    Assignee,
    /// An agent can hold a delegation grant on this Work.
    AgentDelegate,
    /// A priority is declared rather than derived.
    DeclaredPriority,
    /// The Work belongs to a portfolio structure.
    Portfolio,
    /// The Work moves through proposed-change review.
    ChangeReview,
    /// The Work is a continuously operated service with observed health.
    ServiceHealth,
}

/// The declarative profile for one Work Domain.
pub struct WorkDomainProfile {
    pub domain: WorkDomain,
    pub label: &'static str,
    /// `false` means the product has not yet specified this domain; the
    /// profile then admits the full canonical vocabulary.
    pub specified: bool,
    pub urgency: WorkUrgency,
    admitted_classes: &'static [WorkClass],
    admitted_states: &'static [WorkState],
    admitted_fields: &'static [WorkDomainField],
    /// Domain vocabulary for a canonical state. A state absent from this table
    /// keeps its neutral canonical name.
    state_labels: &'static [(WorkState, &'static str)],
}

impl WorkDomainProfile {
    pub fn admits_class(&self, class: &WorkClass) -> bool {
        !self.specified || self.admitted_classes.contains(class)
    }

    pub fn admits_state(&self, state: &WorkState) -> bool {
        !self.specified || self.admitted_states.contains(state)
    }

    pub fn admits_field(&self, field: WorkDomainField) -> bool {
        !self.specified || self.admitted_fields.contains(&field)
    }

    pub fn admitted_classes(&self) -> &'static [WorkClass] {
        self.admitted_classes
    }

    pub fn admitted_states(&self) -> &'static [WorkState] {
        self.admitted_states
    }

    /// The domain's name for a canonical state.
    ///
    /// This is vocabulary, not a second state machine. `state_label` never
    /// changes which canonical state a row is in, so cross-domain filtering,
    /// counting, and sorting stay on the canonical value.
    pub fn state_label(&self, state: &WorkState) -> &'static str {
        self.state_labels
            .iter()
            .find_map(|(candidate, label)| (candidate == state).then_some(*label))
            .unwrap_or_else(|| canonical_state_label(state))
    }

    /// Refuse a summary its own domain does not admit.
    ///
    /// Every native adapter runs this, so "the domain is a profile" is an
    /// enforced property of the index rather than a claim in a comment.
    pub fn admit(&self, summary: &WorkSummary) -> Result<(), WorkIndexError> {
        if summary.domain != self.domain {
            return Err(WorkIndexError::InvalidContract(format!(
                "Work Domain profile {:?} cannot admit Work in domain {:?}",
                self.domain, summary.domain
            )));
        }
        if !self.specified {
            return Ok(());
        }
        if !self.admits_class(&summary.work_class) {
            return Err(WorkIndexError::InvalidContract(format!(
                "Work Domain {} does not admit Work class {:?}",
                self.label, summary.work_class
            )));
        }
        if !self.admits_state(&summary.state) {
            return Err(WorkIndexError::InvalidContract(format!(
                "Work Domain {} does not admit Work state {:?}",
                self.label, summary.state
            )));
        }
        if !self.admits_field(WorkDomainField::Assignee) && summary.assignee.0.is_some() {
            return Err(WorkIndexError::InvalidContract(format!(
                "Work Domain {} does not admit an assignee",
                self.label
            )));
        }
        if !self.admits_field(WorkDomainField::AgentDelegate)
            && summary.agent_delegate.as_ref().is_some_and(Option::is_some)
        {
            return Err(WorkIndexError::InvalidContract(format!(
                "Work Domain {} does not admit an agent delegate",
                self.label
            )));
        }
        if !self.admits_field(WorkDomainField::Portfolio)
            && summary.portfolio.as_ref().is_some_and(Option::is_some)
        {
            return Err(WorkIndexError::InvalidContract(format!(
                "Work Domain {} does not admit a portfolio context",
                self.label
            )));
        }
        let declares_priority = !matches!(summary.priority, WorkPriority::None);
        if declares_priority
            && (self.urgency == WorkUrgency::Observed
                || !self.admits_field(WorkDomainField::DeclaredPriority))
        {
            return Err(WorkIndexError::InvalidContract(format!(
                "Work Domain {} derives urgency from observed state and does not admit a \
                 declared priority",
                self.label
            )));
        }
        Ok(())
    }
}

/// The neutral canonical name for a Work state.
pub const fn canonical_state_label(state: &WorkState) -> &'static str {
    match state {
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

const ALL_STATES: &[WorkState] = &[
    WorkState::Triage,
    WorkState::Planned,
    WorkState::Active,
    WorkState::Waiting,
    WorkState::Blocked,
    WorkState::Failed,
    WorkState::Completed,
    WorkState::Canceled,
    WorkState::Archived,
];

const ALL_CLASSES: &[WorkClass] = &[
    WorkClass::Task,
    WorkClass::Change,
    WorkClass::Run,
    WorkClass::Incident,
    WorkClass::Review,
    WorkClass::Case,
    WorkClass::Investigation,
    WorkClass::Job,
    WorkClass::Outcome,
];

const ALL_FIELDS: &[WorkDomainField] = &[
    WorkDomainField::Assignee,
    WorkDomainField::AgentDelegate,
    WorkDomainField::DeclaredPriority,
    WorkDomainField::Portfolio,
    WorkDomainField::ChangeReview,
    WorkDomainField::ServiceHealth,
];

/// Every profile, in canonical domain order.
pub static WORK_DOMAIN_PROFILES: &[WorkDomainProfile] = &[
    WorkDomainProfile {
        domain: WorkDomain::General,
        label: "General",
        specified: true,
        urgency: WorkUrgency::Declared,
        admitted_classes: &[WorkClass::Task],
        admitted_states: ALL_STATES,
        admitted_fields: &[
            WorkDomainField::Assignee,
            WorkDomainField::AgentDelegate,
            WorkDomainField::DeclaredPriority,
        ],
        state_labels: &[],
    },
    WorkDomainProfile {
        domain: WorkDomain::Development,
        label: "Development",
        specified: true,
        urgency: WorkUrgency::Declared,
        admitted_classes: &[
            WorkClass::Task,
            WorkClass::Change,
            WorkClass::Review,
            WorkClass::Run,
            WorkClass::Outcome,
        ],
        admitted_states: ALL_STATES,
        admitted_fields: &[
            WorkDomainField::Assignee,
            WorkDomainField::AgentDelegate,
            WorkDomainField::DeclaredPriority,
            WorkDomainField::Portfolio,
            WorkDomainField::ChangeReview,
        ],
        state_labels: &[],
    },
    WorkDomainProfile {
        domain: WorkDomain::Ci,
        label: "Continuous integration",
        specified: false,
        urgency: WorkUrgency::Declared,
        admitted_classes: ALL_CLASSES,
        admitted_states: ALL_STATES,
        admitted_fields: ALL_FIELDS,
        state_labels: &[],
    },
    WorkDomainProfile {
        domain: WorkDomain::Deployment,
        label: "Deployment",
        specified: false,
        urgency: WorkUrgency::Declared,
        admitted_classes: ALL_CLASSES,
        admitted_states: ALL_STATES,
        admitted_fields: ALL_FIELDS,
        state_labels: &[],
    },
    WorkDomainProfile {
        domain: WorkDomain::Operations,
        label: "Operations",
        specified: true,
        // A service is operated, not assigned, planned, or reviewed. Nothing
        // in this domain is proposed by a person, so there is no triage,
        // planning, cancellation, or archive state and no declared priority:
        // urgency is exactly what the service was last observed doing.
        urgency: WorkUrgency::Observed,
        admitted_classes: &[WorkClass::Job],
        admitted_states: &[
            WorkState::Active,
            WorkState::Waiting,
            WorkState::Blocked,
            WorkState::Failed,
            WorkState::Completed,
        ],
        admitted_fields: &[WorkDomainField::ServiceHealth],
        state_labels: &[
            (WorkState::Active, "Running"),
            // The canonical vocabulary has no "degraded": it is task-shaped,
            // so a service that still serves while reporting a warning has no
            // exact canonical state. `Waiting` is the closest admitted value
            // and the exact operational state stays in the domain's Blocks.
            (WorkState::Waiting, "Degraded"),
            (WorkState::Blocked, "Needs recovery"),
            (WorkState::Failed, "Unavailable"),
            (WorkState::Completed, "Stopped"),
        ],
    },
    WorkDomainProfile {
        domain: WorkDomain::Incident,
        label: "Incident",
        specified: false,
        urgency: WorkUrgency::Declared,
        admitted_classes: ALL_CLASSES,
        admitted_states: ALL_STATES,
        admitted_fields: ALL_FIELDS,
        state_labels: &[],
    },
    WorkDomainProfile {
        domain: WorkDomain::Research,
        label: "Research",
        specified: false,
        urgency: WorkUrgency::Declared,
        admitted_classes: ALL_CLASSES,
        admitted_states: ALL_STATES,
        admitted_fields: ALL_FIELDS,
        state_labels: &[],
    },
    WorkDomainProfile {
        domain: WorkDomain::Security,
        label: "Security",
        specified: true,
        urgency: WorkUrgency::Declared,
        admitted_classes: &[
            WorkClass::Case,
            WorkClass::Investigation,
            WorkClass::Run,
            WorkClass::Review,
        ],
        admitted_states: ALL_STATES,
        admitted_fields: &[WorkDomainField::DeclaredPriority],
        state_labels: &[
            (WorkState::Active, "Investigating"),
            (WorkState::Waiting, "Awaiting evidence"),
            (WorkState::Blocked, "Recovery required"),
            (WorkState::Failed, "Refused"),
            (WorkState::Completed, "Settled"),
        ],
    },
    WorkDomainProfile {
        domain: WorkDomain::DesignReview,
        label: "Design review",
        specified: false,
        urgency: WorkUrgency::Declared,
        admitted_classes: ALL_CLASSES,
        admitted_states: ALL_STATES,
        admitted_fields: ALL_FIELDS,
        state_labels: &[],
    },
    WorkDomainProfile {
        domain: WorkDomain::ServiceDelivery,
        label: "Service delivery",
        specified: false,
        urgency: WorkUrgency::Declared,
        admitted_classes: ALL_CLASSES,
        admitted_states: ALL_STATES,
        admitted_fields: ALL_FIELDS,
        state_labels: &[],
    },
    WorkDomainProfile {
        domain: WorkDomain::Data,
        label: "Data",
        specified: false,
        urgency: WorkUrgency::Declared,
        admitted_classes: ALL_CLASSES,
        admitted_states: ALL_STATES,
        admitted_fields: ALL_FIELDS,
        state_labels: &[],
    },
];

/// The profile for a Work Domain.
///
/// Every canonical domain has a profile, so this cannot fail and no caller
/// needs a fallback branch.
pub fn work_domain_profile(domain: &WorkDomain) -> &'static WorkDomainProfile {
    WORK_DOMAIN_PROFILES
        .iter()
        .find(|profile| &profile.domain == domain)
        .unwrap_or(&MISSING_PROFILE)
}

/// Only reachable if a new canonical domain is added without a profile row.
/// It admits nothing, so the gap fails closed instead of pretending.
static MISSING_PROFILE: WorkDomainProfile = WorkDomainProfile {
    domain: WorkDomain::General,
    label: "Unprofiled domain",
    specified: true,
    urgency: WorkUrgency::Observed,
    admitted_classes: &[],
    admitted_states: &[],
    admitted_fields: &[],
    state_labels: &[],
};

#[cfg(test)]
mod tests {
    use omega_effectd::all_work_contract::{
        AssigneeKind, HumanAssignee, Nullable, PrincipalRef, WorkPriority,
    };

    use super::*;
    use crate::{LOCAL_OWNER_REF, SummaryInput, make_summary};

    /// Build the summary through the same constructor every native adapter
    /// uses, so a profile assertion cannot pass against a shape the index
    /// never produces.
    fn summary(domain: WorkDomain, work_class: WorkClass, state: WorkState) -> WorkSummary {
        make_summary(SummaryInput {
            work_ref: "work:omega:profile:1".into(),
            title: "Profile fixture".into(),
            domain,
            work_class,
            state,
            priority: WorkPriority::None,
            source_ref: "service:omega:profile".into(),
            source_kind: omega_effectd::all_work_contract::SourceAuthorityKind::OmegaNative,
            adapter_version: "profile.v1".into(),
            source_writable: false,
            revision: 1,
            updated_at: "2026-08-03T00:00:00.000Z".into(),
            observed_at: "2026-08-03T00:00:00.000Z".into(),
            assignee: None,
            agent_delegate: None,
        })
        .expect("the profile fixture must be a valid canonical summary")
    }

    #[test]
    fn every_canonical_domain_has_exactly_one_profile() {
        let domains = [
            WorkDomain::General,
            WorkDomain::Development,
            WorkDomain::Ci,
            WorkDomain::Deployment,
            WorkDomain::Operations,
            WorkDomain::Incident,
            WorkDomain::Research,
            WorkDomain::Security,
            WorkDomain::DesignReview,
            WorkDomain::ServiceDelivery,
            WorkDomain::Data,
        ];
        assert_eq!(WORK_DOMAIN_PROFILES.len(), domains.len());
        for domain in &domains {
            let matches = WORK_DOMAIN_PROFILES
                .iter()
                .filter(|profile| &profile.domain == domain)
                .count();
            assert_eq!(matches, 1, "{domain:?} must have exactly one profile");
            assert_eq!(&work_domain_profile(domain).domain, domain);
        }
    }

    #[test]
    fn operations_refuses_a_declared_priority_while_security_admits_one() {
        let mut operations = summary(WorkDomain::Operations, WorkClass::Job, WorkState::Active);
        work_domain_profile(&WorkDomain::Operations)
            .admit(&operations)
            .expect("observed urgency admits an underived priority");
        operations.priority = WorkPriority::High;
        let refusal = work_domain_profile(&WorkDomain::Operations)
            .admit(&operations)
            .expect_err("observed urgency must refuse a declared priority");
        assert!(
            refusal.to_string().contains("declared priority"),
            "unexpected refusal: {refusal}"
        );

        let mut security = summary(WorkDomain::Security, WorkClass::Case, WorkState::Active);
        security.priority = WorkPriority::High;
        work_domain_profile(&WorkDomain::Security)
            .admit(&security)
            .expect("declared urgency admits a declared priority");
    }

    #[test]
    fn a_profile_refuses_a_class_state_or_field_its_domain_does_not_admit() {
        let profile = work_domain_profile(&WorkDomain::Operations);
        profile
            .admit(&summary(
                WorkDomain::Operations,
                WorkClass::Job,
                WorkState::Failed,
            ))
            .expect("an operated service may be observed unavailable");
        // The admitted row is mutated after construction, because
        // `make_summary` already refuses an inadmissible shape. That is the
        // point: the only way to reach this assertion is to bypass the gate.
        let mut wrong_class = summary(WorkDomain::Operations, WorkClass::Job, WorkState::Active);
        wrong_class.work_class = WorkClass::Task;
        assert!(
            profile.admit(&wrong_class).is_err(),
            "Operations must refuse a task class"
        );
        let mut wrong_state = summary(WorkDomain::Operations, WorkClass::Job, WorkState::Active);
        wrong_state.state = WorkState::Triage;
        assert!(
            profile.admit(&wrong_state).is_err(),
            "Operations must refuse a triage state nobody can perform"
        );
        let mut assigned = summary(WorkDomain::Operations, WorkClass::Job, WorkState::Active);
        assigned.assignee = Nullable(Some(HumanAssignee {
            kind: AssigneeKind::Human,
            principal_ref: PrincipalRef(LOCAL_OWNER_REF.into()),
        }));
        assert!(
            profile.admit(&assigned).is_err(),
            "Operations must refuse an assignee"
        );
    }

    #[test]
    fn a_profile_refuses_work_from_another_domain() {
        let refusal = work_domain_profile(&WorkDomain::Operations)
            .admit(&summary(
                WorkDomain::Security,
                WorkClass::Case,
                WorkState::Active,
            ))
            .expect_err("a profile must refuse another domain's Work");
        assert!(
            refusal.to_string().contains("cannot admit"),
            "unexpected refusal: {refusal}"
        );
    }

    #[test]
    fn an_unspecified_domain_admits_the_full_canonical_vocabulary() {
        let profile = work_domain_profile(&WorkDomain::Incident);
        assert!(!profile.specified);
        profile
            .admit(&summary(
                WorkDomain::Incident,
                WorkClass::Incident,
                WorkState::Triage,
            ))
            .expect("an unspecified domain must not be narrowed by an empty table");
    }

    #[test]
    fn domain_vocabulary_renames_a_state_without_changing_it() {
        assert_eq!(
            work_domain_profile(&WorkDomain::Operations).state_label(&WorkState::Failed),
            "Unavailable"
        );
        assert_eq!(
            work_domain_profile(&WorkDomain::Security).state_label(&WorkState::Failed),
            "Refused"
        );
        assert_eq!(
            work_domain_profile(&WorkDomain::General).state_label(&WorkState::Failed),
            canonical_state_label(&WorkState::Failed)
        );
        assert_eq!(
            work_domain_profile(&WorkDomain::Development).state_label(&WorkState::Active),
            "Active"
        );
    }
}
