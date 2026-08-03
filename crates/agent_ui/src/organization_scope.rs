//! Generation-fenced Organization membership and selection semantics.
//!
//! This module does not discover or grant membership. A source adapter must
//! supply a verified membership bound to the exact selected account,
//! principal, and account generation. Scope selection cannot publish until
//! every prior-scope consumer acknowledges its clear fence.

use std::collections::BTreeSet;

use omega_effectd::all_work_contract::{OrganizationRef, PrincipalRef};
use omega_identity::AccountRef;
use thiserror::Error;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum OrganizationMembershipState {
    Verified,
    Stale,
    Revoked,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OrganizationMembershipProjection {
    pub membership_ref: String,
    pub account_ref: AccountRef,
    pub account_generation: u64,
    pub principal_ref: PrincipalRef,
    pub organization_ref: OrganizationRef,
    pub display_name: String,
    pub source_revision: u64,
    pub state: OrganizationMembershipState,
}

impl OrganizationMembershipProjection {
    pub fn validate_for(
        &self,
        account_ref: &AccountRef,
        account_generation: u64,
        principal_ref: &PrincipalRef,
    ) -> Result<(), OrganizationScopeError> {
        let display_name = self.display_name.trim();
        let lower_display_name = display_name.to_ascii_lowercase();
        let secret_shaped_name = (display_name.len() == 64
            && display_name.bytes().all(|byte| byte.is_ascii_hexdigit()))
            || lower_display_name.contains("nsec1")
            || lower_display_name.contains("ncryptsec1");
        if self.membership_ref.is_empty()
            || self.membership_ref.len() > 256
            || !self.membership_ref.starts_with("membership:")
            || !self.membership_ref.bytes().all(|byte| {
                byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'-' | b'_' | b':')
            })
            || display_name.is_empty()
            || display_name != self.display_name
            || display_name.chars().count() > 64
            || display_name.chars().any(char::is_control)
            || secret_shaped_name
            || self.account_generation == 0
            || self.source_revision == 0
        {
            return Err(OrganizationScopeError::InvalidMembership);
        }
        if &self.account_ref != account_ref
            || self.account_generation != account_generation
            || &self.principal_ref != principal_ref
        {
            return Err(OrganizationScopeError::StaleAccountFence);
        }
        match self.state {
            OrganizationMembershipState::Verified => Ok(()),
            OrganizationMembershipState::Stale => Err(OrganizationScopeError::StaleMembership),
            OrganizationMembershipState::Revoked => Err(OrganizationScopeError::RevokedMembership),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum OrganizationScopeConsumer {
    Navigation,
    Caches,
    Counts,
    Search,
    Recents,
    Work,
    Threads,
    Activity,
}

impl OrganizationScopeConsumer {
    pub const ALL: [Self; 8] = [
        Self::Navigation,
        Self::Caches,
        Self::Counts,
        Self::Search,
        Self::Recents,
        Self::Work,
        Self::Threads,
        Self::Activity,
    ];
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OrganizationScopeClearReceipt {
    pub switch_generation: u64,
    pub account_ref: AccountRef,
    pub account_generation: u64,
    pub consumer: OrganizationScopeConsumer,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OrganizationScopeSwitch {
    switch_generation: u64,
    account_ref: AccountRef,
    account_generation: u64,
    from_organization_ref: Option<OrganizationRef>,
    target: OrganizationMembershipProjection,
    cleared: BTreeSet<OrganizationScopeConsumer>,
}

impl OrganizationScopeSwitch {
    pub fn begin(
        switch_generation: u64,
        account_ref: AccountRef,
        account_generation: u64,
        principal_ref: &PrincipalRef,
        from_organization_ref: Option<OrganizationRef>,
        target: OrganizationMembershipProjection,
    ) -> Result<Self, OrganizationScopeError> {
        if switch_generation == 0 || account_generation == 0 {
            return Err(OrganizationScopeError::InvalidGeneration);
        }
        target.validate_for(&account_ref, account_generation, principal_ref)?;
        if from_organization_ref.as_ref() == Some(&target.organization_ref) {
            return Err(OrganizationScopeError::AlreadySelected);
        }
        Ok(Self {
            switch_generation,
            account_ref,
            account_generation,
            from_organization_ref,
            target,
            cleared: BTreeSet::new(),
        })
    }

    pub fn from_organization_ref(&self) -> Option<&OrganizationRef> {
        self.from_organization_ref.as_ref()
    }

    pub fn target(&self) -> &OrganizationMembershipProjection {
        &self.target
    }

    pub fn acknowledge_clear(
        &mut self,
        receipt: OrganizationScopeClearReceipt,
    ) -> Result<(), OrganizationScopeError> {
        if receipt.switch_generation != self.switch_generation
            || receipt.account_ref != self.account_ref
            || receipt.account_generation != self.account_generation
        {
            return Err(OrganizationScopeError::StaleClearReceipt);
        }
        if !self.cleared.insert(receipt.consumer) {
            return Err(OrganizationScopeError::DuplicateClearReceipt);
        }
        Ok(())
    }

    pub fn remaining_consumers(&self) -> Vec<OrganizationScopeConsumer> {
        OrganizationScopeConsumer::ALL
            .into_iter()
            .filter(|consumer| !self.cleared.contains(consumer))
            .collect()
    }

    pub fn commit(
        self,
        active_account_ref: &AccountRef,
        active_account_generation: u64,
        principal_ref: &PrincipalRef,
    ) -> Result<OrganizationMembershipProjection, OrganizationScopeError> {
        if active_account_ref != &self.account_ref
            || active_account_generation != self.account_generation
        {
            return Err(OrganizationScopeError::StaleAccountFence);
        }
        self.target
            .validate_for(active_account_ref, active_account_generation, principal_ref)?;
        if !self.remaining_consumers().is_empty() {
            return Err(OrganizationScopeError::ScopeNotCleared);
        }
        Ok(self.target)
    }
}

#[derive(Clone, Debug, Error, Eq, PartialEq)]
pub enum OrganizationScopeError {
    #[error("Organization membership is invalid")]
    InvalidMembership,
    #[error("Organization membership belongs to another account generation")]
    StaleAccountFence,
    #[error("Organization membership is stale")]
    StaleMembership,
    #[error("Organization membership is revoked")]
    RevokedMembership,
    #[error("Organization scope generation is invalid")]
    InvalidGeneration,
    #[error("Organization is already selected")]
    AlreadySelected,
    #[error("Organization scope clear receipt is stale")]
    StaleClearReceipt,
    #[error("Organization scope clear receipt is duplicated")]
    DuplicateClearReceipt,
    #[error("prior Organization scope is not fully cleared")]
    ScopeNotCleared,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn account_ref() -> AccountRef {
        AccountRef::new("account:owner").expect("account ref")
    }

    fn principal_ref() -> PrincipalRef {
        PrincipalRef::try_from("principal:nostr:owner".to_string()).expect("principal ref")
    }

    fn membership(state: OrganizationMembershipState) -> OrganizationMembershipProjection {
        OrganizationMembershipProjection {
            membership_ref: "membership:openagents:owner".into(),
            account_ref: account_ref(),
            account_generation: 7,
            principal_ref: principal_ref(),
            organization_ref: OrganizationRef::try_from("organization:openagents".to_string())
                .expect("organization ref"),
            display_name: "OpenAgents".into(),
            source_revision: 3,
            state,
        }
    }

    #[test]
    fn switch_cannot_publish_until_every_prior_scope_consumer_clears() {
        let mut transaction = OrganizationScopeSwitch::begin(
            11,
            account_ref(),
            7,
            &principal_ref(),
            None,
            membership(OrganizationMembershipState::Verified),
        )
        .expect("switch begins");
        assert_eq!(
            transaction
                .clone()
                .commit(&account_ref(), 7, &principal_ref()),
            Err(OrganizationScopeError::ScopeNotCleared)
        );
        for consumer in OrganizationScopeConsumer::ALL {
            transaction
                .acknowledge_clear(OrganizationScopeClearReceipt {
                    switch_generation: 11,
                    account_ref: account_ref(),
                    account_generation: 7,
                    consumer,
                })
                .expect("exact clear receipt");
        }
        assert!(transaction.remaining_consumers().is_empty());
        assert_eq!(
            transaction
                .commit(&account_ref(), 7, &principal_ref())
                .expect("fully cleared switch")
                .organization_ref
                .0,
            "organization:openagents"
        );
    }

    #[test]
    fn stale_account_or_membership_cannot_activate_an_organization() {
        assert_eq!(
            membership(OrganizationMembershipState::Stale).validate_for(
                &account_ref(),
                7,
                &principal_ref()
            ),
            Err(OrganizationScopeError::StaleMembership)
        );
        assert_eq!(
            membership(OrganizationMembershipState::Verified).validate_for(
                &account_ref(),
                8,
                &principal_ref()
            ),
            Err(OrganizationScopeError::StaleAccountFence)
        );
    }

    #[test]
    fn stale_and_duplicate_clear_receipts_are_rejected() {
        let mut transaction = OrganizationScopeSwitch::begin(
            11,
            account_ref(),
            7,
            &principal_ref(),
            None,
            membership(OrganizationMembershipState::Verified),
        )
        .expect("switch begins");
        let receipt = OrganizationScopeClearReceipt {
            switch_generation: 11,
            account_ref: account_ref(),
            account_generation: 7,
            consumer: OrganizationScopeConsumer::Work,
        };
        transaction
            .acknowledge_clear(receipt.clone())
            .expect("first receipt");
        assert_eq!(
            transaction.acknowledge_clear(receipt),
            Err(OrganizationScopeError::DuplicateClearReceipt)
        );
        assert_eq!(
            transaction.acknowledge_clear(OrganizationScopeClearReceipt {
                switch_generation: 10,
                account_ref: account_ref(),
                account_generation: 7,
                consumer: OrganizationScopeConsumer::Threads,
            }),
            Err(OrganizationScopeError::StaleClearReceipt)
        );
    }

    #[test]
    fn revoked_membership_is_not_a_scope_or_authority_source() {
        assert_eq!(
            OrganizationScopeSwitch::begin(
                11,
                account_ref(),
                7,
                &principal_ref(),
                None,
                membership(OrganizationMembershipState::Revoked),
            ),
            Err(OrganizationScopeError::RevokedMembership)
        );
    }
}
