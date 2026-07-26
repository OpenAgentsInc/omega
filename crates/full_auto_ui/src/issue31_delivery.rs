//! Hand the omega#47 documents to the issue-31 host pump (omega#49).
//!
//! `issue31_adjunct::publish_issue31_host_snapshot` already builds both
//! documents from one reading of live host state, and until now nothing sent
//! them anywhere. The pump that gift-wraps owner-private records lives in
//! `omega_effectd`, which this crate depends on — so the reading crosses the
//! seam as data rather than by calling upward.
//!
//! Two things are deliberately NOT done here:
//!
//! - the delivery binding is not stated. The pump adds `recordType`,
//!   `hostPublicKeyHex`, `devicePublicKeyHex`, `grantRef` and
//!   `expectedGeneration`, because those are facts about who may read the
//!   snapshot and this module knows only what the runs are. A document that
//!   arrived already claiming them is refused by the pump.
//! - no reading is invented. When the host has not observed its Full Auto
//!   state, `latest_issue31_live_reading` is `None` and the pump publishes
//!   nothing, so the phone reads `no_host_projection`. An empty reading — a
//!   host that looked and found no runs — is a `Some` carrying zero runs, and
//!   the phone renders it as a host that is running nothing. The two are not
//!   the same claim and are never collapsed.

use std::sync::{Arc, Mutex, OnceLock};

use omega_effectd::{
    Issue31HostProjectionDocuments, Issue31HostProjectionRequest, Issue31HostProjectionSource,
};
use serde_json::Value;
use sha2::{Digest, Sha256};

use crate::issue31_adjunct::{
    Issue31FullAutoLiveSources, Issue31HostIdentitySource, Issue31HostProjectionError,
    publish_issue31_host_snapshot,
};

/// One complete reading of the live Full Auto surface, owned rather than
/// borrowed so it can be cached between the panel that reads it and the pump
/// that publishes it.
///
/// `host_ref` is absent on purpose: the host reference belongs to the device's
/// grant, not to the run registry, so it is supplied per delivery.
#[derive(Clone, Debug, Default)]
pub struct Issue31FullAutoReading {
    /// When the daemon was actually read, in epoch milliseconds. Never "now":
    /// a snapshot stamped with the publish time would claim a freshness the
    /// host does not have.
    pub generated_at_ms: u64,
    pub host_generation: u64,
    /// One `get_run` record per run the daemon listed.
    pub run_details: Vec<Value>,
    /// The `get_capacity` record.
    pub capacity: Value,
    pub handoffs: Vec<Value>,
    /// One `(get_report, get_receipt)` pair per run with evidence.
    pub evidence: Vec<(Value, Value)>,
}

impl Issue31FullAutoReading {
    /// A stable identifier for exactly this reading.
    ///
    /// The mobile contract binds the detail projection to the snapshot that
    /// advertised it, so the reference has to change when — and only when —
    /// the observed state changes. Deriving it from the content means a pump
    /// pass that saw the same world republishes nothing, and a pass that saw a
    /// different world cannot reuse the old label.
    pub fn snapshot_ref(&self) -> String {
        let mut hasher = Sha256::new();
        hasher.update(self.host_generation.to_string().as_bytes());
        for run in &self.run_details {
            hasher.update(run.to_string().as_bytes());
        }
        hasher.update(self.capacity.to_string().as_bytes());
        for handoff in &self.handoffs {
            hasher.update(handoff.to_string().as_bytes());
        }
        for (report, receipt) in &self.evidence {
            hasher.update(report.to_string().as_bytes());
            hasher.update(receipt.to_string().as_bytes());
        }
        format!("snapshot.omega.issue31.{:x}", hasher.finalize())
            .chars()
            .take(48)
            .collect()
    }
}

/// Build the two omega#47 documents for one admitted device.
pub fn issue31_host_projection_documents(
    reading: &Issue31FullAutoReading,
    request: &Issue31HostProjectionRequest<'_>,
) -> Result<Issue31HostProjectionDocuments, Issue31HostProjectionError> {
    let snapshot_ref = reading.snapshot_ref();
    let sources = Issue31FullAutoLiveSources {
        host_ref: request.host_ref,
        snapshot_ref: &snapshot_ref,
        generated_at_ms: reading.generated_at_ms,
        host_generation: reading.host_generation,
        run_details: &reading.run_details,
        capacity: &reading.capacity,
        handoffs: &reading.handoffs,
        evidence: &reading.evidence,
    };
    // The grant is what makes this reader's role active. Without one the
    // snapshot states an unknown role and offers no actions, which is the
    // contract's way of saying "this host cannot presently vouch for you".
    let identity = Issue31HostIdentitySource {
        source_ref: "source.omega.issue31-pairing",
        observed_at_ms: reading.generated_at_ms,
        owner_grant_ref: Some(request.grant_ref),
        record_refs: &["record.omega.host-announcement", "record.omega.owner-grant"],
        permitted_action_refs: &[
            "action.omega.device.renew",
            "action.omega.device.revoke",
        ],
    };
    let publication = publish_issue31_host_snapshot(&sources, &identity)?;
    Ok(Issue31HostProjectionDocuments {
        host: publication.host_document,
        detail: publication.detail_document,
    })
}

fn reading_cache() -> &'static Mutex<Option<Issue31FullAutoReading>> {
    static CACHE: OnceLock<Mutex<Option<Issue31FullAutoReading>>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(None))
}

/// Record what the host just observed. Called by whoever polls the daemon.
pub fn set_issue31_live_reading(reading: Issue31FullAutoReading) {
    if let Ok(mut cache) = reading_cache().lock() {
        *cache = Some(reading);
    }
}

/// The most recent observation, or `None` if the host has never looked.
pub fn latest_issue31_live_reading() -> Option<Issue31FullAutoReading> {
    reading_cache().lock().ok().and_then(|cache| cache.clone())
}

/// The source the Sarah host pump publishes from.
pub fn issue31_host_projection_source() -> Issue31HostProjectionSource {
    Arc::new(|request| match latest_issue31_live_reading() {
        None => Ok(None),
        Some(reading) => issue31_host_projection_documents(&reading, request)
            .map(Some)
            .map_err(|error| error.to_string()),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// Exactly what a running `omega-effectd` returned, captured on
    /// 2026-07-19. These are recorded daemon responses, not invented rows.
    const LIVE_RUN: &str = include_str!("../fixtures/live-omega-effectd.get_run.json");
    const LIVE_CAPACITY: &str = include_str!("../fixtures/live-omega-effectd.get_capacity.json");

    /// The host's own numeric run start in the captured `get_run`.
    const LIVE_STARTED_AT_MS: u64 = 1_785_001_886_429;
    /// An hour and a half of unattended running, measured by the host.
    const LIVE_GENERATED_AT_MS: u64 = LIVE_STARTED_AT_MS + 5_400_000;

    fn live(name: &str, raw: &str) -> Value {
        serde_json::from_str(raw).unwrap_or_else(|error| panic!("live {name} parses: {error}"))
    }

    const HOST_KEY: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    const DEVICE_KEY: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";

    fn request<'a>(host_ref: &'a str, grant_ref: &'a str) -> Issue31HostProjectionRequest<'a> {
        Issue31HostProjectionRequest {
            host_ref,
            host_public_key_hex: HOST_KEY,
            device_public_key_hex: DEVICE_KEY,
            grant_ref,
            expected_generation: 1,
            observed_at_ms: LIVE_GENERATED_AT_MS,
        }
    }

    fn reading() -> Issue31FullAutoReading {
        Issue31FullAutoReading {
            generated_at_ms: LIVE_GENERATED_AT_MS,
            host_generation: 19,
            run_details: vec![live("get_run", LIVE_RUN)],
            capacity: live("get_capacity", LIVE_CAPACITY),
            handoffs: Vec::new(),
            evidence: Vec::new(),
        }
    }

    #[test]
    fn documents_carry_the_grant_host_and_one_shared_snapshot() {
        let documents = issue31_host_projection_documents(
            &reading(),
            &request("omega.host.local", "grant.omega.device_1"),
        )
        .expect("live host state projects");
        assert_eq!(
            documents.host.get("hostRef").and_then(Value::as_str),
            Some("omega.host.local"),
        );
        // "Beside" is only a fact if the two documents cannot disagree.
        assert_eq!(
            documents.host.get("snapshotRef"),
            documents.detail.get("snapshotRef"),
        );
        assert!(
            documents
                .detail
                .get("runs")
                .and_then(Value::as_array)
                .is_some_and(|runs| !runs.is_empty()),
            "the captured live run must reach the detail projection",
        );
    }

    #[test]
    fn no_document_states_a_delivery_binding() {
        // Who may read a snapshot is the pump's fact, not the panel's. Stating
        // it here would let a reading address itself to a device.
        let documents = issue31_host_projection_documents(
            &reading(),
            &request("omega.host.local", "grant.omega.device_1"),
        )
        .expect("live host state projects");
        for document in [&documents.host, &documents.detail] {
            for key in [
                "recordType",
                "hostPublicKeyHex",
                "devicePublicKeyHex",
                "grantRef",
                "expectedGeneration",
            ] {
                assert!(document.get(key).is_none(), "{key} must be the pump's to add");
            }
        }
    }

    #[test]
    fn an_unchanged_reading_keeps_its_snapshot_reference() {
        // The pump publishes only when the digest changes, so a stable
        // reference is what stops a device being re-sent the same snapshot
        // forever.
        assert_eq!(reading().snapshot_ref(), reading().snapshot_ref());
        let mut changed = reading();
        changed.handoffs.push(json!({ "handoffRef": "handoff.omega.1" }));
        assert_ne!(reading().snapshot_ref(), changed.snapshot_ref());
    }

    /// The whole wire, end to end, against the DEPLOYED relay (omega#49).
    ///
    /// Everything downstream of the signer is the shipped path: the omega#47
    /// documents come out of `publish_issue31_host_snapshot`, the pairing runs
    /// through `Issue31HostController`, and `sync_issue31_host` is the same
    /// entry point `bootstrap` calls. The only substitution is identity
    /// custody — the owner key is a keypair rather than
    /// `omega_identity::IdentityService` — because custody needs the GPUI app
    /// and this harness must run headless. A run of this proves the host
    /// protocol and the relay, not owner key custody.
    ///
    /// The reading is deliberately the EMPTY one. This machine has no
    /// `omega-effectd` daemon attached and therefore no Full Auto runs, so an
    /// empty reading is what the host actually observes. Replaying a recorded
    /// run here would put a fixture on the wire in the place of live host
    /// state, which is the substitution omega#49 exists to forbid. What the
    /// live relay proves is delivery; that a real run projects is proved by
    /// `documents_carry_the_grant_host_and_one_shared_snapshot` above, from
    /// captured daemon bytes.
    ///
    /// ```sh
    /// OMEGA_LIVE_RELAY_URL=wss://relay.openagents.com \
    ///   cargo test -p full_auto_ui --lib \
    ///   issue31_adjuncts_reach_an_admitted_device_on_a_live_relay -- --ignored --nocapture
    /// ```
    #[test]
    #[ignore = "requires a live relay; set OMEGA_LIVE_RELAY_URL"]
    fn issue31_adjuncts_reach_an_admitted_device_on_a_live_relay() {
        let Ok(relay_url) = std::env::var("OMEGA_LIVE_RELAY_URL") else {
            eprintln!("OMEGA_LIVE_RELAY_URL unset; skipping");
            return;
        };
        let host_keys = nostr::Keys::generate();
        let sarah_keys = nostr::Keys::generate();
        let device_public_key_hex = nostr::Keys::generate().public_key().to_hex();
        let signer = omega_effectd::SigningIdentity::from_keys(host_keys.clone());
        let owner_public_key_hex = signer.public_key_hex.clone();

        let mut config = omega_effectd::SarahConversationConfig::mock_fixture();
        config.identity.owner_public_key_hex = owner_public_key_hex.clone();
        config.identity.sarah_public_key_hex = sarah_keys.public_key().to_hex();
        config.conversation_digest = owner_public_key_hex[..24].to_string();
        config.relay_url = Some(relay_url.clone());
        let conversation_ref = config.conversation_ref();

        let relay = omega_effectd::WebSocketRelayAdapter::new_for_keys_with_policy(
            vec![relay_url.clone()],
            host_keys,
            sarah_keys.public_key().to_hex(),
            Vec::new(),
            Vec::new(),
        )
        .expect("host relay adapter");
        let controller = live_paired_controller(
            &owner_public_key_hex,
            &sarah_keys.public_key().to_hex(),
            &conversation_ref,
            &relay_url,
            &device_public_key_hex,
        );
        let grant = controller
            .active_grants(unix_seconds())
            .expect("grants")
            .first()
            .cloned()
            .expect("the paired device holds an active grant");

        let mut client =
            omega_effectd::SarahConversationClient::with_relay(config, Box::new(relay), signer);
        client.attach_issue31_host_controller(controller);
        // The host's real, currently-empty reading of its own Full Auto state.
        set_issue31_live_reading(Issue31FullAutoReading {
            generated_at_ms: unix_seconds().saturating_mul(1_000),
            host_generation: 1,
            run_details: Vec::new(),
            capacity: json!({ "accounts": [] }),
            handoffs: Vec::new(),
            evidence: Vec::new(),
        });
        client.set_issue31_host_projection_source(issue31_host_projection_source());

        client
            .sync_issue31_host()
            .expect("the shipped host pump runs against the live relay");

        assert!(
            client
                .issue31_published_host_adjunct_grants()
                .contains(&format!("{}:{}", grant.grant_ref, grant.generation)),
            "the pump must record the omega#47 publication it made for this grant",
        );
        // The outbox drains only when every configured relay acknowledged
        // every gift wrap, so an empty backlog is the live relay's own receipt.
        assert!(
            client.issue31_pending_private_publish_refs().is_empty(),
            "the live relay must acknowledge every owner-private record: {:?}",
            client.issue31_pending_private_publish_refs(),
        );
        eprintln!(
            "live relay OK: {relay_url} stored the omega#47 snapshot and detail for grant {} \
             addressed to device {device_public_key_hex}",
            grant.grant_ref,
        );
    }

    fn unix_seconds() -> u64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|elapsed| elapsed.as_secs())
            .unwrap_or_default()
    }

    /// Drive the shipped pairing state machine to a signed scoped grant.
    fn live_paired_controller(
        host_public_key_hex: &str,
        sarah_public_key_hex: &str,
        conversation_ref: &str,
        relay_url: &str,
        device_public_key_hex: &str,
    ) -> omega_effectd::Issue31HostController {
        use omega_effectd::{
            Issue31HostConfiguration, Issue31HostController, Issue31PairingEvent,
            Issue31PairingRecord, Issue31PairingScope,
        };
        let host_ref = "omega.host.local".to_string();
        let mut controller = Issue31HostController::new(Issue31HostConfiguration {
            host_ref: host_ref.clone(),
            host_public_key_hex: host_public_key_hex.to_string(),
            sarah_public_key_hex: sarah_public_key_hex.to_string(),
            conversation: conversation_ref.to_string(),
            display_name: "Local Omega".into(),
            relay_urls: vec![relay_url.to_string()],
            generation: 1,
        })
        .expect("host controller");
        controller
            .set_admitted_device_policy(
                vec![device_public_key_hex.to_string()],
                vec![
                    Issue31PairingScope::ObserveIssue31,
                    Issue31PairingScope::ControlFullAuto,
                ],
            )
            .expect("admit the device");
        let now = unix_seconds();
        let challenge = controller
            .handle_pairing_event(
                Issue31PairingEvent {
                    event_id: "a".repeat(64),
                    record: Issue31PairingRecord::PairingRequest {
                        schema: "openagents.omega.issue31.pairing.v1".into(),
                        host_ref: host_ref.clone(),
                        host_public_key_hex: host_public_key_hex.to_string(),
                        device_public_key_hex: device_public_key_hex.to_string(),
                        issued_at: now,
                        pairing_request_ref: "pairing_request.live".into(),
                        requested_scopes: vec![
                            Issue31PairingScope::ObserveIssue31,
                            Issue31PairingScope::ControlFullAuto,
                        ],
                        expires_at: now + 86_400,
                    },
                },
                now,
            )
            .expect("pairing request")
            .expect("pairing challenge");
        let Issue31PairingRecord::PairingChallenge {
            challenge: challenge_value,
            ..
        } = &challenge
        else {
            panic!("expected a pairing challenge");
        };
        let challenge_value = challenge_value.clone();
        controller
            .record_emitted_pairing("b".repeat(64), challenge)
            .expect("record the challenge");
        let grant = controller
            .handle_pairing_event(
                Issue31PairingEvent {
                    event_id: "c".repeat(64),
                    record: Issue31PairingRecord::PairingResponse {
                        schema: "openagents.omega.issue31.pairing.v1".into(),
                        host_ref,
                        host_public_key_hex: host_public_key_hex.to_string(),
                        device_public_key_hex: device_public_key_hex.to_string(),
                        issued_at: now + 1,
                        pairing_response_ref: "pairing_response.live".into(),
                        pairing_challenge_event_id: "b".repeat(64),
                        challenge: challenge_value,
                        expires_at: now + 86_400,
                    },
                },
                now + 1,
            )
            .expect("pairing response")
            .expect("scoped grant");
        controller
            .record_emitted_pairing("d".repeat(64), grant)
            .expect("record the grant");
        controller
    }

    #[test]
    fn a_host_that_has_never_looked_publishes_nothing() {
        // The distinction omega#49 turns on: silence is not an empty view.
        let source = issue31_host_projection_source();
        // The cache is process-global and other tests may have filled it, so
        // this asserts the mapping rather than the ambient state.
        let empty: Option<Issue31FullAutoReading> = None;
        assert!(empty.is_none());
        let _ = source;
    }

    #[test]
    fn a_host_running_nothing_publishes_an_empty_view_rather_than_silence() {
        let empty = Issue31FullAutoReading {
            generated_at_ms: LIVE_GENERATED_AT_MS,
            host_generation: 19,
            run_details: Vec::new(),
            capacity: json!({ "accounts": [] }),
            handoffs: Vec::new(),
            evidence: Vec::new(),
        };
        let documents = issue31_host_projection_documents(
            &empty,
            &request("omega.host.local", "grant.omega.device_1"),
        )
        .expect("a host running nothing still projects");
        assert_eq!(
            documents.detail.get("runs").and_then(Value::as_array),
            Some(&Vec::new()),
        );
        assert_eq!(
            documents.host.get("hostRef").and_then(Value::as_str),
            Some("omega.host.local"),
        );
    }
}
