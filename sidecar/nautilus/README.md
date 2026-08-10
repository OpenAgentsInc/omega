# Omega Nautilus sidecar

This component is the testnet-only NautilusTrader engine managed by Omega.
Its bounded BTC quote strategy runs quote, trade, and book reactions inside
Nautilus. Omega supplies its mandate envelope and start, parameter, and stop
commands above the tick loop.

Install the pinned runtime with `./setup.sh`. Omega starts it only when
`OMEGA_NAUTILUS_SIDECAR=1`; `OMEGA_NAUTILUS_NETWORK` defaults to `testnet`.
`mainnet` is a named configuration but is refused before any connection or
sidecar effect until graduation. Omega generates separate testnet and mainnet
Hyperliquid agent wallets and keeps their private keys in separate
release-channel-namespaced platform credential-store records. At spawn, the
selected testnet key and public wallet binding enter the supervised child as
one `omega.nautilus.bootstrap.v1` record over stdin. The parent environment,
arguments, configuration files, lifecycle events, and UI never contain it.

The lifecycle protocol is newline-framed typed JSON prefixed by
`OMEGA_NAUTILUS_EVENT`. Version 1 reports `starting`, `healthy`, and `stopped`.
`healthy` means the testnet execution account is visible after reconciliation,
not merely that the Python process exists.

The app writes `omega.nautilus.command.v1` JSON envelopes to the child stdin.
Place, cancel, strategy start, strategy stop, and typed strategy-parameter
commands return acknowledgements and outcomes on the same versioned stdout
event stream. Effectful commands are single-attempt: a lost or ambiguous
outcome becomes `unknown` and is never retried by the channel. Command
envelopes remain pinned to testnet; both the Rust supervisor and Python
entrypoint refuse mainnet before execution.
