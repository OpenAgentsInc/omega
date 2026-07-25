# Omega issue 31 mobile host adjunct

- Date: 2026-07-24
- Packet: `OMEGA-MOB-31-00`
- Issue: [#44](https://github.com/OpenAgentsInc/omega/issues/44)
- Parent: [#31](https://github.com/OpenAgentsInc/omega/issues/31)

## Result {#result}

Omega exposes a bounded, public-safe decoder for
`openagents.omega.issue31.host.v1`. OpenAgents Mobile joins this adjunct with
the existing signed Nostr records in its local `Issue31WorkroomReadModel`.
Neither the adjunct nor the local aggregate is durable authority.

The decoder lives in `workroom_receipts` because that crate already owns the
pure public-reference and redaction boundary used by the workroom receipt
inspector. It has no GPUI, network, process, or durable-state dependency.
Runtime production of the snapshot stays with Omega's operation owners and
`omega_effectd`; the later Full Auto mobile packet connects that runtime.

## Coverage boundary {#coverage-boundary}

The adjunct has exactly four host-owned projections:

| Projection            | Host-owned fact                                              |
| --------------------- | ------------------------------------------------------------ |
| `connection_identity` | Host, pairing, binding, grant, and revocation references     |
| `full_auto_runs`      | Full Auto record and permitted control references            |
| `provider_accounts`   | Provider roster and host-owned connection-handoff references |
| `evidence_chain`      | Bounded evidence, outcome, decision, and receipt references  |

The other issue 31 capabilities remain Nostr-primary. Owner-private Sarah,
memory, read state, reminders, attention targets, community membership,
community work, and experience are not copied into this schema. The mobile
read model joins their signed records with these four projections by stable
reference.

This split keeps all eleven issue 31 coverage rows visible without creating an
aggregate event or a REST mirror. An unavailable host projection does not hide
an already-confirmed Nostr record.

## Contract states and bounds {#contract-states-and-bounds}

Each projection contains:

- one `omega_host` source reference and observation time
- `current`, `stale`, or `unknown` freshness
- `complete`, `partial`, `missing`, or `unavailable` gap state
- an owner, member, verifier, or observer role and grant state
- at most 16 record references and 16 permitted action references
- one idle, pending, refused, or terminal command state

Millisecond timestamps cannot exceed `8,640,000,000,000,000`, the shared
JavaScript `Date` bound used by the TypeScript decoder.

The decoder requires all four projections once. It rejects duplicate
capabilities and references. `missing` and `unavailable` projections must use
unknown freshness, expose no records or actions, and remain idle. A pending
command requires an active role and a currently permitted action.

All references use the existing public-reference validator and are limited to
256 characters. Unknown fields fail closed. Exact paths, credentials, prompts,
private payloads, and unbounded output cannot enter the decoded projection.
Decoder errors report a reason class and never echo rejected input.

## Shared fixtures {#shared-fixtures}

Rust owns the canonical fixture bytes under
`crates/workroom_receipts/fixtures`. TypeScript mirrors the same four files:

- `openagents.omega.issue31.host.v1.canonical.json`
- `openagents.omega.issue31.host.v1.negative-private-field.json`
- `openagents.omega.issue31.host.v1.negative-unsafe-ref.json`
- `openagents.omega.issue31.host.v1.negative-invalid-state.json`

The canonical fixture covers idle, pending, refused, and terminal states. The
negative fixtures prove unknown private fields, unsafe references, and an
incoherent unavailable projection fail closed.

## Verification {#verification}

```sh
cargo test -p workroom_receipts --lib
./script/clippy -p workroom_receipts
```

## Falsifiers {#falsifiers}

- A GPUI entity or mobile view becomes the record owner.
- The host adjunct duplicates a Sarah or community Nostr record.
- A mobile REST route mirrors the Nostr record.
- A fixture is presented as a connected host source.
- A secret, private payload, exact local path, or raw tool output decodes.
