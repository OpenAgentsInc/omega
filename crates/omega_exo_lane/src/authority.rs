//! One-turn authority for Exo self-modification. omega#87, Tier C.
//!
//! A lane pin does not grant this authority. The host must show the exact
//! observed capabilities to a person and mint one grant for one turn. The
//! grant is consumed immediately before the prompt crosses ACP.

use serde::{Deserialize, Serialize};
use std::cell::Cell;

/// One exact tool module observed in Exo's configuration.
#[derive(Clone, Debug, Deserialize, PartialEq, Eq, PartialOrd, Ord, Serialize)]
pub struct ObservedToolModule {
    pub path: String,
    pub digest: String,
}

/// One exact read-write mount observed in Exo's effective configuration.
#[derive(Clone, Debug, Deserialize, PartialEq, Eq, PartialOrd, Ord, Serialize)]
pub struct ObservedReadWriteMount {
    pub host_path: String,
    pub mount_path: String,
}

/// Everything that can widen a self-modifying Exo turn.
#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
pub struct ObservedExoCapabilityState {
    pub source_commit: String,
    pub source_tree: String,
    pub binary_digest: String,
    pub agent: String,
    pub conversation: String,
    pub generation: u64,
    pub agent_authored_tools: bool,
    pub tool_modules: Vec<ObservedToolModule>,
    pub read_write_mounts: Vec<ObservedReadWriteMount>,
}

/// The closed set of self-modification authority a person can grant.
#[derive(Clone, Debug, Deserialize, PartialEq, Eq, PartialOrd, Ord, Serialize)]
pub enum ExoSelfModificationCapability {
    AgentAuthoredTools,
    ToolModule(ObservedToolModule),
    ReadWriteMount(ObservedReadWriteMount),
}

impl ObservedExoCapabilityState {
    #[must_use]
    pub fn requested_capabilities(&self) -> Vec<ExoSelfModificationCapability> {
        let mut capabilities = Vec::new();
        if self.agent_authored_tools {
            capabilities.push(ExoSelfModificationCapability::AgentAuthoredTools);
        }
        capabilities.extend(
            self.tool_modules
                .iter()
                .cloned()
                .map(ExoSelfModificationCapability::ToolModule),
        );
        capabilities.extend(
            self.read_write_mounts
                .iter()
                .cloned()
                .map(ExoSelfModificationCapability::ReadWriteMount),
        );
        capabilities
    }
}

/// The exact request shown in the confirmation surface.
#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
pub struct ExoSelfModificationGrantRequest {
    pub objective: String,
    pub turn_ref: String,
    pub observed: ObservedExoCapabilityState,
    pub capabilities: Vec<ExoSelfModificationCapability>,
    pub expires_at_ms: u64,
}

/// Only a visible, dedicated human action can supply this origin.
#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq, Serialize)]
pub enum ExoSelfModificationConsentOrigin {
    HumanConfirmationDialog,
}

/// A one-use authority decision.
#[derive(Debug)]
pub struct ExoSelfModificationGrant {
    request: ExoSelfModificationGrantRequest,
    origin: ExoSelfModificationConsentOrigin,
    consumed: Cell<bool>,
}

/// Why a grant did not authorize the send.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ExoGrantRefusal {
    EmptyObjective,
    NoSelfModificationRequested,
    CapabilityMismatch,
    Expired,
    ConfigurationDrift,
    TurnMismatch,
    AlreadyConsumed,
}

/// Durable input for the host's allow/refuse receipt.
#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
pub struct ExoSelfModificationReceipt {
    pub objective: String,
    pub turn_ref: String,
    pub generation: u64,
    pub expires_at_ms: u64,
    pub origin: ExoSelfModificationConsentOrigin,
    pub capabilities: Vec<ExoSelfModificationCapability>,
    pub observed: ObservedExoCapabilityState,
}

impl ExoSelfModificationGrant {
    /// The exact request this grant can authorize.
    #[must_use]
    pub fn request(&self) -> &ExoSelfModificationGrantRequest {
        &self.request
    }

    /// Mint a grant after the dedicated confirmation action.
    pub fn mint(
        request: ExoSelfModificationGrantRequest,
        origin: ExoSelfModificationConsentOrigin,
        now_ms: u64,
    ) -> Result<Self, ExoGrantRefusal> {
        if request.objective.trim().is_empty() {
            return Err(ExoGrantRefusal::EmptyObjective);
        }
        let observed = request.observed.requested_capabilities();
        if observed.is_empty() {
            return Err(ExoGrantRefusal::NoSelfModificationRequested);
        }
        if request.capabilities != observed {
            return Err(ExoGrantRefusal::CapabilityMismatch);
        }
        if now_ms >= request.expires_at_ms {
            return Err(ExoGrantRefusal::Expired);
        }
        Ok(Self {
            request,
            origin,
            consumed: Cell::new(false),
        })
    }

    /// Consume the grant immediately before the ACP prompt.
    pub fn consume(
        &self,
        current: &ObservedExoCapabilityState,
        turn_ref: &str,
        now_ms: u64,
    ) -> Result<ExoSelfModificationReceipt, ExoGrantRefusal> {
        if self.consumed.get() {
            return Err(ExoGrantRefusal::AlreadyConsumed);
        }
        if now_ms >= self.request.expires_at_ms {
            return Err(ExoGrantRefusal::Expired);
        }
        if current != &self.request.observed {
            return Err(ExoGrantRefusal::ConfigurationDrift);
        }
        if turn_ref != self.request.turn_ref {
            return Err(ExoGrantRefusal::TurnMismatch);
        }
        self.consumed.set(true);
        Ok(ExoSelfModificationReceipt {
            objective: self.request.objective.clone(),
            turn_ref: self.request.turn_ref.clone(),
            generation: current.generation,
            expires_at_ms: self.request.expires_at_ms,
            origin: self.origin,
            capabilities: self.request.capabilities.clone(),
            observed: current.clone(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn observed() -> ObservedExoCapabilityState {
        ObservedExoCapabilityState {
            source_commit: "commit".into(),
            source_tree: "tree".into(),
            binary_digest: "sha256:bytes".into(),
            agent: "agent".into(),
            conversation: "conversation".into(),
            generation: 7,
            agent_authored_tools: true,
            tool_modules: vec![ObservedToolModule {
                path: "/tools/guardian.ts".into(),
                digest: "sha256:module".into(),
            }],
            read_write_mounts: vec![ObservedReadWriteMount {
                host_path: "/host/exo".into(),
                mount_path: "/workspace/exo".into(),
            }],
        }
    }

    fn request() -> ExoSelfModificationGrantRequest {
        let observed = observed();
        ExoSelfModificationGrantRequest {
            objective: "Edit, verify, and restart Exo.".into(),
            turn_ref: "turn-1".into(),
            capabilities: observed.requested_capabilities(),
            observed,
            expires_at_ms: 200,
        }
    }

    #[test]
    fn exact_grant_is_one_use() {
        let grant = ExoSelfModificationGrant::mint(
            request(),
            ExoSelfModificationConsentOrigin::HumanConfirmationDialog,
            100,
        )
        .expect("grant");
        assert!(grant.consume(&observed(), "turn-1", 101).is_ok());
        assert_eq!(
            grant.consume(&observed(), "turn-1", 102),
            Err(ExoGrantRefusal::AlreadyConsumed)
        );
    }

    #[test]
    fn every_observed_field_is_generation_fenced() {
        let mutations: [fn(&mut ObservedExoCapabilityState); 8] = [
            |state: &mut ObservedExoCapabilityState| state.source_commit.push('x'),
            |state: &mut ObservedExoCapabilityState| state.source_tree.push('x'),
            |state: &mut ObservedExoCapabilityState| state.binary_digest.push('x'),
            |state: &mut ObservedExoCapabilityState| state.agent.push('x'),
            |state: &mut ObservedExoCapabilityState| state.conversation.push('x'),
            |state: &mut ObservedExoCapabilityState| state.generation += 1,
            |state: &mut ObservedExoCapabilityState| state.tool_modules[0].digest.push('x'),
            |state: &mut ObservedExoCapabilityState| {
                state.read_write_mounts[0].mount_path.push('x')
            },
        ];
        for mutate in mutations {
            let grant = ExoSelfModificationGrant::mint(
                request(),
                ExoSelfModificationConsentOrigin::HumanConfirmationDialog,
                100,
            )
            .expect("grant");
            let mut changed = observed();
            mutate(&mut changed);
            assert_eq!(
                grant.consume(&changed, "turn-1", 101),
                Err(ExoGrantRefusal::ConfigurationDrift)
            );
        }
    }

    #[test]
    fn expired_or_widened_requests_refuse() {
        assert!(matches!(
            ExoSelfModificationGrant::mint(
                request(),
                ExoSelfModificationConsentOrigin::HumanConfirmationDialog,
                200
            ),
            Err(ExoGrantRefusal::Expired)
        ));
        let mut widened = request();
        widened.capabilities.pop();
        assert!(matches!(
            ExoSelfModificationGrant::mint(
                widened,
                ExoSelfModificationConsentOrigin::HumanConfirmationDialog,
                100
            ),
            Err(ExoGrantRefusal::CapabilityMismatch)
        ));
    }

    #[test]
    fn a_grant_cannot_move_to_another_turn() {
        let grant = ExoSelfModificationGrant::mint(
            request(),
            ExoSelfModificationConsentOrigin::HumanConfirmationDialog,
            100,
        )
        .expect("grant");
        assert_eq!(
            grant.consume(&observed(), "turn-2", 101),
            Err(ExoGrantRefusal::TurnMismatch)
        );
        assert!(grant.consume(&observed(), "turn-1", 102).is_ok());
    }
}
