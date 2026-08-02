# Omega entropy repository runs

Omega can run a read-only entropy analysis over the repository selected in the
Forensics workbench. This is a repository-local path. It does not use the
OpenAgents Cloud worker lifecycle and does not grant reporting, disclosure, or
source-write authority.

Edit the visible **Entropy prompt**, then choose **Run entropy scan** in
Forensics. The built-in prompt asks for source-grounded entropy mechanisms,
secret consumers, causal links, missing evidence, and the next falsifiable
check. **Reset prompt** restores that default. **Use prior prompt** copies the
latest run's text into a new editable draft with explicit parent-prompt and
source-run lineage.

Omega requires a clean Git worktree, an
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

Starting a run creates an immutable prompt snapshot before repository
traversal. The snapshot contains the exact text, SHA-256 digest, creation time,
and optional copied-from lineage. Editing, resetting, or copying the draft
afterward cannot change an active or completed run.

The run binds that snapshot and digest, manifest and digest, repository
revision, configured model route, model parameters, read-only project tool
surface, and start time. Prompt text cannot expand the host-selected tools,
repository boundary, traversal limits, or output schema.
Omega reads each file again immediately before its model request and rejects
the read if its digest changed after the manifest was frozen.

Omega restores the account-scoped draft, prompt lineage, and a bounded recent
run history after restart. It keeps at most 64 prompt snapshots and 16 runs per
repository worktree binding. A run interrupted by restart is restored as
cancelled instead of silently resuming against changed process state.

## Comet Forensics workbench {#comet-forensics-workbench}

In Comet mode, choose **Forensics** in the left sidebar. The Forensics surface
uses the existing main workbench region; it does not add another navigation
rail. Choose the active session row or choose **Forensics** again to return to
the transcript.

The live entropy traversal keeps manifest order while file states change. It
shows at most 500 rows at once so large repositories cannot create unbounded UI
work. Filters select all files, candidates, failures, or incomplete work. A
selected row opens a detail pane with observations, hypotheses, exact source
links, mechanisms, confidence boundaries, missing evidence, and the next
check. Exact source links open the pinned file and line through the existing
Forensics source resolver.

The summary keeps completed-file, candidate, limitation, elapsed-time, and
usage-exactness facts separate. If the configured model route does not report
exact usage, the UI says that usage is unavailable. It does not show zero.
After a rerun, the same surface compares prompt A with prompt B and counts
gained, lost, changed, and unchanged file candidates. Cancellation preserves
completed rows and changes queued or reading rows to cancelled.

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
