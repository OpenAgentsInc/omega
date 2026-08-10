#!/usr/bin/env python3
from __future__ import annotations

import argparse
import json
import os
import sys
import threading


EVENT_SCHEMA = "omega.nautilus.lifecycle.v1"
HYPERLIQUID = "HYPERLIQUID"
event_file_descriptor = sys.stdout.fileno()


def emit(event_type: str, generation: int, **fields: object) -> None:
    event = {
        "type": event_type,
        "schema": EVENT_SCHEMA,
        "generation": generation,
        "network": "testnet",
        **fields,
    }
    payload = f"OMEGA_NAUTILUS_EVENT {json.dumps(event, separators=(',', ':'))}\n"
    os.write(event_file_descriptor, payload.encode())


def capture_engine_logs(args: argparse.Namespace) -> tuple[int, threading.Thread]:
    global event_file_descriptor
    event_file_descriptor = os.dup(sys.stdout.fileno())
    read_file_descriptor, write_file_descriptor = os.pipe()
    os.dup2(write_file_descriptor, sys.stdout.fileno())
    os.close(write_file_descriptor)

    def monitor() -> None:
        connected = False
        account_reconciled = False
        startup_reconciled = False
        healthy_emitted = False
        with os.fdopen(read_file_descriptor, encoding="utf-8", errors="replace") as logs:
            for line in logs:
                sys.stderr.write(line)
                sys.stderr.flush()
                connected = connected or "Connected: client_id=HYPERLIQUID" in line
                account_reconciled = account_reconciled or "AccountState(" in line
                startup_reconciled = (
                    startup_reconciled or "Startup reconciliation completed" in line
                )
                if (
                    connected
                    and account_reconciled
                    and startup_reconciled
                    and not healthy_emitted
                ):
                    emit(
                        "healthy",
                        args.generation,
                        venue="hyperliquid",
                        reconciliation_lookback_minutes=args.reconciliation_lookback_minutes,
                    )
                    healthy_emitted = True

    monitor_thread = threading.Thread(target=monitor, name="nautilus-log-monitor", daemon=True)
    monitor_thread.start()
    return event_file_descriptor, monitor_thread


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser()
    parser.add_argument("--network", required=True)
    parser.add_argument("--generation", required=True, type=int)
    parser.add_argument("--reconciliation-lookback-minutes", required=True, type=int)
    args = parser.parse_args()
    if args.network != "testnet":
        parser.error("mainnet is disabled; only testnet is permitted")
    if not 1 <= args.reconciliation_lookback_minutes <= 1_440:
        parser.error("reconciliation lookback must be between 1 and 1440 minutes")
    return args


def main() -> None:
    args = parse_args()
    private_key = os.environ.get("HYPERLIQUID_TESTNET_PK", "")
    if not private_key.startswith("0x") or len(private_key) != 66:
        raise RuntimeError("HYPERLIQUID_TESTNET_PK is missing or malformed")

    from nautilus_trader.adapters.hyperliquid import HyperliquidDataClientConfig
    from nautilus_trader.adapters.hyperliquid import HyperliquidDataClientFactory
    from nautilus_trader.adapters.hyperliquid import HyperliquidEnvironment
    from nautilus_trader.adapters.hyperliquid import HyperliquidExecClientConfig
    from nautilus_trader.adapters.hyperliquid import HyperliquidExecFactoryConfig
    from nautilus_trader.adapters.hyperliquid import HyperliquidExecutionClientFactory
    from nautilus_trader.common import Environment
    from nautilus_trader.live import LiveNode
    from nautilus_trader.model import AccountId
    from nautilus_trader.model import TraderId
    from stream_strategy import OmegaStreamStrategy
    from stream_strategy import StreamPublisher

    trader_id = TraderId.from_str("OMEGA-001")
    account_id = AccountId.from_str("HYPERLIQUID-001")
    builder = (
        LiveNode.builder("OMEGA-HYPERLIQUID-TESTNET", trader_id, Environment.LIVE)
        .with_reconciliation(True)
        .with_reconciliation_lookback_mins(args.reconciliation_lookback_minutes)
        .with_timeout_disconnection_secs(5)
        .with_delay_post_stop_secs(1)
        .with_delay_shutdown_secs(1)
        .add_data_client(
            None,
            HyperliquidDataClientFactory(),
            HyperliquidDataClientConfig(environment=HyperliquidEnvironment.TESTNET),
        )
        .add_exec_client(
            None,
            HyperliquidExecutionClientFactory(),
            HyperliquidExecFactoryConfig(
                trader_id,
                account_id,
                HyperliquidExecClientConfig(environment=HyperliquidEnvironment.TESTNET),
            ),
        )
    )
    event_output, monitor_thread = capture_engine_logs(args)
    node = builder.build()
    stream_publisher = StreamPublisher(event_output, args.generation)
    node.add_strategy(OmegaStreamStrategy(stream_publisher))
    emit("starting", args.generation)
    try:
        node.run()
    finally:
        node.stop()
        node.dispose()
        stream_publisher.close()
        os.dup2(event_output, sys.stdout.fileno())
        monitor_thread.join(timeout=5)
        emit("stopped", args.generation)


if __name__ == "__main__":
    try:
        main()
    except Exception as error:
        print(f"nautilus sidecar failed: {error}", file=sys.stderr, flush=True)
        raise
