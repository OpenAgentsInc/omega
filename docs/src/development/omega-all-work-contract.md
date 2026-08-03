# Omega All Work contract

Omega reads Work through the generated OpenAgents boundary. The first adapter
shows durable Full Auto runs as redacted Work summaries and snapshots. It does
not create a second writable store.

## Immutable source

| Field                 | Value                                                              |
| --------------------- | ------------------------------------------------------------------ |
| OpenAgents commit     | `1ea08b1429cbd888875fef195f9b94bef666e70e`                         |
| Contract              | `openagents.all_work_boundary.v1`                                  |
| Definition SHA-256    | `f40c1d09b12103f0247a6354e020ed7322415c8b228e45a8fd3f8d7ccd3294f8` |
| Rust artifact SHA-256 | `298aa826cb7bdf182742251d53c9ab6a436ba8e386fd292a22701a7dec40cefb` |
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
- `omega-effectd.v2` enables `work.index.read` and `work.snapshot.read`;
- a v1 client that calls either Work method receives
  `incompatible_version`;
- request and result domain payloads use generated Rust and Effect types;
- objective and done-condition text do not enter the Work Index.

The Effect adapter reads the Full Auto registry. That registry remains the
writable authority. Omega combines its generated summaries with separate
read-only native adapters for Threads and Forensics. Each adapter retains its
own source authority and cursor. The qualified rows feed
[Omega Work Index](./omega-work-index.md); they do not create a second writable
store.

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
the immutable source checkout named above.
