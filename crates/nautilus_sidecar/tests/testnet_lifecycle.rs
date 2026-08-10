use std::collections::BTreeSet;
use std::path::PathBuf;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use nautilus_sidecar::{
    CommandOutcome, CommandRequest, LifecycleEvent, NautilusCommand, NautilusConfig,
    NautilusSupervisor, Network, OrderSide, PrivateKey, StrategyParameters, StreamEvent,
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
    let mut account_fields = BTreeSet::new();
    let mut balance_fields = BTreeSet::new();
    let mut order_states = 0;
    while Instant::now() < deadline {
        let frame = supervisor.take_stream_frame().expect("stream frame");
        quotes += usize::from(frame.quote.is_some());
        books += usize::from(frame.book.is_some());
        trades += frame.trades.len();
        for event in frame.state {
            match event {
                StreamEvent::Account { state, .. } => {
                    accounts += 1;
                    account_fields.extend(state.keys().cloned());
                    if let Some(balances) =
                        state.get("balances").and_then(serde_json::Value::as_array)
                        && let Some(balance) =
                            balances.first().and_then(serde_json::Value::as_object)
                    {
                        balance_fields.extend(balance.keys().cloned());
                    }
                }
                StreamEvent::Order { .. } | StreamEvent::OrderState { .. } => order_states += 1,
                _ => {}
            }
        }
        if quotes > 0 && trades > 0 && books > 0 && accounts >= 3 && order_states > 0 {
            break;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    assert!(quotes > 0, "no BTC testnet quotes reached Omega");
    assert!(trades > 0, "no BTC testnet trades reached Omega");
    assert!(books > 0, "no BTC testnet book updates reached Omega");
    assert!(
        accounts >= 3,
        "testnet account state did not refresh after startup"
    );
    assert!(order_states > 0, "no testnet order state reached Omega");
    eprintln!(
        "testnet stream evidence: quotes={quotes} trades={trades} books={books} accounts={accounts} order_states={order_states} account_fields={account_fields:?} balance_fields={balance_fields:?}"
    );
    supervisor.stop().expect("clean testnet shutdown");
}

#[test]
#[ignore = "requires the testnet key, OMEGA_NAUTILUS_TEST_PRICE, and public testnet access"]
fn command_channel_places_cancels_and_controls_an_engine_strategy() {
    let private_key = std::env::var("HYPERLIQUID_TESTNET_PRIVATE_KEY")
        .expect("HYPERLIQUID_TESTNET_PRIVATE_KEY must be configured");
    let price = std::env::var("OMEGA_NAUTILUS_TEST_PRICE")
        .expect("OMEGA_NAUTILUS_TEST_PRICE must be configured");
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
    supervisor.start().expect("testnet health");

    let parameters = supervisor
        .send_command(CommandRequest {
            command_id: "testnet-params-287".into(),
            command: NautilusCommand::SetStrategyParameters {
                strategy_id: "OMEGA-BOUNDED-QUOTE-001".into(),
                parameters: StrategyParameters {
                    min_reprice_interval_ms: 5_000,
                    quote_offset_bps: 200,
                    order_quantity: "0.001".into(),
                    position_headroom_usd: 100,
                    order_budget: 2,
                    mandate_revision: 1,
                },
            },
        })
        .expect("set strategy parameters");
    assert!(
        matches!(
            parameters.outcome,
            CommandOutcome::StrategyParametersApplied { .. }
        ),
        "unexpected parameter outcome: {parameters:?}"
    );
    let start = supervisor
        .send_command(CommandRequest {
            command_id: "testnet-start-287".into(),
            command: NautilusCommand::StartStrategy {
                strategy_id: "OMEGA-BOUNDED-QUOTE-001".into(),
            },
        })
        .expect("start strategy");
    assert!(
        matches!(
            start.outcome,
            CommandOutcome::StrategyStarted { running: true }
        ),
        "unexpected start outcome: {start:?}"
    );
    let strategy_deadline = Instant::now() + Duration::from_secs(30);
    let mut strategy_evidence = None;
    while Instant::now() < strategy_deadline {
        let frame = supervisor
            .take_stream_frame()
            .expect("strategy stream frame");
        for event in frame.state {
            if let StreamEvent::StrategyState {
                phase,
                quote_ticks,
                trade_ticks,
                book_ticks,
                action_count,
                halted_reason,
                ..
            } = event
                && phase == "order_resting"
                && quote_ticks > 0
                && book_ticks > 0
                && action_count > 0
            {
                strategy_evidence = Some((
                    quote_ticks,
                    trade_ticks,
                    book_ticks,
                    action_count,
                    halted_reason,
                ));
                break;
            }
        }
        if strategy_evidence.is_some() {
            break;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    let (quote_ticks, trade_ticks, book_ticks, action_count, halted_reason) =
        strategy_evidence.expect("no in-engine strategy tick/action evidence reached Omega");
    eprintln!(
        "testnet tick strategy evidence: quote_ticks={quote_ticks} trade_ticks={trade_ticks} book_ticks={book_ticks} action_count={action_count} halted_reason={halted_reason:?}"
    );
    let stop = supervisor
        .send_command(CommandRequest {
            command_id: "testnet-stop-287".into(),
            command: NautilusCommand::StopStrategy {
                strategy_id: "OMEGA-BOUNDED-QUOTE-001".into(),
            },
        })
        .expect("stop strategy");
    assert!(
        matches!(
            stop.outcome,
            CommandOutcome::StrategyStopped { running: false }
        ),
        "unexpected stop outcome: {stop:?}"
    );

    let run_nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock after Unix epoch")
        .as_millis();
    let client_order_id = format!("O-287-{run_nonce}");
    let place = supervisor
        .send_command(CommandRequest {
            command_id: "testnet-place-287".into(),
            command: NautilusCommand::PlaceOrder {
                client_order_id: client_order_id.clone(),
                instrument_id: "BTC-USD-PERP.HYPERLIQUID".into(),
                side: OrderSide::Buy,
                quantity: "0.001".into(),
                price,
                post_only: true,
                reduce_only: false,
            },
        })
        .expect("place testnet order");
    assert!(place.acknowledged);
    assert!(place.sent);
    let venue_order_id = match place.outcome {
        CommandOutcome::OrderAccepted {
            client_order_id: accepted_client_order_id,
            venue_order_id,
        } => {
            assert_eq!(accepted_client_order_id, client_order_id);
            venue_order_id
        }
        outcome => panic!("expected accepted order, got {outcome:?}"),
    };

    let cancel = supervisor
        .send_command(CommandRequest {
            command_id: "testnet-cancel-287".into(),
            command: NautilusCommand::CancelOrder {
                client_order_id: client_order_id.clone(),
            },
        })
        .expect("cancel testnet order");
    assert!(cancel.acknowledged);
    assert!(cancel.sent);
    assert!(matches!(
        cancel.outcome,
        CommandOutcome::OrderCanceled {
            client_order_id: canceled_client_order_id,
            venue_order_id: canceled_venue_order_id,
        } if canceled_client_order_id == client_order_id && canceled_venue_order_id == venue_order_id
    ));
    supervisor.stop().expect("clean testnet shutdown");
}
