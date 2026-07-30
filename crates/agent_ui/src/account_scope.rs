use anyhow::{Context as _, ensure};
use omega_identity::{
    AccountLifecycleState, AccountRegistryService, AccountSelectionToken, PublicIdentity,
};

#[derive(Clone, Debug)]
pub(crate) struct AccountScope {
    public_key_hex: String,
    selection_generation: u64,
    token: Option<AccountSelectionToken>,
}

impl AccountScope {
    pub(crate) fn current() -> anyhow::Result<Self> {
        let token = AccountRegistryService::for_channel(*app_identity::CHANNEL)
            .selection_token()
            .context("inspecting the current Omega account selection")?;
        Ok(Self {
            public_key_hex: token.identity.public_key_hex().as_str().to_string(),
            selection_generation: token.generation,
            token: Some(token),
        })
    }

    pub(crate) fn observed() -> Self {
        Self::current().unwrap_or_else(|_| Self {
            public_key_hex: "local".to_string(),
            selection_generation: 0,
            token: None,
        })
    }

    pub(crate) fn namespace(&self, base: &str) -> String {
        Self::namespace_for_public_key(base, &self.public_key_hex)
    }

    pub(crate) fn namespace_for_identity(base: &str, identity: &PublicIdentity) -> String {
        Self::namespace_for_public_key(base, identity.public_key_hex().as_str())
    }

    fn namespace_for_public_key(base: &str, public_key_hex: &str) -> String {
        format!("{base}/accounts/{public_key_hex}")
    }

    pub(crate) fn profile_key(&self, key: &str) -> String {
        key.to_string()
    }

    pub(crate) fn identity(&self) -> Option<PublicIdentity> {
        self.token.as_ref().map(|token| token.identity.clone())
    }

    pub(crate) fn pending_key(&self, key: &str) -> String {
        format!("generations/{}/{key}", self.selection_generation)
    }

    pub(crate) fn ensure_current(&self) -> anyhow::Result<()> {
        if let Some(token) = &self.token {
            AccountRegistryService::for_channel(*app_identity::CHANNEL)
                .validate_selection(token)
                .context("validating the Omega account selection")?;
        } else {
            ensure!(
                AccountRegistryService::for_channel(*app_identity::CHANNEL)
                    .selection_token()
                    .is_err(),
                "the Omega account changed before the background write completed"
            );
        }
        Ok(())
    }

    pub(crate) fn is_purge_barrier_active(&self) -> anyhow::Result<bool> {
        let Some(token) = &self.token else {
            return Ok(false);
        };
        let projection = AccountRegistryService::for_channel(*app_identity::CHANNEL)
            .inspect()
            .context("inspecting the Omega account after a stale write")?;
        Ok(projection.accounts.iter().any(|account| {
            account.account_ref == token.account_ref
                && matches!(
                    account.lifecycle,
                    AccountLifecycleState::ForgetPending | AccountLifecycleState::Forgotten
                )
        }))
    }
}

impl PartialEq for AccountScope {
    fn eq(&self, other: &Self) -> bool {
        self.public_key_hex == other.public_key_hex
            && self.selection_generation == other.selection_generation
    }
}

impl Eq for AccountScope {}

#[cfg(test)]
mod tests {
    use super::*;

    fn scope(public_key_hex: &str, account_generation: u64) -> AccountScope {
        AccountScope {
            public_key_hex: public_key_hex.to_string(),
            selection_generation: account_generation,
            token: None,
        }
    }

    #[test]
    fn profile_keys_are_isolated_by_public_identity() {
        let account_a = scope("aa", 1);
        let account_b = scope("bb", 1);

        assert_ne!(
            account_a.namespace("omega_community"),
            account_b.namespace("omega_community")
        );
        assert_eq!(
            account_a.namespace("omega_community"),
            "omega_community/accounts/aa"
        );
    }

    #[test]
    fn stale_a_generation_is_not_current_after_a_b_a() {
        let first_a_selection = scope("aa", 1);
        let account_b_selection = scope("bb", 2);
        let second_a_selection = scope("aa", 3);

        assert_eq!(
            first_a_selection.namespace("omega_community"),
            second_a_selection.namespace("omega_community")
        );
        assert_ne!(
            first_a_selection.pending_key("outbox"),
            account_b_selection.pending_key("outbox")
        );
        assert_ne!(
            first_a_selection.pending_key("outbox"),
            second_a_selection.pending_key("outbox")
        );
        assert_ne!(first_a_selection, second_a_selection);
    }
}
