//! Deterministic, ephemeral Agent Session scenes for the gated dogfood UI.
//!
//! These projections are presentation fixtures. Their references are visibly
//! simulated and they grant no command, claim, lease, evidence, verification,
//! receipt, release, or Owner Disposition authority.

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum AgentSessionSimulationScene {
    #[default]
    Pending,
    Active,
    AwaitingInput,
    Error,
    Stale,
    Complete,
    Diff,
    Review,
}

impl AgentSessionSimulationScene {
    pub const ALL: [Self; 8] = [
        Self::Pending,
        Self::Active,
        Self::AwaitingInput,
        Self::Error,
        Self::Stale,
        Self::Complete,
        Self::Diff,
        Self::Review,
    ];

    pub const fn label(self) -> &'static str {
        match self {
            Self::Pending => "Pending",
            Self::Active => "Active",
            Self::AwaitingInput => "Input",
            Self::Error => "Error",
            Self::Stale => "Stale",
            Self::Complete => "Complete",
            Self::Diff => "Diff",
            Self::Review => "Review",
        }
    }

    pub const fn activity(self) -> &'static str {
        match self {
            Self::Pending => "Plan queued",
            Self::Active => "Editing bounded files",
            Self::AwaitingInput => "Question awaiting an answer",
            Self::Error => "Provider error",
            Self::Stale => "Session generation stale",
            Self::Complete => "Run completed",
            Self::Diff => "Diff attached",
            Self::Review => "Work Review requested",
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AgentSessionSimulation {
    pub simulation: bool,
    pub ephemeral: bool,
    pub work_ref: String,
    pub assignee_ref: String,
    pub agent_delegate_ref: String,
    pub delegation_grant_ref: String,
    pub repository_claim_ref: String,
    pub lease_ref: String,
    pub thread_ref: String,
    pub session_ref: String,
    pub agent_session_ref: String,
    pub run_ref: String,
    pub host_ref: String,
    pub generation: u64,
    pub plan_ref: String,
    pub activity: &'static str,
    pub question: Option<&'static str>,
    pub result: Option<&'static str>,
    pub artifact_ref: Option<String>,
    pub work_review: Option<&'static str>,
    pub effect_ref: Option<String>,
    pub receipt_ref: Option<String>,
    pub owner_disposition_ref: Option<String>,
}

impl AgentSessionSimulation {
    pub fn for_work(work_ref: &str, scene: AgentSessionSimulationScene) -> Self {
        let key = work_ref.bytes().fold(0_u64, |digest, byte| {
            digest.wrapping_mul(131).wrapping_add(byte as u64)
        });
        let prefix = format!("simulation:{key:016x}");
        Self {
            simulation: true,
            ephemeral: true,
            work_ref: work_ref.into(),
            assignee_ref: format!("{prefix}:assignee:human"),
            agent_delegate_ref: format!("{prefix}:delegate:agent"),
            delegation_grant_ref: format!("{prefix}:grant:bounded"),
            repository_claim_ref: format!("{prefix}:claim:not-live"),
            lease_ref: format!("{prefix}:lease:not-live"),
            thread_ref: format!("{prefix}:thread"),
            session_ref: format!("{prefix}:session"),
            agent_session_ref: format!("{prefix}:agent-session"),
            run_ref: format!("{prefix}:run"),
            host_ref: format!("{prefix}:host"),
            generation: 1,
            plan_ref: format!("{prefix}:plan"),
            activity: scene.activity(),
            question: matches!(scene, AgentSessionSimulationScene::AwaitingInput)
                .then_some("Choose the bounded implementation path"),
            result: matches!(scene, AgentSessionSimulationScene::Complete)
                .then_some("Simulated run result"),
            artifact_ref: matches!(
                scene,
                AgentSessionSimulationScene::Diff | AgentSessionSimulationScene::Complete
            )
            .then(|| format!("{prefix}:artifact:simulated")),
            work_review: matches!(scene, AgentSessionSimulationScene::Review)
                .then_some("Changes requested (simulation)"),
            effect_ref: matches!(
                scene,
                AgentSessionSimulationScene::Active | AgentSessionSimulationScene::Diff
            )
            .then(|| format!("{prefix}:effect:simulated")),
            receipt_ref: None,
            owner_disposition_ref: None,
        }
    }

    pub fn validate(&self) -> bool {
        self.simulation
            && self.ephemeral
            && self.generation > 0
            && self.receipt_ref.is_none()
            && self.owner_disposition_ref.is_none()
            && self.repository_claim_ref.ends_with(":not-live")
            && self.lease_ref.ends_with(":not-live")
            && self.assignee_ref != self.agent_delegate_ref
            && self.thread_ref != self.session_ref
            && self.session_ref != self.agent_session_ref
            && self.agent_session_ref != self.run_ref
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_scene_is_deterministic_and_authority_empty() {
        for scene in AgentSessionSimulationScene::ALL {
            let left = AgentSessionSimulation::for_work("work:fixture:omega:214", scene);
            let right = AgentSessionSimulation::for_work("work:fixture:omega:214", scene);
            assert_eq!(left, right);
            assert!(left.validate());
            assert!(left.receipt_ref.is_none());
            assert!(left.owner_disposition_ref.is_none());
        }
    }
}
