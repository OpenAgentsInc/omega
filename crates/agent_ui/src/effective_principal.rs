//! Public-safe Effective Principal projection for Omega chrome.
//!
//! Account identity, signer availability, Organization membership, and Sync
//! freshness are separate facts. This module projects only the facts the local
//! account registry currently owns and leaves Organization/Sync unclaimed.

use omega_identity::{
    AccountDashboardEntry, AccountDashboardProjection, AccountLifecycleState,
    AccountRegistryService, SignerAvailability, SignerKind,
};

use crate::organization_scope::{OrganizationMembershipProjection, OrganizationScopeError};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum EffectivePrincipalState {
    LocalOnly,
    Enrolled,
    Offline,
    SignerUnavailable,
    Revoked,
    Conflict,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum EffectiveOrganizationState {
    Unavailable,
    Verified,
    Stale,
    Revoked,
    Conflict,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct EffectivePrincipalProjection {
    pub(crate) display_name: String,
    pub(crate) identity_label: String,
    pub(crate) scope_label: String,
    pub(crate) signer_label: String,
    pub(crate) principal_ref: Option<String>,
    pub(crate) organization_ref: Option<String>,
    pub(crate) organization_state: EffectiveOrganizationState,
    pub(crate) state: EffectivePrincipalState,
}

impl Default for EffectivePrincipalProjection {
    fn default() -> Self {
        Self::local_only()
    }
}

impl EffectivePrincipalProjection {
    pub(crate) fn current() -> Self {
        let registry = AccountRegistryService::for_channel(*app_identity::CHANNEL);
        match registry.inspect() {
            Ok(dashboard) => Self::from_dashboard(&dashboard),
            Err(_) => Self::conflict("Identity unavailable"),
        }
    }

    pub(crate) fn from_dashboard(dashboard: &AccountDashboardProjection) -> Self {
        Self::from_dashboard_and_membership(dashboard, None)
    }

    pub(crate) fn from_dashboard_and_membership(
        dashboard: &AccountDashboardProjection,
        membership: Option<&OrganizationMembershipProjection>,
    ) -> Self {
        let Some(active_ref) = dashboard.active.account_ref.as_ref() else {
            return Self::local_only();
        };
        let mut matching = dashboard
            .accounts
            .iter()
            .filter(|account| account.is_active && &account.account_ref == active_ref);
        let Some(account) = matching.next() else {
            return Self::conflict("Identity conflict");
        };
        if matching.next().is_some()
            || dashboard
                .accounts
                .iter()
                .any(|candidate| candidate.is_active && candidate.account_ref != *active_ref)
        {
            return Self::conflict("Identity conflict");
        }
        Self::from_account(account, dashboard.active.generation, membership)
    }

    fn from_account(
        account: &AccountDashboardEntry,
        account_generation: u64,
        membership: Option<&OrganizationMembershipProjection>,
    ) -> Self {
        let display_name = account
            .profile
            .as_ref()
            .and_then(|profile| profile.display_name.as_deref())
            .and_then(safe_display_name)
            .unwrap_or_else(|| account.identity.fingerprint().display());
        let identity_label = format!("Identity {}", account.identity.fingerprint().display());
        let signer_label = signer_label(account.signer.kind, account.signer.availability);
        let state = match account.lifecycle {
            AccountLifecycleState::Forgotten | AccountLifecycleState::ForgetPending => {
                EffectivePrincipalState::Revoked
            }
            AccountLifecycleState::Conflict
            | AccountLifecycleState::RepairRequired
            | AccountLifecycleState::Switching => EffectivePrincipalState::Conflict,
            AccountLifecycleState::Locked | AccountLifecycleState::SignedOut => {
                EffectivePrincipalState::SignerUnavailable
            }
            AccountLifecycleState::CandidateLocal
            | AccountLifecycleState::CandidateExisting
            | AccountLifecycleState::Activating => EffectivePrincipalState::LocalOnly,
            AccountLifecycleState::Active => match account.signer.availability {
                SignerAvailability::Ready => EffectivePrincipalState::Enrolled,
                SignerAvailability::Offline => EffectivePrincipalState::Offline,
                SignerAvailability::Revoked | SignerAvailability::Lost => {
                    EffectivePrincipalState::Revoked
                }
                SignerAvailability::UserApprovalRequired | SignerAvailability::Rejected => {
                    EffectivePrincipalState::SignerUnavailable
                }
            },
        };
        let principal_ref = format!(
            "principal:nostr:{}",
            account.identity.public_key_hex().as_str()
        );
        let membership = (account.lifecycle == AccountLifecycleState::Active)
            .then_some(membership)
            .flatten();
        let (scope_label, organization_ref, organization_state) =
            match omega_effectd::all_work_contract::PrincipalRef::try_from(principal_ref.clone()) {
                Ok(typed_principal_ref) => organization_projection(
                    membership,
                    &account.account_ref,
                    account_generation,
                    &typed_principal_ref,
                ),
                Err(_) => (
                    "Organization unverified".into(),
                    None,
                    EffectiveOrganizationState::Conflict,
                ),
            };
        Self {
            display_name,
            identity_label,
            scope_label,
            signer_label: signer_label.into(),
            principal_ref: Some(principal_ref),
            organization_ref,
            organization_state,
            state,
        }
    }

    fn local_only() -> Self {
        Self {
            display_name: "Local only".into(),
            identity_label: "No enrolled identity".into(),
            scope_label: "Local scope".into(),
            signer_label: "Signer unavailable".into(),
            principal_ref: None,
            organization_ref: None,
            organization_state: EffectiveOrganizationState::Unavailable,
            state: EffectivePrincipalState::LocalOnly,
        }
    }

    fn conflict(label: &str) -> Self {
        Self {
            display_name: label.into(),
            identity_label: "Principal not verified".into(),
            scope_label: "Scope unavailable".into(),
            signer_label: "Signer not trusted".into(),
            principal_ref: None,
            organization_ref: None,
            organization_state: EffectiveOrganizationState::Conflict,
            state: EffectivePrincipalState::Conflict,
        }
    }

    pub(crate) fn status_label(&self) -> &'static str {
        match self.state {
            EffectivePrincipalState::LocalOnly => "Local",
            EffectivePrincipalState::Enrolled => "Enrolled",
            EffectivePrincipalState::Offline => "Offline",
            EffectivePrincipalState::SignerUnavailable => "Signer unavailable",
            EffectivePrincipalState::Revoked => "Revoked",
            EffectivePrincipalState::Conflict => "Unverified",
        }
    }

    pub(crate) fn accessibility_label(&self) -> String {
        format!(
            "Effective principal: {}; {}; {}; {}; {}; Organization {}. Open identity settings",
            self.display_name,
            self.identity_label,
            self.scope_label,
            self.signer_label,
            self.status_label(),
            self.organization_status_label(),
        )
    }

    const fn organization_status_label(&self) -> &'static str {
        match self.organization_state {
            EffectiveOrganizationState::Unavailable => "unavailable",
            EffectiveOrganizationState::Verified => "verified",
            EffectiveOrganizationState::Stale => "stale",
            EffectiveOrganizationState::Revoked => "revoked",
            EffectiveOrganizationState::Conflict => "unverified",
        }
    }
}

fn organization_projection(
    membership: Option<&OrganizationMembershipProjection>,
    account_ref: &omega_identity::AccountRef,
    account_generation: u64,
    principal_ref: &omega_effectd::all_work_contract::PrincipalRef,
) -> (String, Option<String>, EffectiveOrganizationState) {
    let Some(membership) = membership else {
        return (
            "Local scope".into(),
            None,
            EffectiveOrganizationState::Unavailable,
        );
    };
    match membership.validate_for(account_ref, account_generation, principal_ref) {
        Ok(()) => (
            membership.display_name.clone(),
            Some(membership.organization_ref.0.clone()),
            EffectiveOrganizationState::Verified,
        ),
        Err(OrganizationScopeError::StaleMembership) => (
            "Membership stale".into(),
            None,
            EffectiveOrganizationState::Stale,
        ),
        Err(OrganizationScopeError::RevokedMembership) => (
            "Membership revoked".into(),
            None,
            EffectiveOrganizationState::Revoked,
        ),
        Err(_) => (
            "Organization unverified".into(),
            None,
            EffectiveOrganizationState::Conflict,
        ),
    }
}

fn safe_display_name(value: &str) -> Option<String> {
    let value = value.trim();
    let lower = value.to_ascii_lowercase();
    let is_hex_secret = value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit());
    (!value.is_empty()
        && value.chars().count() <= 64
        && !value.chars().any(char::is_control)
        && lower != "anonymous"
        && !lower.contains("nsec1")
        && !lower.contains("ncryptsec1")
        && !is_hex_secret)
        .then(|| value.to_string())
}

fn signer_label(kind: SignerKind, availability: SignerAvailability) -> &'static str {
    match (kind, availability) {
        (_, SignerAvailability::Offline) => "Signer offline",
        (_, SignerAvailability::Revoked) => "Signer revoked",
        (_, SignerAvailability::Lost) => "Signer lost",
        (_, SignerAvailability::Rejected) => "Signer rejected",
        (_, SignerAvailability::UserApprovalRequired) => "Signer approval required",
        (SignerKind::LocalNative, SignerAvailability::Ready) => "Local signer ready",
        (SignerKind::RemoteNip46, SignerAvailability::Ready) => "Remote signer ready",
        (SignerKind::BrowserNip07, SignerAvailability::Ready) => "Browser signer ready",
        (SignerKind::AndroidNip55, SignerAvailability::Ready) => "Android signer ready",
        (SignerKind::DeviceGrant, SignerAvailability::Ready) => "Device signer ready",
        (SignerKind::AgentGrant, SignerAvailability::Ready) => "Agent signer ready",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::organization_scope::{
        OrganizationMembershipProjection, OrganizationMembershipState,
    };
    use nostr::{Keys, SecretKey};
    use omega_identity::{
        AccountDashboardProjection, AccountProfileSummary, AccountRef, AccountRetirementState,
        AccountSignerSummary, ActiveAccountSelection, IdentityRef, PublicIdentity,
        RecoveryProtectionState,
    };

    fn dashboard(active_refs: &[&str]) -> AccountDashboardProjection {
        let accounts = active_refs
            .iter()
            .enumerate()
            .map(|(index, account_ref)| {
                let secret = format!("{}", index + 1).repeat(64);
                let keys = Keys::new(SecretKey::from_hex(secret).expect("secret key"));
                AccountDashboardEntry {
                    account_ref: AccountRef::new(*account_ref).expect("account ref"),
                    identity: PublicIdentity::from_public_key_hex(
                        IdentityRef::new(format!("identity-{index}")).expect("identity ref"),
                        keys.public_key().to_hex(),
                    )
                    .expect("public identity"),
                    fingerprint: format!("fingerprint-{index}"),
                    profile: Some(AccountProfileSummary {
                        display_name: Some(format!("Person {index}")),
                        avatar_ref: None,
                    }),
                    lifecycle: AccountLifecycleState::Active,
                    signer: AccountSignerSummary {
                        kind: SignerKind::LocalNative,
                        availability: SignerAvailability::Ready,
                        last_successful_use: None,
                    },
                    recovery: RecoveryProtectionState::Protected,
                    retirement: AccountRetirementState::NotRetired,
                    is_active: true,
                }
            })
            .collect();
        AccountDashboardProjection {
            active: ActiveAccountSelection {
                account_ref: active_refs
                    .first()
                    .map(|value| AccountRef::new(*value).expect("active account ref")),
                generation: 1,
            },
            accounts,
            pending_switch: None,
            pending_purges: Vec::new(),
        }
    }

    fn membership(
        dashboard: &AccountDashboardProjection,
        state: OrganizationMembershipState,
    ) -> OrganizationMembershipProjection {
        let account = &dashboard.accounts[0];
        OrganizationMembershipProjection {
            membership_ref: "membership:openagents:owner".into(),
            account_ref: account.account_ref.clone(),
            account_generation: dashboard.active.generation,
            principal_ref: omega_effectd::all_work_contract::PrincipalRef::try_from(format!(
                "principal:nostr:{}",
                account.identity.public_key_hex().as_str()
            ))
            .expect("principal ref"),
            organization_ref: omega_effectd::all_work_contract::OrganizationRef::try_from(
                "organization:openagents".to_string(),
            )
            .expect("organization ref"),
            display_name: "OpenAgents".into(),
            source_revision: 4,
            state,
        }
    }

    #[test]
    fn exact_active_account_is_projected_without_claiming_an_organization() {
        let projection = EffectivePrincipalProjection::from_dashboard(&dashboard(&["account-a"]));
        assert_eq!(projection.display_name, "Person 0");
        assert_eq!(projection.scope_label, "Local scope");
        assert_eq!(projection.signer_label, "Local signer ready");
        assert_eq!(projection.state, EffectivePrincipalState::Enrolled);
        assert!(
            projection
                .principal_ref
                .as_deref()
                .is_some_and(|value| value.starts_with("principal:nostr:"))
        );
        assert_eq!(projection.organization_ref, None);
        assert_eq!(
            projection.organization_state,
            EffectiveOrganizationState::Unavailable
        );
    }

    #[test]
    fn exact_verified_membership_projects_organization_without_merging_signer_authority() {
        let dashboard = dashboard(&["account-a"]);
        let membership = membership(&dashboard, OrganizationMembershipState::Verified);
        let projection = EffectivePrincipalProjection::from_dashboard_and_membership(
            &dashboard,
            Some(&membership),
        );
        assert_eq!(projection.scope_label, "OpenAgents");
        assert_eq!(
            projection.organization_ref.as_deref(),
            Some("organization:openagents")
        );
        assert_eq!(
            projection.organization_state,
            EffectiveOrganizationState::Verified
        );
        assert_eq!(projection.signer_label, "Local signer ready");
    }

    #[test]
    fn stale_or_cross_generation_membership_fails_closed() {
        let dashboard = dashboard(&["account-a"]);
        let stale = membership(&dashboard, OrganizationMembershipState::Stale);
        let projection =
            EffectivePrincipalProjection::from_dashboard_and_membership(&dashboard, Some(&stale));
        assert_eq!(projection.organization_ref, None);
        assert_eq!(projection.scope_label, "Membership stale");
        assert_eq!(
            projection.organization_state,
            EffectiveOrganizationState::Stale
        );

        let mut wrong_generation = membership(&dashboard, OrganizationMembershipState::Verified);
        wrong_generation.account_generation += 1;
        let projection = EffectivePrincipalProjection::from_dashboard_and_membership(
            &dashboard,
            Some(&wrong_generation),
        );
        assert_eq!(projection.organization_ref, None);
        assert_eq!(projection.scope_label, "Organization unverified");
        assert_eq!(
            projection.organization_state,
            EffectiveOrganizationState::Conflict
        );
    }

    #[test]
    fn local_candidate_cannot_borrow_a_verified_organization_membership() {
        let mut dashboard = dashboard(&["account-a"]);
        let membership = membership(&dashboard, OrganizationMembershipState::Verified);
        dashboard.accounts[0].lifecycle = AccountLifecycleState::CandidateLocal;
        let projection = EffectivePrincipalProjection::from_dashboard_and_membership(
            &dashboard,
            Some(&membership),
        );
        assert_eq!(projection.state, EffectivePrincipalState::LocalOnly);
        assert_eq!(projection.scope_label, "Local scope");
        assert_eq!(projection.organization_ref, None);
        assert_eq!(
            projection.organization_state,
            EffectiveOrganizationState::Unavailable
        );
    }

    #[test]
    fn conflicting_active_accounts_fail_closed() {
        let projection =
            EffectivePrincipalProjection::from_dashboard(&dashboard(&["account-a", "account-b"]));
        assert_eq!(projection.state, EffectivePrincipalState::Conflict);
        assert_eq!(projection.display_name, "Identity conflict");
        assert!(!projection.accessibility_label().contains("Person"));
    }

    #[test]
    fn unsafe_profile_names_fall_back_to_a_public_fingerprint() {
        let mut dashboard = dashboard(&["account-a"]);
        dashboard.accounts[0]
            .profile
            .as_mut()
            .expect("profile")
            .display_name = Some("line one\nline two".into());
        let projection = EffectivePrincipalProjection::from_dashboard(&dashboard);
        assert_ne!(projection.display_name, "line one\nline two");
        assert!(!projection.display_name.contains("npub"));

        dashboard.accounts[0]
            .profile
            .as_mut()
            .expect("profile")
            .display_name = Some(format!("nsec1{}", "q".repeat(58)));
        let projection = EffectivePrincipalProjection::from_dashboard(&dashboard);
        assert!(!projection.display_name.contains("nsec1"));
    }
}
