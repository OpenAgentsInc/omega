use std::path::PathBuf;
use std::time::Duration;

use nautilus_sidecar::{LifecycleEvent, NautilusConfig, NautilusSupervisor, Network, PrivateKey};

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
    supervisor.stop().expect("clean testnet shutdown");
}
