# Nautilus 72-hour Testnet soak

The unattended Nautilus soak is one immutable, versioned Testnet segment. It
starts only after the current commit passes its scoped tests, clippy, and
`cargo check -p omega`; the Hyperliquid agent is current; venue, engine, and
ledger balances reconcile per asset; and a user approves the fixed soak
mandate in Omega. Mainnet is outside this procedure.

## Isolated profile

Use a dedicated profile whose `config/settings.json` explicitly enables
scheduled wakeups:

```json
{
  "agent": {
    "wakeups": {
      "enabled": true,
      "interval_seconds": 3600,
      "max_turns_per_hour": 2,
      "max_tokens_per_turn": 4096,
      "max_tokens_per_hour": 8192,
      "poll_seconds": 15
    }
  }
}
```

In API Keys, approve the **fixed 72-hour soak mandate**. Omega binds it to
Hyperliquid Testnet and `OMEGA-BOUNDED-QUOTE-001`, gives it a 73-hour lifetime,
and preserves the already approved numeric limits. The segment manifest must
record that mandate's revision, digest, and expiry.

## Seal the segment

Create a canonical configuration artifact containing the complete isolated
settings, exact strategy parameters, and the 40-character bundle commit. Hash
those bytes with SHA-256 and put the digest in a manifest using schema
`omega.nautilus.soak_manifest.v1`. The manifest also fixes the segment ID,
start time, exactly 72 hours of required duration, health and review cadences,
mandate identity, venue, network, and strategy.

Set `started_at_ms` a few minutes in the future. Create the directory once:

```sh
cargo run -p nautilus_governance --bin nautilus_soak -- create SOAK_DIR manifest-input.json
```

Launch only the isolated Omega bundle with `OMEGA_NAUTILUS_SOAK_DIR` set to
that directory. Before `started_at_ms`, perform the account preflight, emit the
initial governance prediction, apply the sealed parameters, and start the
strategy. No parameter or config changes are permitted after `started_at_ms`.
Do not send user messages or manually wake, stop, or restart the strategy
during the segment. A stopped process or failed segment is evidence; it must
not be hidden by restarting, tuning, or splicing segments.

## Health and completion

Account snapshots append `omega.nautilus.soak_health.v1` entries with an
unbroken hash chain. Each entry binds the stream generation and sequence,
strategy phase, rolling-budget wait or halt state, queued wakeup, scheduled
review and prediction counts, ledger head, and venue/engine/ledger assets.
Asset drift, a missing review or prediction, an unannounced stop, or a halt
without a queued wakeup fails closed. Rolling hourly order capacity is a typed
wait and resumes only as old slots age out; it is not an anomaly or permission
for a human nudge.

Read-only health checks are:

```sh
cargo run -p nautilus_governance --bin nautilus_soak -- status SOAK_DIR
tail -n 1 SOAK_DIR/health.jsonl
```

After at least 72 wall-clock hours, finish with the immutable nudge count:

```sh
cargo run -p nautilus_governance --bin nautilus_soak -- finish SOAK_DIR END_MS 0
```

The command creates `omega.nautilus.soak_receipt.v1` exactly once only when
the complete segment, scheduled governance evidence, hash chain, wakeups, and
continuous per-asset reconciliation all pass.
