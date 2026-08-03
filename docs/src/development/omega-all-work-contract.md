# Omega All Work contract

Omega reads Work through the generated OpenAgents boundary. The Full Auto
adapter shows durable runs as redacted Work summaries and snapshots. The
planning adapter reads the persistent OpenAgents planning authority. Neither
adapter creates a second writable store.

## Immutable source

| Field                 | Value                                                              |
| --------------------- | ------------------------------------------------------------------ |
| OpenAgents commit     | `f169754d09f3c56532c7f11660ed3de007c7c612`                         |
| Contract              | `openagents.all_work_boundary.v1`                                  |
| Definition SHA-256    | `f41f9e8b44f95936694c74799027fa78b9e35ffe102a1a85e4b86027bb15748b` |
| Rust artifact SHA-256 | `da18308417540bbac93124ebf15dda7bb192649afc2460fa591f592d3945ab14` |
| Omega receipt         | `crates/omega_effectd/all-work-contract/SOURCE.json`               |

The generated Rust file, compatibility manifest, and shared fixtures are
vendored together. `script/sync-all-work-contract` refuses a source checkout
that is not at the pinned commit, then verifies the Rust digest before it
copies any file.

## Protocol

The process transport remains `openagents.omega.effectd.v1` so existing Omega
clients continue to initialize. Its initialization payload negotiates a
separate All Work profile:

- an omitted All Work request selects `omega-effectd.v1` with no Work methods;
- `omega-effectd.v2` enables `work.index.read`, `work.snapshot.read`, and
  `planning.graph.read`;
- a v1 client that calls any All Work read method receives
  `incompatible_version`;
- request and result domain payloads use generated Rust and Effect types;
- objective and done-condition text do not enter the Work Index;
- the planning graph retains its OpenAgents revision, event cursor,
  reconciliation digest, source coordinates, completeness, and loss facts.

The Effect adapter reads the Full Auto registry. That registry remains the
writable authority. Omega combines its generated summaries with separate
read-only native adapters for Threads and Forensics. Each adapter retains its
own source authority and cursor. The qualified rows feed
[Omega Work Index](./omega-work-index.md); they do not create a second writable
store.

The planning authority is Effect-owned state beneath the injected application
data root. Its initial bounded GitHub reconciliation contains 22 open and six
closed Work rows. GitHub remains an imported read-only source. A successful
read does not grant command, claim, delegation, verification, release, or
owner-disposition authority. Release Planning Records remain planning metadata.

## Verify

Run:

```sh
cargo test -p omega_effectd typed_all_work_reads_cross_the_supervised_process_boundary
cargo test -p omega_effectd all_work
OPENAGENTS_ALL_WORK_SOURCE_ROOT=/path/to/pinned/openagents \
  cargo test -p omega_effectd typed_all_work_index_crosses_the_openagents_process_boundary \
  -- --ignored
./script/clippy -p omega_effectd
```

The OpenAgents `omega-effectd` package also runs an actual child-process test
against its TypeScript binary. The Omega test uses the Rust supervisor and a
newline-framed child process to verify negotiation, Work Index decoding, and
Work snapshot decoding. The ignored cross-repository smoke starts that actual
OpenAgents TypeScript process through the Omega Rust supervisor; it requires
the immutable source checkout named above and also verifies all 28 planning
rows through the generated `PlanningGraphReadResult`.
