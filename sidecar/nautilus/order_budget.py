from __future__ import annotations

from collections import deque


ROLLING_ORDER_WINDOW_NS = 3_600_000_000_000


def rolling_budget_wait_until_ns(
    order_times_ns: deque[int], now_ns: int, order_budget: int
) -> int | None:
    while order_times_ns and now_ns - order_times_ns[0] >= ROLLING_ORDER_WINDOW_NS:
        order_times_ns.popleft()
    if len(order_times_ns) < order_budget:
        return None
    return order_times_ns[0] + ROLLING_ORDER_WINDOW_NS
