# Omega entropy repository runs

Omega can run a read-only entropy analysis over the repository selected in the
Forensics workbench. This is a repository-local path. It does not use the
OpenAgents Cloud worker lifecycle and does not grant reporting, disclosure, or
source-write authority.

Choose **Run entropy scan** in Forensics. Omega requires a clean Git worktree, an
exact 40-character revision, and a configured inline-assistant model. It then
uses the existing project path and model configuration. It does not create a
checkout, shell, credential path, or publication destination.

## Frozen run input {#frozen-run-input}

Before the first model request, Omega traverses the selected worktree and
creates `openagents.omega.entropy-manifest.v1`. The manifest:

- excludes Git's internal `.git` path;
- sorts normalized relative paths before assigning sequence numbers;
- records the language, byte length, and SHA-256 content digest for each
  eligible file;
- records unsupported languages, files larger than 512 KiB, symbolic links,
  and unreadable files as skipped limitations; and
- records recursive Git submodules as available, missing, at the wrong
  revision, or unavailable.

The run binds that manifest and digest to the repository revision, prompt
digest, configured model route, read-only project tool surface, and start time.
Omega reads each file again immediately before its model request and rejects
the read if its digest changed after the manifest was frozen.

## File states and typed output {#file-states-and-typed-output}

The local runner processes one eligible file at a time in manifest order. Files
use these states:

- `queued`
- `reading`
- `analyzed`
- `candidate`
- `skipped`
- `failed`
- `cancelled`

The model response must decode as
`openagents.omega.entropy-file-output.v1`. Prose is not a finding. Typed source
observations include the analyzed file, symbols, suspected entropy mechanism,
secret consumers, exact source references, and a confidence boundary. Typed
hypotheses also include dense causal links, missing evidence, and the next
check.

Unsupported source, incomplete dependencies, file-read errors, request-schema
failures, model failures, and invalid response schemas stay distinct. A run
with one of these conditions finishes as `completed_with_limitations`; it
cannot become a clean result. Cancelling changes every queued or reading file
to `cancelled` and prevents a late model response from changing the terminal
run.

## Coldcard fixture {#coldcard-fixture}

The checked-in Coldcard fixture preserves the historical six-link source
hypothesis:

1. wallet creation consumes `ngu.random.bytes()`;
2. the board defines `MICROPY_HW_ENABLE_RNG` as zero;
3. libNgU tests macro definedness and calls `rng_get()`;
4. the vulnerable board source does not export that provider;
5. MicroPython compiles its Yasmarang fallback when the macro value is zero;
   and
6. the fallback can reach the wallet-seed sink at source tier.

The fixture also requires the missing final linked-artifact evidence and next
check. It does not claim which provider shipped in a firmware artifact.

## Verification {#verification}

```sh
cargo test -p omega_forensics
cargo test -p agent_ui forensics_workbench --features test-support
./script/clippy -p omega_forensics -p agent_ui
```
