use std::path::PathBuf;
use std::time::{Duration, Instant};

use nautilus_sidecar::{
    LifecycleEvent, NautilusConfig, NautilusSupervisor, Network, PrivateKey, StreamEvent,
};

#[test]
#[ignore = "requires HYPERLIQUID_TESTNET_PRIVATE_KEY and public testnet access"]
fn hyperliquid_testnet_reaches_health_and_stops_cleanly() {
    let private_key = std::env::var("HYPERLIQUID_TESTNET_PRIVATE_KEY")
        .expect("HYPERLIQUID_TESTNET_PRIVATE_KEY must be configured");
    let repository_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(|path| path.parent())
        .expect("repository root")
        .to_path_buf();
    let config = NautilusConfig {
        network: Network::Testnet,
        python: repository_root.join("sidecar/nautilus/.venv/bin/python"),
        engine: repository_root.join("sidecar/nautilus/engine.py"),
        reconciliation_lookback_minutes: 60,
        health_timeout: Duration::from_secs(40),
    };
    let mut supervisor = NautilusSupervisor::new(
        config,
        PrivateKey::new(private_key.into_bytes()).expect("valid testnet key"),
    )
    .expect("testnet supervisor");

    let health = supervisor.start().expect("testnet health");
    assert!(matches!(
        health,
        LifecycleEvent::Healthy {
            network: Network::Testnet,
            reconciliation_lookback_minutes: 60,
            ..
        }
    ));
    assert_eq!(supervisor.generation(), 1);
    let deadline = Instant::now() + Duration::from_secs(30);
    let mut quotes = 0;
    let mut trades = 0;
    let mut books = 0;
    let mut accounts = 0;
    let mut order_states = 0;
    while Instant::now() < deadline {
        let frame = supervisor.take_stream_frame().expect("stream frame");
        quotes += usize::from(frame.quote.is_some());
        books += usize::from(frame.book.is_some());
        trades += frame.trades.len();
        for event in frame.state {
            match event {
                StreamEvent::Account { .. } => accounts += 1,
                StreamEvent::Order { .. } | StreamEvent::OrderState { .. } => order_states += 1,
                _ => {}
            }
        }
        if quotes > 0 && trades > 0 && books > 0 && accounts > 0 && order_states > 0 {
            break;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    assert!(quotes > 0, "no BTC testnet quotes reached Omega");
    assert!(trades > 0, "no BTC testnet trades reached Omega");
    assert!(books > 0, "no BTC testnet book updates reached Omega");
    assert!(accounts > 0, "no testnet account state reached Omega");
    assert!(order_states > 0, "no testnet order state reached Omega");
    eprintln!(
        "testnet stream evidence: quotes={quotes} trades={trades} books={books} accounts={accounts} order_states={order_states}"
    );
    supervisor.stop().expect("clean testnet shutdown");
}
