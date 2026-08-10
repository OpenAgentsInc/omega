# Nautilus cost-floor report

Omega admits the bounded quote strategy only when
`OMEGA_NAUTILUS_COST_FLOOR_REPORT` names a validated
`omega.nautilus.cost_floor_report.v1` JSON file.

The report contains exactly these six testnet cells:

| Entry path | Intended clip |
| --- | ---: |
| `taker_taker` | $65 |
| `taker_taker` | $325 |
| `taker_taker` | $650 |
| `maker_taker` | $65 |
| `maker_taker` | $325 |
| `maker_taker` | $650 |

Each cell embeds at least five unique completed round-trip samples. A sample
contains the entry and exit quote generation/sequence, fill
generation/sequence, client order ID, quantity, pre-trade mid, fill price,
liquidity, fee, and signed adverse slippage. Loading a report recomputes every
cell summary from those samples and recomputes `raw_evidence_sha256` across the
ordered six-cell raw matrix. Missing cells, repeated fill evidence, changed
summaries, unsupported clips or paths, and digest mismatches are refused.

The bounded quote strategy uses the `maker_taker` / $65 cell because its fixed
order quantity is the $65 measurement clip. Its typed parameters carry the
cell path, clip, unique sample count, measured round-trip median, explicit
margin, admission floor, and raw-evidence digest. The Rust command validator
and the Python engine bridge both require:

```text
admission_floor_bps = ceil(max(median_cost_micros_bps, 0) / 1_000_000)
                      + cost_margin_bps
quote_offset_bps >= admission_floor_bps
```

No fee-schedule value can stand in for the measured median. An absent or
invalid report prevents parameter admission and strategy start. Start carries
the same evidence digest and the engine refuses a mismatch with its applied
parameters. The report is
testnet evidence and does not make mainnet representable or approved.
