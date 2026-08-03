use std::{sync::Arc, time::SystemTime};

use anyhow::{Context as _, Result, ensure};
use chrono::DateTime;
use nostr::{Event, JsonUtil as _};
use omega_effectd::all_work_contract::SignedWorkroomPreparation;
use omega_identity::{
    AccountLifecycleState, AccountRegistryService, AdmittedSigningRequest,
    DurableIdentityActionDecision, DurableIdentityActionDescriptor, DurableIdentityActionKind,
    IdentityService, ProofRef, ReceiptRef, RecoveryProtectionState, ResourceRef, SignerKind,
    SigningPurpose, UnsignedEventTemplate,
};
use omega_signer_broker::{
    Nip46WebSocketTransport, RemoteSignerMetadata, SignerBroker, SignerRoute,
};
use serde::Deserialize;
use sha2::{Digest as _, Sha256};

#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
#[serde(deny_unknown_fields)]
struct PreparedNostrEvent {
    pubkey: String,
    created_at: u64,
    kind: u16,
    tags: Vec<Vec<String>>,
    content: String,
}

fn unix_time_seconds() -> Result<u64> {
    Ok(SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .context("system clock is before the Unix epoch")?
        .as_secs())
}

fn signing_request(
    preparation: &SignedWorkroomPreparation,
    selection: &omega_identity::AccountSelectionToken,
) -> Result<(
    AdmittedSigningRequest,
    DurableIdentityActionDescriptor,
    PreparedNostrEvent,
)> {
    let event: PreparedNostrEvent = serde_json::from_str(&preparation.unsigned_event_json.0)
        .context("decoding the OpenAgents signed Workroom preparation")?;
    ensure!(
        event.pubkey == preparation.activity.signer_pubkey.0
            && event.pubkey == selection.identity.public_key_hex().as_str()
            && preparation.activity.actor_ref.0 == format!("principal:nostr:{}", event.pubkey),
        "the prepared signed Workroom actor does not match the selected Omega signer"
    );
    ensure!(
        (32_150..=32_163).contains(&event.kind),
        "the prepared event kind is outside the signed Workroom registry"
    );
    let expires_at = DateTime::parse_from_rfc3339(&preparation.expires_at.0)
        .context("decoding the signed Workroom preparation expiry")?
        .timestamp();
    let expires_at = u64::try_from(expires_at).context("signed Workroom expiry is before epoch")?;
    let destination_digest = format!(
        "{:x}",
        Sha256::digest(preparation.activity.workroom_ref.0.as_bytes())
    );
    let payload_digest = format!(
        "{:x}",
        Sha256::digest(preparation.unsigned_event_json.0.as_bytes())
    );
    let authorization_digest = format!(
        "{:x}",
        Sha256::digest(preparation.preparation_ref.0.as_bytes())
    );
    let descriptor = DurableIdentityActionDescriptor {
        intent_ref: ReceiptRef::new(preparation.preparation_ref.0.clone())?,
        kind: DurableIdentityActionKind::WorkroomActivity,
        destination_ref: ResourceRef::new(format!("signed-workroom-{destination_digest}"))?,
        authorization_ref: ProofRef::new(format!(
            "signed-workroom-authorization-{authorization_digest}"
        ))?,
        payload_digest,
        expires_at,
    };
    let request = AdmittedSigningRequest {
        request_ref: descriptor.intent_ref.clone(),
        identity_ref: selection.identity.identity_ref().clone(),
        purpose: SigningPurpose::NostrEvent,
        event: UnsignedEventTemplate {
            created_at: event.created_at,
            kind: event.kind,
            tags: event.tags.clone(),
            content: event.content.clone(),
        },
    };
    Ok((request, descriptor, event))
}

fn verify_signed_event(prepared: &PreparedNostrEvent, signed_event_json: &str) -> Result<()> {
    let event = Event::from_json(signed_event_json)
        .context("decoding the signed Workroom event returned by custody")?;
    event
        .verify()
        .context("verifying the signed Workroom event returned by custody")?;
    let value: serde_json::Value = serde_json::from_str(signed_event_json)?;
    ensure!(
        value.get("pubkey") == Some(&serde_json::json!(prepared.pubkey))
            && value.get("created_at") == Some(&serde_json::json!(prepared.created_at))
            && value.get("kind") == Some(&serde_json::json!(prepared.kind))
            && value.get("tags") == Some(&serde_json::json!(prepared.tags))
            && value.get("content") == Some(&serde_json::json!(prepared.content)),
        "custody signed different bytes than the OpenAgents Workroom preparation"
    );
    Ok(())
}

pub async fn sign_signed_workroom_preparation(
    preparation: &SignedWorkroomPreparation,
) -> Result<String> {
    let registry = AccountRegistryService::for_channel(*app_identity::CHANNEL);
    let selection = registry
        .selection_token()
        .context("reading the selected Omega signing account")?;
    registry
        .validate_signing_selection(&selection)
        .context("validating the selected Omega signing account")?;
    let dashboard = registry
        .inspect()
        .context("reading the Omega account dashboard")?;
    let account = dashboard
        .accounts
        .iter()
        .find(|account| account.is_active && account.account_ref == selection.account_ref)
        .context("the selected Omega signing account is unavailable")?;
    let (request, descriptor, prepared_event) = signing_request(preparation, &selection)?;
    let signed = if account.signer.kind == SignerKind::RemoteNip46 {
        let capability = registry
            .remote_signer_capability(&selection)
            .context("loading the selected NIP-46 signer capability")?;
        let authorization = registry
            .authorize_remote_identity_action(&selection, descriptor, unix_time_seconds()?)
            .context("authorizing the remote signed Workroom event")?;
        let route = SignerRoute::RemoteNip46 {
            metadata: RemoteSignerMetadata { capability },
            transport: Arc::new(Nip46WebSocketTransport::system()),
        };
        let result = SignerBroker::system()
            .sign(&route, selection.clone(), request)
            .await
            .context("signing the Workroom event with the selected NIP-46 signer")?;
        registry
            .validate_remote_identity_action_authorization(&authorization, unix_time_seconds()?)
            .context("revalidating remote Workroom signing authority")?;
        result
    } else {
        ensure!(
            account.lifecycle == AccountLifecycleState::Active
                && account.recovery == RecoveryProtectionState::Protected,
            "activate and protect the selected Omega identity before Workroom signing"
        );
        let identity_service = Arc::new(IdentityService::system(*app_identity::CHANNEL));
        let authorization = match identity_service
            .authorize_or_hold_identity_action(descriptor)
            .context("authorizing the local signed Workroom event")?
        {
            DurableIdentityActionDecision::Authorized(authorization) => authorization,
            DurableIdentityActionDecision::ActivationRequired { .. } => {
                anyhow::bail!("activate the selected Omega identity before Workroom signing")
            }
        };
        identity_service
            .validate_identity_action_authorization(&authorization)
            .context("revalidating local Workroom signing authority")?;
        let route = SignerRoute::Local {
            identity_service: identity_service.clone(),
        };
        let result = SignerBroker::system()
            .sign(&route, selection, request)
            .await
            .context("signing the Workroom event with local Omega custody")?;
        identity_service
            .validate_identity_action_authorization(&authorization)
            .context("revalidating local Workroom signing authority after signing")?;
        result
    };
    verify_signed_event(&prepared_event, &signed.signed_event_json)?;
    Ok(signed.signed_event_json)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prepared_event_rejects_unknown_fields_and_non_workroom_kinds() {
        assert!(
            serde_json::from_str::<PreparedNostrEvent>(
                r#"{"pubkey":"a","created_at":1,"kind":32150,"tags":[],"content":"","relay":"wss://hostile"}"#,
            )
            .is_err()
        );
        let value: PreparedNostrEvent = serde_json::from_str(
            r#"{"pubkey":"a","created_at":1,"kind":1,"tags":[],"content":""}"#,
        )
        .expect("shape");
        assert!(!(32_150..=32_163).contains(&value.kind));
    }
}
