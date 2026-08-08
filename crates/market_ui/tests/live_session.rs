//! Full negotiated NIP-MKT session against a local Immortal dev relay and
//! its no-spend provider (omega#244).
//!
//! Runs only when `OMEGA_MARKET_RELAY_URL` is set, mirroring `live_relay.rs`.
//! In the Immortal repository start the relay and the seeded no-spend
//! provider, then run the test:
//!
//! ```sh
//! ./scripts/dev-relay.sh
//! IMMORTAL_PROVIDER_IDENTITY_SECRET="$(printf '02%.0s' $(seq 32))" \
//!   ./scripts/dev-market-provider.sh
//! OMEGA_MARKET_RELAY_URL=ws://127.0.0.1:18080 \
//!   cargo test -p market_ui --test live_session -- --nocapture
//! ```
//!
//! The session drives RFQ → firm/soft Quote → Order → bilateral Swap
//! Contracts → per-signer Status → Cancel (request/accepted/effective) →
//! zero-spend Close on throwaway keys, entirely through the crate's public
//! session flow and transport.

use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use async_tungstenite::async_std::connect_async;
use async_tungstenite::tungstenite::Message;
use futures::StreamExt as _;
use immortal_client::domain::{Event, MKT_CLOSE_KIND};
use immortal_client::mkt_swp_client::{ParticipantRole, RequesterTerminalState};
use market_ui::{
    IngestOutcome, MarketDiscovery, MarketRelayGate, MarketSession, OfferingListing, SessionPhase,
    SessionSocketEvent, StatusSlot, load_stored_records, run_session_socket,
    throwaway_session_signer, wrap_for_transport,
};
use serde_json::Value;

const SESSION_DEADLINE: Duration = Duration::from_secs(120);

fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or(0)
}

async fn discover_offering(relay_url: &str) -> OfferingListing {
    let mut discovery = MarketDiscovery::new();
    discovery.begin_connect(&MarketRelayGate {
        relay_name: "live".to_owned(),
        advertises_mkt_swp: true,
        max_limit: 256,
    });
    let request = discovery.subscription_request();
    let (mut stream, _response) = connect_async(relay_url)
        .await
        .expect("discovery WebSocket connects");
    discovery.opened();
    stream
        .send(Message::Text(request.into()))
        .await
        .expect("discovery subscription sends");
    let deadline = Instant::now() + Duration::from_secs(15);
    while Instant::now() < deadline {
        let Some(message) = stream.next().await else {
            break;
        };
        let Message::Text(text) = message.expect("discovery frame reads") else {
            continue;
        };
        let outcome = discovery
            .ingest_text(&text, unix_now())
            .expect("discovery frame ingests");
        if outcome == IngestOutcome::SnapshotComplete {
            break;
        }
    }
    discovery
        .offerings()
        .into_iter()
        .find(|offering| offering.status == "active" && offering.profile.starts_with("mkt-swp:"))
        .expect(
            "the dev relay lists an active mkt-swp offering; start \
             scripts/dev-market-provider.sh in the Immortal repository",
        )
}

async fn publish_records(
    session: &MarketSession,
    records: &[Event],
    outgoing: &async_channel::Sender<Event>,
) {
    let now = unix_now();
    for record in records {
        let wraps = wrap_for_transport(record, session.signer(), session.provider_pubkey(), now)
            .expect("records wrap for transport");
        for wrap in wraps {
            outgoing
                .send(wrap.event)
                .await
                .expect("session socket accepts wraps");
        }
    }
}

async fn next_event(
    events: &async_channel::Receiver<SessionSocketEvent>,
    deadline: Instant,
) -> SessionSocketEvent {
    // A wall-clock deadline is intentional: this is a live-relay test, not a
    // deterministic GPUI test.
    let received = smol::future::or(async { events.recv().await.ok() }, async {
        smol::Timer::at(deadline).await;
        None
    })
    .await;
    received.expect("a session event arrives before the deadline")
}

fn provider_status_seen(session: &MarketSession) -> bool {
    session.status_lanes().iter().any(|lane| {
        lane.role == ParticipantRole::Provider
            && lane
                .slots
                .iter()
                .any(|slot| matches!(slot, StatusSlot::Filled { .. }))
    })
}

#[test]
fn live_relay_negotiated_session() {
    let Ok(relay_url) = std::env::var(market_ui::RELAY_URL_ENVIRONMENT_VARIABLE) else {
        return;
    };

    smol::block_on(async move {
        let offering = discover_offering(&relay_url).await;
        println!("offering: {offering:?}");

        let signer = throwaway_session_signer().expect("session key generates");
        let mut session =
            MarketSession::begin(signer.clone(), &offering, unix_now()).expect("session begins");
        let (outgoing_sender, outgoing_receiver) = async_channel::bounded(256);
        let (event_sender, event_receiver) = async_channel::bounded(256);
        let socket = smol::spawn(run_session_socket(
            relay_url.clone(),
            signer,
            outgoing_receiver,
            event_sender,
            unix_now,
        ));

        let deadline = Instant::now() + SESSION_DEADLINE;
        let mut ordered = false;
        let mut cancelled = false;
        let mut requester_closed = false;
        loop {
            let closes = session.closes();
            if requester_closed
                && closes
                    .iter()
                    .any(|close| close.author == ParticipantRole::Provider)
                && closes
                    .iter()
                    .any(|close| close.author == ParticipantRole::Requester)
            {
                break;
            }
            match next_event(&event_receiver, deadline).await {
                SessionSocketEvent::Authenticated => {}
                SessionSocketEvent::SubscriptionLive => {
                    let wraps = session.replay_wraps(unix_now()).expect("records re-wrap");
                    for wrap in wraps {
                        outgoing_sender
                            .send(wrap)
                            .await
                            .expect("session socket accepts replay wraps");
                    }
                }
                SessionSocketEvent::Delivered {
                    delivered,
                    observed_at,
                } => {
                    session
                        .admit_delivery(&delivered, observed_at)
                        .expect("deliveries admit");
                    let now = unix_now();
                    if !ordered && session.can_order(now) {
                        let records = session.order_selected_quote(now).expect("order constructs");
                        println!(
                            "ordered quote {:?}",
                            session.accepted_quote().map(|quote| quote.event.id.clone())
                        );
                        publish_records(&session, &records, &outgoing_sender).await;
                        ordered = true;
                    }
                    if ordered
                        && !cancelled
                        && session.phase() == SessionPhase::Active
                        && provider_status_seen(&session)
                    {
                        let cancel = session.request_cancel(now).expect("cancel constructs");
                        publish_records(&session, &[cancel], &outgoing_sender).await;
                        cancelled = true;
                    }
                    if !requester_closed && session.can_close() {
                        let close = session.close_after_cancel(now).expect("close constructs");
                        publish_records(&session, &[close], &outgoing_sender).await;
                        requester_closed = true;
                    }
                }
                SessionSocketEvent::PublishResult {
                    event_id,
                    accepted,
                    message,
                } => {
                    assert!(accepted, "relay refused {event_id}: {message}");
                }
                SessionSocketEvent::Diagnostic(diagnostic) => {
                    println!("diagnostic: {diagnostic}");
                }
            }
        }
        drop(outgoing_sender);
        socket.cancel().await;

        assert_eq!(session.phase(), SessionPhase::Closed);
        let quotes = session.quotes();
        assert!(!quotes.is_empty(), "the provider quoted the RFQ");
        let accepted = session.accepted_quote().expect("a quote was accepted");
        assert_eq!(accepted.quote_class, "firm");
        assert_eq!(accepted.reservation, "soft");

        // Both signers report a contiguous Status lane; gaps and forks would
        // render, but a clean session must not produce them.
        let lanes = session.status_lanes();
        assert_eq!(lanes.len(), 2, "both signers reported Status");
        for lane in &lanes {
            assert!(!lane.slots.is_empty());
            assert!(
                lane.slots
                    .iter()
                    .all(|slot| matches!(slot, StatusSlot::Filled { .. })),
                "the clean session has no sequence gaps"
            );
            assert!(lane.malformed.is_empty());
        }

        let cancels = session.cancels();
        for action in ["request", "accepted", "effective"] {
            assert!(
                cancels.iter().any(|cancel| cancel.action == action),
                "cancellation reached {action}"
            );
        }

        // The provider's terminal Close carries exact zero-spend accounting.
        let provider_close = session
            .records()
            .iter()
            .find(|event| {
                event.kind == MKT_CLOSE_KIND && event.pubkey == *session.provider_pubkey()
            })
            .expect("the provider closed the session");
        let close_profile: Value =
            serde_json::from_str(&provider_close.content).expect("close content is JSON");
        assert_eq!(
            close_profile.pointer("/mkt_swp/external_spend_effects"),
            Some(&Value::from(0)),
        );
        assert_eq!(
            close_profile
                .pointer("/mkt_swp/loss_accounting/input_committed")
                .and_then(Value::as_str),
            Some("0"),
        );

        // The Immortal requester projection agrees on the terminal state.
        let view = session
            .requester_session_view()
            .expect("the bound session projects");
        assert_eq!(
            view.terminal.claimed_state,
            RequesterTerminalState::Cancelled
        );
        assert!(view.verification.status_gaps.is_empty());
        assert!(view.verification.status_forks.is_empty());

        // The durable store retains every signed record and revalidates on
        // load.
        let store_directory = std::env::temp_dir().join(format!(
            "omega-market-live-session-{}",
            session.session_id()
        ));
        let path = session.persist(&store_directory).expect("session persists");
        let stored = load_stored_records(&path).expect("stored records reload");
        assert_eq!(stored.len(), session.records().len());
        std::fs::remove_dir_all(&store_directory).expect("test store directory is removable");

        println!(
            "session {} closed: {} records, quotes {}, provider close {}",
            session.session_id(),
            session.records().len(),
            quotes.len(),
            provider_close.id,
        );
    });
}
