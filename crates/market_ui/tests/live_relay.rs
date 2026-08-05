//! Live discovery check against a local Immortal dev relay (omega#244).
//!
//! Runs only when `OMEGA_MARKET_RELAY_URL` is set, mirroring the
//! `OMEGA_LIVE_RELAY_URL` pattern in `omega_effectd`. Start the relay with
//! `scripts/dev-relay.sh` and `scripts/dev-market-seed.sh` in the Immortal
//! repository, then:
//!
//! ```sh
//! OMEGA_MARKET_RELAY_URL=ws://127.0.0.1:18080 cargo test -p market_ui --test live_relay -- --nocapture
//! ```

use std::io::{Read as _, Write as _};
use std::net::TcpStream;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use async_tungstenite::async_std::connect_async;
use async_tungstenite::tungstenite::Message;
use futures::StreamExt as _;
use market_ui::{
    ConnectionState, IngestOutcome, MarketDiscovery, MarketDiscoveryConfig,
    validate_market_relay_information,
};

fn fetch_relay_information_plaintext(config: &MarketDiscoveryConfig) -> String {
    let information_url = config
        .relay_information_url()
        .expect("relay URL derives an information URL");
    let authority = information_url
        .strip_prefix("http://")
        .expect("the live test supports plaintext ws relays only");
    let mut stream = TcpStream::connect(authority).expect("relay information endpoint connects");
    let request = format!(
        "GET / HTTP/1.1\r\nHost: {authority}\r\nAccept: application/nostr+json\r\nConnection: close\r\n\r\n"
    );
    stream
        .write_all(request.as_bytes())
        .expect("relay information request sends");
    let mut response = Vec::new();
    stream
        .read_to_end(&mut response)
        .expect("relay information response reads");
    let response = String::from_utf8(response).expect("relay information response is UTF-8");
    let (headers, body) = response
        .split_once("\r\n\r\n")
        .expect("relay information response has headers and a body");
    assert!(
        headers.starts_with("HTTP/1.1 200"),
        "relay information endpoint must return 200, got: {headers}"
    );
    body.to_owned()
}

#[test]
fn live_relay_discovery_snapshot() {
    let Ok(relay_url) = std::env::var(market_ui::RELAY_URL_ENVIRONMENT_VARIABLE) else {
        return;
    };
    let config = MarketDiscoveryConfig {
        relay_websocket_url: relay_url,
    };
    config.validate().expect("relay URL is valid");

    let information = fetch_relay_information_plaintext(&config);
    let gate = validate_market_relay_information(&information)
        .expect("the dev relay advertises the nip-mkt extension");
    assert!(
        gate.advertises_mkt_swp,
        "the dev relay advertises mkt-swp:1"
    );

    let mut discovery = MarketDiscovery::new();
    discovery.begin_connect(&gate);
    let request = discovery.subscription_request();
    let deadline = Instant::now() + Duration::from_secs(15);

    smol::block_on(async move {
        let (mut stream, _response) = connect_async(config.relay_websocket_url.as_str())
            .await
            .expect("relay WebSocket connects");
        discovery.opened();
        stream
            .send(Message::Text(request.into()))
            .await
            .expect("subscription request sends");
        while Instant::now() < deadline {
            let Some(message) = stream.next().await else {
                break;
            };
            let message = message.expect("relay frame reads");
            let Message::Text(text) = message else {
                continue;
            };
            let now = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("current time is after the epoch")
                .as_secs();
            let outcome = discovery
                .ingest_text(&text, now)
                .expect("relay frame ingests");
            if outcome == IngestOutcome::SnapshotComplete {
                break;
            }
        }
        assert_eq!(discovery.connection(), &ConnectionState::Live);
        let providers = discovery.providers();
        let offerings = discovery.offerings();
        println!("providers: {providers:?}");
        println!("offerings: {offerings:?}");
        assert!(
            !providers.is_empty(),
            "the seeded dev relay lists at least one provider"
        );
        assert!(
            !offerings.is_empty(),
            "the seeded dev relay lists at least one offering"
        );
    });
}
