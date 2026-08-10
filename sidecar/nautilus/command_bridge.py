from __future__ import annotations

import datetime
import queue
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
from nautilus_trader.trading import Controller
from nautilus_trader.trading import ImportableStrategyConfig
from nautilus_trader.trading import Strategy
from nautilus_trader.trading import StrategyConfig


COMMAND_SCHEMA = "omega.nautilus.command.v1"
COMMAND_CONTROLLER_ID = "OMEGA-COMMAND-CONTROLLER-001"
EXECUTION_STRATEGY_ID = "OMEGA-COMMAND-EXECUTION-001"
TRIVIAL_STRATEGY_ID = "OMEGA-TRIVIAL-001"
HYPERLIQUID_CLIENT_ID = ClientId.from_str("HYPERLIQUID")
commands: queue.Queue[dict[str, Any]] = queue.Queue()
emit_event = None
generation = 0
execution_strategy = None
trivial_strategy = None


def configure(event_emitter: Any, engine_generation: int) -> None:
    global emit_event, generation
    emit_event = event_emitter
    generation = engine_generation


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


class CommandControllerConfig(DataActorConfig):
    pass


class CommandExecutionStrategyConfig:
    def __new__(cls) -> StrategyConfig:
        return StrategyConfig(
            strategy_id=StrategyId.from_str(EXECUTION_STRATEGY_ID),
            order_id_tag="E287",
        )


class TrivialStrategyConfig:
    def __new__(cls) -> StrategyConfig:
        return StrategyConfig(
            strategy_id=StrategyId.from_str(TRIVIAL_STRATEGY_ID),
            order_id_tag="T287",
        )


class TrivialStrategy(Strategy):
    def __init__(self, config: StrategyConfig) -> None:
        super().__init__(config)
        global trivial_strategy
        trivial_strategy = self
        self.interval_ms = 100
        self.signal = 0
        self.tick_count = 0

    def apply_parameters(self, interval_ms: int, signal: int) -> None:
        self.interval_ms = interval_ms
        self.signal = signal
        if self.is_running():
            self.clock.cancel_timers()
            self._start_timer()

    def on_start(self) -> None:
        self._start_timer()

    def on_stop(self) -> None:
        self.clock.cancel_timers()

    def on_time_event(self, _event: object) -> None:
        self.tick_count += 1

    def _start_timer(self) -> None:
        self.clock.set_timer(
            "omega-trivial-tick",
            datetime.timedelta(milliseconds=self.interval_ms),
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
                time_in_force=TimeInForce.GTC,
                post_only=command["post_only"],
                reduce_only=command["reduce_only"],
                client_order_id=client_order_id,
            )
            self.submit_order(order, client_id=HYPERLIQUID_CLIENT_ID)
            emit(
                command,
                "sent",
                client_order_id=command["client_order_id"],
                mutation_state="sent",
            )
        except Exception:
            emit(
                command,
                "unknown",
                client_order_id=command["client_order_id"],
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
            self.cancel_order(client_order_id, client_id=HYPERLIQUID_CLIENT_ID)
            emit(
                command,
                "sent",
                client_order_id=command["client_order_id"],
                mutation_state="sent",
            )
        except Exception:
            emit(
                command,
                "unknown",
                client_order_id=command["client_order_id"],
                mutation_state="unknown",
                reason_code="dispatch_failed",
            )

    def on_order_denied(self, event: Any) -> None:
        self._emit_order_refusal(event.client_order_id, "risk_denied")

    def on_order_rejected(self, event: Any) -> None:
        self._emit_order_refusal(event.client_order_id, "venue_rejected")

    def on_order_accepted(self, event: Any) -> None:
        command = self.place_commands.pop(str(event.client_order_id), None)
        if command is not None:
            emit(
                command,
                "order_accepted",
                client_order_id=str(event.client_order_id),
                venue_order_id=str(event.venue_order_id),
            )

    def on_order_cancel_rejected(self, event: Any) -> None:
        command = self.cancel_commands.pop(str(event.client_order_id), None)
        if command is not None:
            emit(command, "refused", reason_code="cancel_rejected")

    def on_order_canceled(self, event: Any) -> None:
        command = self.cancel_commands.pop(str(event.client_order_id), None)
        if command is not None:
            emit(
                command,
                "order_canceled",
                client_order_id=str(event.client_order_id),
                venue_order_id=str(event.venue_order_id),
            )

    def _emit_order_refusal(self, client_order_id: ClientOrderId, reason_code: str) -> None:
        command = self.place_commands.pop(str(client_order_id), None)
        if command is not None:
            emit(command, "refused", reason_code=reason_code)


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
                strategy_path="command_bridge:TrivialStrategy",
                config_path="command_bridge:TrivialStrategyConfig",
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
            self.start_strategy_from_id(trivial_strategy.strategy_id)
            emit(command, "strategy_started", running=trivial_strategy.is_running())
        elif command_type == "stop_strategy":
            self.stop_strategy_from_id(trivial_strategy.strategy_id)
            emit(command, "strategy_stopped", running=trivial_strategy.is_running())
        else:
            parameters = command["parameters"]
            trivial_strategy.apply_parameters(
                parameters["interval_ms"],
                parameters["signal"],
            )
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
            "post_only",
            "reduce_only",
        },
        "cancel_order": {"client_order_id"},
        "start_strategy": {"strategy_id"},
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
        if command.get("strategy_id") != TRIVIAL_STRATEGY_ID:
            return "unknown_strategy"
    if command_type == "place_order":
        if command.get("side") not in {"buy", "sell"}:
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
        if not isinstance(parameters, dict) or set(parameters) != {"interval_ms", "signal"}:
            return "invalid_parameters"
        if (
            not isinstance(parameters["interval_ms"], int)
            or isinstance(parameters["interval_ms"], bool)
            or not 10 <= parameters["interval_ms"] <= 60_000
            or not isinstance(parameters["signal"], int)
            or isinstance(parameters["signal"], bool)
        ):
            return "invalid_parameters"
    return None
