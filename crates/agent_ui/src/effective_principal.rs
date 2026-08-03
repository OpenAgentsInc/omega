//! Public-safe Effective Principal projection for Omega chrome.
//!
//! Account identity, signer availability, Organization membership, and Sync
//! freshness are separate facts. The local account registry supplies the
//! account and signer facts. A generated omega-effectd read supplies explicit
//! Organization membership for that exact account, generation, and principal.

use omega_identity::{
    AccountDashboardEntry, AccountDashboardProjection, AccountLifecycleState,
    AccountRegistryService, SignerAvailability, SignerKind,
};
use ui::{Color, IconName};

use crate::organization_scope::{
    OrganizationMembershipProjection, OrganizationMembershipState, OrganizationScopeError,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum EffectivePrincipalState {
    LocalOnly,
    Enrolled,
    Offline,
    SignerUnavailable,
    Revoked,
    Conflict,
}

impl EffectivePrincipalState {
    pub(crate) const ALL: [Self; 6] = [
        Self::LocalOnly,
        Self::Enrolled,
        Self::Offline,
        Self::SignerUnavailable,
        Self::Revoked,
        Self::Conflict,
    ];

    /// Footer cue for this state. The icon shape carries the meaning on its
    /// own so a person who cannot separate the colors still sees six distinct
    /// states; color and the accessible name repeat it.
    pub(crate) const fn cue(self) -> (IconName, Color) {
        match self {
            Self::LocalOnly => (IconName::Lock, Color::Muted),
            Self::Enrolled => (IconName::UserCheck, Color::Success),
            Self::Offline => (IconName::Disconnected, Color::Warning),
            Self::SignerUnavailable => (IconName::LockOff, Color::Warning),
            Self::Revoked => (IconName::XCircle, Color::Error),
            Self::Conflict => (IconName::Warning, Color::Error),
        }
    }
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
    pub(crate) fn inspect_dashboard() -> Option<AccountDashboardProjection> {
        AccountRegistryService::for_channel(*app_identity::CHANNEL)
            .inspect()
            .ok()
    }

    pub(crate) fn current() -> Self {
        match Self::inspect_dashboard() {
            Some(dashboard) => Self::from_dashboard(&dashboard),
            None => Self::conflict("Identity unavailable"),
        }
    }

    pub(crate) fn membership_request(
        dashboard: &AccountDashboardProjection,
    ) -> Option<omega_effectd::all_work_contract::OrganizationMembershipReadRequest> {
        let account = selected_account(dashboard)?;
        let principal_ref = omega_effectd::all_work_contract::PrincipalRef::try_from(format!(
            "principal:nostr:{}",
            account.identity.public_key_hex().as_str()
        ))
        .ok()?;
        Some(
            omega_effectd::all_work_contract::OrganizationMembershipReadRequest {
                account_ref: omega_effectd::all_work_contract::SourceRef::try_from(
                    account.account_ref.as_str().to_string(),
                )
                .ok()?,
                account_generation: omega_effectd::all_work_contract::SafeInteger(
                    dashboard.active.generation,
                ),
                effective_principal_ref: principal_ref,
            },
        )
    }

    pub(crate) fn from_dashboard_and_membership_read(
        dashboard: &AccountDashboardProjection,
        result: &omega_effectd::all_work_contract::OrganizationMembershipReadResult,
    ) -> Self {
        match membership_projection(result) {
            Ok(membership) => Self::from_dashboard_and_membership(dashboard, membership.as_ref()),
            Err(()) => {
                let mut projection = Self::from_dashboard(dashboard);
                projection.scope_label = "Organization unverified".into();
                projection.organization_ref = None;
                projection.organization_state = EffectiveOrganizationState::Conflict;
                projection
            }
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
        // A conflicted, repairing, or switching account is ambiguous: showing
        // either candidate name would claim an identity the registry has not
        // settled, so nothing about it is projected.
        if matches!(
            account.lifecycle,
            AccountLifecycleState::Conflict
                | AccountLifecycleState::RepairRequired
                | AccountLifecycleState::Switching
        ) {
            return Self::conflict("Identity conflict");
        }
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

fn selected_account(dashboard: &AccountDashboardProjection) -> Option<&AccountDashboardEntry> {
    let active_ref = dashboard.active.account_ref.as_ref()?;
    let mut matching = dashboard
        .accounts
        .iter()
        .filter(|account| account.is_active && &account.account_ref == active_ref);
    let account = matching.next()?;
    (matching.next().is_none()
        && !dashboard
            .accounts
            .iter()
            .any(|candidate| candidate.is_active && candidate.account_ref != *active_ref))
    .then_some(account)
}

fn membership_projection(
    result: &omega_effectd::all_work_contract::OrganizationMembershipReadResult,
) -> Result<Option<OrganizationMembershipProjection>, ()> {
    use omega_effectd::all_work_contract::{
        CompletenessState, FreshnessState, OrganizationMembershipState as ContractMembershipState,
    };

    if result.ledger.completeness.state != CompletenessState::Complete
        || result.ledger.freshness.state != FreshnessState::Fresh
    {
        return Err(());
    }
    let mut memberships = result.ledger.memberships.iter();
    let Some(membership) = memberships.next() else {
        return Ok(None);
    };
    if memberships.next().is_some() {
        return Err(());
    }
    let state = match membership.state {
        ContractMembershipState::Verified => OrganizationMembershipState::Verified,
        ContractMembershipState::Stale => OrganizationMembershipState::Stale,
        ContractMembershipState::Revoked => OrganizationMembershipState::Revoked,
    };
    Ok(Some(OrganizationMembershipProjection {
        membership_ref: membership.membership_ref.0.clone(),
        account_ref: omega_identity::AccountRef::new(membership.account_ref.0.clone())
            .map_err(|_| ())?,
        account_generation: membership.account_generation.0,
        principal_ref: membership.effective_principal_ref.clone(),
        organization_ref: membership.organization_ref.clone(),
        display_name: membership.display_name.0.clone(),
        source_revision: membership.source_revision.0,
        state,
    }))
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
                let keys = Keys::new(SecretKey::from_hex(&secret).expect("secret key"));
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
    fn generated_membership_read_projects_only_the_exact_fresh_authority_row() {
        let dashboard = dashboard(&["account-a"]);
        let request = EffectivePrincipalProjection::membership_request(&dashboard)
            .expect("membership request");
        let result: omega_effectd::all_work_contract::OrganizationMembershipReadResult =
            serde_json::from_value(serde_json::json!({
                "ledger": {
                    "contractVersion": "openagents.all_work_boundary.v1",
                    "revision": 4,
                    "memberships": [{
                        "contractVersion": "openagents.all_work_boundary.v1",
                        "membershipRef": "membership:openagents:owner",
                        "accountRef": request.account_ref.0,
                        "accountGeneration": request.account_generation.0,
                        "effectivePrincipalRef": request.effective_principal_ref.0,
                        "organizationRef": "organization:openagents",
                        "displayName": "OpenAgents",
                        "sourceRevision": 4,
                        "state": "verified",
                        "observedAt": "2026-08-03T17:30:00Z"
                    }],
                    "completeness": { "state": "complete", "cursor": null, "gapRefs": [] },
                    "freshness": { "state": "fresh", "observedAt": "2026-08-03T17:30:00Z" }
                }
            }))
            .expect("generated result");

        let projection =
            EffectivePrincipalProjection::from_dashboard_and_membership_read(&dashboard, &result);
        assert_eq!(projection.scope_label, "OpenAgents");
        assert_eq!(
            projection.organization_state,
            EffectiveOrganizationState::Verified
        );
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

    fn membership_read_result(
        rows: serde_json::Value,
        completeness: &str,
        freshness: &str,
    ) -> omega_effectd::all_work_contract::OrganizationMembershipReadResult {
        serde_json::from_value(serde_json::json!({
            "ledger": {
                "contractVersion": "openagents.all_work_boundary.v1",
                "revision": 4,
                "memberships": rows,
                "completeness": { "state": completeness, "cursor": null, "gapRefs": [] },
                "freshness": { "state": freshness, "observedAt": "2026-08-03T17:30:00Z" }
            }
        }))
        .expect("generated result")
    }

    fn membership_row(
        request: &omega_effectd::all_work_contract::OrganizationMembershipReadRequest,
        display_name: &str,
    ) -> serde_json::Value {
        serde_json::json!({
            "contractVersion": "openagents.all_work_boundary.v1",
            "membershipRef": "membership:openagents:owner",
            "accountRef": request.account_ref.0,
            "accountGeneration": request.account_generation.0,
            "effectivePrincipalRef": request.effective_principal_ref.0,
            "organizationRef": "organization:openagents",
            "displayName": display_name,
            "sourceRevision": 4,
            "state": "verified",
            "observedAt": "2026-08-03T17:30:00Z"
        })
    }

    #[test]
    fn every_degraded_signer_and_lifecycle_state_stays_a_separate_visible_fact() {
        let cases: [(
            AccountLifecycleState,
            SignerAvailability,
            EffectivePrincipalState,
            &str,
        ); 8] = [
            (
                AccountLifecycleState::Active,
                SignerAvailability::Offline,
                EffectivePrincipalState::Offline,
                "Signer offline",
            ),
            (
                AccountLifecycleState::Active,
                SignerAvailability::Revoked,
                EffectivePrincipalState::Revoked,
                "Signer revoked",
            ),
            (
                AccountLifecycleState::Active,
                SignerAvailability::Lost,
                EffectivePrincipalState::Revoked,
                "Signer lost",
            ),
            (
                AccountLifecycleState::Active,
                SignerAvailability::UserApprovalRequired,
                EffectivePrincipalState::SignerUnavailable,
                "Signer approval required",
            ),
            (
                AccountLifecycleState::Active,
                SignerAvailability::Rejected,
                EffectivePrincipalState::SignerUnavailable,
                "Signer rejected",
            ),
            (
                AccountLifecycleState::Locked,
                SignerAvailability::Ready,
                EffectivePrincipalState::SignerUnavailable,
                "Local signer ready",
            ),
            (
                AccountLifecycleState::Forgotten,
                SignerAvailability::Ready,
                EffectivePrincipalState::Revoked,
                "Local signer ready",
            ),
            (
                AccountLifecycleState::Switching,
                SignerAvailability::Ready,
                EffectivePrincipalState::Conflict,
                "Signer not trusted",
            ),
        ];
        for (lifecycle, availability, expected_state, expected_signer_label) in cases {
            let mut dashboard = dashboard(&["account-a"]);
            dashboard.accounts[0].lifecycle = lifecycle;
            dashboard.accounts[0].signer.availability = availability;
            let projection = EffectivePrincipalProjection::from_dashboard(&dashboard);
            assert_eq!(
                projection.state, expected_state,
                "{lifecycle:?}/{availability:?} projected the wrong principal state"
            );
            assert_eq!(
                projection.signer_label, expected_signer_label,
                "{lifecycle:?}/{availability:?} collapsed the separate signer fact"
            );
            assert_eq!(
                projection.organization_ref, None,
                "{lifecycle:?}/{availability:?} claimed an Organization without membership"
            );
        }
    }

    #[test]
    fn an_inactive_account_lifecycle_cannot_claim_a_verified_organization() {
        for lifecycle in [
            AccountLifecycleState::Locked,
            AccountLifecycleState::SignedOut,
            AccountLifecycleState::Switching,
            AccountLifecycleState::RepairRequired,
            AccountLifecycleState::Forgotten,
            AccountLifecycleState::ForgetPending,
            AccountLifecycleState::CandidateExisting,
            AccountLifecycleState::Activating,
        ] {
            let mut dashboard = dashboard(&["account-a"]);
            let membership = membership(&dashboard, OrganizationMembershipState::Verified);
            dashboard.accounts[0].lifecycle = lifecycle;
            let projection = EffectivePrincipalProjection::from_dashboard_and_membership(
                &dashboard,
                Some(&membership),
            );
            assert_ne!(
                projection.organization_state,
                EffectiveOrganizationState::Verified,
                "{lifecycle:?} activated an Organization scope"
            );
            assert_eq!(projection.organization_ref, None);
        }
    }

    /// Membership is a source fact about the account, not a signer capability.
    /// A fresh verified row still names the Organization while the signer is
    /// unreachable, and the signer fact stays separately visible so the two are
    /// never read as one synchronized claim.
    #[test]
    fn signer_reachability_and_membership_stay_separate_facts() {
        for (availability, expected_signer_label, expected_state) in [
            (
                SignerAvailability::Offline,
                "Signer offline",
                EffectivePrincipalState::Offline,
            ),
            (
                SignerAvailability::UserApprovalRequired,
                "Signer approval required",
                EffectivePrincipalState::SignerUnavailable,
            ),
        ] {
            let mut dashboard = dashboard(&["account-a"]);
            let membership = membership(&dashboard, OrganizationMembershipState::Verified);
            dashboard.accounts[0].signer.availability = availability;
            let projection = EffectivePrincipalProjection::from_dashboard_and_membership(
                &dashboard,
                Some(&membership),
            );
            assert_eq!(projection.state, expected_state);
            assert_eq!(projection.signer_label, expected_signer_label);
            assert_eq!(
                projection.organization_state,
                EffectiveOrganizationState::Verified
            );
            let accessibility_label = projection.accessibility_label();
            assert!(accessibility_label.contains(expected_signer_label));
            assert!(accessibility_label.contains("Organization verified"));
        }
    }

    #[test]
    fn an_unsettled_account_never_shows_a_candidate_identity() {
        for lifecycle in [
            AccountLifecycleState::Conflict,
            AccountLifecycleState::RepairRequired,
            AccountLifecycleState::Switching,
        ] {
            let mut dashboard = dashboard(&["account-a"]);
            dashboard.accounts[0].lifecycle = lifecycle;
            let projection = EffectivePrincipalProjection::from_dashboard(&dashboard);
            assert_eq!(projection.state, EffectivePrincipalState::Conflict);
            assert_eq!(projection.display_name, "Identity conflict");
            assert!(
                !projection.accessibility_label().contains("Person"),
                "{lifecycle:?} leaked a candidate display name"
            );
            assert_eq!(projection.principal_ref, None);
        }
    }

    #[test]
    fn every_principal_state_has_a_distinct_shape_so_color_is_never_the_only_cue() {
        let mut icons = Vec::new();
        for state in EffectivePrincipalState::ALL {
            let (icon, _color) = state.cue();
            assert!(
                !icons.contains(&icon),
                "{state:?} reuses an icon shape already used by another state"
            );
            icons.push(icon);
        }
        assert_eq!(icons.len(), EffectivePrincipalState::ALL.len());
    }

    #[test]
    fn incomplete_stale_or_ambiguous_membership_reads_fail_closed() {
        let dashboard = dashboard(&["account-a"]);
        let request = EffectivePrincipalProjection::membership_request(&dashboard)
            .expect("membership request");
        let row = membership_row(&request, "OpenAgents");
        let mut cross_account_row = row.clone();
        cross_account_row["accountRef"] = serde_json::json!("account:someone-else");
        let mut cross_generation_row = row.clone();
        cross_generation_row["accountGeneration"] =
            serde_json::json!(request.account_generation.0 + 1);

        let cases = [
            (
                "partial ledger",
                membership_read_result(serde_json::json!([row.clone()]), "partial", "fresh"),
            ),
            (
                "gap ledger",
                membership_read_result(serde_json::json!([row.clone()]), "gap", "fresh"),
            ),
            (
                "stale source",
                membership_read_result(serde_json::json!([row.clone()]), "complete", "stale"),
            ),
            (
                "duplicate rows",
                membership_read_result(
                    serde_json::json!([row.clone(), row.clone()]),
                    "complete",
                    "fresh",
                ),
            ),
            (
                "cross-account row",
                membership_read_result(serde_json::json!([cross_account_row]), "complete", "fresh"),
            ),
            (
                "cross-generation row",
                membership_read_result(
                    serde_json::json!([cross_generation_row]),
                    "complete",
                    "fresh",
                ),
            ),
        ];
        for (label, result) in cases {
            let projection = EffectivePrincipalProjection::from_dashboard_and_membership_read(
                &dashboard, &result,
            );
            assert_eq!(
                projection.organization_ref, None,
                "{label} published an Organization reference"
            );
            assert_eq!(
                projection.organization_state,
                EffectiveOrganizationState::Conflict,
                "{label} did not fail closed"
            );
            assert_eq!(projection.scope_label, "Organization unverified");
        }
    }

    #[test]
    fn an_empty_membership_ledger_keeps_local_scope_rather_than_inventing_one() {
        let dashboard = dashboard(&["account-a"]);
        let result = membership_read_result(serde_json::json!([]), "complete", "fresh");
        let projection =
            EffectivePrincipalProjection::from_dashboard_and_membership_read(&dashboard, &result);
        assert_eq!(projection.scope_label, "Local scope");
        assert_eq!(projection.organization_ref, None);
        assert_eq!(
            projection.organization_state,
            EffectiveOrganizationState::Unavailable
        );
    }

    #[test]
    fn no_secret_shaped_value_reaches_the_visible_or_accessible_identity() {
        let dashboard = dashboard(&["account-a"]);
        let public_key_hex = dashboard.accounts[0]
            .identity
            .public_key_hex()
            .as_str()
            .to_string();
        let request = EffectivePrincipalProjection::membership_request(&dashboard)
            .expect("membership request");
        for hostile_organization_name in [
            format!("nsec1{}", "q".repeat(58)),
            format!("ncryptsec1{}", "q".repeat(58)),
            "a".repeat(64),
        ] {
            let result = membership_read_result(
                serde_json::json!([membership_row(&request, &hostile_organization_name)]),
                "complete",
                "fresh",
            );
            let projection = EffectivePrincipalProjection::from_dashboard_and_membership_read(
                &dashboard, &result,
            );
            assert_eq!(projection.scope_label, "Organization unverified");
            assert!(!projection.accessibility_label().contains("nsec1"));
            assert!(!projection.accessibility_label().contains("ncryptsec1"));
            assert!(
                !projection
                    .accessibility_label()
                    .contains(&hostile_organization_name)
            );
        }

        let projection = EffectivePrincipalProjection::from_dashboard(&dashboard);
        let accessibility_label = projection.accessibility_label();
        for forbidden in ["nsec", "npub", "ncryptsec", public_key_hex.as_str()] {
            assert!(
                !accessibility_label.contains(forbidden),
                "accessibility label leaked `{forbidden}`"
            );
        }
        assert!(!projection.display_name.contains(&public_key_hex));
        assert!(!projection.identity_label.contains(&public_key_hex));
        assert!(!projection.scope_label.contains(&public_key_hex));
        assert!(!projection.signer_label.contains(&public_key_hex));
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
