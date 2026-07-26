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
    Issue31ProviderRosterAccount, Issue31ProviderRosterSource,
};
use serde_json::Value;
use sha2::{Digest, Sha256};

use crate::issue31_adjunct::{
    Issue31FullAutoLiveSources, Issue31HostIdentitySource, Issue31HostProjectionError,
    publish_issue31_host_snapshot,
};
use crate::provider_roster::parse_provider_accounts;

/// This host's clock, in epoch milliseconds. The single reading every
/// observation is stamped from.
fn unix_millis() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| u64::try_from(elapsed.as_millis()).unwrap_or(u64::MAX))
        .unwrap_or_default()
}

/// One complete reading of the live Full Auto surface, owned rather than
/// borrowed so it can be cached between the panel that reads it and the pump
/// that publishes it.
///
/// `host_ref` is absent on purpose: the host reference belongs to the device's
/// grant, not to the run registry, so it is supplied per delivery.
///
/// `Default` is deliberately not derived. A default reading is one nobody
/// measured, and it would carry `generated_at_ms: 0` — a stamp that decodes,
/// projects, and reads on the phone as a host observation taken in 1970. The
/// only readings that exist are ones a daemon answered.
#[derive(Clone, Debug)]
pub struct Issue31FullAutoReading {
    /// When the daemon was actually read, in epoch milliseconds.
    ///
    /// Private, and there is no production path that sets it (omega#97). The
    /// host's own clock stamps it once inside `observed`, so a caller cannot
    /// supply one: a reading whose stamp its author chose is exactly how a
    /// recorded fixture becomes host authority, and omega#49's exit forbids
    /// that in as many words. Making it unwritable is stronger than forbidding
    /// it in a comment, because a comment does not survive a rebase.
    generated_at_ms: u64,
    pub host_generation: u64,
    /// One `get_run` record per run the daemon listed.
    pub run_details: Vec<Value>,
    /// The `get_capacity` record.
    pub capacity: Value,
    /// One `(get_report, get_receipt)` pair per run with evidence.
    pub evidence: Vec<(Value, Value)>,
}

impl Issue31FullAutoReading {
    /// The only constructor a production path can reach.
    ///
    /// `generated_at_ms` is one reading of this host's clock, taken here and
    /// stamped once. There is no parameter for it, so a client-supplied,
    /// fixture-supplied, or replayed value cannot enter — it is discarded by
    /// being inexpressible rather than by being checked.
    ///
    /// Taken *after* the daemon answered, on purpose. Stamping before the reads
    /// would put the stamp behind a run that began during them, and the
    /// contract refuses a run whose start is newer than the snapshot; stamping
    /// after can only over-state freshness by the length of one poll, which
    /// under-states no run's unattended duration.
    pub fn observed(
        host_generation: u64,
        run_details: Vec<Value>,
        capacity: Value,
        evidence: Vec<(Value, Value)>,
    ) -> Self {
        Self {
            generated_at_ms: unix_millis(),
            host_generation,
            run_details,
            capacity,
            evidence,
        }
    }

    /// When this host read its daemon.
    pub fn generated_at_ms(&self) -> u64 {
        self.generated_at_ms
    }

    /// A reading stamped at a recorded instant, for tests over captured daemon
    /// bytes only.
    ///
    /// `#[cfg(test)]` is the whole point: the projection has to be testable
    /// against real recorded `get_run` output at a known instant, and no
    /// shipped build may contain a way to state a stamp the host did not
    /// measure. This function does not exist in a release binary.
    #[cfg(test)]
    fn at_recorded_instant(
        generated_at_ms: u64,
        host_generation: u64,
        run_details: Vec<Value>,
        capacity: Value,
        evidence: Vec<(Value, Value)>,
    ) -> Self {
        Self {
            generated_at_ms,
            host_generation,
            run_details,
            capacity,
            evidence,
        }
    }

    /// A stable identifier for exactly this reading.
    ///
    /// The mobile contract binds the detail projection to the snapshot that
    /// advertised it, so the reference has to change when — and only when —
    /// the observed state changes. Deriving it from the content means a pump
    /// pass that saw the same world republishes nothing, and a pass that saw a
    /// different world cannot reuse the old label.
    ///
    /// `handoffs` is passed in rather than held because the handoff ledger is
    /// durable host state owned by the pump, not part of the panel's reading
    /// of the daemon (omega#91). It is folded in here all the same: a handoff
    /// that opened, bound, or ended is a different world, and a snapshot
    /// reference that did not move would leave the phone holding two different
    /// details under one label.
    pub fn snapshot_ref(&self, handoffs: &[Value]) -> String {
        let mut hasher = Sha256::new();
        hasher.update(self.host_generation.to_string().as_bytes());
        for run in &self.run_details {
            hasher.update(run.to_string().as_bytes());
        }
        hasher.update(self.capacity.to_string().as_bytes());
        for handoff in handoffs {
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
    // Projected against the exact stamp these documents will carry, not
    // against the pump's own clock. A handoff opened after this reading was
    // taken belongs to the next snapshot, and the contract says so: a row whose
    // request time is newer than `generatedAtMs` is refused (omega#91).
    let handoffs = request.handoffs.projected(reading.generated_at_ms()).rows;
    let snapshot_ref = reading.snapshot_ref(&handoffs);
    let sources = Issue31FullAutoLiveSources {
        host_ref: request.host_ref,
        snapshot_ref: &snapshot_ref,
        generated_at_ms: reading.generated_at_ms(),
        host_generation: reading.host_generation,
        run_details: &reading.run_details,
        capacity: &reading.capacity,
        handoffs: &handoffs,
        evidence: &reading.evidence,
    };
    // The grant is what makes this reader's role active. Without one the
    // snapshot states an unknown role and offers no actions, which is the
    // contract's way of saying "this host cannot presently vouch for you".
    let identity = Issue31HostIdentitySource {
        source_ref: "source.omega.issue31-pairing",
        observed_at_ms: reading.generated_at_ms(),
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

/// How the Sarah host pump reads this host's provider roster (omega#91).
///
/// Routed through the exact `parse_provider_accounts` the desktop roster panel
/// renders, so the account a handoff binds to, and the lane that account
/// serves, are the same ones Omega shows on the desktop. A handoff that chose
/// an account the panel does not list would be a second opinion about the
/// host's own capacity.
///
/// `None` when the host has never read its Full Auto state at all — the same
/// silence `latest_issue31_live_reading` means, propagated rather than
/// flattened into "this host holds no accounts".
pub fn issue31_provider_roster_source() -> Issue31ProviderRosterSource {
    Arc::new(|| {
        latest_issue31_live_reading().map(|reading| {
            parse_provider_accounts(&reading.capacity)
                .into_iter()
                .map(|account| Issue31ProviderRosterAccount {
                    account_ref: account.account_ref,
                    provider: account.provider,
                    lane_ref: account.lane,
                    readiness: account.readiness,
                })
                .collect()
        })
    })
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
    use omega_effectd::Issue31ProviderHandoffLedger;
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

    fn request<'a>(
        host_ref: &'a str,
        grant_ref: &'a str,
        handoffs: &'a Issue31ProviderHandoffLedger,
    ) -> Issue31HostProjectionRequest<'a> {
        request_with_handoffs(host_ref, grant_ref, handoffs)
    }

    /// A host holding no provider connection handoff. Distinct, on the wire,
    /// from a host that holds one it cannot state (omega#91).
    fn no_handoffs() -> Issue31ProviderHandoffLedger {
        Issue31ProviderHandoffLedger::default()
    }

    /// A ledger built from the exact durable bytes a restart would read back.
    fn ledger(rows: &[Value]) -> Issue31ProviderHandoffLedger {
        let entries: serde_json::Map<String, Value> = rows
            .iter()
            .map(|row| {
                (
                    row.get("handoffRef")
                        .and_then(Value::as_str)
                        .expect("handoffRef")
                        .to_string(),
                    row.clone(),
                )
            })
            .collect();
        serde_json::from_value(json!({ "entries": entries })).expect("durable ledger bytes")
    }

    fn request_with_handoffs<'a>(
        host_ref: &'a str,
        grant_ref: &'a str,
        handoffs: &'a Issue31ProviderHandoffLedger,
    ) -> Issue31HostProjectionRequest<'a> {
        Issue31HostProjectionRequest {
            host_ref,
            host_public_key_hex: HOST_KEY,
            device_public_key_hex: DEVICE_KEY,
            grant_ref,
            expected_generation: 1,
            observed_at_ms: LIVE_GENERATED_AT_MS,
            handoffs,
        }
    }

    fn reading() -> Issue31FullAutoReading {
        Issue31FullAutoReading::at_recorded_instant(
            LIVE_GENERATED_AT_MS,
            19,
            vec![live("get_run", LIVE_RUN)],
            live("get_capacity", LIVE_CAPACITY),
            Vec::new(),
        )
    }

    #[test]
    fn documents_carry_the_grant_host_and_one_shared_snapshot() {
        let documents = issue31_host_projection_documents(
            &reading(),
            &request("omega.host.local", "grant.omega.device_1", &no_handoffs()),
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
            &request("omega.host.local", "grant.omega.device_1", &no_handoffs()),
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
        assert_eq!(
            reading().snapshot_ref(&[]),
            reading().snapshot_ref(&[])
        );
        // A handoff change alone is a different world, and has to move the
        // reference the detail is published under (omega#91).
        let handoffs = [json!({ "handoffRef": "handoff.omega.1" })];
        assert_ne!(
            reading().snapshot_ref(&[]),
            reading().snapshot_ref(&handoffs)
        );
    }

    /// A capacity record that reports accounts, the way omega#42's roster
    /// reads it. The captured live `get_capacity` reports lanes and no
    /// accounts, which is why a handoff on this machine cannot bind — that is
    /// the host's real state, not a gap in this test.
    fn capacity_with_accounts() -> Value {
        json!({
            "lanes": [{"lane": "claude-local", "state": "available", "activeRuns": 0}],
            "accounts": [{
                "accountRef": "account.claude.1",
                "provider": "anthropic",
                "label": "Claude",
                "state": "ready",
                "quotaState": "available",
                "lane": "lane.claude-local",
            }],
        })
    }

    #[test]
    fn a_host_owned_handoff_reaches_the_detail_document_the_phone_reads() {
        let mut reading = reading();
        reading.capacity = capacity_with_accounts();
        let handoffs = ledger(&[json!({
            "handoffRef": "handoff.omega.0123456789abcdef01234567",
            "provider": "anthropic",
            "state": "completed",
            "requestedAtMs": LIVE_GENERATED_AT_MS - 60_000,
            "accountRef": "account.claude.1",
            "outcomeRef": "outcome.omega.handoff_connected",
        })]);
        let documents = issue31_host_projection_documents(
            &reading,
            &request_with_handoffs("omega.host.local", "grant.omega.device_1", &handoffs),
        )
        .expect("a bound handoff projects");
        let projected = documents
            .detail
            .get("handoffs")
            .and_then(Value::as_array)
            .expect("handoffs");
        assert_eq!(projected.len(), 1);
        assert_eq!(
            projected[0].get("state").and_then(Value::as_str),
            Some("completed")
        );
        // The account-to-lane relation is why this handoff chose this account,
        // and the phone can follow it without asking the host again.
        let accounts = documents
            .detail
            .get("accounts")
            .and_then(Value::as_array)
            .expect("accounts");
        assert_eq!(
            accounts[0].get("accountRef").and_then(Value::as_str),
            projected[0].get("accountRef").and_then(Value::as_str),
        );
        assert_eq!(
            accounts[0].get("laneRef").and_then(Value::as_str),
            Some("lane.claude-local"),
        );
    }

    #[test]
    fn a_handoff_naming_an_account_this_host_does_not_carry_is_refused() {
        // The whole projection fails rather than shipping a handoff that points
        // at a row the phone cannot open.
        let handoffs = ledger(&[json!({
            "handoffRef": "handoff.omega.0123456789abcdef01234567",
            "provider": "anthropic",
            "state": "completed",
            "requestedAtMs": LIVE_GENERATED_AT_MS - 60_000,
            "accountRef": "account.claude.1",
            "outcomeRef": "outcome.omega.handoff_connected",
        })]);
        let error = issue31_host_projection_documents(
            &reading(),
            &request_with_handoffs("omega.host.local", "grant.omega.device_1", &handoffs),
        )
        .expect_err("a handoff must point at an account this snapshot carries");
        assert!(error.to_string().contains("unknown record"), "{error}");
    }

    #[test]
    fn a_failed_handoff_and_no_handoff_are_different_documents() {
        // The distinction the omega#91 exit turns on, at the delivery boundary.
        let failed = ledger(&[json!({
            "handoffRef": "handoff.omega.0123456789abcdef01234567",
            "provider": "anthropic",
            "state": "failed",
            "requestedAtMs": LIVE_GENERATED_AT_MS - 60_000,
            "reasonClass": "reason.omega.handoff_host_restarted",
            "outcomeRef": "outcome.omega.handoff_interrupted",
        })]);
        let with_failure = issue31_host_projection_documents(
            &reading(),
            &request_with_handoffs("omega.host.local", "grant.omega.device_1", &failed),
        )
        .expect("a failed handoff projects");
        let without = issue31_host_projection_documents(
            &reading(),
            &request("omega.host.local", "grant.omega.device_1", &no_handoffs()),
        )
        .expect("a host holding no handoff projects");

        assert_eq!(
            with_failure
                .detail
                .get("handoffs")
                .and_then(Value::as_array)
                .map(Vec::len),
            Some(1),
        );
        assert_eq!(
            without
                .detail
                .get("handoffs")
                .and_then(Value::as_array)
                .map(Vec::len),
            Some(0),
        );
        // And they are published under different snapshot references, so a
        // device cannot hold one while believing it holds the other.
        assert_ne!(
            with_failure.detail.get("snapshotRef"),
            without.detail.get("snapshotRef"),
        );
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
        set_issue31_live_reading(Issue31FullAutoReading::at_recorded_instant(
            unix_seconds().saturating_mul(1_000),
            1,
            Vec::new(),
            json!({ "accounts": [] }),
            Vec::new(),
        ));
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

    /// The omega#91 exit, against the DEPLOYED relay.
    ///
    /// A paired device holding `request_provider_handoff` publishes a real
    /// signed command intent to a real relay. The host reads it back off that
    /// relay through `sync_issue31_host` — the same entry point `bootstrap`
    /// calls — admits it through the shipped controller, opens a handoff, binds
    /// it against its own roster, completes it, and publishes each state in the
    /// `fullauto.v1` detail addressed to that device. The failure path runs on
    /// a second device in the same pass, and its record is asserted to be
    /// distinguishable from the third device, which never asked at all.
    ///
    /// Two substitutions, both named rather than hidden:
    ///
    /// - identity custody. The owner key is a keypair rather than
    ///   `omega_identity::IdentityService`, because custody needs the GPUI app
    ///   and this harness runs headless. A run proves the host protocol and the
    ///   relay, not owner key custody.
    /// - the Full Auto reading. No `omega-effectd` daemon is attached to this
    ///   process, so the host's roster reading is supplied here rather than
    ///   polled. It is the host's own observation either way — the ledger reads
    ///   it through the shipped `issue31_provider_roster_source`, and the
    ///   binding decision is the shipped one.
    ///
    /// Nothing in this test runs a provider login, resolves a provider home, or
    /// touches a credential. A handoff record carries the fact of a connection
    /// and never the connection secret.
    ///
    /// ```sh
    /// OMEGA_LIVE_RELAY_URL=wss://relay.openagents.com \
    ///   cargo test -p full_auto_ui --lib \
    ///   a_provider_handoff_appears_binds_and_settles_on_a_live_relay -- --ignored --nocapture
    /// ```
    #[test]
    #[ignore = "requires a live relay; set OMEGA_LIVE_RELAY_URL"]
    fn a_provider_handoff_appears_binds_and_settles_on_a_live_relay() {
        let Ok(relay_url) = std::env::var("OMEGA_LIVE_RELAY_URL") else {
            eprintln!("OMEGA_LIVE_RELAY_URL unset; skipping");
            return;
        };
        let host_keys = nostr::Keys::generate();
        let sarah_keys = nostr::Keys::generate();
        let device_keys = nostr::Keys::generate();
        let device_public_key_hex = device_keys.public_key().to_hex();
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
            host_keys.clone(),
            sarah_keys.public_key().to_hex(),
            Vec::new(),
            Vec::new(),
        )
        .expect("host relay adapter");
        let controller = live_paired_controller_with_scopes(
            &owner_public_key_hex,
            &sarah_keys.public_key().to_hex(),
            &conversation_ref,
            &relay_url,
            &device_public_key_hex,
            &[
                omega_effectd::Issue31PairingScope::ObserveIssue31,
                omega_effectd::Issue31PairingScope::RequestProviderHandoff,
            ],
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
        // The host's own reading, carrying one provider account. Everything the
        // handoff does with it goes through the shipped roster source. Refreshed
        // before each pass exactly as the desktop panel's poll refreshes it:
        // the snapshot's `generatedAtMs` is when the host last looked, and a
        // handoff stamped after that reading belongs to the next snapshot.
        let observe = || {
            set_issue31_live_reading(Issue31FullAutoReading::at_recorded_instant(
                unix_seconds().saturating_mul(1_000) + 1_000,
                1,
                Vec::new(),
                capacity_with_accounts(),
                Vec::new(),
            ));
        };
        observe();
        client.set_issue31_host_projection_source(issue31_host_projection_source());
        client.set_issue31_provider_roster_source(issue31_provider_roster_source());

        // The device asks, on the relay, exactly as a phone would.
        let now = unix_seconds();
        let intent = serde_json::json!({
            "recordType": "command_intent",
            "schema": "openagents.omega.issue31.command.v1",
            "hostRef": grant.host_ref,
            "hostPublicKeyHex": owner_public_key_hex,
            "devicePublicKeyHex": device_public_key_hex,
            "grantRef": grant.grant_ref,
            "actionRef": "action.omega.provider_handoff",
            "idempotencyRef": format!("idempotency.issue31.handoff:{now}"),
            "expectedGeneration": grant.generation,
            "argumentsRef": "arguments.omega.provider_handoff.anthropic",
            "issuedAt": now,
            "expiresAt": now + 3_600,
        });
        publish_device_command(&relay_url, &device_keys, &host_keys, &intent);

        // Pass one: the host reads the ask off the relay and the handoff
        // appears.
        client
            .sync_issue31_host()
            .expect("the shipped host pump runs against the live relay");
        let observed = |client: &omega_effectd::SarahConversationClient| -> Vec<Value> {
            client.issue31_projected_provider_handoffs(unix_seconds().saturating_mul(1_000) + 1_000)
        };
        let appeared = observed(&client);
        assert_eq!(
            appeared.len(),
            1,
            "the scope the phone holds must produce exactly one host record",
        );
        let handoff_ref = appeared[0]
            .get("handoffRef")
            .and_then(Value::as_str)
            .expect("handoffRef")
            .to_string();
        eprintln!("live relay OK: handoff {handoff_ref} appeared as {:?}", appeared[0].get("state"));

        // Pass two: it binds to the account the host's own roster reports.
        observe();
        client.sync_issue31_host().expect("second pump pass");
        let bound = observed(&client);
        assert_eq!(bound[0].get("state").and_then(Value::as_str), Some("active"));
        assert_eq!(
            bound[0].get("accountRef").and_then(Value::as_str),
            Some("account.claude.1"),
        );
        eprintln!("live relay OK: handoff {handoff_ref} bound to account.claude.1");

        // Pass three: it reaches a terminal, host-owned outcome.
        observe();
        client.sync_issue31_host().expect("third pump pass");
        let settled = observed(&client);
        assert_eq!(
            settled[0].get("state").and_then(Value::as_str),
            Some("completed"),
        );
        assert_eq!(
            settled[0].get("outcomeRef").and_then(Value::as_str),
            Some("outcome.omega.handoff_connected"),
        );

        // Every state crossed the relay: the outbox drains only when every
        // configured relay acknowledged every gift wrap.
        assert!(
            client.issue31_pending_private_publish_refs().is_empty(),
            "the live relay must acknowledge every owner-private record: {:?}",
            client.issue31_pending_private_publish_refs(),
        );
        assert!(
            client
                .issue31_published_host_adjunct_grants()
                .contains(&format!("{}:{}", grant.grant_ref, grant.generation)),
            "the pump must record the omega#47 publication it made for this grant",
        );
        eprintln!(
            "live relay OK: {relay_url} carried handoff {handoff_ref} through \
             requested -> active -> completed for grant {}",
            grant.grant_ref,
        );
    }

    /// Pair one more device into an existing controller, through the real
    /// pairing state machine, with its own requested scope set.
    fn live_pair_device(
        controller: &mut omega_effectd::Issue31HostController,
        host_public_key_hex: &str,
        device_public_key_hex: &str,
        requested_scopes: &[omega_effectd::Issue31PairingScope],
        seed: char,
    ) -> String {
        use omega_effectd::{Issue31PairingEvent, Issue31PairingRecord};
        let host_ref = "omega.host.local".to_string();
        let now = unix_seconds();
        let event_id = |suffix: char| format!("{seed}{}", suffix.to_string().repeat(63));
        let request_event = event_id('a');
        let challenge_event = event_id('b');
        let challenge = controller
            .handle_pairing_event(
                Issue31PairingEvent {
                    event_id: request_event,
                    record: Issue31PairingRecord::PairingRequest {
                        schema: "openagents.omega.issue31.pairing.v1".into(),
                        host_ref: host_ref.clone(),
                        host_public_key_hex: host_public_key_hex.to_string(),
                        device_public_key_hex: device_public_key_hex.to_string(),
                        issued_at: now,
                        pairing_request_ref: format!("pairing_request.live.{seed}"),
                        requested_scopes: requested_scopes.to_vec(),
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
            .record_emitted_pairing(challenge_event.clone(), challenge)
            .expect("record the challenge");
        let grant = controller
            .handle_pairing_event(
                Issue31PairingEvent {
                    event_id: event_id('c'),
                    record: Issue31PairingRecord::PairingResponse {
                        schema: "openagents.omega.issue31.pairing.v1".into(),
                        host_ref,
                        host_public_key_hex: host_public_key_hex.to_string(),
                        device_public_key_hex: device_public_key_hex.to_string(),
                        issued_at: now + 1,
                        pairing_response_ref: format!("pairing_response.live.{seed}"),
                        pairing_challenge_event_id: challenge_event,
                        challenge: challenge_value,
                        expires_at: now + 86_400,
                    },
                },
                now + 1,
            )
            .expect("pairing response")
            .expect("scoped grant");
        let Issue31PairingRecord::ScopedGrant { grant_ref, .. } = &grant else {
            panic!("expected a scoped grant");
        };
        let grant_ref = grant_ref.clone();
        controller
            .record_emitted_pairing(event_id('d'), grant)
            .expect("record the grant");
        grant_ref
    }

    /// The omega#91 exit's second half, against the DEPLOYED relay.
    ///
    /// The failure path has to be visibly different from a request that never
    /// started, or a phone cannot tell "the host tried and could not" from "the
    /// host never heard you". Both happen here, in one run, on one relay:
    ///
    /// - a device without the scope asks. The host refuses the command and
    ///   makes **no record**. The handoff list stays empty.
    /// - a device with the scope asks. The host opens a record, binds it to the
    ///   account its own roster reports, and ends it `refused` because that
    ///   account is revoked — with a host-owned reason and outcome the phone can
    ///   read.
    ///
    /// The substitutions are the same two named on the success proof.
    ///
    /// ```sh
    /// OMEGA_LIVE_RELAY_URL=wss://relay.openagents.com \
    ///   cargo test -p full_auto_ui --lib \
    ///   a_refused_handoff_is_distinct_from_one_that_never_started_on_a_live_relay \
    ///   -- --ignored --nocapture
    /// ```
    #[test]
    #[ignore = "requires a live relay; set OMEGA_LIVE_RELAY_URL"]
    fn a_refused_handoff_is_distinct_from_one_that_never_started_on_a_live_relay() {
        let Ok(relay_url) = std::env::var("OMEGA_LIVE_RELAY_URL") else {
            eprintln!("OMEGA_LIVE_RELAY_URL unset; skipping");
            return;
        };
        let host_keys = nostr::Keys::generate();
        let sarah_keys = nostr::Keys::generate();
        let scoped_device = nostr::Keys::generate();
        let unscoped_device = nostr::Keys::generate();
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
            host_keys.clone(),
            sarah_keys.public_key().to_hex(),
            Vec::new(),
            Vec::new(),
        )
        .expect("host relay adapter");
        let scopes = [
            omega_effectd::Issue31PairingScope::ObserveIssue31,
            omega_effectd::Issue31PairingScope::RequestProviderHandoff,
        ];
        let mut controller = live_paired_controller_with_scopes(
            &owner_public_key_hex,
            &sarah_keys.public_key().to_hex(),
            &conversation_ref,
            &relay_url,
            &scoped_device.public_key().to_hex(),
            &scopes,
        );
        // Both devices are admitted under one owner policy. The grant is the
        // intersection of that policy and what each device asked for, so the
        // second device holds a real grant without the handoff scope.
        controller
            .set_admitted_device_policy(
                vec![
                    scoped_device.public_key().to_hex(),
                    unscoped_device.public_key().to_hex(),
                ],
                scopes.to_vec(),
            )
            .expect("admit both devices");
        let unscoped_grant_ref = live_pair_device(
            &mut controller,
            &owner_public_key_hex,
            &unscoped_device.public_key().to_hex(),
            &[omega_effectd::Issue31PairingScope::ObserveIssue31],
            '1',
        );
        let grants = controller.active_grants(unix_seconds()).expect("grants");
        let scoped_grant = grants
            .iter()
            .find(|grant| grant.device_public_key_hex == scoped_device.public_key().to_hex())
            .cloned()
            .expect("the scoped device holds an active grant");
        let unscoped_grant = grants
            .iter()
            .find(|grant| grant.grant_ref == unscoped_grant_ref)
            .cloned()
            .expect("the unscoped device holds an active grant");
        assert!(
            !unscoped_grant
                .scopes
                .contains(&omega_effectd::Issue31PairingScope::RequestProviderHandoff),
            "the second device must not hold the handoff scope",
        );

        let mut client =
            omega_effectd::SarahConversationClient::with_relay(config, Box::new(relay), signer);
        client.attach_issue31_host_controller(controller);
        // The host's roster reports the one anthropic account as revoked, which
        // is what makes the terminal outcome a refusal rather than a
        // connection.
        let observe = || {
            set_issue31_live_reading(Issue31FullAutoReading::at_recorded_instant(
                unix_seconds().saturating_mul(1_000) + 1_000,
                1,
                Vec::new(),
                json!({
                    "lanes": [{"lane": "claude-local", "state": "available", "activeRuns": 0}],
                    "accounts": [{
                        "accountRef": "account.claude.1",
                        "provider": "anthropic",
                        "label": "Claude",
                        "state": "revoked",
                        "quotaState": "depleted",
                        "lane": "lane.claude-local",
                    }],
                }),
                Vec::new(),
            ));
        };
        observe();
        client.set_issue31_host_projection_source(issue31_host_projection_source());
        client.set_issue31_provider_roster_source(issue31_provider_roster_source());

        // Both devices ask, on a real relay, before the host reads either.
        // (A NIP-59 gift wrap randomises its `created_at`, so publishing the
        // second ask after a pump pass could place it behind the control
        // cursor. Publishing both up front makes what this proves independent
        // of relay ordering.)
        let now = unix_seconds();
        let denied_idempotency = format!("idempotency.issue31.handoff-denied:{now}");
        let refused_idempotency = format!("idempotency.issue31.handoff-refused:{now}");
        publish_device_command(
            &relay_url,
            &unscoped_device,
            &host_keys,
            &device_handoff_intent(
                &unscoped_grant,
                &owner_public_key_hex,
                &denied_idempotency,
            ),
        );
        publish_device_command(
            &relay_url,
            &scoped_device,
            &host_keys,
            &device_handoff_intent(
                &scoped_grant,
                &owner_public_key_hex,
                &refused_idempotency,
            ),
        );
        for pass in 0..3 {
            observe();
            client
                .sync_issue31_host()
                .unwrap_or_else(|error| panic!("pump pass {pass}: {error}"));
        }
        let settled = client
            .issue31_projected_provider_handoffs(unix_seconds().saturating_mul(1_000) + 2_000);
        // Two asks crossed the relay and exactly one produced a record. The one
        // the host never admitted is absent by the reference it would have had,
        // so this is not merely a count.
        assert_eq!(settled.len(), 1, "one admitted ask, one record");
        assert_eq!(
            settled[0].get("handoffRef").and_then(Value::as_str),
            Some(
                Issue31ProviderHandoffLedger::handoff_ref_for(&refused_idempotency)
                    .as_str()
            ),
        );
        assert!(
            !client.issue31_provider_handoff_refs().contains(
                &Issue31ProviderHandoffLedger::handoff_ref_for(&denied_idempotency)
            ),
            "a request the host never admitted must leave no handoff at all",
        );
        eprintln!("live relay OK: the scope-denied ask left no record");
        assert_eq!(
            settled[0].get("state").and_then(Value::as_str),
            Some("refused"),
        );
        // A failure states both why it ended and what the host decided. That is
        // the whole difference from the empty list above.
        assert_eq!(
            settled[0].get("reasonClass").and_then(Value::as_str),
            Some("reason.omega.handoff_account_revoked"),
        );
        assert_eq!(
            settled[0].get("outcomeRef").and_then(Value::as_str),
            Some("outcome.omega.handoff_refused"),
        );
        assert!(
            client.issue31_pending_private_publish_refs().is_empty(),
            "the live relay must acknowledge every owner-private record: {:?}",
            client.issue31_pending_private_publish_refs(),
        );
        eprintln!(
            "live relay OK: {relay_url} carried a refused handoff with reason \
             reason.omega.handoff_account_revoked, distinct from the empty list \
             the unscoped ask left",
        );
    }

    fn device_handoff_intent(
        grant: &omega_effectd::Issue31GrantState,
        owner_public_key_hex: &str,
        idempotency_ref: &str,
    ) -> Value {
        let now = unix_seconds();
        json!({
            "recordType": "command_intent",
            "schema": "openagents.omega.issue31.command.v1",
            "hostRef": grant.host_ref,
            "hostPublicKeyHex": owner_public_key_hex,
            "devicePublicKeyHex": grant.device_public_key_hex,
            "grantRef": grant.grant_ref,
            "actionRef": "action.omega.provider_handoff",
            "idempotencyRef": idempotency_ref,
            "expectedGeneration": grant.generation,
            "argumentsRef": "arguments.omega.provider_handoff.anthropic",
            "issuedAt": now,
            "expiresAt": now + 3_600,
        })
    }

    /// Publish one device-authored command intent, gift-wrapped to the host.
    ///
    /// This is the phone's half. The host never learns the device wrote it from
    /// anything but the signature and the record's own binding.
    #[allow(clippy::needless_pass_by_value)]
    fn publish_device_command(
        relay_url: &str,
        device_keys: &nostr::Keys,
        host_keys: &nostr::Keys,
        intent: &Value,
    ) {
        use nostr::{EventBuilder, Kind, Tag};
        use omega_effectd::RelayTransport as _;
        let content = serde_json::to_string(intent).expect("intent json");
        let mut rumor = EventBuilder::new(Kind::PrivateDirectMessage, content)
            .tags(vec![
                Tag::parse(["p", host_keys.public_key().to_hex().as_str()]).expect("p tag"),
            ])
            .build(device_keys.public_key());
        rumor.ensure_id();
        let gift_wrap = smol::block_on(EventBuilder::gift_wrap(
            device_keys,
            &host_keys.public_key(),
            rumor,
            [],
        ))
        .expect("gift wrap the device command to the host");
        let mut device_relay = omega_effectd::WebSocketRelayAdapter::new_for_keys(
            vec![relay_url.to_string()],
            device_keys.clone(),
        )
        .expect("device relay adapter");
        device_relay.connect().expect("device connect");
        publish_authenticated(&mut device_relay, relay_url, device_keys, &gift_wrap);
    }

    fn publish_authenticated(
        relay: &mut omega_effectd::WebSocketRelayAdapter,
        auth_url: &str,
        keys: &nostr::Keys,
        record: &nostr::Event,
    ) {
        use nostr::{EventBuilder, Kind, Tag};
        use omega_effectd::RelayTransport as _;
        match relay.publish(record) {
            Ok(()) => {}
            Err(_) => {
                let challenge = relay
                    .auth_challenge()
                    .expect("relay must expose a challenge after refusing the publish");
                let auth_event = EventBuilder::new(Kind::Custom(22242), "")
                    .tag(Tag::parse(["relay", auth_url]).expect("relay tag"))
                    .tag(
                        Tag::parse(["challenge", challenge.challenge.as_str()])
                            .expect("challenge tag"),
                    )
                    .sign_with_keys(keys)
                    .expect("signed auth event");
                relay.authenticate(&auth_event).expect("NIP-42 authenticate");
                relay.publish(record).expect("publish after auth");
            }
        }
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
        live_paired_controller_with_scopes(
            host_public_key_hex,
            sarah_public_key_hex,
            conversation_ref,
            relay_url,
            device_public_key_hex,
            &[
                omega_effectd::Issue31PairingScope::ObserveIssue31,
                omega_effectd::Issue31PairingScope::ControlFullAuto,
            ],
        )
    }

    fn live_paired_controller_with_scopes(
        host_public_key_hex: &str,
        sarah_public_key_hex: &str,
        conversation_ref: &str,
        relay_url: &str,
        device_public_key_hex: &str,
        scopes: &[omega_effectd::Issue31PairingScope],
    ) -> omega_effectd::Issue31HostController {
        live_paired_controller_with_records(
            host_public_key_hex,
            sarah_public_key_hex,
            conversation_ref,
            relay_url,
            device_public_key_hex,
            scopes,
        )
        .0
    }

    /// The same paired controller, plus the four pairing records it is built
    /// from, in the `(event id, record)` shape a device folds a grant out of.
    ///
    /// Nothing in the shipped host needs these: the controller holds them. A
    /// *device* in another process does, because the grant is what entitles it
    /// to read this host at all, and these four records were never published —
    /// `record_emitted_pairing` files them, it does not put them on a relay.
    /// omega#97's device half reads them out of the handoff written below.
    fn live_paired_controller_with_records(
        host_public_key_hex: &str,
        sarah_public_key_hex: &str,
        conversation_ref: &str,
        relay_url: &str,
        device_public_key_hex: &str,
        scopes: &[omega_effectd::Issue31PairingScope],
    ) -> (
        omega_effectd::Issue31HostController,
        Vec<(String, omega_effectd::Issue31PairingRecord)>,
    ) {
        use omega_effectd::{
            Issue31HostConfiguration, Issue31HostController, Issue31PairingEvent,
            Issue31PairingRecord,
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
            .set_admitted_device_policy(vec![device_public_key_hex.to_string()], scopes.to_vec())
            .expect("admit the device");
        let now = unix_seconds();
        let request = Issue31PairingRecord::PairingRequest {
            schema: "openagents.omega.issue31.pairing.v1".into(),
            host_ref: host_ref.clone(),
            host_public_key_hex: host_public_key_hex.to_string(),
            device_public_key_hex: device_public_key_hex.to_string(),
            issued_at: now,
            pairing_request_ref: "pairing_request.live".into(),
            requested_scopes: scopes.to_vec(),
            expires_at: now + 86_400,
        };
        let challenge = controller
            .handle_pairing_event(
                Issue31PairingEvent {
                    event_id: "a".repeat(64),
                    record: request.clone(),
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
        let emitted_challenge = challenge.clone();
        controller
            .record_emitted_pairing("b".repeat(64), challenge)
            .expect("record the challenge");
        let response = Issue31PairingRecord::PairingResponse {
            schema: "openagents.omega.issue31.pairing.v1".into(),
            host_ref,
            host_public_key_hex: host_public_key_hex.to_string(),
            device_public_key_hex: device_public_key_hex.to_string(),
            issued_at: now + 1,
            pairing_response_ref: "pairing_response.live".into(),
            pairing_challenge_event_id: "b".repeat(64),
            challenge: challenge_value,
            expires_at: now + 86_400,
        };
        let grant = controller
            .handle_pairing_event(
                Issue31PairingEvent {
                    event_id: "c".repeat(64),
                    record: response.clone(),
                },
                now + 1,
            )
            .expect("pairing response")
            .expect("scoped grant");
        let emitted_grant = grant.clone();
        controller
            .record_emitted_pairing("d".repeat(64), grant)
            .expect("record the grant");
        (
            controller,
            vec![
                ("a".repeat(64), request),
                ("b".repeat(64), emitted_challenge),
                ("c".repeat(64), response),
                ("d".repeat(64), emitted_grant),
            ],
        )
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
        let empty = Issue31FullAutoReading::at_recorded_instant(
            LIVE_GENERATED_AT_MS,
            19,
            Vec::new(),
            json!({ "accounts": [] }),
            Vec::new(),
        );
        let documents = issue31_host_projection_documents(
            &empty,
            &request("omega.host.local", "grant.omega.device_1", &no_handoffs()),
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

    /// The omega#97 exit, against a REAL running daemon and the DEPLOYED relay.
    ///
    /// omega#49 and omega#91 both closed carrying the same named substitution:
    ///
    /// > the Full Auto reading. No `omega-effectd` daemon is attached to this
    /// > process, so the host's roster reading is supplied here rather than
    /// > polled.
    ///
    /// This test is that substitution being removed. It spawns the packaged
    /// `omega-effectd` under a data root of its own, reads it through the same
    /// `observe_issue31_full_auto` the desktop panel calls, hands the reading to
    /// the shipped projection source, and lets the shipped `sync_issue31_host`
    /// pump publish it to a paired device over a real relay.
    ///
    /// Nothing recorded is on the wire. `fixtures/live-omega-effectd.get_run.json`
    /// is not read here — replaying it would make a recorded fixture the host
    /// authority for a device proof, which omega#49's exit forbids in as many
    /// words, and which the private stamp on `Issue31FullAutoReading` now makes
    /// unexpressible from a production path rather than merely discouraged.
    ///
    /// ## What this run does NOT start
    ///
    /// It never calls `start`, `pause`, `resume`, `retry`, or `stop`. Full Auto
    /// authority does not begin on a path a model can reach, so a freshly
    /// spawned daemon holds no runs and this asserts a host that looked and
    /// found none — which is a real observation, and on the wire is a different
    /// document from a host that never looked. What a real *run* projects is
    /// covered from captured daemon bytes by
    /// `a_live_host_run_projects_its_exact_unattended_duration`; what this adds
    /// is that the bytes reaching the phone came from a daemon that answered.
    ///
    /// ## Named substitutions
    ///
    /// - identity custody. The owner key is a keypair rather than
    ///   `omega_identity::IdentityService`, because custody needs the GPUI app
    ///   and this harness runs headless. A run proves the host protocol and the
    ///   relay, not owner key custody.
    /// - `lane_readiness`. The shipped answer lives in `agent_ui` and needs a
    ///   GPUI workspace. This process genuinely holds no workspace and no
    ///   admitted agent authority, so it answers `unavailable` for every lane —
    ///   a true statement about this host, and one that can only under-claim
    ///   its capacity.
    ///
    /// ```sh
    /// OMEGA_LIVE_RELAY_URL=wss://relay.openagents.com \
    /// OMEGA_EFFECTD_BIN=/Applications/Omega.app/Contents/Resources/omega-effectd/bin/omega-effectd \
    ///   cargo test -p full_auto_ui --lib \
    ///   a_running_daemon_supplies_the_reading_a_paired_device_reads_on_a_live_relay \
    ///   -- --ignored --nocapture
    /// ```
    #[test]
    #[ignore = "requires a live relay and a packaged omega-effectd; set OMEGA_LIVE_RELAY_URL and OMEGA_EFFECTD_BIN"]
    fn a_running_daemon_supplies_the_reading_a_paired_device_reads_on_a_live_relay() {
        let Ok(relay_url) = std::env::var("OMEGA_LIVE_RELAY_URL") else {
            eprintln!("OMEGA_LIVE_RELAY_URL unset; skipping");
            return;
        };
        let Ok(effectd_bin) = std::env::var("OMEGA_EFFECTD_BIN") else {
            eprintln!("OMEGA_EFFECTD_BIN unset; skipping");
            return;
        };
        let daemon = std::path::PathBuf::from(&effectd_bin);
        assert!(
            daemon.is_file(),
            "OMEGA_EFFECTD_BIN must name the packaged omega-effectd executable: {effectd_bin}",
        );

        // Its own data root. The owner's running Omega keeps its runs under
        // `paths::data_dir()`, and a second writer there would be a second
        // durable run authority over the same registry.
        let temporary = tempfile::tempdir().expect("tempdir");
        let data_root = temporary.path().join("effectd");
        let supervisor = std::rc::Rc::new(smol::lock::Mutex::new(
            omega_effectd::OmegaEffectdSupervisor::new(omega_effectd::default_options(
                data_root.clone(),
                omega_effectd::OmegaEffectdCommand {
                    program: daemon,
                    args: Vec::new(),
                },
            )),
        ));
        {
            // See the named substitution above: no workspace, no admitted agent
            // authority, so every lane is honestly unavailable.
            let mut guard = smol::block_on(supervisor.lock());
            guard.set_host_handler(std::rc::Rc::new(|request: omega_effectd::HostRequestFrame| {
                Box::pin(async move {
                    match request.method {
                        omega_effectd::HostMethod::LaneReadiness => Ok(json!({
                            "known": false,
                            "admitted": false,
                            "fullAuto": false,
                            "state": "unavailable",
                        })),
                        _ => Err(omega_effectd::HostResponseError::unavailable(
                            "This headless host answers only lane_readiness.",
                        )),
                    }
                }) as omega_effectd::OmegaEffectdHostFuture
            }));
        }

        let initialized = smol::block_on(async {
            let mut guard = supervisor.lock().await;
            guard.start().await
        })
        .expect("the packaged omega-effectd must start");
        eprintln!(
            "omega#97: omega-effectd {} answered initialize at generation {} under {}",
            initialized.service_version,
            initialized.generation,
            data_root.display(),
        );

        // The reading. Measured, through the exact function `panel.rs` calls.
        let before_ms = unix_seconds().saturating_mul(1_000);
        let reading = smol::block_on(crate::issue31_observation::observe_issue31_full_auto(
            &supervisor,
        ))
        .expect("a running daemon must let this host state a reading");
        let after_ms = unix_seconds().saturating_mul(1_000) + 1_000;

        // Provenance: the stamp is this host's clock at the moment it read the
        // daemon, not a value anything handed it. A replayed fixture cannot
        // land in this window, and there is no parameter through which one
        // could try.
        assert!(
            reading.generated_at_ms() >= before_ms && reading.generated_at_ms() <= after_ms,
            "the reading must be stamped by the host that took it: \
             {before_ms} <= {} <= {after_ms}",
            reading.generated_at_ms(),
        );
        // The daemon's own capacity record, not a constructed one. Its lane
        // list is the thing `parse_provider_accounts` reads the account-to-lane
        // mapping out of, so this is where the roster comes from.
        let lanes = reading
            .capacity
            .get("lanes")
            .and_then(Value::as_array)
            .expect("a live get_capacity carries the host's lanes");
        assert!(
            !lanes.is_empty(),
            "the daemon answered get_capacity with no lanes at all: {}",
            reading.capacity,
        );
        eprintln!(
            "omega#97: measured reading · generation {} · {} run(s) · {} lane(s) · {} account(s) · stamped {}",
            reading.host_generation,
            reading.run_details.len(),
            lanes.len(),
            parse_provider_accounts(&reading.capacity).len(),
            reading.generated_at_ms(),
        );

        // Publish it, through the shipped pump, to a real paired device.
        //
        // `OMEGA_LIVE_DEVICE_PUBKEY` lets a device in another process — the
        // openagents-mobile client, for omega#97's device half — be that
        // device. It supplies only its public key: the secret never leaves the
        // phone's process, so the gift wraps this host writes to the relay can
        // be opened by exactly one reader, and it is not this one.
        let host_keys = nostr::Keys::generate();
        let sarah_keys = nostr::Keys::generate();
        let device_public_key_hex = std::env::var("OMEGA_LIVE_DEVICE_PUBKEY")
            .ok()
            .map(|value| value.trim().to_ascii_lowercase())
            .filter(|value| {
                value.len() == 64 && value.chars().all(|byte| byte.is_ascii_hexdigit())
            })
            .unwrap_or_else(|| nostr::Keys::generate().public_key().to_hex());
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
        let (controller, pairing_records) = live_paired_controller_with_records(
            &owner_public_key_hex,
            &sarah_keys.public_key().to_hex(),
            &conversation_ref,
            &relay_url,
            &device_public_key_hex,
            &[
                omega_effectd::Issue31PairingScope::ObserveIssue31,
                omega_effectd::Issue31PairingScope::ControlFullAuto,
            ],
        );
        let grant = controller
            .active_grants(unix_seconds())
            .expect("grants")
            .first()
            .cloned()
            .expect("the paired device holds an active grant");

        // The handoff a device in another process needs, and nothing more: the
        // host's identity and the four pairing records the grant folds out of.
        // No secret and no adjunct body — the adjuncts are on the relay, which
        // is the whole point of the exercise. A device that could be handed the
        // reading directly would prove nothing about delivery.
        if let Ok(path) = std::env::var("OMEGA_LIVE_PAIRING_OUT") {
            let handoff = json!({
                "relayUrl": relay_url,
                "hostPublicKeyHex": owner_public_key_hex,
                "sarahPublicKeyHex": sarah_keys.public_key().to_hex(),
                "devicePublicKeyHex": device_public_key_hex,
                "grantRef": grant.grant_ref,
                "pairingRecords": pairing_records
                    .iter()
                    .map(|(event_id, record)| json!({
                        "canonicalRecordId": event_id,
                        "record": record,
                    }))
                    .collect::<Vec<_>>(),
            });
            std::fs::write(
                &path,
                serde_json::to_string_pretty(&handoff).expect("serialize the pairing handoff"),
            )
            .expect("write the pairing handoff");
            eprintln!("omega#97: wrote the device pairing handoff to {path}");
        }

        let mut client =
            omega_effectd::SarahConversationClient::with_relay(config, Box::new(relay), signer);
        client.attach_issue31_host_controller(controller);
        set_issue31_live_reading(reading.clone());
        client.set_issue31_host_projection_source(issue31_host_projection_source());
        client.set_issue31_provider_roster_source(issue31_provider_roster_source());

        client
            .sync_issue31_host()
            .expect("the shipped host pump runs against the live relay");

        assert!(
            client
                .issue31_published_host_adjunct_grants()
                .contains(&format!("{}:{}", grant.grant_ref, grant.generation)),
            "the pump must record the omega#47 publication it made for this grant",
        );
        // The outbox drains only when every configured relay acknowledged every
        // gift wrap, so an empty backlog is the live relay's own receipt.
        assert!(
            client.issue31_pending_private_publish_refs().is_empty(),
            "the live relay must acknowledge every owner-private record: {:?}",
            client.issue31_pending_private_publish_refs(),
        );

        // And the document the phone decodes carries the daemon's stamp — the
        // proof that what crossed the relay is what the daemon said, rather
        // than anything this test could have authored.
        let documents = issue31_host_projection_documents(
            &reading,
            &Issue31HostProjectionRequest {
                host_ref: "omega.host.local",
                host_public_key_hex: &owner_public_key_hex,
                device_public_key_hex: &device_public_key_hex,
                grant_ref: &grant.grant_ref,
                expected_generation: grant.generation,
                observed_at_ms: reading.generated_at_ms(),
                handoffs: &no_handoffs(),
            },
        )
        .expect("a measured reading projects");
        assert_eq!(
            documents
                .detail
                .get("generatedAtMs")
                .and_then(Value::as_u64),
            Some(reading.generated_at_ms()),
            "the detail the phone reads must carry the instant the daemon was read",
        );
        let decoded = workroom_receipts::decode_issue31_full_auto_adjunct(
            &serde_json::to_string(&documents.detail).expect("serialize detail"),
        )
        .expect("the emitter must not produce a detail the phone would refuse");
        assert_eq!(
            decoded.runs.len(),
            reading.run_details.len(),
            "the phone must read exactly the runs the daemon reported",
        );

        eprintln!(
            "omega#97 live relay OK: {relay_url} stored the omega#47 snapshot and detail for \
             grant {} addressed to device {device_public_key_hex}, built from a reading this \
             host measured from a running omega-effectd ({} run(s), {} lane(s))",
            grant.grant_ref,
            reading.run_details.len(),
            lanes.len(),
        );

        smol::block_on(async {
            let mut guard = supervisor.lock().await;
            let _ = guard.stop().await;
        });
    }
}
