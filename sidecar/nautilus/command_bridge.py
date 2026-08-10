from __future__ import annotations

import datetime
import queue
from collections import deque
from decimal import Decimal
from typing import Any

from nautilus_trader.common import DataActorConfig
from nautilus_trader.model import ClientId
from nautilus_trader.model import ClientOrderId
from nautilus_trader.model import InstrumentId
from nautilus_trader.model import OrderSide
from nautilus_trader.model import Price
from nautilus_trader.model import Quantity
from nautilus_trader.model import StrategyId
from nautilus_trader.model import TimeInForce
from nautilus_trader.model import BookType
from nautilus_trader.trading import Controller
from nautilus_trader.trading import ImportableStrategyConfig
from nautilus_trader.trading import Strategy
from nautilus_trader.trading import StrategyConfig

from order_budget import rolling_budget_wait_until_ns


COMMAND_SCHEMA = "omega.nautilus.command.v1"
COMMAND_CONTROLLER_ID = "OMEGA-COMMAND-CONTROLLER-001"
EXECUTION_STRATEGY_ID = "OMEGA-COMMAND-EXECUTION-001"
BOUNDED_QUOTE_STRATEGY_ID = "OMEGA-BOUNDED-QUOTE-001"
HYPERLIQUID_CLIENT_ID = ClientId.from_str("HYPERLIQUID")
INSTRUMENT = InstrumentId.from_str("BTC-USD-PERP.HYPERLIQUID")
commands: queue.Queue[dict[str, Any]] = queue.Queue()
emit_event = None
generation = 0
execution_strategy = None
bounded_quote_strategy = None
stream_publisher = None


def configure(event_emitter: Any, engine_generation: int) -> None:
    global emit_event, generation
    emit_event = event_emitter
    generation = engine_generation


def configure_stream(publisher: Any) -> None:
    global stream_publisher
    stream_publisher = publisher


def enqueue(command: dict[str, Any]) -> None:
    commands.put(command)


def emit(command: dict[str, Any], event_type: str, **fields: object) -> None:
    if emit_event is None:
        raise RuntimeError("command bridge event output is not configured")
    emit_event(
        event_type,
        generation,
        schema=COMMAND_SCHEMA,
        command_id=command["command_id"],
        command_type=command["type"],
        **fields,
    )


def publish_fill(event: Any) -> None:
    if stream_publisher is None:
        raise RuntimeError("strategy fill output is not configured")
    stream_publisher.publish("fill", event.to_dict(), lossless=True)


class CommandControllerConfig(DataActorConfig):
    pass


class CommandExecutionStrategyConfig:
    def __new__(cls) -> StrategyConfig:
        return StrategyConfig(
            strategy_id=StrategyId.from_str(EXECUTION_STRATEGY_ID),
            order_id_tag="E287",
        )


class BoundedQuoteStrategyConfig:
    def __new__(cls) -> StrategyConfig:
        return StrategyConfig(
            strategy_id=StrategyId.from_str(BOUNDED_QUOTE_STRATEGY_ID),
            order_id_tag="Q290",
        )


class BoundedQuoteStrategy(Strategy):
    def __init__(self, config: StrategyConfig) -> None:
        super().__init__(config)
        global bounded_quote_strategy
        bounded_quote_strategy = self
        self.parameters: dict[str, Any] | None = None
        self.latest_ask: Decimal | None = None
        self.active_client_order_id: ClientOrderId | None = None
        self.pending_price: str | None = None
        self.last_action_ns = 0
        self.order_times_ns: deque[int] = deque()
        self.quote_ticks = 0
        self.trade_ticks = 0
        self.book_ticks = 0
        self.action_count = 0
        self.filled_notional_usd = Decimal("0")
        self.halted_reason: str | None = None
        self.budget_wait_until_ns: int | None = None

    def apply_parameters(self, parameters: dict[str, Any]) -> None:
        self.parameters = parameters.copy()
        self._publish_state("parameters_applied")

    def on_start(self) -> None:
        if self.halted_reason is not None:
            self._publish_state("start_refused_halted")
            return
        if self.parameters is None:
            self._halt("mandate_parameters_missing")
            return
        self.subscribe_quotes(INSTRUMENT, client_id=HYPERLIQUID_CLIENT_ID)
        self.subscribe_trades(INSTRUMENT, client_id=HYPERLIQUID_CLIENT_ID)
        self.subscribe_book_deltas(
            INSTRUMENT,
            BookType.L2_MBP,
            depth=10,
            client_id=HYPERLIQUID_CLIENT_ID,
            managed=True,
        )
        self._publish_state("running")

    def on_stop(self) -> None:
        if self.active_client_order_id is not None:
            self.cancel_order(
                self.active_client_order_id,
                client_id=HYPERLIQUID_CLIENT_ID,
            )
        self._publish_state("stopped")

    def on_quote(self, quote: Any) -> None:
        self.quote_ticks += 1
        self.latest_ask = Decimal(str(quote.ask_price))
        self._react()

    def on_trade(self, _trade: Any) -> None:
        self.trade_ticks += 1
        self._react()

    def on_book_deltas(self, _deltas: Any) -> None:
        self.book_ticks += 1
        self._react()

    def on_order_accepted(self, event: Any) -> None:
        if event.client_order_id == self.active_client_order_id:
            self._publish_state("order_resting")

    def on_order_canceled(self, event: Any) -> None:
        if event.client_order_id != self.active_client_order_id:
            return
        self.active_client_order_id = None
        price = self.pending_price
        self.pending_price = None
        if price is not None and self.halted_reason is None and self.is_running():
            self._place(price)

    def on_order_rejected(self, event: Any) -> None:
        if event.client_order_id == self.active_client_order_id:
            self.active_client_order_id = None
            self._halt("venue_rejected")

    def on_order_denied(self, event: Any) -> None:
        if event.client_order_id == self.active_client_order_id:
            self.active_client_order_id = None
            self._halt("risk_denied")

    def on_order_filled(self, event: Any) -> None:
        if event.client_order_id != self.active_client_order_id:
            return
        publish_fill(event)
        self.filled_notional_usd += Decimal(str(event.last_qty)) * Decimal(str(event.last_px))
        if self.parameters is not None and self.filled_notional_usd > Decimal(
            str(self.parameters["position_headroom_usd"])
        ):
            self._halt("position_limit")
        self._publish_state("fill")

    def _react(self) -> None:
        if self.parameters is None or self.latest_ask is None or self.halted_reason is not None:
            return
        now_ns = self.clock.timestamp_ns()
        interval_ns = self.parameters["min_reprice_interval_ms"] * 1_000_000
        if now_ns - self.last_action_ns < interval_ns:
            return
        wait_until_ns = rolling_budget_wait_until_ns(
            self.order_times_ns, now_ns, self.parameters["order_budget"]
        )
        if wait_until_ns is not None:
            if self.budget_wait_until_ns != wait_until_ns:
                self.budget_wait_until_ns = wait_until_ns
                self._publish_state("budget_wait")
            return
        if self.budget_wait_until_ns is not None:
            self.budget_wait_until_ns = None
            self._publish_state("budget_resumed")
        offset = Decimal(self.parameters["quote_offset_bps"]) / Decimal(10_000)
        price = str((self.latest_ask * (Decimal(1) + offset)).quantize(Decimal("0.1")))
        if self.active_client_order_id is None:
            self._place(price)
        elif self.pending_price is None:
            self.pending_price = price
            self.cancel_order(self.active_client_order_id, client_id=HYPERLIQUID_CLIENT_ID)
            self.last_action_ns = now_ns
            self.action_count += 1
            self._publish_state("cancel_sent")

    def _place(self, price: str) -> None:
        if self.parameters is None:
            self._halt("mandate_parameters_missing")
            return
        expected_notional = Decimal(self.parameters["order_quantity"]) * Decimal(price)
        if self.filled_notional_usd + expected_notional > Decimal(
            str(self.parameters["position_headroom_usd"])
        ):
            self._halt("position_limit")
            return
        now_ns = self.clock.timestamp_ns()
        client_order_id = ClientOrderId.from_str(f"O-290-{now_ns}")
        order = self.order_factory.limit(
            instrument_id=INSTRUMENT,
            order_side=OrderSide.SELL,
            quantity=Quantity.from_str(self.parameters["order_quantity"]),
            price=Price.from_str(price),
            time_in_force=TimeInForce.GTC,
            post_only=True,
            reduce_only=False,
            client_order_id=client_order_id,
        )
        self.active_client_order_id = client_order_id
        self.order_times_ns.append(now_ns)
        self.last_action_ns = now_ns
        self.action_count += 1
        self.submit_order(order, client_id=HYPERLIQUID_CLIENT_ID)
        self._publish_state("order_sent")

    def _halt(self, reason: str) -> None:
        self.halted_reason = reason
        if self.active_client_order_id is not None:
            self.cancel_order(self.active_client_order_id, client_id=HYPERLIQUID_CLIENT_ID)
        self._publish_state("halted")

    def prepare_explicit_start(self, cost_evidence_sha256: str) -> str | None:
        if self.parameters is None:
            return "invalid_parameters"
        if self.parameters.get("cost_evidence_sha256") != cost_evidence_sha256:
            return "invalid_parameters"
        if self.halted_reason is not None:
            return "strategy_halted"
        if self.budget_wait_until_ns is None:
            return None
        now_ns = self.clock.timestamp_ns()
        if now_ns < self.budget_wait_until_ns:
            return "order_budget_wait"
        self.budget_wait_until_ns = None
        self._publish_state("budget_resumed")
        return None

    def _publish_state(self, phase: str) -> None:
        if stream_publisher is None:
            return
        parameters = self.parameters or {}
        stream_publisher.publish(
            "strategy_state",
            {
                "strategy_id": BOUNDED_QUOTE_STRATEGY_ID,
                "phase": phase,
                "running": self.is_running(),
                "halted_reason": self.halted_reason,
                "budget_wait_until_ns": self.budget_wait_until_ns,
                "mandate_revision": parameters.get("mandate_revision", 0),
                "quote_ticks": self.quote_ticks,
                "trade_ticks": self.trade_ticks,
                "book_ticks": self.book_ticks,
                "action_count": self.action_count,
                "active_client_order_id": (
                    str(self.active_client_order_id)
                    if self.active_client_order_id is not None
                    else None
                ),
                "ts_init": self.clock.timestamp_ns(),
            },
            lossless=True,
        )


class CommandExecutionStrategy(Strategy):
    def __init__(self, config: StrategyConfig) -> None:
        super().__init__(config)
        global execution_strategy
        execution_strategy = self
        self.place_commands: dict[str, dict[str, Any]] = {}
        self.cancel_commands: dict[str, dict[str, Any]] = {}

    def place_order(self, command: dict[str, Any]) -> None:
        client_order_id = ClientOrderId.from_str(command["client_order_id"])
        self.place_commands[str(client_order_id)] = command
        try:
            order = self.order_factory.limit(
                instrument_id=InstrumentId.from_str(command["instrument_id"]),
                order_side=OrderSide.BUY if command["side"] == "buy" else OrderSide.SELL,
                quantity=Quantity.from_str(command["quantity"]),
                price=Price.from_str(command["price"]),
                time_in_force=(
                    TimeInForce.IOC
                    if command["time_in_force"] == "ioc"
                    else TimeInForce.GTC
                ),
                post_only=command["post_only"],
                reduce_only=command["reduce_only"],
                client_order_id=client_order_id,
            )
            emit(
                command,
                "sent",
                client_order_id=command["client_order_id"],
                mutation_state="sent",
            )
            self.submit_order(order, client_id=HYPERLIQUID_CLIENT_ID)
        except Exception:
            pending_command = self.place_commands.pop(str(client_order_id), None)
            if pending_command is not None:
                emit(
                    pending_command,
                    "unknown",
                    client_order_id=pending_command["client_order_id"],
                    mutation_state="unknown",
                    reason_code="dispatch_failed",
                )

    def cancel_order_by_id(self, command: dict[str, Any]) -> None:
        client_order_id = ClientOrderId.from_str(command["client_order_id"])
        order = self.cache.order(client_order_id)
        if order is None:
            emit(command, "refused", reason_code="order_not_found")
            return
        self.cancel_commands[str(client_order_id)] = command
        try:
            emit(
                command,
                "sent",
                client_order_id=command["client_order_id"],
                mutation_state="sent",
            )
            self.cancel_order(client_order_id, client_id=HYPERLIQUID_CLIENT_ID)
        except Exception:
            pending_command = self.cancel_commands.pop(str(client_order_id), None)
            if pending_command is not None:
                emit(
                    pending_command,
                    "unknown",
                    client_order_id=pending_command["client_order_id"],
                    mutation_state="unknown",
                    reason_code="dispatch_failed",
                )

    def on_order_denied(self, event: Any) -> None:
        self._emit_post_dispatch_unknown(
            self.place_commands,
            event.client_order_id,
            "post_dispatch_risk_denied",
        )

    def on_order_rejected(self, event: Any) -> None:
        self._emit_post_dispatch_unknown(
            self.place_commands,
            event.client_order_id,
            "post_dispatch_venue_rejected",
        )

    def on_order_accepted(self, event: Any) -> None:
        command = self.place_commands.pop(str(event.client_order_id), None)
        if command is not None:
            emit(
                command,
                "order_accepted",
                client_order_id=str(event.client_order_id),
                venue_order_id=str(event.venue_order_id),
            )

    def on_order_filled(self, event: Any) -> None:
        publish_fill(event)

    def on_order_cancel_rejected(self, event: Any) -> None:
        self._emit_post_dispatch_unknown(
            self.cancel_commands,
            event.client_order_id,
            "post_dispatch_cancel_rejected",
        )

    def on_order_canceled(self, event: Any) -> None:
        command = self.cancel_commands.pop(str(event.client_order_id), None)
        if command is not None:
            emit(
                command,
                "order_canceled",
                client_order_id=str(event.client_order_id),
                venue_order_id=str(event.venue_order_id),
            )

    def _emit_post_dispatch_unknown(
        self,
        pending_commands: dict[str, dict[str, Any]],
        client_order_id: ClientOrderId,
        reason_code: str,
    ) -> None:
        command = pending_commands.pop(str(client_order_id), None)
        if command is not None:
            emit(
                command,
                "unknown",
                client_order_id=str(client_order_id),
                mutation_state="unknown",
                reason_code=reason_code,
            )


class CommandController(Controller):
    def on_start(self) -> None:
        self.create_strategy_from_config(
            ImportableStrategyConfig(
                strategy_path="command_bridge:CommandExecutionStrategy",
                config_path="command_bridge:CommandExecutionStrategyConfig",
                config={},
            ),
            start=True,
        )
        self.create_strategy_from_config(
            ImportableStrategyConfig(
                strategy_path="command_bridge:BoundedQuoteStrategy",
                config_path="command_bridge:BoundedQuoteStrategyConfig",
                config={},
            ),
            start=False,
        )
        self.clock.set_timer(
            "omega-command-drain",
            datetime.timedelta(milliseconds=10),
        )

    def on_stop(self) -> None:
        self.clock.cancel_timers()

    def on_time_event(self, _event: object) -> None:
        while True:
            try:
                command = commands.get_nowait()
            except queue.Empty:
                return
            self._handle(command)

    def _handle(self, command: dict[str, Any]) -> None:
        refusal = validate_command(command)
        if refusal is not None:
            emit(command, "refused", reason_code=refusal)
            return
        emit(command, "acknowledged")
        command_type = command["type"]
        if command_type == "place_order":
            execution_strategy.place_order(command)
        elif command_type == "cancel_order":
            execution_strategy.cancel_order_by_id(command)
        elif command_type == "start_strategy":
            start_refusal = bounded_quote_strategy.prepare_explicit_start(
                command["cost_evidence_sha256"]
            )
            if start_refusal is not None:
                emit(command, "refused", reason_code=start_refusal)
                return
            self.start_strategy_from_id(bounded_quote_strategy.strategy_id)
            emit(command, "strategy_started", running=bounded_quote_strategy.is_running())
        elif command_type == "stop_strategy":
            self.stop_strategy_from_id(bounded_quote_strategy.strategy_id)
            emit(command, "strategy_stopped", running=bounded_quote_strategy.is_running())
        else:
            parameters = command["parameters"]
            bounded_quote_strategy.apply_parameters(parameters)
            emit(command, "strategy_parameters_applied", parameters=parameters)


def validate_command(command: dict[str, Any]) -> str | None:
    common = {"schema", "generation", "network", "command_id", "type"}
    command_type = command.get("type")
    fields = {
        "place_order": {
            "client_order_id",
            "instrument_id",
            "side",
            "quantity",
            "price",
            "time_in_force",
            "post_only",
            "reduce_only",
        },
        "cancel_order": {"client_order_id"},
        "start_strategy": {"strategy_id", "cost_evidence_sha256"},
        "stop_strategy": {"strategy_id"},
        "set_strategy_parameters": {"strategy_id", "parameters"},
    }
    expected = fields.get(command_type)
    if expected is None or set(command) != common | expected:
        return "invalid_command"
    if (
        command.get("schema") != COMMAND_SCHEMA
        or command.get("generation") != generation
        or command.get("network") != "testnet"
    ):
        return "invalid_envelope"
    if not isinstance(command.get("command_id"), str) or not command["command_id"]:
        return "invalid_command_id"
    if command_type in {"start_strategy", "stop_strategy", "set_strategy_parameters"}:
        if command.get("strategy_id") != BOUNDED_QUOTE_STRATEGY_ID:
            return "unknown_strategy"
    if command_type == "start_strategy":
        evidence = command.get("cost_evidence_sha256")
        if (
            not isinstance(evidence, str)
            or len(evidence) != 64
            or any(character not in "0123456789abcdef" for character in evidence)
        ):
            return "invalid_parameters"
    if command_type == "place_order":
        if command.get("side") not in {"buy", "sell"}:
            return "invalid_order"
        if command.get("time_in_force") not in {"gtc", "ioc"}:
            return "invalid_order"
        if not all(
            isinstance(command.get(field), str) and command[field]
            for field in ["client_order_id", "instrument_id", "quantity", "price"]
        ):
            return "invalid_order"
        if not isinstance(command.get("post_only"), bool) or not isinstance(
            command.get("reduce_only"), bool
        ):
            return "invalid_order"
    if command_type == "cancel_order" and (
        not isinstance(command.get("client_order_id"), str)
        or not command["client_order_id"]
    ):
        return "invalid_order"
    if command_type == "set_strategy_parameters":
        parameters = command.get("parameters")
        if not isinstance(parameters, dict) or set(parameters) != {
            "min_reprice_interval_ms",
            "quote_offset_bps",
            "order_quantity",
            "position_headroom_usd",
            "order_budget",
            "mandate_revision",
            "cost_path",
            "cost_clip_usd",
            "cost_sample_count",
            "measured_round_trip_cost_micros_bps",
            "cost_margin_bps",
            "admission_floor_bps",
            "cost_evidence_sha256",
        }:
            return "invalid_parameters"
        measured_ceiling_bps = (
            max(parameters["measured_round_trip_cost_micros_bps"], 0) + 999_999
        ) // 1_000_000 if isinstance(
            parameters["measured_round_trip_cost_micros_bps"], int
        ) and not isinstance(
            parameters["measured_round_trip_cost_micros_bps"], bool
        ) else None
        if (
            not isinstance(parameters["min_reprice_interval_ms"], int)
            or isinstance(parameters["min_reprice_interval_ms"], bool)
            or not 100 <= parameters["min_reprice_interval_ms"] <= 60_000
            or not isinstance(parameters["quote_offset_bps"], int)
            or not 0 <= parameters["quote_offset_bps"] <= 1_000
            or not isinstance(parameters["order_quantity"], str)
            or parameters["order_quantity"] != "0.001"
            or not isinstance(parameters["position_headroom_usd"], int)
            or parameters["position_headroom_usd"] <= 0
            or not isinstance(parameters["order_budget"], int)
            or not 1 <= parameters["order_budget"] <= 100
            or not isinstance(parameters["mandate_revision"], int)
            or parameters["mandate_revision"] <= 0
            or parameters["cost_path"] != "maker_taker"
            or parameters["cost_clip_usd"] != 65
            or not isinstance(parameters["cost_sample_count"], int)
            or isinstance(parameters["cost_sample_count"], bool)
            or parameters["cost_sample_count"] < 5
            or measured_ceiling_bps is None
            or not isinstance(parameters["cost_margin_bps"], int)
            or isinstance(parameters["cost_margin_bps"], bool)
            or parameters["cost_margin_bps"] <= 0
            or not isinstance(parameters["admission_floor_bps"], int)
            or isinstance(parameters["admission_floor_bps"], bool)
            or parameters["admission_floor_bps"]
            != measured_ceiling_bps + parameters["cost_margin_bps"]
            or parameters["quote_offset_bps"] < parameters["admission_floor_bps"]
            or not isinstance(parameters["cost_evidence_sha256"], str)
            or len(parameters["cost_evidence_sha256"]) != 64
            or any(
                character not in "0123456789abcdef"
                for character in parameters["cost_evidence_sha256"]
            )
        ):
            return "invalid_parameters"
    return None
