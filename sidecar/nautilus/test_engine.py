import io
import json
import sys
import unittest
from unittest.mock import patch

import engine


class BootstrapTests(unittest.TestCase):
    def bootstrap(self, **overrides: object) -> dict[str, object]:
        payload: dict[str, object] = {
            "schema": "omega.nautilus.bootstrap.v1",
            "network": "testnet",
            "private_key": "0x" + "11" * 32,
            "owner_address": "0x" + "22" * 20,
            "agent_address": "0x" + "33" * 20,
            "agent_name": "omega-testnet",
        }
        payload.update(overrides)
        return payload

    def read(self, payload: dict[str, object]) -> dict[str, object]:
        encoded = json.dumps(payload) + "\n"
        with patch.object(sys, "stdin", io.StringIO(encoded)):
            return engine.read_bootstrap("testnet")

    def test_approved_owner_address_reaches_bootstrap_consumer(self) -> None:
        payload = self.bootstrap()
        bootstrap = self.read(payload)

        self.assertEqual(bootstrap["owner_address"], payload["owner_address"])

    def test_missing_owner_address_fails_closed(self) -> None:
        payload = self.bootstrap()
        del payload["owner_address"]

        with self.assertRaisesRegex(RuntimeError, "owner address is missing or malformed"):
            self.read(payload)


if __name__ == "__main__":
    unittest.main()
