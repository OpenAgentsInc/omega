//! Public-safe Effective Principal projection for Omega chrome.
//!
//! Account identity, signer availability, Organization membership, and Sync
//! freshness are separate facts. This module projects only the facts the local
//! account registry currently owns and leaves Organization/Sync unclaimed.

use omega_identity::{
    AccountDashboardEntry, AccountDashboardProjection, AccountLifecycleState,
    AccountRegistryService, SignerAvailability, SignerKind,
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

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct EffectivePrincipalProjection {
    pub(crate) display_name: String,
    pub(crate) identity_label: String,
    pub(crate) scope_label: String,
    pub(crate) signer_label: String,
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
        Self::from_account(account)
    }

    fn from_account(account: &AccountDashboardEntry) -> Self {
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
        Self {
            display_name,
            identity_label,
            scope_label: "Local scope".into(),
            signer_label: signer_label.into(),
            state,
        }
    }

    fn local_only() -> Self {
        Self {
            display_name: "Local only".into(),
            identity_label: "No enrolled identity".into(),
            scope_label: "Local scope".into(),
            signer_label: "Signer unavailable".into(),
            state: EffectivePrincipalState::LocalOnly,
        }
    }

    fn conflict(label: &str) -> Self {
        Self {
            display_name: label.into(),
            identity_label: "Principal not verified".into(),
            scope_label: "Scope unavailable".into(),
            signer_label: "Signer not trusted".into(),
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
            "Effective principal: {}; {}; {}; {}; {}. Open identity settings",
            self.display_name,
            self.identity_label,
            self.scope_label,
            self.signer_label,
            self.status_label()
        )
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

    #[test]
    fn exact_active_account_is_projected_without_claiming_an_organization() {
        let projection = EffectivePrincipalProjection::from_dashboard(&dashboard(&["account-a"]));
        assert_eq!(projection.display_name, "Person 0");
        assert_eq!(projection.scope_label, "Local scope");
        assert_eq!(projection.signer_label, "Local signer ready");
        assert_eq!(projection.state, EffectivePrincipalState::Enrolled);
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
