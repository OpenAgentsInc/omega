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

For a single-repository run, Omega requires an exact 40-character revision and
a configured or available default model. For a clean worktree, it reads the
existing project path. If the selected worktree has changes, Omega creates a
temporary local clone and scans the exact selected HEAD instead of silently
including uncommitted source. The temporary clone is deleted with the
Forensics surface. Neither path creates a publication destination. The
multi-project campaign uses the separate isolated-checkout behavior described
below.

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

## Fifteen-product campaign {#fifteen-product-campaign}

Choose **Run 15-project campaign** to apply one frozen entropy prompt to the
versioned wallet-source catalog. The catalog keeps these product rows in this
order: Coldcard MK4/Q1, Trezor Model One/Model T, SeedSigner, Sparrow, Trezor
Safe 3/5/7, BitBox02, Opendime, Bitkey, BlueWallet, Phoenix, Blockstream Jade,
Ledger, SpecterDIY, Electrum for Android, and Samourai Wallet. Each eligible
row binds a public repository to an exact commit. A row also records its
license or access status and recursive dependency policy.

The campaign freezes one catalog digest, prompt snapshot and digest, model
route and parameters, read-only tool surface, and file-selection policy. It
materializes one isolated temporary checkout at a time, verifies the exact
commit, initializes recursive submodules, and gives that checkout its own
manifest and run. The next repository does not start until the current one is
terminal. **Pause after repository** stops at that boundary. **Resume** starts
the next queued repository. **Cancel campaign** retains completed and partial
rows and cancels the active local entropy run.

All 15 rows remain visible while the campaign runs. A row shows source state,
progress, files analyzed, candidate count, limitations, elapsed time, usage
exactness, and the frozen prompt digest. Select a row to use the existing file
traversal and candidate detail. Source citations resolve inside that product's
pinned temporary checkout and recheck its Git revision before opening the
file. The dashboard uses status colors for execution state only; it does not
assign red, yellow, or green product-security grades.

The built-in catalog marks the Ledger row `input_incomplete` because the
public SDK is not complete product source. The contract also supports
`source_unavailable` catalog rows. Neither state can become a clean analysis.
Repository materialization failure, incomplete dependencies, provider failure,
invalid model output, and cancellation remain separate limitations.

Up to four recent campaign projections are stored with the prompt workspace.
If Omega restarts during a campaign, it restores the partial results as a
cancelled campaign instead of silently resuming with new process state.

After prompt B completes, select the same product to compare it with the most
recent prompt A campaign. The comparison preserves both campaign IDs, prompt
digests, source revisions, and project run IDs, then reports gained, lost,
changed, and unchanged candidate references. It does not aggregate candidates
into a cross-product security score.

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
