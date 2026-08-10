# Omega Nautilus sidecar

This component is the testnet-only NautilusTrader engine managed by Omega.
It contains lifecycle and venue connectivity only; strategies and governance
commands are separate layers.

Install the pinned runtime with `./setup.sh`. Omega starts it only when
`OMEGA_NAUTILUS_SIDECAR=1`; `OMEGA_NAUTILUS_NETWORK` defaults to `testnet` and
any other value is refused. The Hyperliquid private key is read from Omega's
private local credential store under
`omega://nautilus/hyperliquid-testnet-private-key`. It is passed to the child
only as `HYPERLIQUID_TESTNET_PK` and is never included in lifecycle events.

The lifecycle protocol is newline-framed typed JSON prefixed by
`OMEGA_NAUTILUS_EVENT`. Version 1 reports `starting`, `healthy`, and `stopped`.
`healthy` means the testnet execution account is visible after reconciliation,
not merely that the Python process exists.
