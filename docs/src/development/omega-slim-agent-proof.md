# Omega slim-agent proof

This protocol records the evidence for Omega Agent ProductSpec revision 2,
acceptance criteria OMEGA-AGENT-AC-15 through OMEGA-AGENT-AC-21.

The protocol validates supplied observations. It does not create those
observations. A complete proof needs one source commit for the installed
journeys, the profile comparison, and the delta sweep.

> **Warn:** Fixture evidence is not packaged evidence. A fixture pass or a
> development build does not prove a signed package, authorize a release, or
> support a public reliability claim.

## Check the proof harness {#check-the-proof-harness}

Run the deterministic validator self-test and source sweep:

```sh
script/prove-omega-slim-agent --harness-check
```

The command checks its positive fixtures and negative controls. It also
requires OMEGA-DELTA-0133 through OMEGA-DELTA-0138 in the ledger and mechanical
registry.

## Record the out-of-box journey {#record-the-out-of-box-journey}

Install the exact candidate in a clean profile. Do not install an external
executor. Complete a coding task with the basic profile and the direct
`google/gemini-3.6-flash` provider.

The `openagents.omega.slim-agent-installed-journey.v1` observation records:

- The full Omega source commit and candidate digest.
- An empty external-executor inventory.
- The `basic` profile and exactly the six coding tools (`bash`, `delegate`,
  `edit`, `read`, `resume_thread`, and `write`) plus the four built-in market
  tools (`market_execute_swap`, `market_network_status`, `market_swap_quote`,
  and `market_swap_status`) in the model request.
- The direct Google provider and `gemini-3.6-flash` model.
- Different content digests before and after the coding change.
- The verification command and its zero exit code.
- A completed `end_turn` result and a content-addressed transcript.

The model can use any subset of the ten tools. It cannot use another tool.
The transcript reference must be a safe relative path and a SHA-256 digest.

## Record the harness journey {#record-the-harness-journey}

Install one executor and request it by exact name through `delegate`. Record a
completed result with its final message, readable session address, and
disclosure. The disclosure must name `omega-agent`, the requested executor,
the provider, and the model.

Remove that executor. Repeat the same request. Record the typed failed result
with `failure_class` equal to `no_executor`.

The two observations belong in one
`openagents.omega.slim-agent-harness-journey.v1` document. The validator rejects
executor substitution and an omitted disclosure chain.

## Compare basic and wide profiles {#compare-basic-and-wide-profiles}

Use one explicit task file, source commit, build, and model for both runs:

```sh
crates/eval_cli/script/zed-eval run rf \
  --from local \
  --tasks target/omega-slim-agent-proof/tasks.txt \
  --model google/gemini-3.6-flash \
  --agent-profile basic

crates/eval_cli/script/zed-eval run rf \
  --from local \
  --tasks target/omega-slim-agent-proof/tasks.txt \
  --model google/gemini-3.6-flash \
  --agent-profile wide
```

Record both run references and the exact ordered task IDs in
`openagents.omega.slim-agent-eval-comparison.v1`. If credentials, capacity, or
another prerequisite prevents the runs, record both profiles as `skipped` with
a reason. A skipped comparison is a gap and keeps the proof incomplete.

`eval-cli --profile basic` uses the closed ten-tool profile.
`eval-cli --profile wide` uses the inherited write profile. The basic mode
rejects `ZED_EVAL_DISABLE_TOOLS` so the comparison cannot remove a tool from
the admitted ten-tool surface.

## Validate the integrated record {#validate-the-integrated-record}

Run:

```sh
script/prove-omega-slim-agent \
  --installed-journey target/omega-slim-agent-proof/installed-journey.json \
  --harness-journey target/omega-slim-agent-proof/harness-journey.json \
  --eval-comparison target/omega-slim-agent-proof/eval-comparison.json
```

The command writes `target/omega-slim-agent-proof/proof.json`. It exits nonzero
and writes `status: incomplete` when evidence is missing, invalid, skipped,
bound to different source commits, not bound to the checkout `HEAD`, or
validated from a checkout with uncommitted tracked changes.

The proof reaches `passed` only when:

1. The installed no-harness coding journey passes.
2. The installed-then-removed harness journey passes.
3. Both profiles complete the same eval task IDs.
4. The six slim deltas are present in the ledger and mechanical registry.

The generated record retains the packaged-evidence authority boundary even
when all four checks pass.
