use std::{
    sync::Arc,
    time::{SystemTime, UNIX_EPOCH},
};

use anyhow::{Context as _, Result, anyhow};
use base64::Engine as _;
use http_client::{AsyncBody, HttpClient, Method, Request};
use omega_identity::{
    AdmittedSigningRequest, IdentityService, ReceiptRef, SigningPurpose, UnsignedEventTemplate,
};
use serde::Deserialize;
use sha2::{Digest as _, Sha256};
use smol::io::AsyncReadExt as _;

pub const OPENAGENTS_NOSTR_SESSION_URL: &str = "https://openagents.com/api/omega/auth/session";

const MAX_HTTP_BODY_BYTES: u64 = 64 * 1024;
const MAX_ACCESS_TOKEN_BYTES: usize = 16 * 1024;
const NIP98_KIND: u16 = 27_235;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MintedOpenAgentsSession {
    pub access_token: String,
    pub expires_in: u64,
    pub user: MintedOpenAgentsUser,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MintedOpenAgentsUser {
    pub user_id: String,
}

pub async fn mint_openagents_nostr_session(
    http_client: &Arc<dyn HttpClient>,
) -> Result<MintedOpenAgentsSession> {
    let identity_service = IdentityService::system(*app_identity::CHANNEL);
    let inspection = identity_service
        .inspect()
        .context("inspect Omega identity for OpenAgents authentication")?;
    let identity = inspection
        .identity
        .ok_or_else(|| anyhow!("Omega identity is not available"))?;
    let created_at = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .context("system clock is before Unix epoch")?
        .as_secs();
    let empty_payload_hash = format!("{:x}", Sha256::digest([]));
    let semantic_binding = serde_json::to_vec(&serde_json::json!({
        "url": OPENAGENTS_NOSTR_SESSION_URL,
        "method": "POST",
        "payload": empty_payload_hash,
        "createdAt": created_at,
        "publicKey": identity.public_key_hex().as_str(),
    }))
    .context("serialize OpenAgents authentication binding")?;
    let digest = format!("{:x}", Sha256::digest(semantic_binding));
    let request_ref = ReceiptRef::new(format!("nip98.{}", &digest[..32]))
        .context("construct OpenAgents authentication receipt reference")?;
    let signed = identity_service
        .sign(&AdmittedSigningRequest {
            request_ref,
            identity_ref: identity.identity_ref().clone(),
            purpose: SigningPurpose::NostrEvent,
            event: UnsignedEventTemplate {
                created_at,
                kind: NIP98_KIND,
                tags: vec![
                    vec!["u".to_string(), OPENAGENTS_NOSTR_SESSION_URL.to_string()],
                    vec!["method".to_string(), "POST".to_string()],
                    vec!["payload".to_string(), empty_payload_hash],
                ],
                content: String::new(),
            },
        })
        .context("sign OpenAgents authentication proof")?;
    let authorization =
        base64::engine::general_purpose::STANDARD.encode(signed.signed_event_json.as_bytes());
    let request = Request::builder()
        .method(Method::POST)
        .uri(OPENAGENTS_NOSTR_SESSION_URL)
        .header("authorization", format!("Nostr {authorization}"))
        .header("content-type", "application/octet-stream")
        .body(AsyncBody::empty())?;
    let mut response = http_client
        .send(request)
        .await
        .context("send OpenAgents authentication proof")?;
    let status = response.status();
    let mut body = Vec::new();
    response
        .body_mut()
        .take(MAX_HTTP_BODY_BYTES)
        .read_to_end(&mut body)
        .await
        .context("read OpenAgents authentication response")?;
    if !status.is_success() {
        return Err(anyhow!(
            "OpenAgents background authentication failed ({status})"
        ));
    }
    let session: MintedOpenAgentsSession =
        serde_json::from_slice(&body).context("OpenAgents authentication response was invalid")?;
    if session.access_token.trim().is_empty()
        || session.access_token.len() > MAX_ACCESS_TOKEN_BYTES
        || session.expires_in == 0
        || session.user.user_id.trim().is_empty()
    {
        return Err(anyhow!("OpenAgents authentication response was incomplete"));
    }
    Ok(session)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn endpoint_is_https_and_owner_scoped() {
        assert_eq!(
            OPENAGENTS_NOSTR_SESSION_URL,
            "https://openagents.com/api/omega/auth/session"
        );
        assert_eq!(
            format!("{:x}", Sha256::digest([])),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
    }
}
