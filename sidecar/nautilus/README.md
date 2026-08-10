# Omega Nautilus sidecar

This component is the testnet-only NautilusTrader engine managed by Omega.
Its bounded BTC quote strategy runs quote, trade, and book reactions inside
Nautilus. Omega supplies its mandate envelope and start, parameter, and stop
commands above the tick loop.

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

The app writes `omega.nautilus.command.v1` JSON envelopes to the child stdin.
Place, cancel, strategy start, strategy stop, and typed strategy-parameter
commands return acknowledgements and outcomes on the same versioned stdout
event stream. Effectful commands are single-attempt: a lost or ambiguous
outcome becomes `unknown` and is never retried by the channel. Mainnet remains
unrepresentable in the Rust command types and refused by the Python entrypoint.
