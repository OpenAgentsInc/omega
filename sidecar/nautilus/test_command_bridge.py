import unittest
from collections import deque

from order_budget import ROLLING_ORDER_WINDOW_NS, rolling_budget_wait_until_ns


class RollingOrderBudgetTests(unittest.TestCase):
    def test_full_budget_waits_without_classifying_a_breach(self) -> None:
        order_times = deque([10, 20])

        wait_until = rolling_budget_wait_until_ns(order_times, 30, 2)

        self.assertEqual(wait_until, 10 + ROLLING_ORDER_WINDOW_NS)
        self.assertEqual(list(order_times), [10, 20])

    def test_budget_resumes_only_after_the_oldest_slot_ages_out(self) -> None:
        order_times = deque([10, 20])

        wait_until = rolling_budget_wait_until_ns(
            order_times, 10 + ROLLING_ORDER_WINDOW_NS, 2
        )

        self.assertIsNone(wait_until)
        self.assertEqual(list(order_times), [20])


if __name__ == "__main__":
    unittest.main()
