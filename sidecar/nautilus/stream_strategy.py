from __future__ import annotations

from collections import deque
from datetime import timedelta
import json
import os
import threading
from typing import Any

from nautilus_trader.model import BookType
from nautilus_trader.model import ClientId
from nautilus_trader.model import InstrumentId
from nautilus_trader.model import StrategyId
from nautilus_trader.model import Venue
from nautilus_trader.trading import Strategy
from nautilus_trader.trading import StrategyConfig


STREAM_PREFIX = "OMEGA_NAUTILUS_STREAM "
STREAM_SCHEMA = "omega.nautilus.stream.v1"
INSTRUMENT = InstrumentId.from_str("BTC-USD-PERP.HYPERLIQUID")
CLIENT = ClientId.from_str("HYPERLIQUID")
VENUE = Venue.from_str("HYPERLIQUID")
ACCOUNT_SNAPSHOT_INTERVAL_NS = 10_000_000_000


class StreamPublisher:
    def __init__(self, file_descriptor: int, generation: int) -> None:
        self._file_descriptor = file_descriptor
        self._generation = generation
        self._sequence = 0
        self._lossless: deque[dict[str, Any]] = deque()
        self._trades: deque[dict[str, Any]] = deque(maxlen=2_048)
        self._latest: dict[str, dict[str, Any]] = {}
        self._condition = threading.Condition()
        self._closed = False
        self._thread = threading.Thread(
            target=self._run,
            name="omega-nautilus-stream",
            daemon=True,
        )
        self._thread.start()

    def publish(self, event_type: str, payload: dict[str, Any], *, lossless: bool) -> None:
        with self._condition:
            self._sequence += 1
            payload = {key: value for key, value in payload.items() if key != "type"}
            event = {
                "type": event_type,
                "schema": STREAM_SCHEMA,
                "generation": self._generation,
                "sequence": self._sequence,
                "network": "testnet",
                **payload,
            }
            if lossless:
                self._lossless.append(event)
            elif event_type == "trade":
                self._trades.append(event)
            else:
                self._latest[event_type] = event
            self._condition.notify()

    def close(self) -> None:
        with self._condition:
            self._closed = True
            self._condition.notify()
        self._thread.join(timeout=5)

    def _run(self) -> None:
        while True:
            with self._condition:
                self._condition.wait_for(
                    lambda: self._closed
                    or bool(self._lossless)
                    or bool(self._trades)
                    or bool(self._latest)
                )
                if self._lossless:
                    event = self._lossless.popleft()
                elif self._trades:
                    event = self._trades.popleft()
                elif self._latest:
                    _, event = self._latest.popitem()
                elif self._closed:
                    return
                else:
                    continue
            payload = f"{STREAM_PREFIX}{json.dumps(event, separators=(',', ':'))}\n"
            os.write(self._file_descriptor, payload.encode())


class OmegaStreamStrategy(Strategy):
    def __init__(self, publisher: StreamPublisher) -> None:
        super().__init__(
            StrategyConfig(
                strategy_id=StrategyId.from_str("OMEGA-STREAM-001"),
                log_events=False,
                log_commands=False,
            )
        )
        self._publisher = publisher
        self._last_account_event_count = -1
        self._last_account_snapshot_ns = 0
        self._last_order_signature = ""
        self._last_position_signature = ""
        self._published_fill_ids: set[str] = set()

    def on_start(self) -> None:
        self.subscribe_quotes(INSTRUMENT, client_id=CLIENT)
        self.subscribe_trades(INSTRUMENT, client_id=CLIENT)
        self.subscribe_book_deltas(
            INSTRUMENT,
            BookType.L2_MBP,
            depth=10,
            client_id=CLIENT,
            managed=True,
        )
        self.clock.set_timer(
            "omega-state-snapshot",
            timedelta(seconds=1),
            callback=self._publish_state,
            fire_immediately=True,
        )

    def on_quote(self, quote: Any) -> None:
        self._publisher.publish("quote", quote.to_dict(), lossless=False)

    def on_trade(self, trade: Any) -> None:
        self._publisher.publish("trade", trade.to_dict(), lossless=False)

    def on_book_deltas(self, deltas: Any) -> None:
        self._publisher.publish(
            "book",
            {
                "instrument_id": str(deltas.instrument_id),
                "deltas": [delta.to_dict() for delta in deltas.deltas],
                "ts_event": deltas.ts_event,
                "ts_init": deltas.ts_init,
            },
            lossless=False,
        )

    def on_order_event(self, event: Any) -> None:
        self._publisher.publish("order", event.to_dict(), lossless=True)

    def on_order_filled(self, event: Any) -> None:
        self._publish_fill(event.to_dict())

    def on_position_event(self, event: Any) -> None:
        self._publisher.publish("position", event.to_dict(), lossless=True)

    def _publish_fill(self, event: dict[str, Any]) -> None:
        fill_id = event.get("trade_id") or event.get("event_id")
        if not isinstance(fill_id, str) or not fill_id or fill_id in self._published_fill_ids:
            return
        self._published_fill_ids.add(fill_id)
        self._publisher.publish("fill", event, lossless=True)

    def _publish_state(self, _: Any) -> None:
        account = self.cache.account_for_venue(VENUE)
        now_ns = self.clock.timestamp_ns()
        account_changed = (
            account is not None and account.event_count != self._last_account_event_count
        )
        account_snapshot_due = (
            now_ns - self._last_account_snapshot_ns >= ACCOUNT_SNAPSHOT_INTERVAL_NS
        )
        if account is not None and (account_changed or account_snapshot_due):
            last_event = account.last_event
            if last_event is not None:
                self._publisher.publish("account", last_event.to_dict(), lossless=True)
                self._last_account_event_count = account.event_count
                self._last_account_snapshot_ns = now_ns

        orders = [order.to_dict() for order in self.cache.orders_open(venue=VENUE)]
        # An IOC remainder can never rest at the venue. Hyperliquid reconciliation can
        # reconstruct a partial fill without a terminal cancel event, leaving Nautilus'
        # local cache at PARTIALLY_FILLED after the official openOrders set is empty.
        # Do not publish that impossible remainder as live state.
        orders = [
            order
            for order in orders
            if not (
                order.get("time_in_force") == "IOC"
                and order.get("status") == "PARTIALLY_FILLED"
            )
        ]
        order_signature = json.dumps(orders, sort_keys=True, separators=(",", ":"))
        if order_signature != self._last_order_signature:
            self._publisher.publish(
                "order_state",
                {"venue": str(VENUE), "orders": orders, "ts_init": self.clock.timestamp_ns()},
                lossless=True,
            )
            self._last_order_signature = order_signature

        all_positions = [position.to_dict() for position in self.cache.positions(venue=VENUE)]
        for position in all_positions:
            for event in position.get("events", []):
                if isinstance(event, dict) and event.get("type") == "OrderFilled":
                    self._publish_fill(event)

        positions = [position.to_dict() for position in self.cache.positions_open(venue=VENUE)]
        position_signature = json.dumps(positions, sort_keys=True, separators=(",", ":"))
        if position_signature != self._last_position_signature:
            self._publisher.publish(
                "position_state",
                {
                    "venue": str(VENUE),
                    "positions": positions,
                    "ts_init": self.clock.timestamp_ns(),
                },
                lossless=True,
            )
            self._last_position_signature = position_signature
